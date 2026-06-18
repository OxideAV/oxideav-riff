# oxideav-riff

Pure-Rust, clean-room implementation of the **RIFF** (Resource
Interchange File Format) chunk-walking primitives plus typed decoders
for the common WAV/BWF metadata chunks, per the publicly-published
*Multimedia Programming Interface and Data Specifications 1.0* (IBM +
Microsoft, August 1991).

## What this crate provides

The **shared chunk-walking primitives** that every RIFF-family parser
needs — a `ChunkHeader` decoder, a non-recursive [`Walker`] over a
parent chunk's children, FourCC helpers, and the crate's own `Error` /
`Result` aliases — plus a growing set of typed chunk-body decoders.

The walker is codec-agnostic; the typed decoders stack on top of it.
Codec-specific chunk bodies not yet covered (`data` / `iXML` / `axml` /
`id3 `) are deferred to later work; the `RF64` / `BW64` 64-bit-extended
outer wrappers are now handled via the `ds64` decoder (EBU Tech 3306),
the ADM `chna` channel-allocation table via the `chna` decoder (ITU-R
BS.2088), the Sonic Foundry / Sony ACID `acid` loop-metadata block via
the `acid` decoder, and the sampler-instrument pair (`smpl` / `inst`) via
the `smpl` / `inst` decoders.

## The walker

The wire-format invariants enforced:

- **8-byte chunk header decode** — 4-byte ASCII FourCC + 4-byte
  little-endian `ckSize` (the payload length, not including the header
  or the pad byte), per §1.3.
- **Pad-byte tracking** — `Walker::skip()` and `Walker::read_body()`
  consume the trailing `0x00` pad byte after any odd-length body, so
  the next sibling header starts on a 2-byte boundary.
- **Parent-budget enforcement** — every child header is checked against
  the remaining `ckSize` budget of its parent group chunk; a child
  whose body would overflow the parent is rejected with
  `Error::invalid`, not silently truncated. A clean EOF before the
  parent budget is satisfied surfaces as a truncated-parent error.
- **Group descent** — `Walker::read_inner_form_type()` reads the
  4-byte form-type / list-type tag of a `RIFF` or `LIST` child and
  charges those 4 bytes against the parent budget so the caller can
  construct a nested walker over the remaining `size - 4` bytes.
- **FourCC rendering** — `fourcc_to_string()` escapes non-printable
  bytes as `\xNN` so debug dumps of malformed files stay readable;
  `is_printable_fourcc()` is a cheap up-front garbage gate.

## Typed chunk-body decoders

- **`fmt ` WAV format descriptor** ([`WaveFormat`]) — covers the
  `WAVEFORMAT` (16-byte) / `WAVEFORMATEX` (18-byte + `cbSize`
  extension) / `WAVEFORMATEXTENSIBLE` (40-byte) forms, the `Samples`
  union, `dwChannelMask`, the 16-bit mixed-endian `SubFormat` GUID, and
  the `DEFINE_WAVEFORMATEX_GUID` sub-format → legacy `wFormatTag`
  resolver. Over-running `cbSize` is rejected, not truncated.
- **`KSDATAFORMAT_SUBTYPE_*` GUID catalogue** ([`KsSubtype`]) — a
  classifier that takes a decoded `SubFormat` GUID and identifies its
  family: the `WAVEFORMATEX`-derived subtypes (`…_PCM` /
  `…_IEEE_FLOAT` / `…_ALAW` / `…_DOLBY_AC3_SPDIF` / …) and the
  IEC 61937 compressed-passthrough subtypes (discriminated by the
  `0x0cea` `Data2` marker), returning the symbolic name and a codec
  description. The MAT 2.0 Atmos and Media-Foundation `MFAudioFormat_*`
  namespaces are deferred.
- **`LIST INFO` metadata** ([`InfoList`] / [`InfoTag`]) — the 23
  baseline `INFO` sub-IDs the 1991 spec registers, each carrying its
  field name via `InfoTag::label`, plus a ZSTR body decoder
  (`zstr_bytes` / `zstr_value`) and `InfoList::collect_from` which
  walks a `LIST INFO` sub-tree into an ordered `(tag, value)` list
  (duplicates and unknown vendor codes preserved, per the spec's
  "ignore but don't reject" rule).
