# oxideav-riff

Pure-Rust, clean-room implementation of the **RIFF** (Resource Interchange
File Format) chunk-walking primitives, per the publicly-published
*Multimedia Programming Interface and Data Specifications 1.0* that IBM
and Microsoft released in August 1991 and re-affirmed in the modern
Microsoft Learn *Resource Interchange File Format (RIFF)* page.

## Status — round 301

This crate ships the **shared chunk-walking primitives** that every
RIFF-family parser needs: a `ChunkHeader` decoder, a non-recursive
[`Walker`] over a parent chunk's children, FourCC helpers, and the
crate's own `Error` / `Result` aliases (with a default-on `registry`
feature that re-exports `oxideav-core`'s framework error type so the
walker plugs into the broader OxideAV pipeline without conversion
boilerplate).

Round 267 added the first typed chunk-body decoder: the WAV `fmt `
descriptor via [`WaveFormat`], covering the `WAVEFORMAT` (16-byte) /
`WAVEFORMATEX` (18-byte + `cbSize` extension) / `WAVEFORMATEXTENSIBLE`
(40-byte) forms, the `Samples` union + `dwChannelMask` +
`SubFormat` GUID sub-fields, and the `DEFINE_WAVEFORMATEX_GUID`
sub-format → legacy-`wFormatTag` resolver.

Round 275 adds the **`LIST INFO` metadata decoder** ([`InfoList`] /
[`InfoTag`]): the 23 baseline `INFO` sub-IDs the 1991 RIFF MCI spec
registers (`IARL` / `IART` / `ICMS` / `ICMT` / `ICOP` / `ICRD` /
`ICRP` / `IDIM` / `IDPI` / `IENG` / `IGNR` / `IKEY` / `ILGT` / `IMED`
/ `INAM` / `IPLT` / `IPRD` / `ISBJ` / `ISFT` / `ISHP` / `ISRC` /
`ISRF` / `ITCH`), each carrying its spec field name via
`InfoTag::label`, plus a ZSTR body decoder (`zstr_bytes` /
`zstr_value`) and `InfoList::collect_from` which walks a `LIST INFO`
sub-tree into an ordered `(tag, value)` list (duplicates and unknown
vendor codes preserved, per the spec's "ignore but don't reject"
rule).

Round 289 adds the **BWF `bext` broadcast-extension decoder**
([`BroadcastExtension`]): the 602-byte fixed prefix (Description /
Originator / OriginatorReference / OriginationDate / OriginationTime /
the 64-bit TimeReference / Version / 64-byte SMPTE 330M UMID / the five
loudness measurements) plus the trailing variable-length CodingHistory,
per EBU Tech 3285 v2 — with the spec's §1.1 version gating (UMID exposed
only for Version >= 1, [`Loudness`] only for Version >= 2).

Round 295 adds the **named `KSDATAFORMAT_SUBTYPE_*` GUID catalogue**
([`KsSubtype`]): a classifier that takes a decoded `SubFormat`
[`Guid`] and identifies its family — the `WAVEFORMATEX`-derived subtypes
(`…_PCM` / `…_IEEE_FLOAT` / `…_ALAW` / `…_DOLBY_AC3_SPDIF` / …) and the
Windows-7+ IEC 61937 compressed-passthrough subtypes (`…_IEC61937_MPEG1`
/ `…_IEC61937_DOLBY_DIGITAL_PLUS` / `…_IEC61937_DTS_HD` / …),
discriminated by the `0x0cea` `Data2` marker — returning the symbolic
name and a codec description.

Round 301 adds the **`cue ` cue-points decoder** ([`CueChunk`] /
[`CuePoint`]): the `dwCuePoints` count prefix plus the array of 24-byte
`<cue-point>` records (`dwName` / `dwPosition` / `fccChunk` /
`dwChunkStart` / `dwBlockStart` / `dwSampleOffset`), with the body-length
↔ count cross-check that rejects a truncated or over-long chunk.

