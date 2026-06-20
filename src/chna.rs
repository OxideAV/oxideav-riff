//! Typed decoder for the BW64 / RF64 `chna` chunk (ADM channel allocation).
//!
//! The `chna` chunk binds the *Audio Definition Model* (ADM) per-track
//! identifiers carried in a file's XML metadata (`axml` / `bxml` / `sxml`)
//! to the physical track indices in the `data` chunk. It is the binary
//! "channel-assignment" table object-based-audio workflows (Dolby Atmos
//! Master File, Auro-3D, MPEG-H 3D Audio production) use to map each
//! interleaved PCM track to its `audioTrackUID`, `audioTrackFormatID` (or
//! an `audioChannelFormat` for bare linear PCM), and optional
//! `audioPackFormatID`.
//!
//! ```text
//! chna ( <ckSize:u32 LE>
//!   <numTracks:u16 LE>   // tracks used in the file
//!   <numUIDs:u16 LE>     // UIDs used (>= numTracks; one track may carry many)
//!   <audioID> ...        // N records, 40 bytes each; N = (ckSize - 4) / 40
//! )
//!
//! audioID := struct {           // 40 bytes, fixed
//!     WORD     trackIndex;       // 1-based index into the 'data' interleave; 0 = unused
//!     CHAR[12] UID;             // audioTrackUID, "ATU_xxxxxxxx"
//!     CHAR[14] trackRef;        // audioTrackFormatID "AT_xxxxxxxx_xx"
//!                               //   (or audioChannelFormat "AC_xxxxxxxx_00" for bare PCM)
//!     CHAR[11] packRef;         // audioPackFormatID "AP_xxxxxxxx", or 11 NULs when none
//!     CHAR     pad;             // single byte so each record is even-length
//! }
//! ```
//!
//! `trackIndex` is little-endian; the character-array fields are
//! fixed-width raw ASCII that are **not** null-terminated — the value
//! fills exactly the field width, so they are preserved verbatim as byte
//! arrays and compared at full width (an all-zero `packRef` means "no
//! pack", a `0` `trackIndex` marks an unused over-provisioned slot).
//!
//! ## Over-provisioning
//!
//! The on-disk record count `N` derived from `ckSize` may **exceed**
//! `numUIDs`: a writer is allowed to pre-allocate spare zeroed records so
//! UIDs can be appended later without resizing the chunk. A reader derives
//! `N` from `ckSize`, tolerates `N > numUIDs`, and skips records whose
//! `trackIndex` is zero. [`ChannelAllocation::active`] yields only the
//! used (non-zero-`trackIndex`) records in on-wire order.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/bs2088-chna-chunk-layout.md` — clean-room
//!   transcription of the `chna` binary layout (§1: the `numTracks` /
//!   `numUIDs` preamble, the 40-byte `audioID` record field order/widths,
//!   the `ckSize = 4 + N*40` rule, the `N >= numUIDs` over-provisioning
//!   convention, and the common-vs-custom ID range rule), from
//! - `docs/container/riff/metadata/R-REC-BS.2088.pdf` §8 — ITU-R BS.2088-2,
//!   *Long-form file format for the international exchange of audio
//!   programme materials with metadata* (the BW64 file format), the binary
//!   definition of `struct chna_chunk` / `struct audioID` that BS.2076
//!   (ADM) and EBU Tech 3285s5 reference but defer.

use crate::error::{Error, Result};

/// FourCC of the channel-allocation chunk.
pub const FOURCC_CHNA: [u8; 4] = *b"chna";

/// Size in bytes of one on-wire `<audioID>` record
/// (`trackIndex` 2 + `UID` 12 + `trackRef` 14 + `packRef` 11 + `pad` 1).
pub const AUDIO_ID_LEN: usize = 40;

/// Length of the `numTracks` + `numUIDs` preamble preceding the record
/// array (two `u16` fields).
pub const CHNA_PREFIX_LEN: usize = 4;

/// Width of the `UID` (`audioTrackUID`) field, e.g. `ATU_00000001`.
pub const UID_LEN: usize = 12;

/// Width of the `trackRef` field (`audioTrackFormatID` `AT_xxxxxxxx_xx`
/// or `audioChannelFormat` `AC_xxxxxxxx_00`).
pub const TRACK_REF_LEN: usize = 14;

