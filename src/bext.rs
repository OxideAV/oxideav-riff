//! Typed decoder for the BWF `bext` (Broadcast Audio Extension) chunk.
//!
//! A *Broadcast Wave Format* (BWF) file is an ordinary RIFF/WAVE file
//! that carries one extra chunk, FourCC `bext`, holding broadcast
//! production metadata: a free-text description, originator + reference,
//! origination date/time, a 64-bit sample time-code, a SMPTE 330M UMID,
//! the file's loudness measurements, and a free-text *coding history*.
//! The EBU published it as Tech 3285 so broadcasters can exchange audio
//! together with its production provenance.
//!
//! This module decodes a `bext` chunk **body** (the bytes the
//! [`crate::Walker`] yields from `Walker::read_body`) into the typed
//! [`BroadcastExtension`] struct. It does **not** read the chunk header
//! itself — the caller locates the `bext` chunk with the walker and
//! hands the body slice here, exactly like [`crate::WaveFormat`] for
//! `fmt `.
//!
//! ## Wire layout (§ "Broadcast Audio Extension chunk", Tech 3285 v2)
//!
//! The chunk body is a fixed 602-byte prefix followed by a
//! variable-length `CodingHistory`:
//!
//! ```text
//! +0    Description           CHAR[256]   ASCII, NUL-padded
//! +256  Originator            CHAR[32]    ASCII, NUL-padded
//! +288  OriginatorReference   CHAR[32]    ASCII, NUL-padded
//! +320  OriginationDate       CHAR[10]    ASCII "yyyy-mm-dd"
//! +330  OriginationTime       CHAR[8]     ASCII "hh-mm-ss"
//! +338  TimeReferenceLow      DWORD LE    first-sample count, low word
//! +342  TimeReferenceHigh     DWORD LE    first-sample count, high word
//! +346  Version               WORD  LE    BWF version (0 / 1 / 2)
//! +348  UMID                  BYTE[64]    SMPTE 330M UMID  (Version >= 1)
//! +412  LoudnessValue         WORD  LE    i16, LUFS × 100  (Version >= 2)
//! +414  LoudnessRange         WORD  LE    i16, LU   × 100  (Version >= 2)
//! +416  MaxTruePeakLevel      WORD  LE    i16, dBTP × 100  (Version >= 2)
//! +418  MaxMomentaryLoudness  WORD  LE    i16, LUFS × 100  (Version >= 2)
//! +420  MaxShortTermLoudness  WORD  LE    i16, LUFS × 100  (Version >= 2)
//! +422  Reserved              BYTE[180]   set to 0 for Version 1 / 2
//! +602  CodingHistory         CHAR[]      ASCII, CR/LF-separated strings
//! ```
//!
//! ## Version compatibility (§1.1)
//!
//! The three published BWF versions share the same 602-byte prefix; the
//! difference is which bytes carry meaning:
//!
//! - **Version 0** (1997) — the `UMID` and loudness fields are part of
//!   the reserved area and read as zero.
//! - **Version 1** (2001) — 64 of the reserved bytes carry the `UMID`.
//! - **Version 2** (2011) — 10 further reserved bytes carry the five
//!   loudness measurements.
//!
//! [`BroadcastExtension::umid`] returns the 64 UMID bytes only when
//! `version >= 1`, and [`BroadcastExtension::loudness`] returns the
//! [`Loudness`] measurements only when `version >= 2`, mirroring the
//! spec's forwards/backwards-compatibility rule (older readers ignore
//! the bytes newer versions reuse). The raw bytes remain reachable via
//! the public `umid_bytes` / individual loudness fields for callers that
//! want the unconditional view.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/ebu-tech3285-bwf.pdf` — EBU Tech 3285
//!   v2, *Specification of the Broadcast Wave Format (BWF)*: the
//!   `broadcast_audio_extension` struct, the per-field descriptions, and
//!   §1.1 "Version compatibility".
//! - `docs/container/riff/metadata/README.md` — the staged catalogue
//!   that records the 602-byte prefix layout and version gating.

use crate::error::{Error, Result};

/// The fixed-length prefix of a `bext` chunk, in bytes. Everything past
/// this offset is the variable-length `CodingHistory` field.
pub const BEXT_PREFIX_LEN: usize = 602;

