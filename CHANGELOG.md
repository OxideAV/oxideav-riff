# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Round 289 — BWF `bext` broadcast-extension decoder.** A typed
  reader for the Broadcast Audio Extension chunk, per EBU Tech 3285 v2
  (`broadcast_audio_extension` struct + per-field descriptions + §1.1
  "Version compatibility").

  - `bext::BroadcastExtension::parse(body)` decodes the 602-byte fixed
    prefix — `Description` / `Originator` / `OriginatorReference`
    (NUL-padded ASCII), `OriginationDate` (`"yyyy-mm-dd"`) /
    `OriginationTime` (`"hh-mm-ss"`), the 64-bit `TimeReference`
    reassembled from its low/high words, the `Version` word, the 64-byte
    SMPTE 330M `UMID`, and the five 16-bit-signed loudness fields — plus
    the trailing variable-length `CodingHistory` (chunk size − 602). A
    body shorter than the 602-byte prefix is rejected as truncated.
  - Version gating per §1.1: `umid()` returns the UMID only when
    `version >= 1`; `loudness()` returns the `bext::Loudness`
    measurements only when `version >= 2` (the bytes are reserved in
    earlier versions). Raw bytes (`umid_bytes`, the `*_x100` fields)
    stay reachable unconditionally.
  - `bext::Loudness` — `value` / `range` / `max_true_peak` /
    `max_momentary` / `max_short_term`, each a `round(100 × …)` integer,
    with `_x100` raw accessors and `_lufs` / `_lu` / `_dbtp`
    natural-unit accessors.
  - String accessors (`description()`, `originator()`, …) trim at the
    first NUL and lossily decode to `String`; `coding_history()`
    additionally strips trailing NUL padding.
  - 10 new unit tests covering the prefix-length invariant, short-body
    rejection, NUL-trimmed string fields, 64-bit TimeReference
    reassembly, UMID + loudness version gating with scaling, the
    CodingHistory trailing field + its NUL-padding trim, and lossy
    non-ASCII decode.
  - Re-exported at the crate root: `BroadcastExtension`, `Loudness`,
    `BEXT_PREFIX_LEN`, and the field-length constants.

- **Round 275 — `LIST INFO` metadata decoder.** A typed reader for
  the registered `INFO` identification-metadata namespace, per the
  1991 RIFF MCI §2 "INFO List Chunk" + "NULL-Terminated String
  (ZSTR) Format".

  - `info::InfoTag` — the 23 baseline `INFO` sub-IDs the spec
    registers, exposed as associated constants (`INAM`, `IART`,
    `ICOP`, …) with `InfoTag::label()` mapping each to its spec field
    name, `InfoTag::is_baseline()`, and the `InfoTag::BASELINE`
    ordered table. Unknown / vendor four-character codes are
    preserved verbatim (the spec instructs applications to ignore,
    not reject, unrecognised IDs).
  - `info::zstr_bytes` / `info::zstr_value` — ZSTR body decode: bytes
    up to the first `0x00`, with tolerance for bodies that rely only
    on the RIFF pad byte (no embedded terminator). `zstr_value`
    lossily decodes to `String`.
  - `info::InfoList` — an ordered `(InfoTag, String)` collection.
    `collect_from(&mut Walker)` walks a `LIST INFO` sub-tree (built
    after reading the `INFO` list-type with
    `Walker::read_inner_form_type`) into the list; `get(tag)` returns
    the first value, `entries()` exposes all (order + duplicates
    preserved). A non-`INFO` list-type is rejected.
  - 12 new unit tests covering the baseline table, label mapping,
    ZSTR edge cases (missing terminator, embedded NUL, invalid
    UTF-8), order/duplicate preservation, odd-length-body pad
    re-sync, unknown-tag retention, and the non-INFO rejection.
  - Re-exported at the crate root: `InfoList`, `InfoTag`,
    `zstr_bytes`, `zstr_value`.

