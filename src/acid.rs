//! Typed decoder for the WAV / RIFF `acid` chunk (Acidizer loop metadata).
//!
//! The `acid` chunk is the loop-metadata block that Sonic Foundry's
//! **ACID** (later Sony, then MAGIX) writes into "ACIDized" WAV loops. It
//! carries the loop's tempo, beat count, time signature, and musical root
//! note so a host can pitch- and time-stretch the loop to the project
//! tempo. It is an ordinary RIFF subchunk inside the top-level
//! `RIFF....WAVE` form — a sibling of `fmt ` / `data` / `fact` / `cue `.
//!
//! ```text
//! acid ( <ckSize:u32 LE = 24>
//!   <flags:u32 LE>             // type / property bitfield
//!   <rootNote:u16 LE>          // MIDI note number (valid iff flag bit 1)
//!   <unknown1:u16 LE>          // observed constant 0x8000
//!   <unknown2:f32 LE>          // observed constant 0.0
//!   <numBeats:u32 LE>          // number of beats in the loop
//!   <meterDenominator:u16 LE>  // time-signature denominator
//!   <meterNumerator:u16 LE>    // time-signature numerator
//!   <tempo:f32 LE>             // tempo in BPM
//! )
//! ```
//!
//! ## Provenance
//!
//! The `acid` chunk was never published by Sonic Foundry / Sony — it is an
//! undocumented vendor chunk. The field layout used here is the
//! community reverse-engineering of the 24-byte body, cross-checked across
//! two independent public sources and transcribed into the clean-room
//! field spec `docs/container/riff/acid-chunk.md`. Two fields (`unknown1`
//! at offset 6, `unknown2` at offset 8) are observed constants whose
//! purpose is undocumented; this decoder preserves them verbatim so an
//! atypical writer's value is never silently discarded. The meter
//! numerator/denominator order follows that field spec; real ACID files
//! almost always write 4/4, so in-the-wild data cannot disambiguate it.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/acid-chunk.md` — clean-room field spec for the
//!   24-byte `acid` body: the per-offset field table (`flags` @0,
//!   `rootNote` @4, `unknown1` @6, `unknown2` @8, `numBeats` @12,
//!   `meterDenominator` @16, `meterNumerator` @18, `tempo` @20), the
//!   `flags` bit table (one-shot/loop, root-note-set, stretch,
//!   disk-based, high-octave), and the MIDI root-note value mapping.
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §1.3 +
//!   `docs/container/riff/metadata/ms-xaudio2-riff.html` — the parent
//!   RIFF little-endian, word-aligned subchunk framing the `acid` chunk
//!   sits inside.

use crate::error::{Error, Result};

/// FourCC of the acid chunk.
pub const FOURCC_ACID: [u8; 4] = *b"acid";

/// On-wire body length of a conforming `acid` chunk: a fixed 24-byte
/// Acidizer record. The chunk size is always even, so no pad byte ever
/// follows it.
pub const ACID_LEN: usize = 24;

/// `flags` bit 0 — set: this is a **one-shot** (non-looping) sample;
/// clear: a tempo-mapped **loop**. The primary type discriminator.
pub const FLAG_ONE_SHOT: u32 = 0x01;

/// `flags` bit 1 — set: the [`Acid::root_note`] field is valid; clear: no
/// root note is assigned and `root_note` should be ignored.
pub const FLAG_ROOT_NOTE_SET: u32 = 0x02;

/// `flags` bit 2 — set: **stretch** (tempo-following) enabled.
pub const FLAG_STRETCH: u32 = 0x04;

/// `flags` bit 3 — set: **disk-based**; clear: RAM-based.
pub const FLAG_DISK_BASED: u32 = 0x08;

/// `flags` bit 4 — "high octave" / Acidizer octave flag. Reverse-engineers
/// report this shifts the octave the [`Acid::root_note`] MIDI value is
/// interpreted in (the `0x3C`-`0x47` "High C..High B" block when set
/// vs `0x30`-`0x3B` when clear). Documented as octave-related but not
/// fully pinned down.
pub const FLAG_HIGH_OCTAVE: u32 = 0x10;

