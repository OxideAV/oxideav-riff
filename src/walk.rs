//! Higher-level walker over a RIFF chunk tree.
//!
//! [`Walker`] wraps a `Read + Seek` source and yields successive
//! children of a parent group chunk as [`ChunkRef`] descriptors. Each
//! [`ChunkRef`] records the parent-relative byte offset of the body
//! plus its payload length so the consumer can:
//!
//! - read the body bytes via [`Walker::read_body`] (consuming the
//!     walker's current position), or
//! - seek to it later (the absolute offset is recorded), or
//! - skip it via [`Walker::skip`] and continue iterating siblings.
//!
//! The walker is **non-recursive** by design — codec-specific
//! consumers know which group ckIDs they care about (`LIST INFO`,
//! `LIST adtl`, `LIST hdrl`, …) and dispatch the recursion themselves.
//! The walker just enforces the parent's `ckSize` budget: it stops
//! cleanly when the consumed bytes equal the parent payload length,
//! and rejects any sibling whose own header would overflow that
//! budget.
//!
//! ## Wire-format invariants enforced
//!
//! - Each child header is read from the current position; its
//!   payload + pad-byte must fit within the remaining parent budget.
//!     An over-reported `ckSize` is rejected as
//!     `Error::invalid("RIFF: chunk overflows parent")` — not silently
//!     truncated.
//! - At the end-of-parent boundary, [`Walker::read_next`] returns
//!     `Ok(None)`. A clean EOF before the budget is satisfied is an
//!     `Error::invalid("RIFF: truncated parent")` since the parent's
//!     `ckSize` lied about how much payload it contained.
//! - Group children (`RIFF`/`LIST`) keep their `is_group()` bit set
//!     on the [`ChunkRef`]; the consumer that wants to descend reads
//!     the inner form type with [`Walker::read_inner_form_type`] and
//!     constructs a child [`Walker`] over `payload_len - 4` bytes
//!     starting just past the form-type word.
//!
//! ## 64-bit (`RF64` / `BW64`) entry points
//!
//! The base [`Walker::open_root`] is strict on the `RIFF` FourCC. The
//! `RF64` / `BW64` 64-bit-extended forms (EBU Tech 3306 / BS.2088) carry
//! the `0xFFFFFFFF` sentinel in the outer 32-bit `ckSize` field; the real
//! 64-bit RIFF size lives in the mandatory `ds64` chunk that must
//! immediately follow the form-type word. [`Walker::open_rf64`] reads that
//! `ds64` chunk, resolves the outer size from its `riffSize`, and returns
//! a walker positioned at the *first chunk after `ds64`* (with the
//! parsed [`crate::ds64::DataSize64`] available via
//! [`Walker::data_size_64`] so a consumer can resolve any `0xFFFFFFFF`
//! `data` / `fact` / table-listed chunk sizes it later encounters).
//! [`Walker::open_bw64`] is the alias for the ADM-carrying `BW64` magic;
//! both accept either magic.

use std::io::{Read, Seek, SeekFrom};

use crate::chunk::{read_chunk_header, read_form_type, ChunkHeader};
use crate::ds64::{is_rf64_magic, DataSize64, FOURCC_DS64};
use crate::error::{Error, Result};

/// One child chunk yielded by [`Walker::read_next`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRef {
    /// 4-byte FourCC of the chunk.
    pub id: [u8; 4],
    /// Payload length per the chunk header (does not include the
    /// 8-byte header, does not include the pad byte).
    pub size: u32,
    /// Absolute byte offset of the payload within the underlying
    /// reader. Subtract 8 to recover the offset of the header itself.
    pub body_offset: u64,
}

impl ChunkRef {
    /// `true` if this child is a `RIFF` or `LIST` group whose body
    /// starts with a form-type FourCC followed by nested chunks.
    pub const fn is_group(&self) -> bool {
        matches!(self.id, super::FOURCC_RIFF | super::FOURCC_LIST)
    }

    /// Number of bytes the body + optional pad byte consume on the
    /// wire. Same semantics as [`ChunkHeader::padded_size`].
    pub const fn padded_size(&self) -> u64 {
        self.size as u64 + (self.size & 1) as u64
    }

    /// Absolute byte offset just past this chunk's body+pad — i.e.
    /// where the next sibling's header begins.
    pub const fn end_offset(&self) -> u64 {
        self.body_offset + self.padded_size()
    }
}