Remaining codec-specific chunk bodies (`data` / `iXML` / `plst` /
`LIST adtl` / `smpl` / `inst` / `axml` / `chna` / `ds64` RF64 /
`id3 `) are deferred to subsequent rounds and stack on top of the walker.

## What the walker covers (round 257)

The wire-format invariants enforced:

- **8-byte chunk header decode** — 4-byte ASCII FourCC + 4-byte
  little-endian `ckSize`, per the 1991 spec §1.3. `ckSize` is the
  payload length and does **not** include the header or the pad
  byte.
- **Pad-byte tracking** — `Walker::skip()` and `Walker::read_body()`
  consume the trailing 0x00 pad byte after any odd-length body, so
  the next sibling header starts at a 2-byte boundary as required.
- **Parent-budget enforcement** — every child header is checked
  against the remaining `ckSize` budget of its parent group chunk;
  a child whose body would overflow the parent is rejected with
  `Error::invalid("RIFF: chunk overflows parent")`, not silently
  truncated. A clean EOF before the parent budget is satisfied
  surfaces as `Error::invalid("RIFF: truncated parent — …")`.
- **Group descent** — `Walker::read_inner_form_type()` reads the
  4-byte form-type / list-type tag of a `RIFF` or `LIST` child and
  charges those 4 bytes against the parent walker's budget so the
  caller can construct a nested walker over the remaining
  `size - 4` bytes.
- **FourCC rendering** — `fourcc_to_string()` escapes non-printable
  bytes as `\xNN` so debug dumps of malformed files stay readable;
  `is_printable_fourcc()` is a cheap up-front gate against obvious
  garbage at the file head.

What the walker explicitly does **not** cover yet:

- `RF64` / `BW64` 64-bit-extended outer wrappers (EBU Tech 3306
  §4) — the `ds64` side-table needs reading before the outer
  `ckSize` field becomes trustworthy. A separate `walk_rf64`
  constructor will land in a later round.
- Any other specific chunk body — the walker stays codec-agnostic;
  only the `fmt ` body has a typed decoder so far (see below).

## The `fmt ` chunk decoder (round 267)

[`WaveFormat::parse`] takes a `fmt ` chunk body (pulled from the
walker via `Walker::read_body`) and returns a typed descriptor:

- **Base `WAVEFORMAT` prefix** — `format_tag` / `channels` /
  `sample_rate` / `avg_bytes_per_sec` / `block_align` /
  `bits_per_sample`, all little-endian per the 1991 spec §2.
- **`WAVEFORMATEX` extension** — the optional 2-byte `cbSize` at
  +16 and its `cbSize`-counted trailing bytes, exposed raw as
  `extension` (over-running `cbSize` is rejected, not truncated).
- **`WAVEFORMATEXTENSIBLE` tail** — when `format_tag == 0xFFFE`,
  the `Samples` union (`wValidBitsPerSample` / `wSamplesPerBlock`),
  the `dwChannelMask` speaker bitmap, and the 16-byte `SubFormat`
  GUID are parsed into `ExtensibleFields`. A `0xFFFE` tag with
  fewer than 22 extension bytes is rejected.
- **`SubFormat` resolver** — `Guid::from_le_wire` decodes the
  Microsoft mixed-endian GUID (LE `Data1`/`Data2`/`Data3`, BE
  `Data4`); `Guid::waveformatex_tag` recovers the legacy 16-bit
  `wFormatTag` from a `DEFINE_WAVEFORMATEX_GUID`-template subtype
  (so an extensible PCM descriptor resolves back to `0x0001`), and
  returns `None` for non-template GUIDs (Dolby AC-3, DTS, …).
  `WaveFormat::effective_format_tag` folds that together.
- **`wFormatTag` constants** — `WAVE_FORMAT_PCM` / `_ADPCM` /
  `_IEEE_FLOAT` / `_ALAW` / `_MULAW` / `_EXTENSIBLE`.

The full named `KSDATAFORMAT_SUBTYPE_*` GUID catalogue (the
symbolic-name ↔ codec table beyond the `DEFINE_WAVEFORMATEX_GUID`
template) lands in round 295 (see below).

