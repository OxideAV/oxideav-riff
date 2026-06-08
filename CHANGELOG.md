# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- `fmt ` chunk body decoder + `WAVEFORMATEX` + `WAVEFORMATEXTENSIBLE`
  + `KSDATAFORMAT_SUBTYPE_*` GUID resolver.
- WAV metadata-bearing chunks: `LIST INFO` sub-IDs (RIFF MCI §3 +
  RecordingBlogs + ExifTool catalog), BWF `bext` (EBU Tech 3285),
  `iXML`, `cue ` / `plst` / `LIST adtl`, `smpl` / `inst`, ADM
  `axml` / `chna`, `id3 ` chunk.
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

No external library source (FFmpeg / libavformat / libsndfile /
the Windows SDK `mmreg.h` header / DirectShow SDK) was consulted.
The sibling `oxideav-avi` crate's own internal `riff.rs` was
referenced as a clean-room precedent (same project, same
provenance), but the new walker is a fresh write-up against the
spec rather than a copy of the AVI-internal primitives.