- **Round 267 — `fmt ` chunk decoder.** First typed chunk-body
  primitive: `waveformat::WaveFormat::parse(body)` decodes a WAV
  `fmt ` chunk body (the bytes the walker yields) into a typed
  descriptor, per the 1991 RIFF MCI §2 base layout + the Microsoft
  Learn `WAVEFORMATEXTENSIBLE` references.

  - Base `WAVEFORMAT` prefix — `format_tag`, `channels`,
    `sample_rate`, `avg_bytes_per_sec`, `block_align`,
    `bits_per_sample` (all little-endian).
  - `WAVEFORMATEX` extension — the optional `cbSize` at +16 and its
    counted trailing bytes, exposed raw as `extension`; a `cbSize`
    that over-runs the body length is rejected.
  - `WAVEFORMATEXTENSIBLE` tail (`ExtensibleFields`) — parsed when
    `format_tag == WAVE_FORMAT_EXTENSIBLE (0xFFFE)`: the `Samples`
    union (`samples`), `dwChannelMask` (`channel_mask`), and the
    16-byte `SubFormat` GUID. A `0xFFFE` tag with fewer than 22
    extension bytes is rejected.
  - `Guid` — Microsoft mixed-endian GUID (`from_le_wire`,
    `to_hyphenated`) with `is_waveformatex_derived` /
    `waveformatex_tag` recovering the legacy 16-bit `wFormatTag`
    from a `DEFINE_WAVEFORMATEX_GUID`-template subtype, plus the
    `KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE` template constant.
  - `WaveFormat::is_extensible` / `effective_format_tag` /
    `channel_mask_count` convenience accessors.
  - `WAVE_FORMAT_PCM` / `_ADPCM` / `_IEEE_FLOAT` / `_ALAW` /
    `_MULAW` / `_EXTENSIBLE` `wFormatTag` constants.
  - 12 new unit tests covering the bare-`WAVEFORMAT`,
    `WAVEFORMATEX`-with-extension, extensible-PCM, non-template
    `SubFormat`, mixed-endian GUID decode, and the short-body /
    `cbSize`-overrun / truncated-extensible rejection paths.

- **Round 257 — bootstrap.** Initial release of the `oxideav-riff`
  crate: a shared, clean-room **RIFF chunk-walker** that every
  RIFF-family parser (WAV, AVI, WebP, AMV, ANI, …) can plug into.
  Implements the 1991 IBM + Microsoft *Multimedia Programming
  Interface and Data Specifications 1.0* §1.3 wire format:

  - `ChunkHeader { id: [u8; 4], size: u32 }` — 8-byte header decode
    via `read_chunk_header(r)`, returning `Ok(None)` at clean EOF
    and `Err(Error::invalid)` on a partial header (parent
    `ckSize` lied).
  - `Walker::open_root(r)` — opens the outer `RIFF` chunk at offset
    0, validates the FourCC + minimum `ckSize >= 4` (room for the
    form-type word), and positions just past the form type so the
    first `.read_next()` yields the first top-level child. Strict on
    `RIFF` — the `RF64` / `BW64` 64-bit-extended variants
    (EBU Tech 3306 §4) are deferred to a later round.
  - `Walker::open_within(r, header)` — wrap an already-located
    group chunk (`RIFF` or `LIST`) so the caller can descend into
    nested sub-trees without re-reading the outer header.
  - `Walker::read_next()` — yields the next `ChunkRef { id, size,
    body_offset }`. Enforces parent budget: a child whose body +
    pad would overflow the parent's `ckSize` is rejected with
    `Error::invalid("RIFF: chunk overflows parent")`; a clean EOF
    before the parent budget is satisfied surfaces as
    `Error::invalid("RIFF: truncated parent — …")`.
  - `Walker::read_body(chunk)` / `Walker::skip(chunk)` — consume
    the body + pad byte, advancing both the underlying reader and
    the walker's parent-budget counter.
  - `Walker::read_inner_form_type(chunk)` — for `RIFF` / `LIST`
    children, reads the 4-byte form-type / list-type tag and
    charges 4 bytes against the parent walker's budget, leaving
    the reader positioned at the first nested child.
  - `ChunkHeader::padded_size()` and `ChunkRef::padded_size()` /
    `end_offset()` — pre-computed wire-byte counts (body + pad)
    for callers that want to seek past a chunk without reading
    the body.
  - `FOURCC_RIFF` / `FOURCC_LIST` constants.
  - `fourcc::fourcc_bytes(b"RIFF")` `const` helper for compile-
    time tag literals.
  - `fourcc::fourcc_to_string()` — debug-safe rendering, escapes
    non-printable bytes as `\xNN` so debug dumps of malformed
    files stay readable.
  - `fourcc::is_printable_fourcc()` — `const` predicate for
    cheap rejection of obvious garbage (e.g. a JPEG SOI marker
    mis-fed into a RIFF parser).

