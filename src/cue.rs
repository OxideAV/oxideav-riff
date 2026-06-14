//! Typed decoder for the WAV / RIFF `cue ` cue-points chunk.
//!
//! A `cue ` chunk identifies a series of positions ("cue points") in the
//! waveform sample stream — markers a player can seek to. The 1991 RIFF
//! MCI spec defines its body as a 4-byte count followed by that many
//! fixed-width cue-point records:
//!
//! ```text
//! cue ( <ckSize:u32 LE>
//!   <dwCuePoints:u32 LE>     // count of cue-point records that follow
//!   <cue-point> ...          // dwCuePoints records, 24 bytes each
//! )
//!
//! cue-point := struct {
//!     DWORD  dwName;          // unique identifier for this cue point
//!     DWORD  dwPosition;      // sequential sample number in the play order
//!     FOURCC fccChunk;        // chunk ID holding the cue ('data' / 'slnt')
//!     DWORD  dwChunkStart;    // byte offset of that chunk in the 'wavl' data
//!     DWORD  dwBlockStart;    // byte offset of the enclosing block
//!     DWORD  dwSampleOffset;  // sample offset of the point within the block
//! }
//! ```
//!
//! Every multi-byte field is little-endian. `fccChunk` is a raw FourCC
//! (`data` for a `data`/PCM chunk, `slnt` for a silent chunk) and is kept
//! as a `[u8; 4]` so non-`data`/`slnt` values round-trip verbatim.
//!
//! The interpretation of `dwChunkStart` / `dwBlockStart` /
//! `dwSampleOffset` depends on whether the file wraps its samples in a
//! `wavl` LIST or carries a single `data` chunk, and on whether that data
//! is PCM or compressed; this decoder records the raw values and leaves
//! that interpretation to the caller (it has no access to the surrounding
//! chunk tree). The spec's worked tables for those cases are reproduced
//! in the crate README.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "Cue-Points Chunk" (the `<cue-ck>` / `<cue-point>` grammar, the
//!   per-field descriptions, and the file-position worked examples).

use crate::error::{Error, Result};

/// FourCC of the cue-points chunk (note the trailing space).
pub const FOURCC_CUE: [u8; 4] = *b"cue ";

/// Size in bytes of one on-wire `<cue-point>` record (six 32-bit
/// fields).
pub const CUE_POINT_LEN: usize = 24;

/// A single decoded cue-point record.
///
/// All six fields are the raw little-endian DWORDs from the wire;
/// `fcc_chunk` is the raw 4-byte FourCC. The semantics of the three
/// offset fields depend on the surrounding file layout (see the module
/// docs); this struct intentionally does not resolve them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CuePoint {
    /// `dwName` — the unique identifier of this cue point. Other chunks
    /// (`plst`, `labl`, `note`, `ltxt`) reference a cue point by this
    /// value, so it must be unique within the chunk.
    pub name: u32,
    /// `dwPosition` — the sequential sample number of this cue point
    /// within the play order defined by the `plst` playlist chunk.
    pub position: u32,
    /// `fccChunk` — the FourCC of the chunk that contains the cue point
    /// (`data` for a data/PCM chunk, `slnt` for a silent chunk).
    pub fcc_chunk: [u8; 4],
    /// `dwChunkStart` — byte offset of the start of the chunk named by
    /// `fcc_chunk`, relative to the start of the `wavl` LIST data
    /// section (zero when there is a single `data` chunk and no `wavl`).
    pub chunk_start: u32,
    /// `dwBlockStart` — byte offset of the start of the block holding
    /// the cue point, relative to the start of the `wavl` LIST data
    /// section. For compressed data this is where decompression may
    /// begin.
    pub block_start: u32,
    /// `dwSampleOffset` — sample offset of the cue point relative to the
    /// start of the block named by `block_start`.
    pub sample_offset: u32,
}

impl CuePoint {
    /// Decode one 24-byte cue-point record from `raw`.
    ///
    /// `raw` must be exactly [`CUE_POINT_LEN`] bytes; otherwise an
    /// `Error::invalid` is returned.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() != CUE_POINT_LEN {
            return Err(Error::invalid("RIFF: cue-point record is not 24 bytes"));
        }
        let dw =
            |off: usize| u32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]);
        let mut fcc = [0u8; 4];
        fcc.copy_from_slice(&raw[8..12]);
        Ok(CuePoint {
            name: dw(0),
            position: dw(4),
            fcc_chunk: fcc,
            chunk_start: dw(12),
            block_start: dw(16),
            sample_offset: dw(20),
        })
    }

    /// `true` if `fcc_chunk` names a `data` chunk.
    pub fn is_data(&self) -> bool {
        &self.fcc_chunk == b"data"
    }

    /// `true` if `fcc_chunk` names a `slnt` (silent) chunk.
    pub fn is_silent(&self) -> bool {
        &self.fcc_chunk == b"slnt"
    }
}

/// A decoded `cue ` chunk: the ordered list of cue points it carries.
///
/// On-wire order is preserved. The decoded record count must match the
/// declared `dwCuePoints` header, and the chunk body length must be
/// exactly `4 + dwCuePoints * 24` bytes — a short or over-long body is
/// rejected rather than silently truncated, so a malformed chunk does
/// not yield a partially-populated list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueChunk {
    points: Vec<CuePoint>,
}