/// Width of the `packRef` (`audioPackFormatID`) field, e.g. `AP_00010002`.
pub const PACK_REF_LEN: usize = 11;

/// A single decoded `audioID` record — one track-to-ADM-identifier
/// binding.
///
/// The three identifier fields are kept as fixed-width raw byte arrays
/// because the spec stores them as non-null-terminated ASCII of an exact
/// width; preserving the raw bytes round-trips an unused (all-zero) record
/// and an unrecognised value verbatim. Convenience accessors
/// ([`AudioId::uid_str`] / [`AudioId::track_ref_str`] /
/// [`AudioId::pack_ref_str`]) trim the trailing NULs and decode to UTF-8
/// for the common all-ASCII case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioId {
    /// `trackIndex` — 1-based index of the track within the `data`
    /// chunk's interleave order. `0` marks an unused (over-provisioned)
    /// record that a reader skips.
    pub track_index: u16,
    /// `UID` — the `audioTrackUID` value, format `ATU_xxxxxxxx` (`ATU_`
    /// plus 8 hex digits). All-zero in an unused record.
    pub uid: [u8; UID_LEN],
    /// `trackRef` — the `audioTrackFormatID` reference `AT_xxxxxxxx_xx`,
    /// or, for bare linear-PCM essence (where audioTrackFormat /
    /// audioStreamFormat are omitted), an `audioChannelFormat` reference
    /// `AC_xxxxxxxx_00`. All-zero in an unused record.
    pub track_ref: [u8; TRACK_REF_LEN],
    /// `packRef` — the `audioPackFormatID` reference `AP_xxxxxxxx`, or
    /// 11 NUL bytes when no audioPackFormat is required.
    pub pack_ref: [u8; PACK_REF_LEN],
    /// `pad` — the single trailing byte that makes each record an even
    /// length (so the chunk stays word-aligned). Preserved verbatim; the
    /// spec does not pin its value (examples use `0x00`).
    pub pad: u8,
}

impl AudioId {
    /// Decode one 40-byte `audioID` record from `raw`.
    ///
    /// `raw` must be exactly [`AUDIO_ID_LEN`] bytes; otherwise an
    /// `Error::invalid` is returned.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() != AUDIO_ID_LEN {
            return Err(Error::invalid("RIFF: chna audioID record is not 40 bytes"));
        }
        let track_index = u16::from_le_bytes([raw[0], raw[1]]);
        let mut uid = [0u8; UID_LEN];
        uid.copy_from_slice(&raw[2..2 + UID_LEN]);
        let mut track_ref = [0u8; TRACK_REF_LEN];
        track_ref.copy_from_slice(&raw[14..14 + TRACK_REF_LEN]);
        let mut pack_ref = [0u8; PACK_REF_LEN];
        pack_ref.copy_from_slice(&raw[28..28 + PACK_REF_LEN]);
        let pad = raw[39];
        Ok(AudioId {
            track_index,
            uid,
            track_ref,
            pack_ref,
            pad,
        })
    }

    /// `true` if this is an unused (over-provisioned) record — its
    /// `trackIndex` is zero. A reader skips such records.
    pub fn is_unused(&self) -> bool {
        self.track_index == 0
    }

    /// `true` if `packRef` is all-NUL, i.e. no `audioPackFormat` is
    /// referenced by this record.
    pub fn has_no_pack(&self) -> bool {
        self.pack_ref.iter().all(|&b| b == 0)
    }

    /// The `UID` field decoded as a trimmed UTF-8 string (trailing NULs
    /// removed), or `None` if the trimmed bytes are not valid UTF-8.
    pub fn uid_str(&self) -> Option<&str> {
        trimmed_str(&self.uid)
    }

    /// The `trackRef` field decoded as a trimmed UTF-8 string, or `None`
    /// if the trimmed bytes are not valid UTF-8.
    pub fn track_ref_str(&self) -> Option<&str> {
        trimmed_str(&self.track_ref)
    }

    /// The `packRef` field decoded as a trimmed UTF-8 string. Returns
    /// `Some("")` when `packRef` is all-NUL (no pack), or `None` if the
    /// trimmed bytes are not valid UTF-8.
    pub fn pack_ref_str(&self) -> Option<&str> {
        trimmed_str(&self.pack_ref)
    }

    /// Serialize this `audioID` record as a fixed [`AUDIO_ID_LEN`] (40)
    /// bytes — `trackIndex` (little-endian) then the three fixed-width raw
    /// identifier fields and the trailing `pad` byte, all verbatim. Exact
    /// inverse of [`AudioId::parse`].
    pub fn encode(&self) -> [u8; AUDIO_ID_LEN] {
        let mut b = [0u8; AUDIO_ID_LEN];
        b[0..2].copy_from_slice(&self.track_index.to_le_bytes());
        b[2..2 + UID_LEN].copy_from_slice(&self.uid);
        b[14..14 + TRACK_REF_LEN].copy_from_slice(&self.track_ref);
        b[28..28 + PACK_REF_LEN].copy_from_slice(&self.pack_ref);
        b[39] = self.pad;
        b
    }
}

