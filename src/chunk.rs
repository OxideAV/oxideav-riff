//! Low-level RIFF chunk-header reader.
//!
//! Per the 1991 RIFF spec
//! (`docs/container/riff/metadata/microsoft-riffmci.pdf` §1.3) every
//! chunk has an 8-byte header:
//!
//! ```text
//! +0  ckID    : [u8; 4]   FourCC
//! +4  ckSize  : u32 LE    payload length
//! ```
//!
//! The body is exactly `ckSize` bytes, followed by an implicit
//! 0x00 pad byte if `ckSize` is odd (so the next header starts at a
//! 2-byte boundary). `ckSize` does NOT include the header itself
//! and does NOT include the pad byte.
//!
//! Two reserved ckID values, `RIFF` and `LIST`, mark *group* chunks
//! whose payload begins with an extra 4-byte FourCC ("form type" for
//! RIFF, "list type" for LIST) followed by zero or more nested
//! chunks; their `is_group()` predicate returns `true`. All other
//! ckIDs are leaf payloads — the consumer pulls `ckSize` bytes out
//! of the reader and dispatches on the FourCC.
//!
//! This module exposes the minimum reading surface (header decode,
//! group-form-type read, body skip, pad skip). A higher-level walker
//! over the chunk tree lives in [`crate::walk`].

use std::io::{Read, Seek, SeekFrom};

use crate::error::{Error, Result};
use crate::fourcc::fourcc_bytes;

/// Outer wrapper FourCC. Appears exactly once at file offset 0 (with
/// `RF64` / `BW64` as the EBU Tech 3306 §4 64-bit-extended siblings).
pub const FOURCC_RIFF: [u8; 4] = fourcc_bytes(b"RIFF");

/// Big-endian outer-wrapper FourCC (§2 "Notation Conventions"). A
/// `RIFX` file is byte-for-byte identical to a `RIFF` file except the
/// first four bytes are `RIFX` and every integer field (notably the
/// `ckSize` length words) is stored in Motorola big-endian order rather
/// than Intel little-endian. The 4-byte FourCCs themselves are ASCII and
/// thus byte-order-independent.
pub const FOURCC_RIFX: [u8; 4] = fourcc_bytes(b"RIFX");

/// Nested-grouping FourCC. May appear inside `RIFF` (or another
/// `LIST`) with its own 4-byte list-type subtag and a body of nested
/// chunks.
pub const FOURCC_LIST: [u8; 4] = fourcc_bytes(b"LIST");

/// Decoded 8-byte RIFF chunk header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkHeader {
    /// 4-byte ASCII FourCC.
    pub id: [u8; 4],
    /// Payload length in bytes (does not include the 8-byte header,
    /// does not include the pad byte).
    pub size: u32,
}

impl ChunkHeader {
    /// `true` if this chunk introduces a sub-tree (its payload starts
    /// with a 4-byte form/list type and contains nested chunks).
    pub const fn is_group(&self) -> bool {
        matches!(self.id, FOURCC_RIFF | FOURCC_LIST)
    }

    /// Number of bytes the body + optional pad byte consume on the
    /// wire. Use this when seeking past a chunk you don't want to
    /// decode.
    pub const fn padded_size(&self) -> u64 {
        // RIFF requires a 0x00 pad byte after any odd-length body so
        // the next header lands on a 2-byte boundary.
        self.size as u64 + (self.size & 1) as u64
    }
}

/// Read a single 8-byte chunk header.
///
/// Returns `Ok(None)` at clean EOF (zero bytes available before the
/// next header). Returns `Err(Error::invalid)` if the reader hits EOF
/// part-way through the 8 bytes — that's a structural failure (the
/// containing parent's `ckSize` over-reported its payload).
pub fn read_chunk_header<R: Read + ?Sized>(r: &mut R) -> Result<Option<ChunkHeader>> {
    let mut buf = [0u8; 8];
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => {
                return if got == 0 {
                    // Clean EOF — natural end-of-stream between
                    // chunks. The caller decides whether that's
                    // acceptable (parent ckSize satisfied) or an
                    // error (parent truncated).
                    Ok(None)
                } else {
                    Err(Error::invalid("RIFF: truncated chunk header"))
                };
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Some(ChunkHeader {
        id: [buf[0], buf[1], buf[2], buf[3]],
        size: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
    }))
}

