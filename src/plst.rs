//! Typed decoder for the WAV / RIFF `plst` playlist chunk.
//!
//! A `plst` chunk specifies a play order for a series of cue points: it
//! turns the unordered marker table of the `cue ` chunk into a sequence
//! of segments to render, each segment naming a cue point, the number of
//! samples to play from it, and how many times to loop that section. The
//! 1991 RIFF MCI spec defines its body as a 4-byte count followed by that
//! many fixed-width play-segment records:
//!
//! ```text
//! plst ( <ckSize:u32 LE>
//!   <dwSegments:u32 LE>       // count of play-segment records that follow
//!   <play-segment> ...        // dwSegments records, 12 bytes each
//! )
//!
//! play-segment := struct {
//!     DWORD dwName;           // cue point name (matches a 'cue ' dwName)
//!     DWORD dwLength;         // length of the section in samples
//!     DWORD dwLoops;          // number of times to play the section
//! }
//! ```
//!
//! Every multi-byte field is little-endian. `dwName` references a cue
//! point by the unique identifier carried in the `cue ` chunk's
//! `dwName` field; this decoder records the value but does not resolve
//! the reference (it has no view of the surrounding chunk tree). The
//! resolution against a [`crate::CueChunk`] is left to the caller.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "Playlist Chunk" (the `<playlist-ck>` / `<play-segment>` grammar
//!   and the per-field descriptions).

use crate::error::{Error, Result};

/// FourCC of the playlist chunk.
pub const FOURCC_PLST: [u8; 4] = *b"plst";

/// Size in bytes of one on-wire `<play-segment>` record (three 32-bit
/// fields).
pub const PLAY_SEGMENT_LEN: usize = 12;

/// A single decoded play-segment record.
///
/// All three fields are the raw little-endian DWORDs from the wire.
/// `name` references a cue point in the companion `cue ` chunk by its
/// `dwName`; this struct does not resolve that reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlaySegment {
    /// `dwName` — the cue point name this segment plays. Must match one
    /// of the `dwName` values listed in the `cue ` cue-point table.
    pub name: u32,
    /// `dwLength` — the length of the section to play, in samples.
    pub length: u32,
    /// `dwLoops` — the number of times to play the section.
    pub loops: u32,
}

impl PlaySegment {
    /// Decode one 12-byte play-segment record from `raw`.
    ///
    /// `raw` must be exactly [`PLAY_SEGMENT_LEN`] bytes; otherwise an
    /// `Error::invalid` is returned.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() != PLAY_SEGMENT_LEN {
            return Err(Error::invalid("RIFF: play-segment record is not 12 bytes"));
        }
        let dw =
            |off: usize| u32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]);
        Ok(PlaySegment {
            name: dw(0),
            length: dw(4),
            loops: dw(8),
        })
    }

    /// Serialize this play segment as a [`PLAY_SEGMENT_LEN`]-byte record —
    /// the three fields in `dwName` / `dwLength` / `dwLoops` order, each
    /// little-endian. Exact inverse of [`PlaySegment::parse`].
    pub fn encode(&self) -> [u8; PLAY_SEGMENT_LEN] {
        let mut b = [0u8; PLAY_SEGMENT_LEN];
        b[0..4].copy_from_slice(&self.name.to_le_bytes());
        b[4..8].copy_from_slice(&self.length.to_le_bytes());
        b[8..12].copy_from_slice(&self.loops.to_le_bytes());
        b
    }
}

/// A decoded `plst` chunk: the ordered list of play segments it carries.
///
/// On-wire order is the play order. The decoded record count must match
/// the declared `dwSegments` header, and the chunk body length must be
/// exactly `4 + dwSegments * 12` bytes — a short or over-long body is
/// rejected rather than silently truncated, so a malformed chunk does
/// not yield a partially-populated list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Playlist {
    segments: Vec<PlaySegment>,
}