/// Walker over the immediate children of a parent group chunk.
///
/// Construction options:
///
/// - [`Walker::open_root`] reads the outer `RIFF` chunk header from
///     offset 0, validates the FourCC, and returns the walker
///     positioned just past the 4-byte form-type word so the next
///     call to [`Walker::read_next`] yields the first top-level
///     child.
/// - [`Walker::open_within`] starts a walker over an arbitrary
///     already-validated parent — handy for descending into a `LIST`
///     sub-tree without re-reading its outer header.
#[derive(Debug)]
pub struct Walker<'r, R: Read + Seek + ?Sized> {
    inner: &'r mut R,
    /// Total payload length (the parent's `ckSize`).
    payload_len: u64,
    /// Bytes already consumed inside the parent (header + body + pad
    /// of every yielded child plus, for the root, the leading 4-byte
    /// form-type word).
    consumed: u64,
    /// Form-type / list-type of the parent group chunk (`WAVE`,
    /// `AVI `, `INFO`, …).
    form_type: [u8; 4],
    /// The parsed `ds64` chunk, present only when the walker was opened
    /// over an `RF64` / `BW64` outer wrapper via [`Walker::open_rf64`].
    /// Carries the 64-bit `riffSize` / `dataSize` / `sampleCount` plus
    /// the per-chunk size-override table.
    data_size_64: Option<DataSize64>,
}

impl<'r, R: Read + Seek + ?Sized> Walker<'r, R> {
    /// Open the outermost `RIFF` chunk at offset 0 of `r` and return a
    /// walker positioned at the first top-level child (i.e. the next
    /// [`Walker::read_next`] call yields that child).
    ///
    /// Fails if the file does not begin with the `RIFF` FourCC (the
    /// 64-bit `RF64` / `BW64` variants are handled by a separate
    /// constructor in a later round; this function is strict on
    /// `RIFF`).
    pub fn open_root(r: &'r mut R) -> Result<Self> {
        r.seek(SeekFrom::Start(0))?;
        let header = read_chunk_header(r)?
            .ok_or_else(|| Error::invalid("RIFF: empty input, no outer chunk header"))?;
        if header.id != super::FOURCC_RIFF {
            return Err(Error::invalid("RIFF: outer chunk is not 'RIFF'"));
        }
        if header.size < 4 {
            // Outer payload must hold at least the 4-byte form type.
            return Err(Error::invalid(
                "RIFF: outer ckSize < 4 — no room for form type",
            ));
        }
        let form_type = read_form_type(r)?;
        Ok(Self {
            inner: r,
            payload_len: header.size as u64,
            // The 4-byte form-type word counts against the parent's
            // ckSize budget.
            consumed: 4,
            form_type,
            data_size_64: None,
        })
    }

    /// Open an `RF64` / `BW64` 64-bit-extended outer wrapper.
    ///
    /// The outer 8-byte header carries the `RF64` (or ADM-carrying
    /// `BW64`) FourCC and the `0xFFFFFFFF` size sentinel; the *real*
    /// 64-bit RIFF size lives in the mandatory `ds64` chunk that must
    /// immediately follow the form-type word (EBU Tech 3306 / BS.2088).
    /// This constructor:
    ///
    /// 1. reads + validates the outer `RF64` / `BW64` header,
    /// 2. reads the 4-byte form type (`WAVE`),
    /// 3. reads the `ds64` chunk (header + body), resolving the outer
    ///    `riffSize`, and
    /// 4. returns a walker whose payload budget is that resolved 64-bit
    ///    size, positioned at the *first chunk after `ds64`* so the next
    ///    [`Walker::read_next`] yields it.
    ///
    /// The parsed `ds64` is available via [`Walker::data_size_64`] so the
    /// consumer can resolve any later `0xFFFFFFFF` `data` / `fact` /
    /// table-listed chunk size.
    ///
    /// Errors if the outer FourCC is neither `RF64` nor `BW64`, if the
    /// first chunk is not `ds64`, or if the resolved `riffSize` is
    /// smaller than the bytes already consumed (form type + `ds64`).
    pub fn open_rf64(r: &'r mut R) -> Result<Self> {
        r.seek(SeekFrom::Start(0))?;
        let header = read_chunk_header(r)?
            .ok_or_else(|| Error::invalid("RF64: empty input, no outer chunk header"))?;
        if !is_rf64_magic(&header.id) {
            return Err(Error::invalid(
                "RF64: outer chunk is neither 'RF64' nor 'BW64'",
            ));
        }
        if header.size < 4 {
            return Err(Error::invalid(
                "RF64: outer ckSize < 4 — no room for form type",
            ));
        }
        let form_type = read_form_type(r)?;

        // The ds64 chunk MUST be the first chunk after the form type.
        let ds64_header = read_chunk_header(r)?
            .ok_or_else(|| Error::invalid("RF64: missing mandatory 'ds64' chunk"))?;
        if ds64_header.id != FOURCC_DS64 {
            return Err(Error::invalid(
                "RF64: first chunk after the form type is not 'ds64'",
            ));
        }
        let mut ds64_body = vec![0u8; ds64_header.size as usize];
        r.read_exact(&mut ds64_body)?;
        if ds64_header.size & 1 == 1 {
            let mut pad = [0u8; 1];
            r.read_exact(&mut pad)?;
        }
        let ds64 = DataSize64::parse(&ds64_body)?;

        // The resolved outer payload size (riffSize) is the budget; it
        // counts everything after the outer 8-byte header, i.e. the
        // form-type word + the ds64 chunk + every following chunk.
        let payload_len = ds64.riff_size;
        // Bytes consumed so far inside that payload: the 4-byte form type
        // plus the ds64 chunk header + padded body.
        let consumed = 4 + 8 + ds64_header.padded_size();
        if consumed > payload_len {
            return Err(Error::invalid(
                "RF64: ds64 riffSize is smaller than the form-type + ds64 prologue",
            ));
        }
        Ok(Self {
            inner: r,
            payload_len,
            consumed,
            form_type,
            data_size_64: Some(ds64),
        })
    }