/// Read the 4-byte form-type / list-type that follows a group chunk
/// header (`RIFF` or `LIST`).
///
/// Caller is responsible for checking [`ChunkHeader::is_group`] before
/// invoking — the function itself just reads four bytes.
pub fn read_form_type<R: Read + ?Sized>(r: &mut R) -> Result<[u8; 4]> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(b)
}

/// Append an 8-byte chunk header to `out`: the 4-byte FourCC followed by
/// the little-endian `ckSize` (§1.3).
///
/// This is the write-side mirror of [`read_chunk_header`]: it emits the
/// header exactly as that function decodes it, so a header round-trips
/// `id` and `size` byte-for-byte. It does **not** emit the body or the
/// pad byte — see [`encode_chunk`] for the full leaf-chunk emitter.
pub fn write_chunk_header(out: &mut Vec<u8>, id: &[u8; 4], size: u32) {
    out.extend_from_slice(id);
    out.extend_from_slice(&size.to_le_bytes());
}

/// Append a complete leaf chunk — 8-byte header (`id` + `body.len()` as
/// the little-endian `ckSize`) followed by `body` and, when `body.len()`
/// is odd, the trailing `0x00` pad byte (§1.3) — to `out`.
///
/// This is the write-side mirror of the walker's read-one-chunk step:
/// the bytes it appends are exactly what [`read_chunk_header`] +
/// [`crate::walk::Walker::read_body`] would consume for the same chunk.
/// `ckSize` records the *un-padded* body length, matching the read side;
/// the pad byte keeps the next sibling header on a 2-byte boundary but is
/// not counted in `ckSize`.
///
/// Returns `Err(Error::invalid)` if `body` is longer than [`u32::MAX`]
/// (the on-wire `ckSize` field is a 32-bit value; an RF64 / BW64 file
/// defers such oversized chunks to the `ds64` table with a sentinel
/// header instead, see [`crate::ds64`]).
pub fn encode_chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) -> Result<()> {
    let size = u32::try_from(body.len())
        .map_err(|_| Error::invalid("RIFF: chunk body exceeds 32-bit ckSize range"))?;
    write_chunk_header(out, id, size);
    out.extend_from_slice(body);
    if size & 1 == 1 {
        out.push(0);
    }
    Ok(())
}

/// Skip past a chunk's body and any trailing pad byte.
///
/// Uses [`Seek::seek`] on the underlying reader; consumers operating
/// on a plain `Read` stream should pull the bytes through manually
/// instead.
pub fn skip_chunk<R: Seek + ?Sized>(r: &mut R, header: &ChunkHeader) -> Result<()> {
    let n = header.padded_size();
    if n > 0 {
        r.seek(SeekFrom::Current(n as i64))?;
    }
    Ok(())
}

