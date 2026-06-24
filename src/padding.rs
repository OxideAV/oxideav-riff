//! Typed decoder / encoder for the RIFF `JUNK` filler chunk.
//!
//! `JUNK` is the 1991 RIFF MCI spec's general-purpose padding chunk:
//!
//! > A JUNK chunk represents padding, filler or outdated information. It
//! > contains no relevant data; it is a space filler of arbitrary size.
//! > `<JUNK chunk> ➝ JUNK( <filler> )` where `<filler>` contains random
//! > data.
//!
//! A reader ignores its contents (the AVI RIFF File Reference notes that
//! "Data can be aligned in an AVI file by inserting 'JUNK' chunks as
//! needed. Applications should ignore the contents of a 'JUNK' chunk").
//! A writer emits it for two purposes:
//!
//! 1. **Alignment** — pad so a following chunk (typically `data`) begins
//!    on a sector / granularity boundary.
//! 2. **RF64 / BW64 reservation** — a 32-bit RIFF/WAVE writer that may
//!    later need to grow past 4 GiB reserves a `JUNK` chunk (`ckSize`
//!    ≥ 28, the size of a future minimal `ds64` body) up front; when the
//!    file grows, the writer renames `JUNK` → `ds64`, fills in the real
//!    64-bit sizes, sets the 32-bit `RIFF` / `data` `ckSize` fields to
//!    the `0xFFFFFFFF` sentinel, and replaces the leading `RIFF` FOURCC
//!    with `BW64` (BS.2088-2 §2.5; the same MBWF / RF64 scheme as EBU
//!    Tech 3306).
//!
//! This module preserves the `JUNK` body **verbatim** so a read → write
//! round-trip is byte-exact (the filler bytes are "random" but a
//! conformant muxer must not reshuffle a chunk it merely passes through),
//! and offers constructors for the two writer use-cases.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "JUNK (Filler) Chunk".
//! - `docs/container/riff/avi-riff-file-reference.md` — the AVI form's
//!   use of `JUNK` for data alignment.
//! - `docs/container/riff/metadata/bs2088-chna-chunk-layout.md` §4.3 —
//!   the BS.2088-2 §2.5 `JUNK` ↔ `ds64` on-the-fly RF64 conversion (the
//!   28-byte reservation rule).

use crate::error::Result;

/// The `JUNK` filler chunk FourCC.
pub const FOURCC_JUNK: [u8; 4] = *b"JUNK";

/// The `PAD ` padding chunk FourCC — the other general-purpose padding /
/// alignment chunk a RIFF reader ignores alongside `JUNK` (the metadata
/// catalogue's "`JUNK` / `PAD ` → ignore" rule). Semantically identical
/// to `JUNK`: a writer emits it to align a following chunk on a boundary,
/// and a reader skips its contents.
pub const FOURCC_PAD: [u8; 4] = *b"PAD ";

/// `true` if `fourcc` is one of the two ignore-on-read padding chunk
/// identifiers (`JUNK` or `PAD `).
pub const fn is_padding_fourcc(fourcc: &[u8; 4]) -> bool {
    matches!(fourcc, b"JUNK" | b"PAD ")
}

/// Minimum `JUNK` body length, in bytes, for a chunk reserved to be
/// rewritten in place as a `ds64` (the 28-byte minimal `ds64` body:
/// `riffSize` + `dataSize` + `sampleCount`, each `u64`, plus the 4-byte
/// `tableLength`). See [`crate::ds64::DS64_PREFIX_LEN`].
pub const DS64_RESERVATION_LEN: usize = crate::ds64::DS64_PREFIX_LEN;