- **BWF `bext` broadcast extension** ([`BroadcastExtension`]) — the
  602-byte fixed prefix (Description / Originator / OriginatorReference
  / OriginationDate / OriginationTime / 64-bit TimeReference / Version /
  64-byte SMPTE 330M UMID / five loudness measurements) plus the
  trailing variable-length CodingHistory, per EBU Tech 3285 v2 — with
  the §1.1 version gating (UMID exposed only for Version ≥ 1,
  [`Loudness`] only for Version ≥ 2).
- **`cue ` cue points** ([`CueChunk`] / [`CuePoint`]) — the
  `dwCuePoints` count prefix plus the array of 24-byte `<cue-point>`
  records, with a body-length ↔ count cross-check that rejects a
  truncated or over-long chunk. Offset fields are recorded raw, since
  their interpretation depends on the surrounding chunk tree.
- **`plst` playlist** ([`Playlist`] / [`PlaySegment`]) — the
  `dwSegments` count prefix plus the array of 12-byte `<play-segment>`
  records (`dwName` / `dwLength` / `dwLoops`), ordering the cue points
  of a `cue ` chunk into a play sequence, with the same body-length ↔
  count cross-check.
- **`LIST adtl` associated data** ([`AdtlList`] / [`AdtlEntry`]) — the
  `labl` / `note` (cue-point label + comment ZSTRs), `ltxt`
  (length-bounded text segment) and `file` (embedded media) child
  chunks, collected in on-wire order with cue-point cross-reference
  lookups (`by_cue_name` / `label` / `note`) and verbatim preservation
  of unrecognised child FourCCs. The `ltxt` `wCountry` / `wLanguage` /
  `wDialect` numeric codes are kept as raw `u16` values and now also
  resolve to names via `LabeledText::country_name` / `language_name`
  (the §2 code tables — see the `CSET` decoder below).
- **RF64 / BW64 `ds64` data-size-64** ([`DataSize64`] / [`ChunkSize64`])
  — the MBWF / RF64 64-bit size extension that lifts the format's 4 GiB
  cap. The 28-byte fixed prefix carries the three mandatory 64-bit
  values (`riffSize` / `dataSize` / `sampleCount`) that replace the
  `0xFFFFFFFF`-sentinel 32-bit fields elsewhere in the file, followed by
  the optional `<chunkSize64>` table of per-chunk 64-bit size overrides
  (each a FourCC + 64-bit size), with a body-length ↔ count cross-check.
  `is_sentinel` / `is_rf64_magic` classify the `0xFFFFFFFF` deferral
  marker and the `RF64` / `BW64` outer FourCCs; `DataSize64::resolve`
  returns a chunk's real size given its 32-bit header value, consulting
  the table only when the value is the sentinel.
- **`fact` chunk** ([`Fact`]) — the mandatory `dwSampleLength`
  per-channel sample count (required for compressed and `wavl`-LIST
  waveforms, optional for plain PCM `data`), with the spec's
  forward-compatibility rule honoured: reserved trailing fields sized by
  the chunk header are kept verbatim in `extra` rather than rejected.
  `is_deferred` / `sample_length()` recognise the RF64 / BW64
  `0xFFFFFFFF` sentinel that hands the true 64-bit count to the `ds64`
  chunk's `sampleCount`, returning `None` so callers resolve it there.