## The `KSDATAFORMAT_SUBTYPE_*` GUID catalogue (round 295)

[`KsSubtype::resolve`] takes the `SubFormat` [`Guid`] of a
`WAVEFORMATEXTENSIBLE` descriptor and classifies it:

- **`WaveFormatEx { tag }`** — the GUID matches the
  `…-0000-0010-8000-00aa00389b71` base template (`Data2 == 0x0000`); the
  `Data1` low word is the legacy `WAVE_FORMAT_*` tag. The catalogued
  symbolic names cover `…_WAVEFORMATEX` / `…_PCM` / `…_ADPCM` /
  `…_IEEE_FLOAT` / `…_ALAW` / `…_MULAW` / `…_DTS` / `…_DRM` / `…_MPEG` /
  `…_DOLBY_AC3_SPDIF` (the worked example from *Converting Between Format
  Tags and Subformat GUIDs*).
- **`Iec61937 { cea861_type }`** — the GUID carries the `0x0cea` `Data2`
  discriminator (the Windows-7+ S/PDIF / HDMI compressed-passthrough
  family); the `Data1` low word is then a CEA-861 *stream-type* index
  (not a `wFormatTag`). Catalogued: `…_IEC61937_MPEG1` / `…_MPEG2` /
  `…_MPEG3` / `…_AAC` / `…_ATRAC` / `…_ONE_BIT_AUDIO` /
  `…_DOLBY_DIGITAL_PLUS` / `…_DTS_HD` / `…_DOLBY_MLP` / `…_DST`.
- **`Other(Guid)`** — neither template matches (a vendor/proprietary
  root GUID); the raw value is preserved for full-128-bit matching by
  the caller.

`KsSubtype::symbolic_name()` returns the `KSDATAFORMAT_SUBTYPE_*`
constant name and `description()` a short codec string (both `None` for
uncatalogued tags / indices / vendor GUIDs). The `waveformatex_guid` /
`iec61937_guid` builders reconstruct a template GUID from a tag /
stream-type index, and `waveformatex_name` / `iec61937_name` expose the
lookup tables directly. The MAT 2.0 Atmos variants and the
Media-Foundation `MFAudioFormat_*` parallel namespace stay deferred.

## The `LIST INFO` metadata decoder (round 275)

A `RIFF`/`WAVE` (or AVI / WebP) file may carry a `LIST` chunk whose
list-type FourCC is `INFO` — the registered global identification-
metadata namespace from the 1991 RIFF MCI spec §2. Each child chunk's
body is a **ZSTR** (NULL-terminated ASCII text).

- **[`InfoTag`]** — the 23 baseline four-character codes the spec
  registers, exposed as associated constants (`InfoTag::INAM`, …) with
  the spec's field name reachable via `InfoTag::label()`
  (`"Name"`, `"Copyright"`, …) and `InfoTag::is_baseline()` to test
  membership. `InfoTag::BASELINE` is the full ordered table. Unknown /
  vendor codes (`IMP3`, `ITRK`, …) round-trip verbatim — the spec
  says to ignore, not reject, unrecognised IDs.
- **ZSTR body decode** — `zstr_bytes()` returns the bytes up to the
  first `0x00`; `zstr_value()` lossily decodes them to a `String`.
  A body that relies only on the RIFF pad byte (no embedded `NUL`)
  yields the whole body.
- **[`InfoList`]** — an ordered `(InfoTag, String)` collection.
  `collect_from(&mut Walker)` drives a sub-walker already positioned
  over a `LIST INFO` body (built after the caller reads the `INFO`
  list-type with `Walker::read_inner_form_type`) and gathers every tag
  in on-wire order; `get(tag)` returns the first value, `entries()`
  exposes all (duplicates preserved).

The common vendor / iTunes-era extensions (`ITRK`, `ILNG`, `IMP3`,
`IDIT`, …) catalogued by ExifTool and the `LIST adtl` associated-data
sub-chunks (`labl` / `note` / `ltxt` / `file`) stay deferred to a
later round; they stack on `InfoList` and the walker.