/// A decoded WAV `acid` (Acidizer) chunk.
///
/// Every multi-byte field is little-endian. The two `unknown*` fields are
/// observed constants in known files (`unknown1 == 0x8000`,
/// `unknown2 == 0.0`); they are retained verbatim rather than asserted, so
/// the decoder never rejects a file over an undocumented field and an
/// atypical value round-trips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Acid {
    /// `flags` — the type / property bitfield. Interpret with the
    /// `FLAG_*` masks and the [`Acid::is_one_shot`] / [`Acid::is_loop`] /
    /// [`Acid::root_note_set`] / [`Acid::stretch_enabled`] /
    /// [`Acid::disk_based`] / [`Acid::high_octave`] accessors.
    pub flags: u32,
    /// `rootNote` — MIDI note number of the loop's musical root (e.g.
    /// `0x3C` = 60 = middle C). Meaningful only when [`FLAG_ROOT_NOTE_SET`]
    /// is set in `flags`; use [`Acid::root_note`] to get it gated.
    pub root_note: u16,
    /// `unknown1` (offset 6) — observed constant `0x8000` in all known
    /// files; purpose undocumented. Preserved verbatim.
    pub unknown1: u16,
    /// `unknown2` (offset 8) — observed constant `0.0` in all known files;
    /// purpose undocumented. Preserved verbatim as the raw little-endian
    /// `f32` bit pattern's decoded value.
    pub unknown2: f32,
    /// `numBeats` — the number of beats in the loop.
    pub num_beats: u32,
    /// `meterDenominator` — time-signature denominator (real ACID files
    /// always write 4).
    pub meter_denominator: u16,
    /// `meterNumerator` — time-signature numerator (real ACID files
    /// always write 4).
    pub meter_numerator: u16,
    /// `tempo` — tempo in BPM (IEEE-754 single precision).
    pub tempo: f32,
}