/// Trim trailing NUL bytes from a fixed-width field and decode as UTF-8.
fn trimmed_str(field: &[u8]) -> Option<&str> {
    let end = field
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    core::str::from_utf8(&field[..end]).ok()
}

/// A decoded `chna` chunk: the ADM track-to-identifier allocation table.
///
/// Carries the declared `numTracks` / `numUIDs` header counts plus every
/// `audioID` record present on the wire (including any over-provisioned
/// all-zero records). The chunk body length must be exactly
/// `4 + N * 40` for some non-negative `N` — a body whose length is not
/// `4` plus a whole multiple of 40 is rejected rather than silently
/// truncated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelAllocation {
    /// `numTracks` — the number of tracks used in the file (a track with
    /// multiple ID sets still counts once).
    pub num_tracks: u16,
    /// `numUIDs` — the number of UIDs used; may exceed `numTracks` (one
    /// track can carry several UIDs for different time periods). Per the
    /// spec this should match the number of non-unused records.
    pub num_uids: u16,
    records: Vec<AudioId>,
}

impl ChannelAllocation {
    /// An empty `chna` chunk (zero tracks, zero UIDs, no records).
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode a `chna` chunk body (already pad-stripped by the walker).
    ///
    /// The body is `<numTracks:u16 LE> <numUIDs:u16 LE>` followed by
    /// `N = (len - 4) / 40` records of [`AUDIO_ID_LEN`] bytes each. A body
    /// shorter than the 4-byte preamble, or whose record region is not a
    /// whole multiple of 40 bytes, is an `Error::invalid`.
    ///
    /// `N` may exceed `numUIDs` (over-provisioning); the surplus zeroed
    /// records are retained and reported as unused.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < CHNA_PREFIX_LEN {
            return Err(Error::invalid(
                "RIFF: chna chunk shorter than 4-byte numTracks/numUIDs preamble",
            ));
        }
        let num_tracks = u16::from_le_bytes([body[0], body[1]]);
        let num_uids = u16::from_le_bytes([body[2], body[3]]);
        let records_region = &body[CHNA_PREFIX_LEN..];
        if records_region.len() % AUDIO_ID_LEN != 0 {
            return Err(Error::invalid(
                "RIFF: chna record region is not a whole multiple of 40 bytes",
            ));
        }
        let n = records_region.len() / AUDIO_ID_LEN;
        let mut records = Vec::with_capacity(n);
        for i in 0..n {
            let off = i * AUDIO_ID_LEN;
            records.push(AudioId::parse(&records_region[off..off + AUDIO_ID_LEN])?);
        }
        Ok(ChannelAllocation {
            num_tracks,
            num_uids,
            records,
        })
    }

    /// Build a `chna` chunk from explicit header counts and the full
    /// on-wire record list (including any over-provisioned zeroed slots).
    ///
    /// The write-side counterpart of [`ChannelAllocation::parse`]; emit
    /// the body with [`ChannelAllocation::encode_body`]. The `num_uids`
    /// header is independent of the record count (over-provisioning), so
    /// it is taken from the caller rather than derived.
    pub fn from_records(num_tracks: u16, num_uids: u16, records: Vec<AudioId>) -> Self {
        ChannelAllocation {
            num_tracks,
            num_uids,
            records,
        }
    }

    /// Append an `audioID` record, returning `self` for chaining.
    pub fn push(mut self, record: AudioId) -> Self {
        self.records.push(record);
        self
    }

    /// Serialize this `chna` chunk *body* — the `numTracks` / `numUIDs`
    /// preamble (both little-endian, emitted verbatim) followed by every
    /// `audioID` record in on-wire order, including over-provisioned
    /// zeroed slots. Exact inverse of [`ChannelAllocation::parse`].
    pub fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CHNA_PREFIX_LEN + self.records.len() * AUDIO_ID_LEN);
        out.extend_from_slice(&self.num_tracks.to_le_bytes());
        out.extend_from_slice(&self.num_uids.to_le_bytes());
        for record in &self.records {
            out.extend_from_slice(&record.encode());
        }
        out
    }

    /// Every `audioID` record on the wire, in order (including unused
    /// over-provisioned all-zero records).
    pub fn records(&self) -> &[AudioId] {
        &self.records
    }

    /// The on-disk record count `N` (the length of [`records`], i.e.
    /// `(ckSize - 4) / 40`). May exceed [`num_uids`](Self::num_uids).
    ///
    /// [`records`]: Self::records
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Iterator over only the *used* records — those whose `trackIndex`
    /// is non-zero — in on-wire order, skipping over-provisioned slots.
    pub fn active(&self) -> impl Iterator<Item = &AudioId> {
        self.records.iter().filter(|r| !r.is_unused())
    }

    /// Number of used (non-zero-`trackIndex`) records. For a conforming
    /// file this equals [`num_uids`](Self::num_uids); a mismatch flags a
    /// non-conforming writer rather than being rejected.
    pub fn active_count(&self) -> usize {
        self.records.iter().filter(|r| !r.is_unused()).count()
    }

    /// `true` if the count of used records matches the declared
    /// `numUIDs` header (the spec's stated invariant).
    pub fn uid_count_consistent(&self) -> bool {
        self.active_count() == self.num_uids as usize
    }

    /// The first used record bound to physical `track_index` (1-based),
    /// or `None`. A track may legitimately carry several records (one per
    /// time period); this returns the first in on-wire order.
    pub fn by_track_index(&self, track_index: u16) -> Option<&AudioId> {
        if track_index == 0 {
            return None;
        }
        self.records.iter().find(|r| r.track_index == track_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one 40-byte `audioID` record from its fields, NUL-padding the
    /// fixed-width string fields to width.
    fn rec(track_index: u16, uid: &[u8], track_ref: &[u8], pack_ref: &[u8], pad: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity(AUDIO_ID_LEN);
        v.extend_from_slice(&track_index.to_le_bytes());
        let mut field = |src: &[u8], width: usize| {
            let mut f = vec![0u8; width];
            f[..src.len()].copy_from_slice(src);
            v.extend_from_slice(&f);
        };
        field(uid, UID_LEN);
        field(track_ref, TRACK_REF_LEN);
        field(pack_ref, PACK_REF_LEN);
        v.push(pad);
        assert_eq!(v.len(), AUDIO_ID_LEN);
        v
    }

    /// Build a `chna` chunk body from counts + records.
    fn chna_body(num_tracks: u16, num_uids: u16, records: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&num_tracks.to_le_bytes());
        body.extend_from_slice(&num_uids.to_le_bytes());
        for r in records {
            body.extend_from_slice(r);
        }
        body
    }

    #[test]
    fn record_is_40_bytes() {
        assert_eq!(AUDIO_ID_LEN, 40);
        assert_eq!(CHNA_PREFIX_LEN + AUDIO_ID_LEN, 44);
        assert_eq!(2 + UID_LEN + TRACK_REF_LEN + PACK_REF_LEN + 1, AUDIO_ID_LEN);
        assert_eq!(&FOURCC_CHNA, b"chna");
    }

    #[test]
    fn parse_stereo_example() {
        // BS.2088-2 §8.3.1 stereo: numTracks=2, numUIDs=2, N=2, ckSize=84.
        let body = chna_body(
            2,
            2,
            &[
                rec(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00010002", 0),
                rec(2, b"ATU_00000002", b"AT_00010002_01", b"AP_00010002", 0),
            ],
        );
        // ckSize check: 4 + 2*40 = 84.
        assert_eq!(body.len(), 84);
        let chna = ChannelAllocation::parse(&body).unwrap();
        assert_eq!(chna.num_tracks, 2);
        assert_eq!(chna.num_uids, 2);
        assert_eq!(chna.record_count(), 2);
        assert_eq!(chna.active_count(), 2);
        assert!(chna.uid_count_consistent());

        let r0 = &chna.records()[0];
        assert_eq!(r0.track_index, 1);
        assert_eq!(r0.uid_str(), Some("ATU_00000001"));
        assert_eq!(r0.track_ref_str(), Some("AT_00010001_01"));
        assert_eq!(r0.pack_ref_str(), Some("AP_00010002"));
        assert!(!r0.is_unused());
        assert!(!r0.has_no_pack());
    }

    #[test]
    fn over_provisioning_skips_unused_records() {
        // BS.2088-2 §8.3.2 shape: numUIDs=4 but N=32 (28 zeroed spares).
        let mut records = vec![
            rec(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00031001", 0),
            rec(1, b"ATU_00000002", b"AT_00010001_02", b"AP_00031001", 0),
            rec(2, b"ATU_00000003", b"AT_00010002_01", b"AP_00031001", 0),
            rec(2, b"ATU_00000004", b"AT_00010002_02", b"AP_00031001", 0),
        ];
        for _ in 4..32 {
            records.push(rec(0, &[], &[], &[], 0)); // unused zeroed slot
        }
        let body = chna_body(2, 4, &records);
        assert_eq!(body.len(), 4 + 32 * 40); // 1284, §8.3.2
        let chna = ChannelAllocation::parse(&body).unwrap();
        assert_eq!(chna.num_tracks, 2);
        assert_eq!(chna.num_uids, 4);
        assert_eq!(chna.record_count(), 32);
        assert_eq!(chna.active_count(), 4);
        assert!(chna.uid_count_consistent());
        assert_eq!(chna.active().count(), 4);
        // The 5th on-wire record is an unused over-provisioned slot.
        assert!(chna.records()[4].is_unused());
        assert!(chna.records()[31].is_unused());
    }

    #[test]
    fn unused_record_fields_are_zero() {
        let body = chna_body(0, 0, &[rec(0, &[], &[], &[], 0)]);
        let chna = ChannelAllocation::parse(&body).unwrap();
        let r = &chna.records()[0];
        assert!(r.is_unused());
        assert!(r.has_no_pack());
        assert_eq!(r.uid_str(), Some(""));
        assert_eq!(r.track_ref_str(), Some(""));
        assert_eq!(r.pack_ref_str(), Some(""));
    }

    #[test]
    fn linear_pcm_channel_format_track_ref() {
        // Bare PCM essence references an audioChannelFormat AC_xxxxxxxx_00
        // and carries no pack (11 NUL bytes).
        let body = chna_body(1, 1, &[rec(1, b"ATU_00000001", b"AC_00010001_00", &[], 0)]);
        let chna = ChannelAllocation::parse(&body).unwrap();
        let r = &chna.records()[0];
        assert_eq!(r.track_ref_str(), Some("AC_00010001_00"));
        assert!(r.has_no_pack());
        assert_eq!(r.pack_ref_str(), Some(""));
    }

    #[test]
    fn by_track_index_finds_first_match() {
        let body = chna_body(
            2,
            3,
            &[
                rec(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00010002", 0),
                rec(1, b"ATU_00000002", b"AT_00010001_02", b"AP_00010002", 0),
                rec(2, b"ATU_00000003", b"AT_00010003_01", b"AP_00010004", 0),
            ],
        );
        let chna = ChannelAllocation::parse(&body).unwrap();
        // Track 1 has two records; by_track_index returns the first.
        assert_eq!(
            chna.by_track_index(1).unwrap().uid_str(),
            Some("ATU_00000001")
        );
        assert_eq!(
            chna.by_track_index(2).unwrap().uid_str(),
            Some("ATU_00000003")
        );
        assert!(chna.by_track_index(99).is_none());
        // trackIndex 0 is the unused sentinel; never a lookup hit.
        assert!(chna.by_track_index(0).is_none());
    }

    #[test]
    fn empty_chna_chunk() {
        let body = chna_body(0, 0, &[]);
        assert_eq!(body.len(), 4);
        let chna = ChannelAllocation::parse(&body).unwrap();
        assert_eq!(chna.record_count(), 0);
        assert_eq!(chna.active_count(), 0);
        assert!(chna.uid_count_consistent());
        assert!(chna.active().next().is_none());
    }

    #[test]
    fn pad_byte_is_preserved() {
        // The spec doesn't pin pad's value; preserve it verbatim.
        let body = chna_body(
            1,
            1,
            &[rec(
                1,
                b"ATU_00000001",
                b"AT_00010001_01",
                b"AP_00010002",
                0xAB,
            )],
        );
        let chna = ChannelAllocation::parse(&body).unwrap();
        assert_eq!(chna.records()[0].pad, 0xAB);
    }

    #[test]
    fn uid_count_inconsistent_when_declared_mismatches_active() {
        // Declares numUIDs=3 but supplies only 1 used record.
        let body = chna_body(
            1,
            3,
            &[rec(
                1,
                b"ATU_00000001",
                b"AT_00010001_01",
                b"AP_00010002",
                0,
            )],
        );
        let chna = ChannelAllocation::parse(&body).unwrap();
        assert_eq!(chna.active_count(), 1);
        assert!(!chna.uid_count_consistent());
    }

    #[test]
    fn short_preamble_is_rejected() {
        let err = ChannelAllocation::parse(&[0u8; 3]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 4-byte numTracks/numUIDs preamble"));
    }

    #[test]
    fn non_multiple_record_region_is_rejected() {
        // 4-byte preamble + 39 bytes (not a whole 40-byte record).
        let mut body = vec![0u8; CHNA_PREFIX_LEN];
        body.extend_from_slice(&[0u8; AUDIO_ID_LEN - 1]);
        let err = ChannelAllocation::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("not a whole multiple of 40 bytes"));
    }

    #[test]
    fn record_parse_rejects_wrong_length() {
        let err = AudioId::parse(&[0u8; AUDIO_ID_LEN - 1]).unwrap_err();
        assert!(format!("{err}").contains("not 40 bytes"));
    }

    #[test]
    fn non_utf8_field_returns_none() {
        // Invalid UTF-8 in the UID field (0xFF byte before the NUL run).
        let mut raw = rec(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00010002", 0);
        raw[2] = 0xFF; // first byte of UID
        let r = AudioId::parse(&raw).unwrap();
        assert!(r.uid_str().is_none());
    }

    #[test]
    fn record_encode_is_parse_inverse() {
        let raw = rec(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00010002", 0xAB);
        let r = AudioId::parse(&raw).unwrap();
        assert_eq!(&r.encode()[..], &raw[..]);
        assert_eq!(AudioId::parse(&r.encode()).unwrap(), r);
    }

    #[test]
    fn encode_body_round_trips_stereo() {
        let body = chna_body(
            2,
            2,
            &[
                rec(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00010002", 0),
                rec(2, b"ATU_00000002", b"AT_00010002_01", b"AP_00010002", 0),
            ],
        );
        let chna = ChannelAllocation::parse(&body).unwrap();
        let encoded = chna.encode_body();
        assert_eq!(encoded, body);
        assert_eq!(ChannelAllocation::parse(&encoded).unwrap(), chna);
    }

    #[test]
    fn encode_body_preserves_over_provisioning() {
        let mut records = vec![rec(
            1,
            b"ATU_00000001",
            b"AT_00010001_01",
            b"AP_00031001",
            0,
        )];
        for _ in 1..8 {
            records.push(rec(0, &[], &[], &[], 0));
        }
        let body = chna_body(1, 1, &records);
        let chna = ChannelAllocation::parse(&body).unwrap();
        let encoded = chna.encode_body();
        // Over-provisioned zeroed slots and numUIDs < record_count survive.
        assert_eq!(encoded, body);
        let reparsed = ChannelAllocation::parse(&encoded).unwrap();
        assert_eq!(reparsed.record_count(), 8);
        assert_eq!(reparsed.num_uids, 1);
    }

    #[test]
    fn builder_encode_parse_round_trip() {
        let chna = ChannelAllocation::from_records(1, 1, Vec::new())
            .push(AudioId::parse(&rec(1, b"ATU_00000001", b"AC_00010001_00", &[], 0)).unwrap());
        let encoded = chna.encode_body();
        let reparsed = ChannelAllocation::parse(&encoded).unwrap();
        assert_eq!(reparsed, chna);
        assert_eq!(
            reparsed.records()[0].track_ref_str(),
            Some("AC_00010001_00")
        );
    }
}
