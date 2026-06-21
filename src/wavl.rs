//! Typed decoders for the WAV `wavl` wave-data-list and its `slnt`
//! silence segments.
//!
//! Most WAV files store their sound samples in a single `data` chunk.
//! The 1991 RIFF MCI spec also defines an alternative, **scattered**
//! storage form: a `LIST` chunk whose list-type FourCC is `wavl`, holding
//! a sequence of `data` (sample) and `slnt` (silence) child chunks in
//! play order. The `wavl` form lets a writer represent long runs of
//! silence compactly — a `slnt` chunk records only a *count* of silent
//! samples rather than that many zero bytes — and lets an editor splice
//! takes without rewriting the whole waveform.
//!
//! ## Wire layout
//!
//! ```text
//! <wave-data>  -> { <data-ck> | <wave-list> }
//!
//! <wave-list>  -> LIST( 'wavl'
//!                   { <data-ck> | <silence-ck> }...   // play order
//!                 )
//!
//! <data-ck>    -> data( <wave-data:bytes> )           // sample bytes
//! <silence-ck> -> slnt( <dwSamples:u32 LE> )          // silent samples
//! ```
//!
//! A `fact` chunk is **required** whenever the waveform lives in a `wavl`
//! LIST (the [`crate::fact::Fact`] `dwSampleLength` carries the total
//! sample count, which a reader cannot derive cheaply from the scattered
//! `data` byte runs and silence counts alone).
//!
//! ## `slnt` semantics
//!
//! Per the spec's note, a `slnt` segment represents silence — *not*
//! necessarily a run of zero/baseline samples. A player holding the last
//! sample value before the silence avoids the click a hard jump to zero
//! would produce. This decoder records only the on-wire `dwSamples`
//! count; how a renderer fills the gap is its concern.
//!
//! ## What this module decodes
//!
//! - [`Silence`] — the fixed 4-byte `slnt` body (`dwSamples`).
//! - [`WaveDataList`] — a `wavl` LIST collected into an ordered list of
//!   [`WaveSegment`]s (`Data` byte runs and `Silence` sample counts),
//!   preserving on-wire order. The total `data`-byte count and total
//!   silent-sample count are exposed for cross-checking against `fmt ` /
//!   `fact`. Unrecognised child FourCCs are preserved verbatim (the spec
//!   instructs applications to ignore — but not reject — chunk IDs they
//!   do not understand).
//!
//! Like the other typed decoders in this crate, [`WaveDataList`] does not
//! walk the outer chunk tree itself: the caller uses the [`crate::Walker`]
//! to locate the `wavl` LIST, reads its list-type with
//! [`crate::Walker::read_inner_form_type`], opens a nested walker over the
//! body, and hands it to [`WaveDataList::collect_from`].
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "Storage of WAVE Data" (the `<wave-data>` / `<wave-list>` /
//!   `<data-ck>` / `<silence-ck>` grammar and the `slnt` `dwSamples`
//!   field) plus the "FACT Chunk" required-for-`wavl` rule.

use crate::error::{Error, Result};

/// FourCC of the `wavl` wave-data-list (a `LIST` list-type).
pub const FOURCC_WAVL: [u8; 4] = *b"wavl";

/// FourCC of a `slnt` silence chunk.
pub const FOURCC_SLNT: [u8; 4] = *b"slnt";

/// FourCC of a `data` sample chunk (the leaf carried both bare and
/// inside a `wavl` LIST).
pub const FOURCC_DATA: [u8; 4] = *b"data";

/// On-wire length of a `slnt` chunk body: the single `dwSamples` DWORD.
pub const SLNT_LEN: usize = 4;

/// A decoded `slnt` silence chunk.
///
/// Carries the count of silent samples the segment spans. The value is a
/// *sample* count (per the spec's `dwSamples` field), independent of the
/// channel count or sample width — those come from the surrounding
/// `fmt ` chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Silence {
    /// `dwSamples` — the number of silent samples this segment spans.
    pub samples: u32,
}