/// Length of the `Description` field, in bytes.
pub const DESCRIPTION_LEN: usize = 256;
/// Length of the `Originator` field, in bytes.
pub const ORIGINATOR_LEN: usize = 32;
/// Length of the `OriginatorReference` field, in bytes.
pub const ORIGINATOR_REFERENCE_LEN: usize = 32;
/// Length of the `OriginationDate` field, in bytes (`"yyyy-mm-dd"`).
pub const ORIGINATION_DATE_LEN: usize = 10;
/// Length of the `OriginationTime` field, in bytes (`"hh-mm-ss"`).
pub const ORIGINATION_TIME_LEN: usize = 8;
/// Length of the SMPTE 330M `UMID` field, in bytes.
pub const UMID_LEN: usize = 64;
/// Length of the `Reserved` field, in bytes.
pub const RESERVED_LEN: usize = 180;

/// The five loudness measurements added in BWF Version 2 (§ field
/// table). Each is a 16-bit signed integer equal to `round(100 ×` the
/// underlying value `)`, so a stored `-2305` represents `-23.05`.
///
/// The units differ per field (LUFS / LU / dBTP); the `* _x100`
/// accessors return the raw scaled integer and the `_lu` / `_lufs` /
/// `_dbtp` accessors divide by 100 into the natural floating-point
/// unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loudness {
    /// Integrated Loudness Value of the file, in LUFS × 100.
    pub value_x100: i16,
    /// Loudness Range of the file, in LU × 100.
    pub range_x100: i16,
    /// Maximum True Peak Level of the file, in dBTP × 100.
    pub max_true_peak_x100: i16,
    /// Highest Momentary Loudness Level of the file, in LUFS × 100.
    pub max_momentary_x100: i16,
    /// Highest Short-Term Loudness Level of the file, in LUFS × 100.
    pub max_short_term_x100: i16,
}

impl Loudness {
    /// Integrated Loudness Value in LUFS (the stored value ÷ 100).
    pub fn value_lufs(&self) -> f32 {
        self.value_x100 as f32 / 100.0
    }

    /// Loudness Range in LU (the stored value ÷ 100).
    pub fn range_lu(&self) -> f32 {
        self.range_x100 as f32 / 100.0
    }

    /// Maximum True Peak Level in dBTP (the stored value ÷ 100).
    pub fn max_true_peak_dbtp(&self) -> f32 {
        self.max_true_peak_x100 as f32 / 100.0
    }

    /// Highest Momentary Loudness Level in LUFS (the stored value ÷ 100).
    pub fn max_momentary_lufs(&self) -> f32 {
        self.max_momentary_x100 as f32 / 100.0
    }

    /// Highest Short-Term Loudness Level in LUFS (the stored value ÷ 100).
    pub fn max_short_term_lufs(&self) -> f32 {
        self.max_short_term_x100 as f32 / 100.0
    }
}

/// A decoded `bext` (Broadcast Audio Extension) chunk.
///
/// String fields are exposed both as the trimmed text (NUL-terminated /
/// NUL-padded ASCII, per the spec) via the accessor methods and as the
/// raw fixed-length byte arrays for callers that need the exact wire
/// bytes. Numeric fields are little-endian per the struct definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BroadcastExtension {
    /// `Description[256]` — free description of the sound sequence
    /// (NUL-padded ASCII).
    pub description: [u8; DESCRIPTION_LEN],
    /// `Originator[32]` — name of the originator / producer.
    pub originator: [u8; ORIGINATOR_LEN],
    /// `OriginatorReference[32]` — unambiguous reference allocated by
    /// the originating organisation (EBU R 99 format).
    pub originator_reference: [u8; ORIGINATOR_REFERENCE_LEN],
    /// `OriginationDate[10]` — date of creation, `"yyyy-mm-dd"`.
    pub origination_date: [u8; ORIGINATION_DATE_LEN],
    /// `OriginationTime[8]` — time of creation, `"hh-mm-ss"`.
    pub origination_time: [u8; ORIGINATION_TIME_LEN],
    /// 64-bit first-sample-count-since-midnight time-code, reassembled
    /// from `TimeReferenceLow` / `TimeReferenceHigh`.
    pub time_reference: u64,
    /// `Version` — the BWF version (0, 1, or 2).
    pub version: u16,
    /// `UMID[64]` — raw SMPTE 330M Unique Material Identifier bytes
    /// (all-zero in a Version 0 file). Use [`BroadcastExtension::umid`]
    /// for the version-gated view.
    pub umid_bytes: [u8; UMID_LEN],
    /// `LoudnessValue` — Integrated Loudness, LUFS × 100 (Version 2).
    pub loudness_value_x100: i16,
    /// `LoudnessRange` — Loudness Range, LU × 100 (Version 2).
    pub loudness_range_x100: i16,
    /// `MaxTruePeakLevel` — Maximum True Peak, dBTP × 100 (Version 2).
    pub max_true_peak_x100: i16,
    /// `MaxMomentaryLoudness` — highest Momentary Loudness, LUFS × 100
    /// (Version 2).
    pub max_momentary_x100: i16,
    /// `MaxShortTermLoudness` — highest Short-Term Loudness, LUFS × 100
    /// (Version 2).
    pub max_short_term_x100: i16,
    /// `CodingHistory` — unrestricted ASCII, a collection of CR/LF
    /// terminated coding-process descriptions. Empty when the chunk is
    /// exactly the 602-byte prefix.
    pub coding_history: Vec<u8>,
}