    /// Alias for [`Walker::open_rf64`] named for the ADM-carrying `BW64`
    /// magic. Both magics are accepted by either constructor; this name
    /// documents intent at the call site.
    pub fn open_bw64(r: &'r mut R) -> Result<Self> {
        Self::open_rf64(r)
    }

    /// Start a walker over the body of an already-located group
    /// chunk.
    ///
    /// `header` is the parent's [`ChunkHeader`] (must be `is_group()`)
    /// and `r` must be positioned at the first byte of the parent's
    /// payload — i.e. at the form-type word.
    ///
    /// The constructor consumes the 4-byte form-type word itself so
    /// the first [`Walker::read_next`] call yields the first nested
    /// child.
    pub fn open_within(r: &'r mut R, header: &ChunkHeader) -> Result<Self> {
        if !header.is_group() {
            return Err(Error::invalid(
                "RIFF: open_within called on a non-group chunk",
            ));
        }
        if header.size < 4 {
            return Err(Error::invalid(
                "RIFF: group ckSize < 4 — no room for form type",
            ));
        }
        let form_type = read_form_type(r)?;
        Ok(Self {
            inner: r,
            payload_len: header.size as u64,
            consumed: 4,
            form_type,
            data_size_64: None,
        })
    }

    /// Form-type / list-type FourCC of the parent group chunk.
    pub const fn form_type(&self) -> [u8; 4] {
        self.form_type
    }

    /// The parsed `ds64` chunk, if this walker was opened over an `RF64`
    /// / `BW64` outer wrapper via [`Walker::open_rf64`]. `None` for a
    /// plain 32-bit `RIFF` walker. Use it to resolve any `0xFFFFFFFF`
    /// `data` / `fact` / table-listed chunk size encountered while
    /// walking.
    pub fn data_size_64(&self) -> Option<&DataSize64> {
        self.data_size_64.as_ref()
    }

    /// Total payload length of the parent chunk (its `ckSize`).
    pub const fn payload_len(&self) -> u64 {
        self.payload_len
    }

    /// Remaining unwalked bytes inside the parent payload.
    pub const fn remaining(&self) -> u64 {
        self.payload_len - self.consumed
    }