impl Silence {
    /// Construct a silence segment spanning `samples` silent samples.
    pub const fn new(samples: u32) -> Self {
        Silence { samples }
    }

    /// Decode a `slnt` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be exactly [`SLNT_LEN`] bytes — the single
    /// `dwSamples` DWORD. The spec defines no extension mechanism for the
    /// `slnt` body, so an off-length body is an `Error::invalid` rather
    /// than a forward-compatible trailer.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() != SLNT_LEN {
            return Err(Error::invalid(
                "RIFF: slnt chunk body is not exactly 4 bytes (dwSamples)",
            ));
        }
        Ok(Silence {
            samples: u32::from_le_bytes([body[0], body[1], body[2], body[3]]),
        })
    }

    /// Serialize this `slnt` chunk *body* (the 4-byte little-endian
    /// `dwSamples`, without the 8-byte chunk header or pad byte). The
    /// exact inverse of [`Silence::parse`].
    pub fn encode_body(&self) -> Vec<u8> {
        self.samples.to_le_bytes().to_vec()
    }

    /// Serialize the whole `slnt` chunk — the 8-byte header (`slnt` +
    /// `ckSize` = 4) wrapping [`Silence::encode_body`]. The body is an
    /// even length, so no pad byte is emitted.
    pub fn encode_chunk(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + SLNT_LEN);
        // SLNT_LEN (4) is always within u32 range; the encode never fails.
        crate::chunk::encode_chunk(&mut out, &FOURCC_SLNT, &self.encode_body())
            .expect("slnt body is 4 bytes, well within u32 ckSize");
        out
    }
}

/// One segment of a `wavl` wave-data-list, in play order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaveSegment {
    /// A `data` chunk: a run of raw sample bytes (PCM or compressed,
    /// per the surrounding `fmt `). The bytes are kept verbatim; their
    /// sample count depends on `fmt `, so it is not computed here.
    Data(Vec<u8>),
    /// A `slnt` chunk: a [`Silence`] run of `dwSamples` silent samples.
    Silence(Silence),
    /// An unrecognised child chunk preserved verbatim (FourCC + body).
    ///
    /// The spec instructs applications to ignore — but not drop — chunk
    /// IDs they do not understand, so a `wavl` LIST round-trips even when
    /// it carries a vendor segment this decoder does not model.
    Other([u8; 4], Vec<u8>),
}

impl WaveSegment {
    /// `true` if this is a `data` sample segment.
    pub fn is_data(&self) -> bool {
        matches!(self, WaveSegment::Data(_))
    }

    /// `true` if this is a `slnt` silence segment.
    pub fn is_silence(&self) -> bool {
        matches!(self, WaveSegment::Silence(_))
    }

    /// The raw sample bytes if this is a `data` segment, else `None`.
    pub fn data_bytes(&self) -> Option<&[u8]> {
        match self {
            WaveSegment::Data(b) => Some(b),
            _ => None,
        }
    }

    /// The silent-sample count if this is a `slnt` segment, else `None`.
    pub fn silence_samples(&self) -> Option<u32> {
        match self {
            WaveSegment::Silence(s) => Some(s.samples),
            _ => None,
        }
    }
}

/// A decoded `wavl` wave-data-list.
///
/// Holds the `data` / `slnt` child segments in their on-wire (play)
/// order. Build one by walking a `wavl` LIST with
/// [`WaveDataList::collect_from`], or assemble it from scratch with
/// [`WaveDataList::new`] + the `push_*` builders and re-emit it with
/// [`WaveDataList::encode_chunk`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WaveDataList {
    segments: Vec<WaveSegment>,
}

impl WaveDataList {
    /// An empty wave-data-list.
    pub fn new() -> Self {
        WaveDataList {
            segments: Vec::new(),
        }
    }

    /// The segments in play order.
    pub fn segments(&self) -> &[WaveSegment] {
        &self.segments
    }