impl Playlist {
    /// An empty `plst` chunk (zero play segments).
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode a `plst` chunk body (already pad-stripped by the walker).
    ///
    /// The body is `<dwSegments:u32 LE>` followed by `dwSegments`
    /// records of [`PLAY_SEGMENT_LEN`] bytes each. The declared count
    /// must account for exactly the remaining body length; a mismatch
    /// (the header claims more or fewer segments than the body holds) is
    /// an `Error::invalid`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 4 {
            return Err(Error::invalid("RIFF: plst chunk shorter than 4-byte count"));
        }
        let count = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
        let records = &body[4..];
        let expected = count
            .checked_mul(PLAY_SEGMENT_LEN)
            .ok_or_else(|| Error::invalid("RIFF: plst segment count overflows"))?;
        if records.len() != expected {
            return Err(Error::invalid(
                "RIFF: plst chunk body length disagrees with dwSegments",
            ));
        }
        let mut segments = Vec::with_capacity(count);
        for i in 0..count {
            let off = i * PLAY_SEGMENT_LEN;
            segments.push(PlaySegment::parse(&records[off..off + PLAY_SEGMENT_LEN])?);
        }
        Ok(Playlist { segments })
    }

    /// Build a `plst` chunk from a list of play segments (in play order).
    ///
    /// The write-side counterpart of [`Playlist::parse`]: emit the body
    /// with [`Playlist::encode_body`], which writes the `dwSegments`
    /// count prefix that always agrees with the record count.
    pub fn from_segments(segments: Vec<PlaySegment>) -> Self {
        Playlist { segments }
    }

    /// Append a play segment, returning `self` for chaining.
    pub fn push(mut self, segment: PlaySegment) -> Self {
        self.segments.push(segment);
        self
    }

    /// Serialize this `plst` chunk *body* — the `dwSegments` count
    /// (always equal to the record count) followed by the 12-byte
    /// play-segment records in play order. Exact inverse of
    /// [`Playlist::parse`].
    pub fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.segments.len() * PLAY_SEGMENT_LEN);
        out.extend_from_slice(&(self.segments.len() as u32).to_le_bytes());
        for segment in &self.segments {
            out.extend_from_slice(&segment.encode());
        }
        out
    }

    /// The play segments in on-wire (play) order.
    pub fn segments(&self) -> &[PlaySegment] {
        &self.segments
    }

    /// Number of play segments.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// `true` if there are no play segments.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// First play segment whose `dwName` equals `name`, or `None`.
    ///
    /// Unlike a cue point's `dwName`, a playlist may legitimately
    /// reference the same cue point in more than one segment (e.g. to
    /// play a section, then replay it with a different loop count), so
    /// this returns the first match in play order.
    pub fn by_name(&self, name: u32) -> Option<&PlaySegment> {
        self.segments.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one 12-byte play-segment record.
    fn seg(name: u32, length: u32, loops: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(PLAY_SEGMENT_LEN);
        v.extend_from_slice(&name.to_le_bytes());
        v.extend_from_slice(&length.to_le_bytes());
        v.extend_from_slice(&loops.to_le_bytes());
        v
    }

    /// Build a `plst` chunk body from a list of records (count prefix +
    /// records).
    fn plst_body(records: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for r in records {
            body.extend_from_slice(r);
        }
        body
    }

    #[test]
    fn segment_record_is_12_bytes() {
        assert_eq!(PLAY_SEGMENT_LEN, 12);
        let s = seg(1, 2, 3);
        assert_eq!(s.len(), 12);
    }

    #[test]
    fn parse_single_segment() {
        let body = plst_body(&[seg(1, 44_100, 1)]);
        let pl = Playlist::parse(&body).unwrap();
        assert_eq!(pl.len(), 1);
        let s = &pl.segments()[0];
        assert_eq!(s.name, 1);
        assert_eq!(s.length, 44_100);
        assert_eq!(s.loops, 1);
    }

    #[test]
    fn parse_multiple_segments_preserves_order() {
        let body = plst_body(&[seg(10, 1000, 1), seg(20, 2000, 4), seg(10, 500, 2)]);
        let pl = Playlist::parse(&body).unwrap();
        assert_eq!(pl.len(), 3);
        assert_eq!(pl.segments()[0].name, 10);
        assert_eq!(pl.segments()[1].name, 20);
        assert_eq!(pl.segments()[1].loops, 4);
        // A cue point may appear in more than one play segment.
        assert_eq!(pl.segments()[2].name, 10);
        assert_eq!(pl.segments()[2].loops, 2);
    }

    #[test]
    fn by_name_finds_first_segment() {
        let body = plst_body(&[seg(7, 100, 1), seg(9, 200, 1), seg(7, 300, 5)]);
        let pl = Playlist::parse(&body).unwrap();
        assert_eq!(pl.by_name(9).unwrap().length, 200);
        // First match wins when a cue is referenced twice.
        assert_eq!(pl.by_name(7).unwrap().length, 100);
        assert!(pl.by_name(99).is_none());
    }

    #[test]
    fn empty_playlist() {
        let body = plst_body(&[]);
        let pl = Playlist::parse(&body).unwrap();
        assert!(pl.is_empty());
        assert_eq!(pl.len(), 0);
    }

    #[test]
    fn body_shorter_than_count_is_rejected() {
        let err = Playlist::parse(&[]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 4-byte count"));
    }

    #[test]
    fn count_disagreeing_with_body_is_rejected() {
        // Declares 2 segments but supplies only one record's worth.
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&seg(1, 0, 0));
        let err = Playlist::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with dwSegments"));
    }

    #[test]
    fn overlong_body_is_rejected() {
        // Declares 1 segment but supplies two records.
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&seg(1, 0, 0));
        body.extend_from_slice(&seg(2, 0, 0));
        let err = Playlist::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with dwSegments"));
    }

    #[test]
    fn segment_parse_rejects_wrong_length() {
        let err = PlaySegment::parse(&[0u8; 11]).unwrap_err();
        assert!(format!("{err}").contains("not 12 bytes"));
    }

    #[test]
    fn segment_encode_is_parse_inverse() {
        let raw = seg(20, 2000, 4);
        let s = PlaySegment::parse(&raw).unwrap();
        assert_eq!(&s.encode()[..], &raw[..]);
        assert_eq!(PlaySegment::parse(&s.encode()).unwrap(), s);
    }

    #[test]
    fn encode_body_round_trips() {
        let body = plst_body(&[seg(10, 1000, 1), seg(20, 2000, 4), seg(10, 500, 2)]);
        let pl = Playlist::parse(&body).unwrap();
        let encoded = pl.encode_body();
        assert_eq!(encoded, body);
        assert_eq!(Playlist::parse(&encoded).unwrap(), pl);
    }

    #[test]
    fn empty_encode_body_round_trips() {
        let pl = Playlist::new();
        assert_eq!(pl.encode_body(), 0u32.to_le_bytes().to_vec());
        assert!(Playlist::parse(&pl.encode_body()).unwrap().is_empty());
    }

    #[test]
    fn builder_encode_parse_round_trip() {
        let pl = Playlist::from_segments(vec![PlaySegment::parse(&seg(1, 100, 1)).unwrap()])
            .push(PlaySegment::parse(&seg(2, 200, 3)).unwrap());
        let encoded = pl.encode_body();
        let reparsed = Playlist::parse(&encoded).unwrap();
        assert_eq!(reparsed, pl);
        assert_eq!(encoded[0..4], 2u32.to_le_bytes());
    }
}