impl BroadcastExtension {
    /// Decode a `bext` chunk body.
    ///
    /// The body must be at least [`BEXT_PREFIX_LEN`] (602) bytes — the
    /// fixed-length prefix — or the chunk is rejected as truncated.
    /// Anything past the prefix is taken verbatim as `CodingHistory`
    /// (the field the spec defines as the chunk size minus 602).
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < BEXT_PREFIX_LEN {
            return Err(Error::invalid(format!(
                "RIFF: bext chunk too short ({} bytes, need at least {BEXT_PREFIX_LEN})",
                body.len()
            )));
        }

        let mut description = [0u8; DESCRIPTION_LEN];
        description.copy_from_slice(&body[0..256]);
        let mut originator = [0u8; ORIGINATOR_LEN];
        originator.copy_from_slice(&body[256..288]);
        let mut originator_reference = [0u8; ORIGINATOR_REFERENCE_LEN];
        originator_reference.copy_from_slice(&body[288..320]);
        let mut origination_date = [0u8; ORIGINATION_DATE_LEN];
        origination_date.copy_from_slice(&body[320..330]);
        let mut origination_time = [0u8; ORIGINATION_TIME_LEN];
        origination_time.copy_from_slice(&body[330..338]);

        let time_low = u32::from_le_bytes([body[338], body[339], body[340], body[341]]);
        let time_high = u32::from_le_bytes([body[342], body[343], body[344], body[345]]);
        let time_reference = ((time_high as u64) << 32) | time_low as u64;

        let version = u16::from_le_bytes([body[346], body[347]]);

        let mut umid_bytes = [0u8; UMID_LEN];
        umid_bytes.copy_from_slice(&body[348..412]);

        let loudness_value_x100 = i16::from_le_bytes([body[412], body[413]]);
        let loudness_range_x100 = i16::from_le_bytes([body[414], body[415]]);
        let max_true_peak_x100 = i16::from_le_bytes([body[416], body[417]]);
        let max_momentary_x100 = i16::from_le_bytes([body[418], body[419]]);
        let max_short_term_x100 = i16::from_le_bytes([body[420], body[421]]);

        // body[422..602] is the 180-byte Reserved field (zero in v1/v2);
        // not retained.
        let coding_history = body[BEXT_PREFIX_LEN..].to_vec();