    /// Number of segments (both `data` and `slnt`, plus any preserved
    /// vendor segments).
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// `true` if the list holds no segments.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Total number of raw `data` sample bytes across every `data`
    /// segment. This is a *byte* count, not a sample count — the sample
    /// count depends on `fmt ` (`wBlockAlign` / compression) and is the
    /// caller's to compute.
    pub fn total_data_bytes(&self) -> u64 {
        self.segments
            .iter()
            .filter_map(WaveSegment::data_bytes)
            .map(|b| b.len() as u64)
            .sum()
    }

    /// Total number of silent samples across every `slnt` segment.
    ///
    /// Returned as a `u64` so a long file with many silence runs cannot
    /// overflow a single `u32` `dwSamples`.
    pub fn total_silent_samples(&self) -> u64 {
        self.segments
            .iter()
            .filter_map(WaveSegment::silence_samples)
            .map(u64::from)
            .sum()
    }

    /// Append a `data` sample segment.
    pub fn push_data(&mut self, bytes: impl Into<Vec<u8>>) {
        self.segments.push(WaveSegment::Data(bytes.into()));
    }

    /// Append a `slnt` silence segment of `samples` silent samples.
    pub fn push_silence(&mut self, samples: u32) {
        self.segments
            .push(WaveSegment::Silence(Silence::new(samples)));
    }

    /// Append a pre-built segment (including a preserved `Other` one).
    pub fn push_segment(&mut self, seg: WaveSegment) {
        self.segments.push(seg);
    }

    /// Decode one child chunk `(FourCC, body)` into a [`WaveSegment`] and
    /// append it, dispatching on the FourCC: `data` → [`WaveSegment::Data`],
    /// `slnt` → [`WaveSegment::Silence`] (a malformed `slnt` body is
    /// rejected), anything else → [`WaveSegment::Other`] preserved
    /// verbatim.
    pub fn push_child(&mut self, fourcc: [u8; 4], body: &[u8]) -> Result<()> {
        match fourcc {
            FOURCC_DATA => self.push_data(body.to_vec()),
            FOURCC_SLNT => self
                .segments
                .push(WaveSegment::Silence(Silence::parse(body)?)),
            other => self.segments.push(WaveSegment::Other(other, body.to_vec())),
        }
        Ok(())
    }

    /// Collect a whole `wavl` LIST sub-tree from a [`crate::Walker`]
    /// already positioned over the `wavl` list body (i.e. constructed
    /// after the caller read the `wavl` list-type with
    /// [`crate::Walker::read_inner_form_type`]).
    ///
    /// Each child chunk is read in full and dispatched by FourCC. The
    /// walker's parent-budget enforcement still applies, so a child
    /// overflowing the `LIST` body surfaces as the walker's existing
    /// `Error::invalid`. A `slnt` child whose body is not exactly 4 bytes
    /// is rejected here.
    pub fn collect_from<R: std::io::Read + std::io::Seek + ?Sized>(
        walker: &mut crate::Walker<'_, R>,
    ) -> Result<Self> {
        if walker.form_type() != FOURCC_WAVL {
            return Err(Error::invalid(
                "RIFF: collect_from called on a non-wavl LIST",
            ));
        }
        let mut list = Self::new();
        while let Some(chunk) = walker.read_next()? {
            let body = walker.read_body(&chunk)?;
            list.push_child(chunk.id, &body)?;
        }
        Ok(list)
    }

    /// Serialize the `wavl` LIST *body*: the `wavl` list-type FourCC
    /// followed by one framed child chunk per segment, in order. Each
    /// child is framed with [`crate::chunk::encode_chunk`], so an
    /// odd-length `data` segment gets its RIFF pad byte.
    pub fn encode_list_body(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(&FOURCC_WAVL);
        for seg in &self.segments {
            match seg {
                WaveSegment::Data(b) => crate::chunk::encode_chunk(&mut out, &FOURCC_DATA, b)?,
                WaveSegment::Silence(s) => {
                    crate::chunk::encode_chunk(&mut out, &FOURCC_SLNT, &s.encode_body())?
                }
                WaveSegment::Other(id, b) => crate::chunk::encode_chunk(&mut out, id, b)?,
            }
        }
        Ok(out)
    }