- **BW64 `chna` ADM channel allocation** ([`ChannelAllocation`] /
  [`AudioId`]) — the *Audio Definition Model* track-to-identifier table
  that binds each physical `data`-chunk track to its `audioTrackUID`
  (`ATU_…`), `audioTrackFormatID` (`AT_…`, or an `audioChannelFormat`
  `AC_…` for bare linear PCM) and optional `audioPackFormatID` (`AP_…`).
  The 4-byte `numTracks` / `numUIDs` preamble precedes an array of fixed
  40-byte `audioID` records (`trackIndex` u16 LE + `UID[12]` +
  `trackRef[14]` + `packRef[11]` + `pad`), with the record count derived
  from the body length (`N = (ckSize − 4) / 40`) and a multiple-of-40
  cross-check. The `N ≥ numUIDs` over-provisioning convention is honoured:
  spare zeroed records (`trackIndex == 0`) are retained but skipped by
  `active()` / `by_track_index()`, and `uid_count_consistent()` reports
  whether the used-record count matches the declared `numUIDs`. The
  fixed-width identifier fields are kept as raw byte arrays (non-NUL-
  terminated ASCII per spec) with `uid_str` / `track_ref_str` /
  `pack_ref_str` trimmed-UTF-8 accessors. The ADM XML resolution of the
  ID references (into `axml` / `bxml` / `sxml`, or BS.2094 common
  definitions) is the caller's concern, outside the binary chunk.
- **`acid` Acidizer loop metadata** ([`Acid`]) — the Sonic Foundry /
  Sony ACID loop-metadata block that "ACIDized" WAV loops carry so a host
  can pitch- and time-stretch the loop to the project tempo. The fixed
  24-byte body decodes the `flags` property bitfield (one-shot vs loop,
  root-note-set, stretch, disk-based, high-octave), the MIDI `rootNote`
  (gated by the root-note-set flag), `numBeats`, the meter
  denominator/numerator, and the IEEE-754 single-precision `tempo` in BPM.
  Because this is a fixed-width undocumented vendor record with no
  extension mechanism, an off-length body is rejected rather than treated
  as a future field; the two reverse-engineered `unknown` constants
  (`0x8000` and `0.0` in known files) are retained verbatim so an
  atypical writer's values round-trip. The layout is a community
  reverse-engineering, not vendor-authoritative — the meter
  numerator/denominator order and the bit-4 octave semantics are RE facts
  cross-checked across two independent public sources, not a published
  spec.
- **`CSET` character set + country / language / dialect codes**
  ([`CharacterSet`] / [`country_name`] / [`language_name`]) — the
  file-wide `CSET` chunk (`wCodePage` / `wCountryCode` / `wLanguageCode` /
  `wDialect`, the 8-byte body), plus the two §2 numeric-code tables it
  registers: the 29-entry **Country Codes** table (`001` USA … `358`
  Finland) and the 44-entry **Language and Dialect Codes** table, the
  latter keyed on the (`wLanguage`, `wDialect`) *pair* so a single
  language code resolves to its dialect-specific name (`12`/`1` French,
  `12`/`2` Belgian French, `12`/`3` Canadian French, `12`/`4` Swiss
  French; `4`/`1` Traditional vs `4`/`2` Simplified Chinese). The spec's
  zero-field defaulting is exposed via the `…_defaulted` variants and the
  `CharacterSet::country` / `language` accessors (country `0` → USA,
  language/dialect `(0, 0)` → US English). These resolve the
  `LIST adtl` `ltxt` record's `wCountry` / `wLanguage` / `wDialect` raw
  codes — previously recorded as opaque `u16` — via the new
  `LabeledText::country_name` / `language_name` accessors.
- **`inst` instrument** ([`Inst`]) — the fixed 7-byte MIDI playback-hint
  record a sampler uses to map a single WAV sample across a keyboard:
  `UnshiftedNote` (the key the sample plays unshifted), signed `FineTune`
  (cents) and `Gain` (dB) offsets, and the inclusive key
  (`LowNote`..=`HighNote`) and velocity (`LowVelocity`..=`HighVelocity`)
  zones the sample covers. The signed offsets keep their raw byte too;
  `covers_note` / `covers_velocity` test a MIDI note / velocity against
  the zones. An off-length body is rejected (fixed-width record).
- **`smpl` sampler** ([`Smpl`] / [`SampleLoop`]) — the richer sampler
  detail: the 36-byte fixed header (`Manufacturer` / `Product` /
  `SamplePeriod` / `MIDIUnityNote` + `MIDIPitchFraction` / `SMPTEFormat`
  + `SMPTEOffset` / `NumSampleLoops` / `SamplerDataLen`), the
  `NumSampleLoops` table of 24-byte `<sample-loop>` records, and the
  opaque `SamplerData` trailer. The body length must agree with the
  declared loop count and sampler-data length, with overflow-safe size
  arithmetic. The per-loop field breakdown is not in the in-tree
  clean-room material, so each loop record is preserved verbatim as a
  24-byte `SampleLoop`; `smpte_offset_parts` unpacks the packed
  `HH:MM:SS:FF` offset.