        Ok(Self {
            description,
            originator,
            originator_reference,
            origination_date,
            origination_time,
            time_reference,
            version,
            umid_bytes,
            loudness_value_x100,
            loudness_range_x100,
            max_true_peak_x100,
            max_momentary_x100,
            max_short_term_x100,
            coding_history,
        })
    }

    /// `Description` text, trimmed at the first NUL.
    ///
    /// Per the spec the field is NUL-terminated when shorter than its
    /// 256-byte capacity; the bytes up to the first `0x00` are returned,
    /// lossily decoded from ASCII / Latin-1 into UTF-8.
    pub fn description(&self) -> String {
        trimmed_string(&self.description)
    }

    /// `Originator` text, trimmed at the first NUL.
    pub fn originator(&self) -> String {
        trimmed_string(&self.originator)
    }

    /// `OriginatorReference` text, trimmed at the first NUL.
    pub fn originator_reference(&self) -> String {
        trimmed_string(&self.originator_reference)
    }

    /// `OriginationDate` text (`"yyyy-mm-dd"`), trimmed at the first NUL.
    pub fn origination_date(&self) -> String {
        trimmed_string(&self.origination_date)
    }

    /// `OriginationTime` text (`"hh-mm-ss"`), trimmed at the first NUL.
    pub fn origination_time(&self) -> String {
        trimmed_string(&self.origination_time)
    }

    /// The SMPTE 330M UMID, gated on version.
    ///
    /// Returns the 64 UMID bytes only when `version >= 1` (the version
    /// at which the field was introduced); a Version 0 chunk reads
    /// all-zero in this range and the spec instructs readers to ignore
    /// it, so this returns `None`. The unconditional raw bytes remain
    /// available as [`BroadcastExtension::umid_bytes`].
    pub fn umid(&self) -> Option<&[u8; UMID_LEN]> {
        if self.version >= 1 {
            Some(&self.umid_bytes)
        } else {
            None
        }
    }

    /// The five loudness measurements, gated on version.
    ///
    /// Returns [`Loudness`] only when `version >= 2` (the version at
    /// which the loudness fields were introduced); for Versions 0 and 1
    /// these bytes are part of the reserved area and the spec says to
    /// ignore them, so this returns `None`.
    pub fn loudness(&self) -> Option<Loudness> {
        if self.version >= 2 {
            Some(Loudness {
                value_x100: self.loudness_value_x100,
                range_x100: self.loudness_range_x100,
                max_true_peak_x100: self.max_true_peak_x100,
                max_momentary_x100: self.max_momentary_x100,
                max_short_term_x100: self.max_short_term_x100,
            })
        } else {
            None
        }
    }

    /// `CodingHistory` text, the trailing variable-length ASCII field.
    ///
    /// Each coding-process description is CR/LF terminated; this returns
    /// the whole field lossily decoded, trailing NUL padding (if any)
    /// removed.
    pub fn coding_history(&self) -> String {
        let trimmed = match self.coding_history.iter().position(|&b| b == 0) {
            Some(nul) => &self.coding_history[..nul],
            None => &self.coding_history[..],
        };
        String::from_utf8_lossy(trimmed).into_owned()
    }
}

