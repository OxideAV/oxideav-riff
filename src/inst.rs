//! Typed decoder for the WAV / RIFF `inst` (instrument) chunk.
//!
//! The `inst` chunk attaches MIDI-instrument playback hints to a WAV
//! sample so a sampler or soft-synth can map the single recorded sample
//! across a keyboard: which key the sample was recorded at, how to
//! fine-tune and gain-adjust it, and the key / velocity ranges over which
//! the sample should sound. It is a small fixed 7-byte record — one byte
//! per field — and is a sibling of the `smpl` (sampler) chunk that carries
//! the loop-point detail.
//!
//! ```text
//! inst ( <ckSize:u32 LE = 7>
//!   <UnshiftedNote:u8>   // MIDI note the sample was recorded at (0-127)
//!   <FineTune:i8>        // pitch fine-tune, in cents
//!   <Gain:i8>            // playback gain, in decibels
//!   <LowNote:u8>         // lowest MIDI note this sample should play
//!   <HighNote:u8>        // highest MIDI note this sample should play
//!   <LowVelocity:u8>     // lowest MIDI velocity this sample responds to
//!   <HighVelocity:u8>    // highest MIDI velocity this sample responds to
//! )
//! ```
//!
//! The two range pairs (`LowNote`..=`HighNote` and
//! `LowVelocity`..=`HighVelocity`) describe the key-zone and
//! velocity-zone the sample covers; `UnshiftedNote` is the key at which
//! the sample plays back unshifted, with keys away from it pitch-shifted.
//!
//! ## Field widths and signedness
//!
//! Every field is a single byte and the fields appear in the order above
//! (the catalogued field order; each field at a 1-byte stride). The MIDI
//! note and velocity fields are unsigned 0-127 magnitudes; `FineTune` and
//! `Gain` are offsets that take both signs in practice (a sample can be
//! tuned sharp or flat and attenuated or boosted), so they are exposed as
//! signed `i8`. The raw byte of each field is also retained verbatim so a
//! caller that wants the unsigned reading is never blocked by the signed
//! interpretation.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/exiftool-riff-tags.html` — the
//!   *RIFF Instrument Tags* table (`'inst'` chunk → `Instrument` table),
//!   enumerating the seven 1-byte fields and their order: `UnshiftedNote`,
//!   `FineTune`, `Gain`, `LowNote`, `HighNote`, `LowVelocity`,
//!   `HighVelocity`.
//! - `docs/container/riff/metadata/README.md` — "`inst` (7 bytes) carries
//!   gain / coarse-tune / fine-tune / velocity ranges", confirming the
//!   fixed 7-byte body length.

use crate::error::{Error, Result};

/// FourCC of the instrument chunk.
pub const FOURCC_INST: [u8; 4] = *b"inst";

/// On-wire body length of a conforming `inst` chunk: a fixed 7 bytes, one
/// per field. The chunk size is odd, so a `0x00` pad byte follows the body
/// (the walker strips it before this decoder sees the body).
pub const INST_LEN: usize = 7;

/// A decoded WAV `inst` (instrument) chunk.
///
/// All seven fields are single bytes. The MIDI note / velocity fields are
/// unsigned magnitudes; `fine_tune` and `gain` are kept as raw bytes and
/// re-exposed as signed offsets through [`Inst::fine_tune`] / [`Inst::gain`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inst {
    /// `UnshiftedNote` — the MIDI note number (0-127) at which the sample
    /// plays back unshifted. Notes either side of it are pitch-shifted.
    pub unshifted_note: u8,
    /// `FineTune` — pitch fine-tune in cents, as a raw byte. Read it as a
    /// signed cents offset with [`Inst::fine_tune`].
    pub fine_tune_raw: u8,
    /// `Gain` — playback gain in decibels, as a raw byte. Read it as a
    /// signed decibel offset with [`Inst::gain`].
    pub gain_raw: u8,
    /// `LowNote` — the lowest MIDI note this sample should play.
    pub low_note: u8,
    /// `HighNote` — the highest MIDI note this sample should play.
    pub high_note: u8,
    /// `LowVelocity` — the lowest MIDI velocity this sample responds to.
    pub low_velocity: u8,
    /// `HighVelocity` — the highest MIDI velocity this sample responds to.
    pub high_velocity: u8,
}

