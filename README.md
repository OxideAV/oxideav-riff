# oxideav-riff

Pure-Rust, clean-room implementation of the **RIFF** (Resource Interchange
File Format) chunk-walking primitives, per the publicly-published
*Multimedia Programming Interface and Data Specifications 1.0* that IBM
and Microsoft released in August 1991 and re-affirmed in the modern
Microsoft Learn *Resource Interchange File Format (RIFF)* page.

## Status — round 257 bootstrap

This crate ships the **shared chunk-walking primitives** that every
RIFF-family parser needs: a `ChunkHeader` decoder, a non-recursive
[`Walker`] over a parent chunk's children, FourCC helpers, and the
crate's own `Error` / `Result` aliases (with a default-on `registry`
feature that re-exports `oxideav-core`'s framework error type so the
walker plugs into the broader OxideAV pipeline without conversion
boilerplate).

The single goal for the bootstrap round is the chunk walker — codec-
specific chunk bodies (`fmt ` / `data` / `LIST INFO` sub-IDs / `bext`
BWF / `iXML` / `cue ` / `plst` / `LIST adtl` / `smpl` / `inst` /
`axml` / `chna` / `ds64` RF64 / `id3 `) and the `WAVEFORMATEX`
+ `WAVEFORMATEXTENSIBLE` + `KSDATAFORMAT_SUBTYPE_*` GUID resolver are
deferred to subsequent rounds and will stack on top of the walker.

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
- The `fmt ` chunk body, `WAVEFORMATEX`, `WAVEFORMATEXTENSIBLE`,
  and the `KSDATAFORMAT_SUBTYPE_*` GUID resolver.
- Any specific chunk body — the walker is intentionally codec-
  agnostic.

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
- `docs/container/riff/waveformatextensible/README.md` and the
  consolidated `ksdataformat-subtype-guids.md` catalogue — staged
  for the next round's `fmt ` / `WAVEFORMATEXTENSIBLE` decoder.
- `docs/container/riff/metadata/README.md` — staged catalogue of
  the WAV metadata-bearing chunks (`LIST INFO`, `bext`, `iXML`,
  `cue ` / `plst` / `LIST adtl`, `smpl` / `inst`, `axml` /
  `chna`, `ds64`) for later rounds.

## License

MIT — see [LICENSE](LICENSE).