## Standalone build

`oxideav-core` is gated behind the default-on `registry` feature (which
re-exports core's framework error type so the walker plugs into the
broader pipeline without conversion boilerplate). Drop the framework
dependency entirely with:

```toml
oxideav-riff = { version = "0.0", default-features = false }
```

Without `registry`, the crate exposes its own [`Error`] / [`Result`]
aliases so it can be used as a pure parsing library.

## Quick start

```rust
use std::io::Cursor;
use oxideav_riff::{Walker, fourcc_to_string};

let bytes = std::fs::read("input.wav").unwrap();
let mut cur = Cursor::new(bytes);
let mut walker = Walker::open_root(&mut cur).unwrap();
assert_eq!(&walker.form_type(), b"WAVE");

while let Some(chunk) = walker.read_next().unwrap() {
    println!("chunk {} ({} bytes)", fourcc_to_string(&chunk.id), chunk.size);
    walker.skip(&chunk).unwrap();
}
```

## Clean-room references

- `docs/container/riff/metadata/microsoft-riffmci.pdf` §1–2 — the
  canonical original RIFF + WAV + AVI spec (1991); §2 also carries the
  `CSET` chunk grammar plus the *Country Codes* and *Language and Dialect
  Codes* tables consumed by the `cset` decoder.
- `docs/container/riff/metadata/ms-xaudio2-riff.html` — modern
  reformulation of the RIFF wire layout.
- `docs/container/riff/avi-riff-file-reference.md` — AVI RIFF File
  Reference cross-check.
- `docs/container/riff/rfc2361-wav.txt` — RFC 2361, the `wFormatTag`
  codec-format-ID registry consumed by the `fmt ` decoder.
- `docs/container/riff/waveformatextensible/` — the
  `WAVEFORMATEX(TENSIBLE)` field layout, the `DEFINE_WAVEFORMATEX_GUID`
  resolver, and the named-GUID + IEC 61937 catalogues for `KsSubtype`.
- `docs/container/riff/metadata/ebu-tech3285-bwf.pdf` — EBU Tech 3285
  v2, the source for the `bext` decoder.
- `docs/container/riff/metadata/ebu-tech3306-v1.pdf` §A.2 +
  `…/ebu-tech3306-v2.pdf` §1 — EBU Tech 3306 (MBWF / RF64), the source
  for the `ds64` data-size-64 decoder and the `RF64` / `BW64` outer
  wrappers.
- `docs/container/riff/metadata/R-REC-BS.2088.pdf` §8 +
  `…/bs2088-chna-chunk-layout.md` — ITU-R BS.2088-2 (the BW64 file
  format), the binary `chna` `struct chna_chunk` / `struct audioID`
  layout that BS.2076 (ADM) and EBU Tech 3285s5 reference but defer; the
  source for the `chna` channel-allocation decoder.
- `docs/container/riff/acid-chunk.md` — clean-room field spec for the
  Sonic Foundry / Sony ACID `acid` chunk (the 24-byte Acidizer record:
  per-offset field table, `flags` bit table, MIDI root-note mapping); the
  source for the `acid` decoder.
- `docs/container/riff/metadata/exiftool-riff-tags.html` — the *RIFF
  Instrument Tags* and *RIFF Sampler Tags* tables (the `'inst'` 7-field
  1-byte layout and the `'smpl'` nine 4-byte fields + `NumSampleLoops` +
  `SamplerDataLen` + `SamplerData`), plus the metadata README's
  `inst` 7-byte / `smpl` 36-byte + N × 24 size summary; the source for the
  `inst` / `smpl` decoders.
- `docs/container/riff/metadata/README.md` — staged catalogue of the
  WAV metadata-bearing chunks for later work.

## License

MIT — see [LICENSE](LICENSE).