impl Acid {
    /// Decode an `acid` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be exactly [`ACID_LEN`] (24) bytes — the fixed
    /// Acidizer record. A shorter or longer body is an `Error::invalid`:
    /// unlike the forward-extensible `fact` chunk, the `acid` body is a
    /// fixed-width vendor record with no documented extension mechanism,
    /// so an off-length body signals a malformed or foreign chunk rather
    /// than a future field.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() != ACID_LEN {
            return Err(Error::invalid(
                "RIFF: acid chunk body is not exactly 24 bytes",
            ));
        }
        let flags = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let root_note = u16::from_le_bytes([body[4], body[5]]);
        let unknown1 = u16::from_le_bytes([body[6], body[7]]);
        let unknown2 = f32::from_le_bytes([body[8], body[9], body[10], body[11]]);
        let num_beats = u32::from_le_bytes([body[12], body[13], body[14], body[15]]);
        let meter_denominator = u16::from_le_bytes([body[16], body[17]]);
        let meter_numerator = u16::from_le_bytes([body[18], body[19]]);
        let tempo = f32::from_le_bytes([body[20], body[21], body[22], body[23]]);
        Ok(Acid {
            flags,
            root_note,
            unknown1,
            unknown2,
            num_beats,
            meter_denominator,
            meter_numerator,
            tempo,
        })
    }

    /// `true` if [`FLAG_ONE_SHOT`] is set — a non-looping one-shot sound
    /// rather than a tempo-mapped loop.
    pub fn is_one_shot(&self) -> bool {
        self.flags & FLAG_ONE_SHOT != 0
    }

    /// `true` if this is a tempo-mapped loop (the complement of
    /// [`Acid::is_one_shot`]).
    pub fn is_loop(&self) -> bool {
        !self.is_one_shot()
    }

    /// `true` if [`FLAG_ROOT_NOTE_SET`] is set — the [`Acid::root_note`]
    /// field carries a valid MIDI note assignment.
    pub fn root_note_set(&self) -> bool {
        self.flags & FLAG_ROOT_NOTE_SET != 0
    }

    /// `true` if [`FLAG_STRETCH`] is set — tempo-following stretch enabled.
    pub fn stretch_enabled(&self) -> bool {
        self.flags & FLAG_STRETCH != 0
    }

    /// `true` if [`FLAG_DISK_BASED`] is set — disk-based rather than
    /// RAM-based.
    pub fn disk_based(&self) -> bool {
        self.flags & FLAG_DISK_BASED != 0
    }

    /// `true` if [`FLAG_HIGH_OCTAVE`] is set — the octave flag affecting
    /// how the root-note MIDI value is mapped to an octave.
    pub fn high_octave(&self) -> bool {
        self.flags & FLAG_HIGH_OCTAVE != 0
    }

    /// The MIDI root note, gated by [`FLAG_ROOT_NOTE_SET`]: `Some(note)`
    /// when the flag is set, `None` when no root note is assigned (in
    /// which case the raw `root_note` field should be ignored).
    pub fn root_note(&self) -> Option<u16> {
        if self.root_note_set() {
            Some(self.root_note)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 24-byte `acid` body from explicit fields.
    #[allow(clippy::too_many_arguments)]
    fn body(
        flags: u32,
        root_note: u16,
        unknown1: u16,
        unknown2: f32,
        num_beats: u32,
        meter_den: u16,
        meter_num: u16,
        tempo: f32,
    ) -> Vec<u8> {
        let mut b = Vec::with_capacity(ACID_LEN);
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&root_note.to_le_bytes());
        b.extend_from_slice(&unknown1.to_le_bytes());
        b.extend_from_slice(&unknown2.to_le_bytes());
        b.extend_from_slice(&num_beats.to_le_bytes());
        b.extend_from_slice(&meter_den.to_le_bytes());
        b.extend_from_slice(&meter_num.to_le_bytes());
        b.extend_from_slice(&tempo.to_le_bytes());
        b
    }

    #[test]
    fn consts_match_spec() {
        assert_eq!(&FOURCC_ACID, b"acid");
        assert_eq!(ACID_LEN, 24);
        assert_eq!(FLAG_ONE_SHOT, 0x01);
        assert_eq!(FLAG_ROOT_NOTE_SET, 0x02);
        assert_eq!(FLAG_STRETCH, 0x04);
        assert_eq!(FLAG_DISK_BASED, 0x08);
        assert_eq!(FLAG_HIGH_OCTAVE, 0x10);
    }

    #[test]
    fn parse_typical_138bpm_loop() {
        // A tempo-mapped loop, root note set, stretch on; middle-C root;
        // 138.0 BPM, 4/4, 8 beats. unknown1 = 0x8000, unknown2 = 0.0.
        let acid = Acid::parse(&body(
            FLAG_ROOT_NOTE_SET | FLAG_STRETCH,
            0x3C,
            0x8000,
            0.0,
            8,
            4,
            4,
            138.0,
        ))
        .unwrap();
        assert!(acid.is_loop());
        assert!(!acid.is_one_shot());
        assert!(acid.root_note_set());
        assert!(acid.stretch_enabled());
        assert!(!acid.disk_based());
        assert_eq!(acid.root_note(), Some(0x3C));
        assert_eq!(acid.num_beats, 8);
        assert_eq!(acid.meter_denominator, 4);
        assert_eq!(acid.meter_numerator, 4);
        assert_eq!(acid.tempo, 138.0);
        assert_eq!(acid.unknown1, 0x8000);
        assert_eq!(acid.unknown2, 0.0);
    }

    #[test]
    fn tempo_138_bytes_round_trip() {
        // 138.0 BPM little-endian f32 == 00 00 0A 43 per the field spec.
        let b = body(0, 0, 0x8000, 0.0, 0, 4, 4, 138.0);
        assert_eq!(&b[20..24], &[0x00, 0x00, 0x0A, 0x43]);
        let acid = Acid::parse(&b).unwrap();
        assert_eq!(acid.tempo, 138.0);
    }

    #[test]
    fn one_shot_has_no_root_note_when_flag_clear() {
        // One-shot, root-note flag clear: root_note() gates to None even
        // though the raw field carries a value.
        let acid = Acid::parse(&body(FLAG_ONE_SHOT, 0x3C, 0x8000, 0.0, 0, 4, 4, 120.0)).unwrap();
        assert!(acid.is_one_shot());
        assert!(!acid.is_loop());
        assert!(!acid.root_note_set());
        assert_eq!(acid.root_note(), None);
        // Raw field is still preserved.
        assert_eq!(acid.root_note, 0x3C);
    }

    #[test]
    fn high_octave_and_disk_based_flags() {
        let acid = Acid::parse(&body(
            FLAG_DISK_BASED | FLAG_HIGH_OCTAVE | FLAG_ROOT_NOTE_SET,
            0x3C,
            0x8000,
            0.0,
            16,
            4,
            4,
            90.0,
        ))
        .unwrap();
        assert!(acid.disk_based());
        assert!(acid.high_octave());
        assert!(acid.root_note_set());
    }

    #[test]
    fn unknown_fields_preserved_verbatim() {
        // An atypical writer that deviates from the observed constants is
        // not rejected; both unknown fields round-trip.
        let acid = Acid::parse(&body(0, 0, 0x1234, 1.5, 0, 4, 4, 100.0)).unwrap();
        assert_eq!(acid.unknown1, 0x1234);
        assert_eq!(acid.unknown2, 1.5);
    }

    #[test]
    fn short_body_is_rejected() {
        let err = Acid::parse(&[0u8; 23]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 24 bytes"));
    }

    #[test]
    fn long_body_is_rejected() {
        let err = Acid::parse(&[0u8; 25]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 24 bytes"));
    }

    #[test]
    fn empty_body_is_rejected() {
        let err = Acid::parse(&[]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 24 bytes"));
    }

    #[test]
    fn default_is_all_zero() {
        let acid = Acid::default();
        assert_eq!(acid.flags, 0);
        assert!(acid.is_loop());
        assert!(!acid.root_note_set());
        assert_eq!(acid.root_note(), None);
        assert_eq!(acid.tempo, 0.0);
    }
}