/// A decoded RIFF `JUNK` filler chunk.
///
/// The body is preserved verbatim. A `JUNK` chunk carries no structured
/// fields — its sole semantic content is its length — so the decoder is
/// a thin verbatim wrapper that exists to (a) keep the bytes for a
/// byte-exact mux round-trip and (b) classify the BS.2088 `ds64`
/// reservation case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Junk {
    /// The filler bytes, exactly as they appeared on the wire (the
    /// trailing RIFF pad byte, if the length was odd, is *not* part of
    /// the body — the walker strips it before the body reaches here).
    pub filler: Vec<u8>,
}

impl Junk {
    /// Wrap an already-extracted `JUNK` body verbatim.
    pub fn from_body(body: &[u8]) -> Self {
        Self {
            filler: body.to_vec(),
        }
    }

    /// A `JUNK` chunk of `len` zero bytes — the common alignment-padding
    /// case (the spec says the filler is "random data", but a zero fill
    /// is the conventional, reproducible choice).
    pub fn zeroed(len: usize) -> Self {
        Self {
            filler: vec![0u8; len],
        }
    }

    /// A `JUNK` chunk sized to be rewritten in place as a future `ds64`
    /// (a [`DS64_RESERVATION_LEN`]-byte zero fill), per BS.2088-2 §2.5.
    pub fn ds64_reservation() -> Self {
        Self::zeroed(DS64_RESERVATION_LEN)
    }

    /// The filler-body length in bytes (the chunk's `ckSize`).
    pub fn len(&self) -> usize {
        self.filler.len()
    }

    /// `true` if the chunk has a zero-length body.
    pub fn is_empty(&self) -> bool {
        self.filler.is_empty()
    }

    /// `true` if this `JUNK` chunk is large enough to be rewritten in
    /// place as a `ds64` chunk (`ckSize` ≥ [`DS64_RESERVATION_LEN`]),
    /// i.e. it could be a deferred RF64 / BW64 size reservation per
    /// BS.2088-2 §2.5. This is a *capacity* test only — a `JUNK` chunk
    /// of sufficient size used purely for alignment also passes.
    pub fn is_ds64_reservation(&self) -> bool {
        self.filler.len() >= DS64_RESERVATION_LEN
    }

    /// The `JUNK` *body* bytes — identical to [`Junk::filler`], provided
    /// for symmetry with the other typed encoders' `encode_body`.
    pub fn encode_body(&self) -> Vec<u8> {
        self.filler.clone()
    }

    /// Serialize the whole `JUNK` chunk — the `JUNK` header (with the
    /// computed `ckSize`) wrapping the filler body, framed with
    /// [`crate::chunk::encode_chunk`] so an odd-length body gets its
    /// trailing RIFF pad byte. Append directly to a RIFF file being
    /// muxed.
    pub fn encode_chunk(&self) -> Result<Vec<u8>> {
        self.encode_chunk_with(&FOURCC_JUNK)
    }

    /// Serialize the whole padding chunk under a caller-chosen FourCC —
    /// pass [`FOURCC_JUNK`] or [`FOURCC_PAD`] to preserve the exact
    /// spelling a source file used for a byte-exact round-trip. Both are
    /// ignore-on-read padding; the body is framed identically.
    pub fn encode_chunk_with(&self, fourcc: &[u8; 4]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        crate::chunk::encode_chunk(&mut out, fourcc, &self.filler)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{read_chunk_header, FOURCC_LIST, FOURCC_RIFF};
    use std::io::Cursor;

    #[test]
    fn from_body_preserves_bytes_verbatim() {
        let body = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x7F];
        let j = Junk::from_body(&body);
        assert_eq!(j.filler, body);
        assert_eq!(j.len(), 6);
        assert!(!j.is_empty());
    }

    #[test]
    fn zeroed_builds_a_zero_fill() {
        let j = Junk::zeroed(10);
        assert_eq!(j.len(), 10);
        assert!(j.filler.iter().all(|&b| b == 0));
    }

    #[test]
    fn empty_junk_is_empty() {
        let j = Junk::zeroed(0);
        assert!(j.is_empty());
        assert_eq!(j.len(), 0);
    }