/// Decode a fixed-length, NUL-padded ASCII field to a `String`,
/// stopping at the first NUL byte and lossily mapping non-UTF-8 bytes.
fn trimmed_string(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `bext` body: the 602-byte prefix with the given
    /// version + optional coding history, all other fields zero.
    fn bext_body(version: u16, coding_history: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; BEXT_PREFIX_LEN];
        b[346..348].copy_from_slice(&version.to_le_bytes());
        b.extend_from_slice(coding_history);
        b
    }

    #[test]
    fn prefix_len_matches_field_offsets() {
        // Sum of every fixed field must equal the 602-byte prefix.
        let total = DESCRIPTION_LEN
            + ORIGINATOR_LEN
            + ORIGINATOR_REFERENCE_LEN
            + ORIGINATION_DATE_LEN
            + ORIGINATION_TIME_LEN
            + 4 // TimeReferenceLow
            + 4 // TimeReferenceHigh
            + 2 // Version
            + UMID_LEN
            + 2 * 5 // five loudness words
            + RESERVED_LEN;
        assert_eq!(total, BEXT_PREFIX_LEN);
    }

    #[test]
    fn rejects_short_body() {
        let body = vec![0u8; BEXT_PREFIX_LEN - 1];
        let err = BroadcastExtension::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("bext chunk too short"));
    }

    #[test]
    fn parses_minimal_prefix_only() {
        let body = bext_body(2, b"");
        let bext = BroadcastExtension::parse(&body).unwrap();
        assert_eq!(bext.version, 2);
        assert_eq!(bext.description(), "");
        assert!(bext.coding_history().is_empty());
    }

    #[test]
    fn decodes_string_fields_trimmed_at_nul() {
        let mut body = bext_body(1, b"");
        body[0..11].copy_from_slice(b"My session\0");
        body[256..262].copy_from_slice(b"OxAV\0\0");
        body[288..291].copy_from_slice(b"R1\0");
        body[320..330].copy_from_slice(b"2026-06-13");
        body[330..338].copy_from_slice(b"14-05-09");
        let bext = BroadcastExtension::parse(&body).unwrap();
        assert_eq!(bext.description(), "My session");
        assert_eq!(bext.originator(), "OxAV");
        assert_eq!(bext.originator_reference(), "R1");
        assert_eq!(bext.origination_date(), "2026-06-13");
        assert_eq!(bext.origination_time(), "14-05-09");
    }

    #[test]
    fn reassembles_64bit_time_reference() {
        let mut body = bext_body(0, b"");
        // low = 0x89AB_CDEF, high = 0x0000_0012 → 0x0000_0012_89AB_CDEF
        body[338..342].copy_from_slice(&0x89AB_CDEFu32.to_le_bytes());
        body[342..346].copy_from_slice(&0x0000_0012u32.to_le_bytes());
        let bext = BroadcastExtension::parse(&body).unwrap();
        assert_eq!(bext.time_reference, 0x0000_0012_89AB_CDEF);
    }

    #[test]
    fn umid_gated_on_version() {
        // Version 0 → no UMID even if bytes are non-zero.
        let mut body = bext_body(0, b"");
        body[348] = 0x06;
        let v0 = BroadcastExtension::parse(&body).unwrap();
        assert!(v0.umid().is_none());
        // The raw bytes are still reachable.
        assert_eq!(v0.umid_bytes[0], 0x06);

        // Version 1 → UMID present.
        body[346..348].copy_from_slice(&1u16.to_le_bytes());
        let v1 = BroadcastExtension::parse(&body).unwrap();
        assert_eq!(v1.umid().unwrap()[0], 0x06);
    }

    #[test]
    fn loudness_gated_on_version() {
        let mut body = bext_body(1, b"");
        // Put a loudness value in; v1 must still ignore it.
        body[412..414].copy_from_slice(&(-2305i16).to_le_bytes());
        let v1 = BroadcastExtension::parse(&body).unwrap();
        assert!(v1.loudness().is_none());

        // Version 2 → loudness exposed and scaled.
        body[346..348].copy_from_slice(&2u16.to_le_bytes());
        body[414..416].copy_from_slice(&700i16.to_le_bytes());
        body[416..418].copy_from_slice(&(-150i16).to_le_bytes());
        body[418..420].copy_from_slice(&(-1820i16).to_le_bytes());
        body[420..422].copy_from_slice(&(-1995i16).to_le_bytes());
        let v2 = BroadcastExtension::parse(&body).unwrap();
        let l = v2.loudness().unwrap();
        assert_eq!(l.value_x100, -2305);
        assert!((l.value_lufs() - (-23.05)).abs() < 1e-4);
        assert!((l.range_lu() - 7.0).abs() < 1e-4);
        assert!((l.max_true_peak_dbtp() - (-1.5)).abs() < 1e-4);
        assert!((l.max_momentary_lufs() - (-18.20)).abs() < 1e-4);
        assert!((l.max_short_term_lufs() - (-19.95)).abs() < 1e-4);
    }

    #[test]
    fn coding_history_is_the_trailing_field() {
        let body = bext_body(2, b"A=PCM,F=48000,W=16,M=stereo\r\n");
        let bext = BroadcastExtension::parse(&body).unwrap();
        assert_eq!(bext.coding_history(), "A=PCM,F=48000,W=16,M=stereo\r\n");
    }

    #[test]
    fn coding_history_trims_trailing_nul_padding() {
        // An odd-length coding history padded with a NUL (RIFF pad) or
        // a writer that zero-fills.
        let body = bext_body(2, b"A=PCM\0\0");
        let bext = BroadcastExtension::parse(&body).unwrap();
        assert_eq!(bext.coding_history(), "A=PCM");
    }

    #[test]
    fn lossy_decode_on_non_ascii_description() {
        let mut body = bext_body(1, b"");
        body[0] = b'a';
        body[1] = 0xFF; // not valid UTF-8
        body[2] = b'b';
        body[3] = 0;
        let bext = BroadcastExtension::parse(&body).unwrap();
        let d = bext.description();
        assert!(d.starts_with('a') && d.ends_with('b'));
        assert!(d.contains('\u{FFFD}'));
    }
}