    /// Read the next child chunk header. Returns `Ok(None)` when the
    /// parent's payload budget is exactly satisfied.
    ///
    /// Named `read_next` rather than `next` so the walker does not
    /// shadow [`std::iter::Iterator::next`] — RIFF children can fail
    /// to decode mid-stream (truncated parent, child overflows
    /// parent budget, …), so the natural return shape is
    /// `Result<Option<…>>` rather than the `Option<…>` that the
    /// `Iterator` trait requires.
    pub fn read_next(&mut self) -> Result<Option<ChunkRef>> {
        // Standard termination: exactly hit the parent boundary.
        if self.consumed == self.payload_len {
            return Ok(None);
        }
        // 8 header bytes must fit in the remaining budget.
        if self.payload_len - self.consumed < 8 {
            return Err(Error::invalid(
                "RIFF: truncated parent — less than a chunk header left",
            ));
        }
        let header = match read_chunk_header(self.inner)? {
            Some(h) => h,
            None => {
                // The reader hit clean EOF while the parent ckSize
                // promised more bytes — wire-format violation.
                return Err(Error::invalid(
                    "RIFF: truncated parent — EOF inside parent body",
                ));
            }
        };
        self.consumed += 8;
        // Body + pad must fit in the remaining budget.
        let padded = header.padded_size();
        if padded > self.payload_len - self.consumed {
            return Err(Error::invalid("RIFF: chunk overflows parent"));
        }
        // Record the absolute body offset BEFORE advancing the
        // consumed counter past the body — the body itself starts
        // at the underlying reader's current position.
        let body_offset = self.inner.stream_position()?;
        Ok(Some(ChunkRef {
            id: header.id,
            size: header.size,
            body_offset,
        }))
    }