    #[test]
    fn ds64_reservation_is_28_bytes_and_classifies() {
        assert_eq!(DS64_RESERVATION_LEN, 28);
        let j = Junk::ds64_reservation();
        assert_eq!(j.len(), 28);
        assert!(j.is_ds64_reservation());
        // A 27-byte filler is one short — not a valid ds64 reservation.
        assert!(!Junk::zeroed(27).is_ds64_reservation());
        // 28 exactly, and anything larger, qualifies.
        assert!(Junk::zeroed(28).is_ds64_reservation());
        assert!(Junk::zeroed(100).is_ds64_reservation());
    }

    #[test]
    fn encode_chunk_frames_header_and_round_trips_even_body() {
        let j = Junk::from_body(&[1, 2, 3, 4]);
        let chunk = j.encode_chunk().unwrap();
        assert_eq!(&chunk[0..4], b"JUNK");
        assert_eq!(&chunk[4..8], &4u32.to_le_bytes());
        assert_eq!(&chunk[8..12], &[1, 2, 3, 4]);
        // Even body: no pad byte.
        assert_eq!(chunk.len(), 12);

        // Re-read the header and body back.
        let mut cur = Cursor::new(chunk);
        let header = read_chunk_header(&mut cur).unwrap().unwrap();
        assert_eq!(header.id, FOURCC_JUNK);
        assert!(!header.is_group());
        assert_eq!(header.size, 4);
    }

    #[test]
    fn encode_chunk_pads_odd_body() {
        // 3-byte body needs a trailing RIFF pad byte.
        let j = Junk::from_body(&[0xAA, 0xBB, 0xCC]);
        let chunk = j.encode_chunk().unwrap();
        assert_eq!(&chunk[4..8], &3u32.to_le_bytes());
        // header(8) + body(3) + pad(1) = 12.
        assert_eq!(chunk.len(), 12);
        assert_eq!(chunk[11], 0, "missing RIFF pad byte");
        // The recorded ckSize excludes the pad.
        assert_eq!(
            u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            3
        );
    }

    #[test]
    fn encode_body_matches_filler() {
        let j = Junk::zeroed(5);
        assert_eq!(j.encode_body(), vec![0u8; 5]);
    }

    #[test]
    fn junk_is_not_a_group_fourcc() {
        // Sanity: JUNK must never collide with the two reserved group IDs.
        assert_ne!(FOURCC_JUNK, FOURCC_RIFF);
        assert_ne!(FOURCC_JUNK, FOURCC_LIST);
    }

    #[test]
    fn ds64_reservation_filler_is_zero() {
        let j = Junk::ds64_reservation();
        assert!(j.filler.iter().all(|&b| b == 0));
    }

    #[test]
    fn pad_fourcc_is_recognised() {
        assert_eq!(FOURCC_PAD, *b"PAD ");
        assert!(is_padding_fourcc(b"PAD "));
        assert!(is_padding_fourcc(b"JUNK"));
        assert!(!is_padding_fourcc(b"data"));
        assert!(!is_padding_fourcc(b"PAD\0"));
        // PAD must never collide with the reserved group IDs.
        assert_ne!(FOURCC_PAD, FOURCC_RIFF);
        assert_ne!(FOURCC_PAD, FOURCC_LIST);
    }

    #[test]
    fn encode_chunk_with_pad_fourcc() {
        let j = Junk::zeroed(4);
        let chunk = j.encode_chunk_with(&FOURCC_PAD).unwrap();
        assert_eq!(&chunk[0..4], b"PAD ");
        assert_eq!(&chunk[4..8], &4u32.to_le_bytes());
        // Same framing as JUNK: header(8) + body(4).
        assert_eq!(chunk.len(), 12);

        let mut cur = Cursor::new(chunk);
        let header = read_chunk_header(&mut cur).unwrap().unwrap();
        assert_eq!(header.id, FOURCC_PAD);
        assert_eq!(header.size, 4);
    }
}