/// Skip only the pad byte after the caller has already consumed the
/// `size` bytes of a chunk's body.
pub fn skip_pad<R: Seek + ?Sized>(r: &mut R, size: u32) -> Result<()> {
    if size & 1 == 1 {
        r.seek(SeekFrom::Current(1))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn header_constants_match_ascii() {
        assert_eq!(&FOURCC_RIFF, b"RIFF");
        assert_eq!(&FOURCC_LIST, b"LIST");
    }

    #[test]
    fn header_is_group_for_riff_and_list_only() {
        let riff = ChunkHeader {
            id: *b"RIFF",
            size: 0,
        };
        let list = ChunkHeader {
            id: *b"LIST",
            size: 0,
        };
        let data = ChunkHeader {
            id: *b"data",
            size: 0,
        };
        let fmt = ChunkHeader {
            id: *b"fmt ",
            size: 0,
        };
        assert!(riff.is_group());
        assert!(list.is_group());
        assert!(!data.is_group());
        assert!(!fmt.is_group());
    }

    #[test]
    fn padded_size_pads_odd() {
        assert_eq!(
            ChunkHeader {
                id: *b"data",
                size: 0
            }
            .padded_size(),
            0
        );
        assert_eq!(
            ChunkHeader {
                id: *b"data",
                size: 1
            }
            .padded_size(),
            2
        );
        assert_eq!(
            ChunkHeader {
                id: *b"data",
                size: 7
            }
            .padded_size(),
            8
        );
        assert_eq!(
            ChunkHeader {
                id: *b"data",
                size: 8
            }
            .padded_size(),
            8
        );
        // u32::MAX is odd: 4_294_967_295 + 1 = 4_294_967_296 fits in u64.
        assert_eq!(
            ChunkHeader {
                id: *b"data",
                size: u32::MAX
            }
            .padded_size(),
            u32::MAX as u64 + 1
        );
    }

    #[test]
    fn read_header_decodes_le_size() {
        let bytes = [b'd', b'a', b't', b'a', 0x10, 0x00, 0x00, 0x00];
        let mut cur = Cursor::new(&bytes[..]);
        let h = read_chunk_header(&mut cur).unwrap().unwrap();
        assert_eq!(&h.id, b"data");
        assert_eq!(h.size, 0x10);
    }

    #[test]
    fn read_header_returns_none_at_clean_eof() {
        let mut cur = Cursor::new(&[][..]);
        assert!(read_chunk_header(&mut cur).unwrap().is_none());
    }

    #[test]
    fn read_header_errors_on_partial_header() {
        // Only 5 of 8 header bytes.
        let mut cur = Cursor::new(&[b'd', b'a', b't', b'a', 0x10][..]);
        let err = read_chunk_header(&mut cur).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("truncated"), "{msg}");
    }

    #[test]
    fn read_form_type_reads_four_bytes() {
        let mut cur = Cursor::new(&b"WAVE"[..]);
        assert_eq!(&read_form_type(&mut cur).unwrap(), b"WAVE");
    }

    #[test]
    fn skip_chunk_advances_padded() {
        // 7-byte body → 8 bytes consumed including pad.
        let mut cur = Cursor::new(vec![0u8; 16]);
        cur.set_position(0);
        let h = ChunkHeader {
            id: *b"data",
            size: 7,
        };
        skip_chunk(&mut cur, &h).unwrap();
        assert_eq!(cur.position(), 8);
    }

    #[test]
    fn skip_pad_skips_only_for_odd_sizes() {
        let mut cur = Cursor::new(vec![0u8; 4]);
        cur.set_position(0);
        skip_pad(&mut cur, 4).unwrap();
        assert_eq!(cur.position(), 0);
        skip_pad(&mut cur, 5).unwrap();
        assert_eq!(cur.position(), 1);
    }

    #[test]
    fn write_chunk_header_is_read_inverse() {
        let mut out = Vec::new();
        write_chunk_header(&mut out, b"data", 0x10);
        assert_eq!(out, vec![b'd', b'a', b't', b'a', 0x10, 0x00, 0x00, 0x00]);
        let mut cur = Cursor::new(&out[..]);
        let h = read_chunk_header(&mut cur).unwrap().unwrap();
        assert_eq!(&h.id, b"data");
        assert_eq!(h.size, 0x10);
    }

    #[test]
    fn encode_chunk_even_body_no_pad() {
        let mut out = Vec::new();
        encode_chunk(&mut out, b"fmt ", &[0x01, 0x00, 0x02, 0x00]).unwrap();
        assert_eq!(
            out,
            vec![b'f', b'm', b't', b' ', 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00]
        );
        // No pad byte for an even-length body.
        assert_eq!(out.len(), 8 + 4);
    }

    #[test]
    fn encode_chunk_odd_body_adds_pad() {
        let mut out = Vec::new();
        encode_chunk(&mut out, b"data", &[0x10, 0x20, 0x30, 0x40, 0x50]).unwrap();
        // ckSize records the un-padded length (5), the body is 5 bytes,
        // and a single 0x00 pad byte follows.
        assert_eq!(out.len(), 8 + 5 + 1);
        assert_eq!(out[4..8], 5u32.to_le_bytes());
        assert_eq!(*out.last().unwrap(), 0x00);
    }

    #[test]
    fn encode_chunk_round_trips_through_header_read() {
        let mut out = Vec::new();
        encode_chunk(&mut out, b"junk", &[0xAA, 0xBB, 0xCC]).unwrap();
        let mut cur = Cursor::new(&out[..]);
        let h = read_chunk_header(&mut cur).unwrap().unwrap();
        assert_eq!(&h.id, b"junk");
        assert_eq!(h.size, 3);
        assert_eq!(h.padded_size(), 4);
    }
}