    /// Serialize the whole `wavl` LIST chunk — the `LIST` header (with the
    /// computed `ckSize`) wrapping [`WaveDataList::encode_list_body`].
    /// The exact inverse of [`WaveDataList::collect_from`].
    pub fn encode_chunk(&self) -> Result<Vec<u8>> {
        let body = self.encode_list_body()?;
        let mut out = Vec::new();
        crate::chunk::encode_chunk(&mut out, &crate::chunk::FOURCC_LIST, &body)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn fourcc_and_len_constants() {
        assert_eq!(&FOURCC_WAVL, b"wavl");
        assert_eq!(&FOURCC_SLNT, b"slnt");
        assert_eq!(&FOURCC_DATA, b"data");
        assert_eq!(SLNT_LEN, 4);
    }

    #[test]
    fn silence_parses_dword() {
        let s = Silence::parse(&44_100u32.to_le_bytes()).unwrap();
        assert_eq!(s.samples, 44_100);
    }

    #[test]
    fn silence_zero_samples() {
        let s = Silence::parse(&[0, 0, 0, 0]).unwrap();
        assert_eq!(s.samples, 0);
    }

    #[test]
    fn silence_rejects_short_body() {
        let err = Silence::parse(&[0, 0, 0]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 4 bytes"));
    }

    #[test]
    fn silence_rejects_long_body() {
        let err = Silence::parse(&[0, 0, 0, 0, 0]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 4 bytes"));
    }

    #[test]
    fn silence_body_round_trips() {
        let s = Silence::new(1_234_567);
        let body = s.encode_body();
        assert_eq!(body, 1_234_567u32.to_le_bytes());
        assert_eq!(Silence::parse(&body).unwrap(), s);
    }

    #[test]
    fn silence_chunk_emits_header_and_no_pad() {
        let s = Silence::new(7);
        let chunk = s.encode_chunk();
        // 8-byte header + 4-byte body, even → no pad byte.
        assert_eq!(chunk.len(), 12);
        assert_eq!(&chunk[0..4], b"slnt");
        assert_eq!(chunk[4..8], 4u32.to_le_bytes());
        assert_eq!(chunk[8..12], 7u32.to_le_bytes());
    }

    /// Build a `wavl` LIST blob from a list of `(fourcc, body)` children.
    fn wavl_blob(children: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"wavl");
        for (id, b) in children {
            crate::chunk::encode_chunk(&mut body, id, b).unwrap();
        }
        let mut out = Vec::new();
        crate::chunk::encode_chunk(&mut out, b"LIST", &body).unwrap();
        out
    }

    fn open_wavl_walker(blob: &[u8]) -> Vec<WaveSegment> {
        let mut cur = Cursor::new(blob.to_vec());
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        assert!(header.is_group());
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        assert_eq!(walker.form_type(), FOURCC_WAVL);
        WaveDataList::collect_from(&mut walker)
            .unwrap()
            .segments()
            .to_vec()
    }

    #[test]
    fn collect_from_walks_data_and_slnt_in_order() {
        let blob = wavl_blob(&[
            (b"data", &[1, 2, 3, 4]),
            (b"slnt", &100u32.to_le_bytes()),
            (b"data", &[5, 6]),
        ]);
        let segs = open_wavl_walker(&blob);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].data_bytes(), Some(&[1, 2, 3, 4][..]));
        assert_eq!(segs[1].silence_samples(), Some(100));
        assert_eq!(segs[2].data_bytes(), Some(&[5, 6][..]));
    }