impl Inst {
    /// Decode an `inst` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be exactly [`INST_LEN`] (7) bytes — the fixed
    /// instrument record. A shorter or longer body is an `Error::invalid`:
    /// the `inst` chunk is a fixed-width record with no documented
    /// extension mechanism, so an off-length body signals a malformed or
    /// foreign chunk rather than a future field.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() != INST_LEN {
            return Err(Error::invalid(
                "RIFF: inst chunk body is not exactly 7 bytes",
            ));
        }
        Ok(Inst {
            unshifted_note: body[0],
            fine_tune_raw: body[1],
            gain_raw: body[2],
            low_note: body[3],
            high_note: body[4],
            low_velocity: body[5],
            high_velocity: body[6],
        })
    }

    /// `FineTune` read as a signed cents offset (a sample may be tuned
    /// sharp `+` or flat `-`).
    pub fn fine_tune(&self) -> i8 {
        self.fine_tune_raw as i8
    }

    /// `Gain` read as a signed decibel offset (a sample may be boosted `+`
    /// or attenuated `-`).
    pub fn gain(&self) -> i8 {
        self.gain_raw as i8
    }

    /// The inclusive MIDI key range `(LowNote, HighNote)` this sample
    /// covers.
    pub fn note_range(&self) -> (u8, u8) {
        (self.low_note, self.high_note)
    }

    /// The inclusive MIDI velocity range `(LowVelocity, HighVelocity)`
    /// this sample responds to.
    pub fn velocity_range(&self) -> (u8, u8) {
        (self.low_velocity, self.high_velocity)
    }

    /// `true` if a MIDI `note` falls within the sample's key range
    /// (inclusive of both bounds).
    pub fn covers_note(&self, note: u8) -> bool {
        self.low_note <= note && note <= self.high_note
    }

    /// `true` if a MIDI `velocity` falls within the sample's velocity
    /// range (inclusive of both bounds).
    pub fn covers_velocity(&self, velocity: u8) -> bool {
        self.low_velocity <= velocity && velocity <= self.high_velocity
    }

    /// Serialize this `inst` chunk *body* — the fixed 7 bytes a walker
    /// would hand to [`Inst::parse`], one byte per field in the catalogued
    /// order (`UnshiftedNote`, `FineTune`, `Gain`, `LowNote`, `HighNote`,
    /// `LowVelocity`, `HighVelocity`).
    ///
    /// The signed `fine_tune` / `gain` re-emit from their retained raw
    /// bytes, so a parsed `inst` round-trips byte-for-byte through
    /// [`Inst::parse`] regardless of the sign interpretation.
    pub fn encode_body(&self) -> [u8; INST_LEN] {
        [
            self.unshifted_note,
            self.fine_tune_raw,
            self.gain_raw,
            self.low_note,
            self.high_note,
            self.low_velocity,
            self.high_velocity,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 7-byte `inst` body from explicit field bytes.
    #[allow(clippy::too_many_arguments)]
    fn body(
        unshifted_note: u8,
        fine_tune: u8,
        gain: u8,
        low_note: u8,
        high_note: u8,
        low_velocity: u8,
        high_velocity: u8,
    ) -> Vec<u8> {
        vec![
            unshifted_note,
            fine_tune,
            gain,
            low_note,
            high_note,
            low_velocity,
            high_velocity,
        ]
    }

    #[test]
    fn consts_match_spec() {
        assert_eq!(&FOURCC_INST, b"inst");
        assert_eq!(INST_LEN, 7);
    }

    #[test]
    fn parse_typical_piano_zone() {
        // Sample recorded at middle C (60), no tune/gain offset, covering
        // the full keyboard and all velocities.
        let inst = Inst::parse(&body(60, 0, 0, 0, 127, 0, 127)).unwrap();
        assert_eq!(inst.unshifted_note, 60);
        assert_eq!(inst.fine_tune(), 0);
        assert_eq!(inst.gain(), 0);
        assert_eq!(inst.note_range(), (0, 127));
        assert_eq!(inst.velocity_range(), (0, 127));
        assert!(inst.covers_note(60));
        assert!(inst.covers_note(0));
        assert!(inst.covers_note(127));
        assert!(inst.covers_velocity(64));
    }

    #[test]
    fn fine_tune_and_gain_signed() {
        // FineTune = -50 cents (0xCE), Gain = -6 dB (0xFA).
        let inst = Inst::parse(&body(60, 0xCE, 0xFA, 48, 72, 1, 127)).unwrap();
        assert_eq!(inst.fine_tune(), -50);
        assert_eq!(inst.gain(), -6);
        // Raw bytes still available for an unsigned reading.
        assert_eq!(inst.fine_tune_raw, 0xCE);
        assert_eq!(inst.gain_raw, 0xFA);
    }

    #[test]
    fn fine_tune_and_gain_positive() {
        let inst = Inst::parse(&body(60, 25, 3, 48, 72, 1, 127)).unwrap();
        assert_eq!(inst.fine_tune(), 25);
        assert_eq!(inst.gain(), 3);
    }

    #[test]
    fn key_and_velocity_zone_bounds() {
        // A sample zoned to C3..C4 (48..60), velocity 64..127.
        let inst = Inst::parse(&body(54, 0, 0, 48, 60, 64, 127)).unwrap();
        assert!(!inst.covers_note(47));
        assert!(inst.covers_note(48));
        assert!(inst.covers_note(60));
        assert!(!inst.covers_note(61));
        assert!(!inst.covers_velocity(63));
        assert!(inst.covers_velocity(64));
        assert!(inst.covers_velocity(127));
    }

    #[test]
    fn short_body_is_rejected() {
        let err = Inst::parse(&[0u8; 6]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 7 bytes"));
    }

    #[test]
    fn long_body_is_rejected() {
        let err = Inst::parse(&[0u8; 8]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 7 bytes"));
    }

    #[test]
    fn empty_body_is_rejected() {
        let err = Inst::parse(&[]).unwrap_err();
        assert!(format!("{err}").contains("not exactly 7 bytes"));
    }

    #[test]
    fn default_is_all_zero() {
        let inst = Inst::default();
        assert_eq!(inst.unshifted_note, 0);
        assert_eq!(inst.fine_tune(), 0);
        assert_eq!(inst.gain(), 0);
        assert_eq!(inst.note_range(), (0, 0));
    }

    #[test]
    fn encode_body_round_trips() {
        let b = body(60, 0xCE, 0xFA, 48, 72, 1, 127);
        let inst = Inst::parse(&b).unwrap();
        let encoded = inst.encode_body();
        assert_eq!(&encoded[..], &b[..]);
        assert_eq!(Inst::parse(&encoded).unwrap(), inst);
    }

    #[test]
    fn encode_body_is_seven_bytes() {
        let inst = Inst::default();
        assert_eq!(inst.encode_body().len(), INST_LEN);
    }
}