impl CueChunk {
    /// An empty `cue ` chunk (zero cue points).
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode a `cue ` chunk body (already pad-stripped by the walker).
    ///
    /// The body is `<dwCuePoints:u32 LE>` followed by `dwCuePoints`
    /// records of [`CUE_POINT_LEN`] bytes each. The declared count must
    /// account for exactly the remaining body length; a mismatch (the
    /// header claims more or fewer points than the body holds) is an
    /// `Error::invalid`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 4 {
            return Err(Error::invalid("RIFF: cue chunk shorter than 4-byte count"));
        }
        let count = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
        let records = &body[4..];
        let expected = count
            .checked_mul(CUE_POINT_LEN)
            .ok_or_else(|| Error::invalid("RIFF: cue point count overflows"))?;
        if records.len() != expected {
            return Err(Error::invalid(
                "RIFF: cue chunk body length disagrees with dwCuePoints",
            ));
        }
        let mut points = Vec::with_capacity(count);
        for i in 0..count {
            let off = i * CUE_POINT_LEN;
            points.push(CuePoint::parse(&records[off..off + CUE_POINT_LEN])?);
        }
        Ok(CueChunk { points })
    }

    /// The cue points in on-wire order.
    pub fn points(&self) -> &[CuePoint] {
        &self.points
    }

    /// Number of cue points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// `true` if there are no cue points.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// First cue point whose `dwName` equals `name`, or `None`.
    ///
    /// The spec requires `dwName` to be unique within a chunk, so this
    /// is effectively a keyed lookup; it tolerates a non-conforming
    /// duplicate by returning the first match.
    pub fn by_name(&self, name: u32) -> Option<&CuePoint> {
        self.points.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one 24-byte cue-point record.
    fn rec(
        name: u32,
        position: u32,
        fcc: &[u8; 4],
        chunk_start: u32,
        block_start: u32,
        sample_offset: u32,
    ) -> Vec<u8> {
        let mut v = Vec::with_capacity(CUE_POINT_LEN);
        v.extend_from_slice(&name.to_le_bytes());
        v.extend_from_slice(&position.to_le_bytes());
        v.extend_from_slice(fcc);
        v.extend_from_slice(&chunk_start.to_le_bytes());
        v.extend_from_slice(&block_start.to_le_bytes());
        v.extend_from_slice(&sample_offset.to_le_bytes());
        v
    }

    /// Build a `cue ` chunk body from a list of records (count prefix +
    /// records).
    fn cue_body(records: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for r in records {
            body.extend_from_slice(r);
        }
        body
    }

    #[test]
    fn point_record_is_24_bytes() {
        assert_eq!(CUE_POINT_LEN, 24);
        let r = rec(1, 2, b"data", 3, 4, 5);
        assert_eq!(r.len(), 24);
    }

    #[test]
    fn parse_single_pcm_data_point() {
        // Single 'data' chunk: chunk_start / block_start zero, only
        // sample_offset is meaningful (the spec's single-data PCM case).
        let body = cue_body(&[rec(1, 44_100, b"data", 0, 0, 44_100)]);
        let cue = CueChunk::parse(&body).unwrap();
        assert_eq!(cue.len(), 1);
        let p = &cue.points()[0];
        assert_eq!(p.name, 1);
        assert_eq!(p.position, 44_100);
        assert!(p.is_data());
        assert!(!p.is_silent());
        assert_eq!(p.chunk_start, 0);
        assert_eq!(p.block_start, 0);
        assert_eq!(p.sample_offset, 44_100);
    }

    #[test]
    fn parse_multiple_points_preserves_order() {
        let body = cue_body(&[
            rec(10, 0, b"data", 0, 0, 0),
            rec(20, 1000, b"slnt", 8, 8, 12),
            rec(30, 2000, b"data", 64, 64, 0),
        ]);
        let cue = CueChunk::parse(&body).unwrap();
        assert_eq!(cue.len(), 3);
        assert_eq!(cue.points()[0].name, 10);
        assert_eq!(cue.points()[1].name, 20);
        assert_eq!(cue.points()[2].name, 30);
        assert!(cue.points()[1].is_silent());
        assert_eq!(cue.points()[1].block_start, 8);
        assert_eq!(cue.points()[1].sample_offset, 12);
    }

    #[test]
    fn by_name_finds_point() {
        let body = cue_body(&[rec(7, 1, b"data", 0, 0, 0), rec(9, 2, b"data", 0, 0, 0)]);
        let cue = CueChunk::parse(&body).unwrap();
        assert_eq!(cue.by_name(9).unwrap().position, 2);
        assert!(cue.by_name(99).is_none());
    }

    #[test]
    fn empty_cue_chunk() {
        let body = cue_body(&[]);
        let cue = CueChunk::parse(&body).unwrap();
        assert!(cue.is_empty());
        assert_eq!(cue.len(), 0);
    }

    #[test]
    fn body_shorter_than_count_is_rejected() {
        // Header claims 4 bytes minimum but body is empty.
        let err = CueChunk::parse(&[]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 4-byte count"));
    }

    #[test]
    fn count_disagreeing_with_body_is_rejected() {
        // Declares 2 points but supplies only one record's worth.
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&rec(1, 0, b"data", 0, 0, 0));
        let err = CueChunk::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with dwCuePoints"));
    }

    #[test]
    fn overlong_body_is_rejected() {
        // Declares 1 point but supplies two records.
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&rec(1, 0, b"data", 0, 0, 0));
        body.extend_from_slice(&rec(2, 0, b"data", 0, 0, 0));
        let err = CueChunk::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with dwCuePoints"));
    }

    #[test]
    fn point_parse_rejects_wrong_length() {
        let err = CuePoint::parse(&[0u8; 23]).unwrap_err();
        assert!(format!("{err}").contains("not 24 bytes"));
    }

    #[test]
    fn non_data_slnt_fourcc_round_trips() {
        let body = cue_body(&[rec(1, 0, b"junk", 0, 0, 0)]);
        let cue = CueChunk::parse(&body).unwrap();
        let p = &cue.points()[0];
        assert_eq!(&p.fcc_chunk, b"junk");
        assert!(!p.is_data());
        assert!(!p.is_silent());
    }
}