## The `bext` Broadcast Audio Extension decoder (round 289)

A *Broadcast Wave Format* (BWF) file is a RIFF/WAVE file with one extra
chunk, FourCC `bext`, carrying production metadata. [`BroadcastExtension::parse`]
takes a `bext` chunk body (pulled from the walker via
`Walker::read_body`) and returns a typed descriptor, per EBU Tech 3285 v2:

- **602-byte fixed prefix** — `Description[256]` / `Originator[32]` /
  `OriginatorReference[32]` (NUL-padded ASCII, exposed both as the raw
  byte arrays and as trimmed-at-NUL `String` accessors),
  `OriginationDate[10]` (`"yyyy-mm-dd"`) / `OriginationTime[8]`
  (`"hh-mm-ss"`), the 64-bit `TimeReference` reassembled from its
  low/high words, the `Version` word, the 64-byte `UMID`, and the five
  16-bit-signed loudness fields.
- **Version gating (§1.1)** — `umid()` returns the UMID only when
  `version >= 1`; `loudness()` returns the [`Loudness`] measurements
  only when `version >= 2`, mirroring the spec's forwards/backwards-
  compatibility rule (older readers ignore the bytes newer versions
  reuse). The unconditional raw bytes stay reachable on the public
  fields.
- **`Loudness`** — `value`/`range`/`max_true_peak`/`max_momentary`/
  `max_short_term`, each a `round(100 × …)` integer, with `_x100` raw
  accessors and `_lufs` / `_lu` / `_dbtp` natural-unit accessors.
- **`CodingHistory`** — the trailing variable-length field (chunk size
  − 602), the collection of CR/LF-separated coding-process descriptions,
  decoded with trailing NUL padding stripped.

The `iXML` companion metadata block, the `qlty` / `mext` BWF
supplements, and the `axml` / `chna` ADM chunks stay deferred to later
rounds.

## The `cue ` cue-points chunk decoder (round 301)

A `cue ` chunk (note the trailing space in the FourCC) marks a series of
positions in the sample stream — seek markers a player can jump to, and
the anchors the `plst` playlist and `LIST adtl` associated-data chunks
reference. [`CueChunk::parse`] takes a `cue ` chunk body (pulled from the
walker via `Walker::read_body`) and returns a typed table:

- **Count + record array** — a `dwCuePoints` `u32` count followed by that
  many 24-byte `<cue-point>` records. The body length must equal
  `4 + dwCuePoints × 24` exactly; a body that is shorter than the count
  word, or whose length disagrees with the declared count, is rejected
  with `Error::invalid` rather than yielding a partially-populated table.
- **[`CuePoint`]** — the six little-endian fields: `name` (`dwName`, the
  unique identifier other chunks reference), `position` (`dwPosition`,
  the sequential play-order sample number), `fcc_chunk` (`fccChunk`, the
  raw FourCC of the containing chunk — `data` or `slnt`), `chunk_start`,
  `block_start`, and `sample_offset`. `is_data()` / `is_silent()` test
  the FourCC; a non-`data`/`slnt` value round-trips verbatim.
- **Offset interpretation deferred to the caller** — `dwChunkStart` /
  `dwBlockStart` / `dwSampleOffset` mean different things depending on
  whether the file wraps its samples in a `wavl` LIST or carries a single
  `data` chunk, and whether the data is PCM or compressed. The spec's
  worked cases are:

  | Layout                       | `chunk_start` | `block_start`                 | `sample_offset`              |
  | ---------------------------- | ------------- | ----------------------------- | ---------------------------- |
  | single PCM `data`            | 0             | 0                             | sample pos within `data`     |
  | single compressed `data`     | 0             | block pos within `data`       | sample pos within the block  |
  | `wavl` PCM `data`            | `data` pos in `wavl`     | cue pos in `wavl` data | 0               |
  | `wavl` `slnt`               | `slnt` pos in `wavl`     | `slnt` data pos in `wavl` | sample pos in `slnt` |

  The decoder records the raw values and does not resolve them, since it
  has no view of the surrounding chunk tree.

