//! Clean-room **RIFF** (Resource Interchange File Format) primitives.
//!
//! RIFF is the chunked little-endian container format Microsoft and IBM
//! published in 1991 as part of the *Multimedia Programming Interface and
//! Data Specifications 1.0*. It is the carrier for WAV (RIFF/WAVE), AVI
//! (RIFF/AVI ), WebP (RIFF/WEBP), AMV (RIFF-like, big-endian sizes), the
//! Windows ANI cursor format (RIFF/ACON), and several others.
//!
//! This crate scopes itself to the **shared chunk-walking primitives**
//! that every RIFF-family parser needs: a `ChunkHeader` decoder, an
//! `Iter`-style walker over the top-level RIFF/LIST tree, FourCC
//! helpers, and a `Reader` wrapper that tracks the current chunk's
//! body length so consumers don't accidentally read past it. Codec-
//! specific chunks (`fmt `, `data`, `LIST INFO`, BWF `bext`, RF64
//! `ds64`, the AVI `idx1` index, the WebP `VP8 ` / `VP8L` payloads,
//! …) are intentionally **not** parsed here — they live in their own
//! codec crates which call into the walker.
//!
//! Round 257 lands the chunk-walker as the foundation; round 267
//! stacks the WAV `fmt`-chunk decoder ([`waveformat::WaveFormat`])
//! with its `WAVEFORMATEX` + `WAVEFORMATEXTENSIBLE` sub-fields and the
//! `DEFINE_WAVEFORMATEX_GUID` sub-format resolver on top. The full
//! `KSDATAFORMAT_SUBTYPE_*` named-GUID catalogue is a later round.
//!
//! ## Wire format (§1.3 of the 1991 spec)
//!
//! Every RIFF chunk is laid out as:
//!
//! ```text
//! +0  ckID    : 4 bytes  ASCII FourCC, padded with trailing 0x20 if shorter
//! +4  ckSize  : u32 LE   payload length in bytes, NOT including ckID/ckSize
//!                        and NOT including the trailing pad byte
//! +8  ckData  : ckSize bytes payload
//! +8+ckSize   : pad      0x00 pad byte if ckSize is odd (so the next
//!                        chunk header starts at a 2-byte boundary)
//! ```
//!
//! Two reserved ckIDs introduce **list chunks** whose payload starts
//! with an additional 4-byte FourCC ("form type") followed by zero or
//! more nested child chunks:
//!
//! | ckID   | Role                                                       |
//! |--------|------------------------------------------------------------|
//! | `RIFF` | Outermost wrapper; appears exactly once at file offset 0   |
//! | `LIST` | Nested grouping inside a `RIFF` (or another `LIST`)        |
//!
//! All multi-byte fields are little-endian (a deliberate choice that
//! distinguishes RIFF from IFF-85, which is big-endian and uses
//! `FORM`/`LIST`/`CAT ` as its group IDs). The total file size is
//! ck-size of the outer `RIFF` + 8 (header) bytes.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §1-2 — IBM
//!   + Microsoft, *Multimedia Programming Interface and Data
//!   Specifications 1.0*, August 1991.
//! - `docs/container/riff/metadata/ms-xaudio2-riff.html` — Microsoft
//!   Learn, modern reformulation of the same wire layout for the
//!   Win32 XAudio2 reference.
//! - `docs/container/riff/avi-riff-file-reference.md` — DirectShow
//!   AVI RIFF File Reference; the AVI form's use of the base RIFF
//!   primitives, useful as a cross-check that the FourCC + size
//!   encoding matches across forms.
//!
//! ## Standalone build
//!
//! `oxideav-core` is gated behind the default-on `registry` feature.
//! Drop the framework dependency entirely with:
//!
//! ```toml
//! oxideav-riff = { version = "0.0", default-features = false }
//! ```
//!
//! Without `registry`, the crate exposes its own [`Error`] /
//! [`Result`] aliases (defined in [`error`]); with `registry`, those
//! aliases re-export [`oxideav_core::Error`] / [`oxideav_core::Result`]
//! so the walker plugs cleanly into framework consumers.
//!
//! ## Quick start
//!
//! ```
//! use std::io::Cursor;
//! use oxideav_riff::{ChunkHeader, FOURCC_RIFF, read_chunk_header, read_form_type};
//!
//! // Minimal RIFF/WAVE skeleton: just the outer header + form type.
//! let bytes: &[u8] = &[
//!     b'R', b'I', b'F', b'F',
//!     0x04, 0x00, 0x00, 0x00,   // ckSize = 4 (just the form type)
//!     b'W', b'A', b'V', b'E',
//! ];
//! let mut cur = Cursor::new(bytes);
//! let outer: ChunkHeader = read_chunk_header(&mut cur).unwrap().unwrap();
//! assert_eq!(outer.id, FOURCC_RIFF);
//! assert_eq!(outer.size, 4);
//! assert!(outer.is_group());
//!
//! let form = read_form_type(&mut cur).unwrap();
//! assert_eq!(&form, b"WAVE");
//! ```

#![doc(html_root_url = "https://docs.rs/oxideav-riff/0.0.1")]

pub mod chunk;
pub mod error;
pub mod fourcc;
pub mod walk;
pub mod waveformat;

pub use chunk::{
    read_chunk_header, read_form_type, skip_chunk, skip_pad, ChunkHeader, FOURCC_LIST, FOURCC_RIFF,
};
pub use error::{Error, Result};
pub use fourcc::{fourcc_bytes, fourcc_to_string, is_printable_fourcc};
pub use walk::{ChunkRef, Walker};
pub use waveformat::{
    ExtensibleFields, Guid, WaveFormat, KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE, WAVE_FORMAT_ADPCM,
    WAVE_FORMAT_ALAW, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IEEE_FLOAT, WAVE_FORMAT_MULAW,
    WAVE_FORMAT_PCM,
};