    #[test]
    fn collect_from_handles_odd_length_data_pad() {
        // A 3-byte data run needs a RIFF pad byte; the walker must
        // re-sync onto the next child.
        let blob = wavl_blob(&[
            (b"data", &[0xAA, 0xBB, 0xCC]),
            (b"slnt", &5u32.to_le_bytes()),
        ]);
        let segs = open_wavl_walker(&blob);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].data_bytes(), Some(&[0xAA, 0xBB, 0xCC][..]));
        assert_eq!(segs[1].silence_samples(), Some(5));
    }

    #[test]
    fn collect_from_preserves_unknown_child() {
        let blob = wavl_blob(&[(b"data", &[1, 2]), (b"junk", &[9, 9, 9, 9])]);
        let segs = open_wavl_walker(&blob);
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[1], WaveSegment::Other(id, b)
            if id == b"junk" && b == &[9, 9, 9, 9]));
    }

    #[test]
    fn collect_from_rejects_malformed_slnt() {
        let blob = wavl_blob(&[(b"slnt", &[0, 0, 0])]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let err = WaveDataList::collect_from(&mut walker).unwrap_err();
        assert!(format!("{err}").contains("not exactly 4 bytes"));
    }

    #[test]
    fn collect_from_rejects_non_wavl_list() {
        // A LIST INFO blob fed to the wavl collector.
        let mut body = Vec::new();
        body.extend_from_slice(b"INFO");
        crate::chunk::encode_chunk(&mut body, b"INAM", b"x\0").unwrap();
        let mut out = Vec::new();
        crate::chunk::encode_chunk(&mut out, b"LIST", &body).unwrap();
        let mut cur = Cursor::new(out);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let err = WaveDataList::collect_from(&mut walker).unwrap_err();
        assert!(format!("{err}").contains("non-wavl LIST"));
    }

    #[test]
    fn totals_account_data_bytes_and_silent_samples() {
        let mut list = WaveDataList::new();
        list.push_data(vec![0u8; 10]);
        list.push_silence(48_000);
        list.push_data(vec![0u8; 6]);
        list.push_silence(2_000);
        assert_eq!(list.len(), 4);
        assert_eq!(list.total_data_bytes(), 16);
        assert_eq!(list.total_silent_samples(), 50_000);
    }

    #[test]
    fn total_silent_samples_cannot_overflow_u32() {
        let mut list = WaveDataList::new();
        list.push_silence(u32::MAX);
        list.push_silence(u32::MAX);
        assert_eq!(list.total_silent_samples(), 2 * u64::from(u32::MAX));
    }

    #[test]
    fn builder_round_trips_through_collect() {
        let mut list = WaveDataList::new();
        list.push_data(vec![1, 2, 3, 4]);
        list.push_silence(256);
        list.push_data(vec![7, 7, 7]); // odd → pad on encode
        list.push_silence(0);
        let chunk = list.encode_chunk().unwrap();

        let segs = open_wavl_walker(&chunk);
        assert_eq!(segs.len(), 4);
        let mut collected = WaveDataList::new();
        for s in segs {
            collected.push_segment(s);
        }
        assert_eq!(collected, list);
    }

    #[test]
    fn encode_list_body_starts_with_wavl_type() {
        let mut list = WaveDataList::new();
        list.push_silence(1);
        let body = list.encode_list_body().unwrap();
        assert_eq!(&body[0..4], b"wavl");
    }

    #[test]
    fn empty_list_round_trips() {
        let list = WaveDataList::new();
        assert!(list.is_empty());
        let chunk = list.encode_chunk().unwrap();
        let segs = open_wavl_walker(&chunk);
        assert!(segs.is_empty());
    }

    #[test]
    fn push_child_dispatches_by_fourcc() {
        let mut list = WaveDataList::new();
        list.push_child(*b"data", &[1, 2, 3]).unwrap();
        list.push_child(*b"slnt", &9u32.to_le_bytes()).unwrap();
        list.push_child(*b"vend", &[0xFF]).unwrap();
        assert!(list.segments()[0].is_data());
        assert!(list.segments()[1].is_silence());
        assert!(matches!(&list.segments()[2], WaveSegment::Other(id, _) if id == b"vend"));
    }
}