    /// Read the body of the chunk just yielded by [`Walker::read_next`]
    /// into a `Vec`. Advances the walker past the body + pad byte.
    ///
    /// The caller MUST pass the [`ChunkRef`] returned by the most
    /// recent [`Walker::read_next`] call; reading body B after a
    /// different chunk has been yielded would consume from the wrong
    /// stream position. The function does not re-seek to
    /// `body_offset` — that's deliberate, the consumer pays for
    /// seeks explicitly.
    pub fn read_body(&mut self, chunk: &ChunkRef) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; chunk.size as usize];
        self.inner.read_exact(&mut buf)?;
        if chunk.size & 1 == 1 {
            let mut pad = [0u8; 1];
            self.inner.read_exact(&mut pad)?;
        }
        self.consumed += chunk.padded_size();
        Ok(buf)
    }

    /// Skip the body + pad byte of the chunk just yielded by
    /// [`Walker::read_next`].
    pub fn skip(&mut self, chunk: &ChunkRef) -> Result<()> {
        let padded = chunk.padded_size();
        if padded > 0 {
            self.inner.seek(SeekFrom::Current(padded as i64))?;
        }
        self.consumed += padded;
        Ok(())
    }

    /// Read the inner form/list-type of a group child.
    ///
    /// Fails if `chunk` is not a group chunk. On success the walker
    /// has consumed 4 bytes of the child's payload; the caller can
    /// then construct a nested walker over the remaining
    /// `chunk.size - 4` bytes of the child's body.
    pub fn read_inner_form_type(&mut self, chunk: &ChunkRef) -> Result<[u8; 4]> {
        if !chunk.is_group() {
            return Err(Error::invalid(
                "RIFF: read_inner_form_type called on a non-group chunk",
            ));
        }
        if chunk.size < 4 {
            return Err(Error::invalid(
                "RIFF: nested group ckSize < 4 — no room for form type",
            ));
        }
        let ft = read_form_type(self.inner)?;
        // The 4 bytes count against the PARENT walker's budget.
        self.consumed += 4;
        Ok(ft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Minimal RIFF/WAVE skeleton with two leaf chunks:
    /// `fmt ` (4-byte body) and `data` (3-byte body — exercises pad).
    fn synthetic_wave() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(4u32 + 8 + 4 + 8 + 3 + 1).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        // fmt chunk
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&[0x01, 0x00, 0x02, 0x00]); // arbitrary
                                                        // data chunk (odd-length → pad)
        v.extend_from_slice(b"data");
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        v.push(0); // pad
        v
    }

    #[test]
    fn open_root_decodes_form_type_and_yields_children() {
        let bytes = synthetic_wave();
        let mut cur = Cursor::new(bytes);
        let mut walker = Walker::open_root(&mut cur).unwrap();
        assert_eq!(&walker.form_type(), b"WAVE");

        let first = walker.read_next().unwrap().unwrap();
        assert_eq!(&first.id, b"fmt ");
        assert_eq!(first.size, 4);
        // The body of fmt starts right after the outer "RIFF"/size/form
        // (12 bytes) + the fmt header (8 bytes) = 20.
        assert_eq!(first.body_offset, 20);
        let body = walker.read_body(&first).unwrap();
        assert_eq!(body, vec![0x01, 0x00, 0x02, 0x00]);

        let second = walker.read_next().unwrap().unwrap();
        assert_eq!(&second.id, b"data");
        assert_eq!(second.size, 3);
        assert_eq!(second.padded_size(), 4);
        let body = walker.read_body(&second).unwrap();
        assert_eq!(body, vec![0xAA, 0xBB, 0xCC]);

        // Parent budget exhausted exactly.
        assert!(walker.read_next().unwrap().is_none());
    }

    #[test]
    fn skip_advances_past_body_and_pad() {
        let bytes = synthetic_wave();
        let mut cur = Cursor::new(bytes);
        let mut walker = Walker::open_root(&mut cur).unwrap();
        let fmt = walker.read_next().unwrap().unwrap();
        walker.skip(&fmt).unwrap();
        let data = walker.read_next().unwrap().unwrap();
        assert_eq!(&data.id, b"data");
        walker.skip(&data).unwrap();
        assert!(walker.read_next().unwrap().is_none());
    }

    #[test]
    fn open_root_rejects_non_riff_outer() {
        let mut bytes = synthetic_wave();
        bytes[0] = b'X';
        let mut cur = Cursor::new(bytes);
        let err = Walker::open_root(&mut cur).unwrap_err();
        assert!(format!("{err}").contains("outer chunk is not 'RIFF'"));
    }

    #[test]
    fn open_root_rejects_short_outer() {
        // RIFF + ckSize=2 + 2 random bytes — payload < 4.
        let bytes = [b'R', b'I', b'F', b'F', 0x02, 0x00, 0x00, 0x00, b'W', b'A'];
        let mut cur = Cursor::new(&bytes[..]);
        let err = Walker::open_root(&mut cur).unwrap_err();
        assert!(format!("{err}").contains("ckSize < 4"));
    }

    #[test]
    fn walker_rejects_child_that_overflows_parent() {
        // Outer parent says payload is 16 bytes (form-type + one
        // child header), but the child claims a 1MB body.
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fake");
        v.extend_from_slice(&1_000_000u32.to_le_bytes());
        let mut cur = Cursor::new(v);
        let mut walker = Walker::open_root(&mut cur).unwrap();
        let err = walker.read_next().unwrap_err();
        assert!(format!("{err}").contains("overflows parent"));
    }

    #[test]
    fn walker_rejects_truncated_parent() {
        // Outer parent claims 32 bytes but only 12 are present.
        let bytes = [
            b'R', b'I', b'F', b'F', 0x20, 0x00, 0x00, 0x00, b'W', b'A', b'V', b'E',
        ];
        let mut cur = Cursor::new(&bytes[..]);
        let mut walker = Walker::open_root(&mut cur).unwrap();
        // 20 bytes remain unwalked; first child header read should
        // hit EOF and surface a truncated-parent error.
        let err = walker.read_next().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("truncated parent"), "{msg}");
    }

    /// Build a minimal `ds64` chunk body (28-byte prefix, empty table)
    /// declaring the given riff/data/sample sizes.
    fn ds64_body(riff_size: u64, data_size: u64, sample_count: u64) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&riff_size.to_le_bytes());
        v.extend_from_slice(&data_size.to_le_bytes());
        v.extend_from_slice(&sample_count.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // tableLength = 0
        v
    }

    /// `RF64`/`WAVE { ds64 fmt(4) data(4) }`, with the outer + data
    /// ckSize fields set to the 0xFFFFFFFF sentinel.
    fn synthetic_rf64(magic: &[u8; 4]) -> Vec<u8> {
        let mut v = Vec::new();
        // body after outer header: WAVE(4) + ds64 hdr(8)+body(28)
        //   + fmt hdr(8)+body(4) + data hdr(8)+body(4) = 64
        let riff_size: u64 = 4 + 8 + 28 + 8 + 4 + 8 + 4;
        v.extend_from_slice(magic);
        v.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // outer sentinel
        v.extend_from_slice(b"WAVE");
        // ds64
        v.extend_from_slice(b"ds64");
        v.extend_from_slice(&28u32.to_le_bytes());
        v.extend_from_slice(&ds64_body(riff_size, 4, 1));
        // fmt
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 2, 0]);
        // data
        v.extend_from_slice(b"data");
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&[9, 8, 7, 6]);
        v
    }

    #[test]
    fn open_rf64_resolves_size_and_yields_post_ds64_chunks() {
        let bytes = synthetic_rf64(b"RF64");
        let mut cur = Cursor::new(bytes);
        let mut walker = Walker::open_rf64(&mut cur).unwrap();
        assert_eq!(&walker.form_type(), b"WAVE");
        // ds64 is consumed by the constructor and exposed.
        let ds64 = walker.data_size_64().expect("ds64 parsed");
        assert_eq!(ds64.riff_size, 64);
        assert_eq!(ds64.data_size, 4);
        assert_eq!(ds64.sample_count, 1);

        // First yielded chunk is the fmt that follows ds64.
        let fmt = walker.read_next().unwrap().unwrap();
        assert_eq!(&fmt.id, b"fmt ");
        let body = walker.read_body(&fmt).unwrap();
        assert_eq!(body, vec![1, 0, 2, 0]);

        let data = walker.read_next().unwrap().unwrap();
        assert_eq!(&data.id, b"data");
        let body = walker.read_body(&data).unwrap();
        assert_eq!(body, vec![9, 8, 7, 6]);

        // Budget exactly satisfied.
        assert!(walker.read_next().unwrap().is_none());
    }

    #[test]
    fn open_bw64_accepts_the_adm_magic() {
        let bytes = synthetic_rf64(b"BW64");
        let mut cur = Cursor::new(bytes);
        let mut walker = Walker::open_bw64(&mut cur).unwrap();
        assert_eq!(&walker.form_type(), b"WAVE");
        assert_eq!(walker.data_size_64().unwrap().riff_size, 64);
        let fmt = walker.read_next().unwrap().unwrap();
        assert_eq!(&fmt.id, b"fmt ");
    }

    #[test]
    fn open_rf64_rejects_plain_riff_magic() {
        let mut bytes = synthetic_rf64(b"RF64");
        bytes[0..4].copy_from_slice(b"RIFF");
        let mut cur = Cursor::new(bytes);
        let err = Walker::open_rf64(&mut cur).unwrap_err();
        assert!(format!("{err}").contains("neither 'RF64' nor 'BW64'"));
    }

    #[test]
    fn open_rf64_rejects_missing_ds64() {
        // Replace the ds64 FourCC with something else.
        let mut bytes = synthetic_rf64(b"RF64");
        // Outer hdr(8) + WAVE(4) = offset 12 is the first chunk FourCC.
        bytes[12..16].copy_from_slice(b"junk");
        let mut cur = Cursor::new(bytes);
        let err = Walker::open_rf64(&mut cur).unwrap_err();
        assert!(format!("{err}").contains("not 'ds64'"));
    }

    #[test]
    fn open_root_still_rejects_rf64_magic() {
        // The base constructor stays strict on RIFF.
        let bytes = synthetic_rf64(b"RF64");
        let mut cur = Cursor::new(bytes);
        let err = Walker::open_root(&mut cur).unwrap_err();
        assert!(format!("{err}").contains("not 'RIFF'"));
    }

    #[test]
    fn descend_into_nested_list_chunk() {
        // RIFF / WAVE { LIST(INFO) { INAM "Hi" } }
        let mut v = Vec::new();
        // outer
        v.extend_from_slice(b"RIFF");
        // ckSize = 4 (WAVE) + 8 (LIST hdr) + 4 (INFO) + 8 (INAM hdr) + 2 (body) = 26
        v.extend_from_slice(&26u32.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        // LIST
        v.extend_from_slice(b"LIST");
        // ckSize = 4 (INFO) + 8 (INAM hdr) + 2 (body) = 14
        v.extend_from_slice(&14u32.to_le_bytes());
        v.extend_from_slice(b"INFO");
        // INAM child
        v.extend_from_slice(b"INAM");
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(b"Hi");

        let mut cur = Cursor::new(v);
        let mut outer = Walker::open_root(&mut cur).unwrap();
        let list = outer.read_next().unwrap().unwrap();
        assert!(list.is_group());
        assert_eq!(&list.id, b"LIST");
        let form = outer.read_inner_form_type(&list).unwrap();
        assert_eq!(&form, b"INFO");
        // Manually consume the LIST body (children + pad) — the
        // INFO sub-walker bytes are accounted against the *outer*
        // walker via skip after reading.
        let body_len = list.size as i64 - 4; // minus the form-type word
        outer.inner.seek(SeekFrom::Current(body_len)).unwrap();
        // Tell the outer walker we read those bytes.
        outer.consumed += body_len as u64;
        assert!(outer.read_next().unwrap().is_none());
    }
}