- **Default-on `registry` feature.** With `registry` enabled the
  crate re-exports `oxideav_core::Error` / `oxideav_core::Result`
  so the walker plugs cleanly into framework consumers. Drop
  `default-features = false` to use the standalone in-tree
  `Error` enum (`Invalid(String)` + `Io(std::io::Error)`) and
  remove the framework dependency entirely.

- **24 unit tests** covering:
  - `ChunkHeader` constants + `is_group` + `padded_size` (incl.
    the `u32::MAX` odd-size edge).
  - `read_chunk_header` LE decode + clean-EOF + truncated-header
    paths.
  - `Walker::open_root` happy path, non-`RIFF` rejection,
    `ckSize < 4` rejection.
  - `Walker::read_next` round-trip + parent-budget enforcement (child
    overflow rejected) + truncated-parent detection.
  - `Walker::skip` advancing past body + pad.
  - `Walker::read_inner_form_type` reading the nested form-type
    word for `LIST` descent.
  - `fourcc_bytes` / `fourcc_to_string` printable + escaped
    rendering paths + `is_printable_fourcc` boundary checks.

### Known gaps (deferred to later rounds)

- `RF64` / `BW64` 64-bit-extended outer wrapper + `ds64`
  side-table (EBU Tech 3306 §4).
- Full named `KSDATAFORMAT_SUBTYPE_*` GUID catalogue (the
  symbolic-name ↔ codec table beyond the `DEFINE_WAVEFORMATEX_GUID`
  template `WaveFormat` already resolves).
- WAV metadata-bearing chunks: the `LIST INFO` vendor / iTunes-era
  sub-IDs beyond the 23-entry baseline (RecordingBlogs + ExifTool
  catalog), BWF `iXML` / `qlty` / `mext`, `cue ` / `plst` /
  `LIST adtl`, `smpl` / `inst`, ADM `axml` / `chna`, `id3 ` chunk.
- Higher-level recursive walker (`walk_tree`) for callers that
  want one-shot enumeration of every nested chunk.
- Streaming writer (begin/finish reservation pattern) — currently
  out of scope; the AVI / WebP crates carry their own form-
  specific writers.

### Clean-room provenance

All wire-format details are sourced from `docs/container/riff/`:

- `metadata/microsoft-riffmci.pdf` §1-2 (1991 IBM + Microsoft
  base spec).
- `metadata/ms-xaudio2-riff.html` (modern Microsoft Learn
  reformulation).
- `avi-riff-file-reference.md` (DirectShow AVI RIFF File
  Reference — cross-check that FourCC + size encoding matches
  across forms).
- `rfc2361-wav.txt` (the `wFormatTag` registry values).
- `waveformatextensible/` — Microsoft Learn *WAVEFORMATEXTENSIBLE
  structure*, *Extensible Wave-Format Descriptors*, and
  *Converting Between Format Tags and Subformat GUIDs* (the
  `DEFINE_WAVEFORMATEX_GUID` base-template macro).

Clean-room implementation. The sibling `oxideav-avi` crate's own
internal `riff.rs` was referenced as a clean-room precedent (same
project, same provenance), but the new walker is a fresh write-up
against the spec.
