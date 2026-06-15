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
Codec-specific chunk bodies not yet covered (`data` / `iXML` / `smpl` /
`inst` / `axml` / `chna` / `ds64` RF64 / `id3 `) and the `RF64` / `BW64`
64-bit-extended outer wrappers (EBU Tech 3306) are deferred to later
work.

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
  `wDialect` numeric-code tables are recorded as raw `u16` values; a
  typed lookup for them is deferred.

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
  canonical original RIFF + WAV + AVI spec (1991).
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
- `docs/container/riff/metadata/README.md` — staged catalogue of the
  WAV metadata-bearing chunks for later work.

## License

MIT — see [LICENSE](LICENSE).