- **Lookups** — `points()` exposes the records in on-wire order;
  `by_name(name)` returns the first cue point with a matching `dwName`
  (the spec requires `dwName` to be unique, so this is effectively keyed
  access); `len()` / `is_empty()` round out the API. `FOURCC_CUE` and
  `CUE_POINT_LEN` are exposed as constants.

The companion `plst` playlist chunk (which orders cue IDs into a play
sequence) and the `LIST adtl` associated-data sub-chunks (`labl` /
`note` / `ltxt` / `file`, which attach text and segments to cue IDs)
stay deferred to later rounds; they stack on `CueChunk` and the walker.

## Standalone build

`oxideav-core` is gated behind the default-on `registry` feature.
Drop the framework dependency entirely with:

```toml
oxideav-riff = { version = "0.0", default-features = false }
```

Without `registry`, the crate exposes its own [`Error`] / [`Result`]
aliases (defined in `error.rs`) so it can be used as a pure parsing
library by callers that don't want the OxideAV dependency tree.

## Quick start

```rust
use std::io::Cursor;
use oxideav_riff::{Walker, fourcc_to_string};

// Minimal RIFF/WAVE skeleton.
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

- `docs/container/riff/metadata/microsoft-riffmci.pdf` §1-2 — IBM
  + Microsoft, *Multimedia Programming Interface and Data
  Specifications 1.0*, August 1991. The canonical original RIFF +
  WAV + AVI spec.
- `docs/container/riff/metadata/ms-xaudio2-riff.html` — Microsoft
  Learn, modern reformulation of the RIFF wire layout for the
  Win32 XAudio2 reference.
- `docs/container/riff/avi-riff-file-reference.md` — DirectShow
  AVI RIFF File Reference; useful cross-check that the FourCC +
  size encoding matches across forms.
- `docs/container/riff/rfc2361-wav.txt` — RFC 2361, the
  `wFormatTag` codec-format-ID registry consumed by the round-267
  `fmt ` decoder.
- `docs/container/riff/waveformatextensible/` — Microsoft Learn
  *WAVEFORMATEXTENSIBLE structure*, *Extensible Wave-Format
  Descriptors*, and *Converting Between Format Tags and Subformat
  GUIDs* — the source for the round-267 `WAVEFORMATEX(TENSIBLE)`
  field layout + `DEFINE_WAVEFORMATEX_GUID` sub-format resolver.
  The consolidated `ksdataformat-subtype-guids.md` named-GUID
  catalogue, `ms-subformat-guids-compressed-audio.md` (the CEA-861
  IEC 61937 stream-type table), and
  `ms-converting-format-tags-and-subformat-guids.md` (the
  `DEFINE_WAVEFORMATEX_GUID` macro + the `…_DOLBY_AC3_SPDIF` worked
  example) are the source for the round-295 `KsSubtype` catalogue.
- `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
  "INFO List Chunk" (the registered global `INFO` form-type + the
  23-entry baseline tag table) and "NULL-Terminated String (ZSTR)
  Format" — the source for the round-275 `LIST INFO` decoder.
- `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
  "Cue-Points Chunk" (the `<cue-ck>` / `<cue-point>` grammar, the
  per-field descriptions, and the file-position worked examples) — the
  source for the round-301 `cue ` decoder.
- `docs/container/riff/metadata/ebu-tech3285-bwf.pdf` — EBU Tech 3285
  v2, *Specification of the Broadcast Wave Format (BWF)*: the
  `broadcast_audio_extension` struct, the per-field descriptions, and
  §1.1 "Version compatibility" — the source for the round-289 `bext`
  decoder.
- `docs/container/riff/metadata/README.md` — staged catalogue of
  the WAV metadata-bearing chunks (`LIST INFO`, `bext`, `iXML`,
  `cue ` / `plst` / `LIST adtl`, `smpl` / `inst`, `axml` /
  `chna`, `ds64`) for later rounds.

## License

MIT — see [LICENSE](LICENSE).
