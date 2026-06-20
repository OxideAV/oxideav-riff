//! Typed decoder for the WAV / RIFF `smpl` (sampler) chunk.
//!
//! The `smpl` chunk carries the sampler-instrument detail a hardware or
//! software sampler needs to play a WAV sample as a MIDI instrument: the
//! pitch the sample is mapped to (`MIDIUnityNote` + `MIDIPitchFraction`),
//! the timing reference (`SamplePeriod`, optional SMPTE offset), and a
//! table of **sample loops** — the loop points used to sustain a held note
//! indefinitely without retriggering. It is the richer companion to the
//! 7-byte `inst` (instrument) chunk.
//!
//! ```text
//! smpl ( <ckSize:u32 LE>
//!   <Manufacturer:u32 LE>        // MMA manufacturer ID (0 = unspecified)
//!   <Product:u32 LE>             // manufacturer's product code
//!   <SamplePeriod:u32 LE>        // duration of one sample, in nanoseconds
//!   <MIDIUnityNote:u32 LE>       // MIDI note the sample plays at unshifted
//!   <MIDIPitchFraction:u32 LE>   // pitch fraction of a semitone
//!   <SMPTEFormat:u32 LE>         // 0 / 24 / 25 / 29 / 30 fps
//!   <SMPTEOffset:u32 LE>         // SMPTE offset (HH:MM:SS:FF packed)
//!   <NumSampleLoops:u32 LE>      // number of <sample-loop> records that follow
//!   <SamplerDataLen:u32 LE>      // byte length of the trailing SamplerData
//!   <sample-loop> * NumSampleLoops   // 24-byte loop records (see below)
//!   <SamplerData:SamplerDataLen>     // opaque manufacturer-specific block
//! )
//! ```
//!
//! The fixed header is 36 bytes (nine `u32` fields), followed by
//! `NumSampleLoops` loop records of 24 bytes each, followed by a
//! `SamplerDataLen`-byte opaque trailer.
//!
//! ## Sample-loop records
//!
//! Each loop record is a fixed 24-byte block. The in-tree clean-room
//! material fixes the **record stride** (24 bytes per loop) and the loop
//! *count* (`NumSampleLoops`) but does not enumerate the per-record field
//! breakdown, so this decoder keeps each loop record as a verbatim
//! 24-byte [`SampleLoop`] rather than splitting it into named fields it
//! cannot source. A caller that knows the field layout can decode the
//! preserved bytes; nothing is dropped. See the *Clean-room sources* note.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/exiftool-riff-tags.html` — the
//!   *RIFF Sampler Tags* table (`'smpl'` chunk → `Sampler` table) with
//!   the nine fixed fields at a 4-byte stride: `Manufacturer`, `Product`,
//!   `SamplePeriod`, `MIDIUnityNote`, `MIDIPitchFraction`, `SMPTEFormat`
//!   (enumerated 0 / 24 / 25 / 29 / 30 fps), `SMPTEOffset` (HH:MM:SS:FF),
//!   `NumSampleLoops`, `SamplerDataLen`, then `SamplerData`.
//! - `docs/container/riff/metadata/README.md` — "`smpl` (~36 bytes + N ×
//!   24 loop entries) carries MIDI note assignment + sample-loop point
//!   sets", confirming the 36-byte fixed header and the 24-byte loop
//!   record stride.

use crate::error::{Error, Result};

/// FourCC of the sampler chunk.
pub const FOURCC_SMPL: [u8; 4] = *b"smpl";

/// Byte length of the fixed `smpl` header preceding the loop table: nine
/// `u32` fields = 36 bytes.
pub const SMPL_HEADER_LEN: usize = 36;

/// Byte length of a single `<sample-loop>` record in the loop table.
pub const SAMPLE_LOOP_LEN: usize = 24;

/// `SMPTEFormat` value: no SMPTE offset.
pub const SMPTE_FORMAT_NONE: u32 = 0;

/// A single 24-byte `<sample-loop>` record from the `smpl` loop table.
///
/// The in-tree clean-room material fixes the 24-byte record stride but not
/// the per-field breakdown, so the record is retained verbatim. The raw
/// bytes round-trip exactly; a caller with the field layout decodes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleLoop {
    /// The 24 raw bytes of this loop record, in on-wire order.
    pub raw: [u8; SAMPLE_LOOP_LEN],
}

impl SampleLoop {
    /// Wrap a 24-byte slice into a [`SampleLoop`]. The slice must be
    /// exactly [`SAMPLE_LOOP_LEN`] bytes.
    fn from_slice(bytes: &[u8]) -> Self {
        let mut raw = [0u8; SAMPLE_LOOP_LEN];
        raw.copy_from_slice(bytes);
        SampleLoop { raw }
    }

    /// The 24 raw bytes of this loop record, in on-wire order. Since the
    /// record is preserved verbatim this is the byte-exact serialization.
    pub fn encode(&self) -> [u8; SAMPLE_LOOP_LEN] {
        self.raw
    }
}

/// A decoded WAV `smpl` (sampler) chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Smpl {
    /// `Manufacturer` — the MMA manufacturer ID of the target sampler
    /// (`0` = unspecified / any).
    pub manufacturer: u32,
    /// `Product` — the manufacturer's product code for the target sampler.
    pub product: u32,
    /// `SamplePeriod` — the duration of one sample, in nanoseconds (the
    /// reciprocal of the sample rate).
    pub sample_period: u32,
    /// `MIDIUnityNote` — the MIDI note number at which the sample plays
    /// back at its recorded pitch.
    pub midi_unity_note: u32,
    /// `MIDIPitchFraction` — the fraction of a semitone above
    /// `MIDIUnityNote`, as a 32-bit fixed-point fraction (`0x80000000`
    /// is a half-semitone).
    pub midi_pitch_fraction: u32,
    /// `SMPTEFormat` — the SMPTE frame rate the [`Smpl::smpte_offset`]
    /// references: `0` none, or `24` / `25` / `29` / `30` fps.
    pub smpte_format: u32,
    /// `SMPTEOffset` — the SMPTE time offset of the sample's start,
    /// packed `HH:MM:SS:FF` (one byte per field, most-significant first).
    pub smpte_offset: u32,
    /// `NumSampleLoops` — the number of [`SampleLoop`] records present (the
    /// length of [`Smpl::loops`]).
    pub num_sample_loops: u32,
    /// `SamplerDataLen` — the byte length of the trailing opaque
    /// [`Smpl::sampler_data`] block.
    pub sampler_data_len: u32,
    /// The loop table — `num_sample_loops` records of 24 bytes each,
    /// preserved verbatim.
    pub loops: Vec<SampleLoop>,
    /// `SamplerData` — the opaque, manufacturer-specific trailer (length
    /// `sampler_data_len`). Empty when `sampler_data_len` is `0`.
    pub sampler_data: Vec<u8>,
}

impl Smpl {
    /// Decode an `smpl` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be at least [`SMPL_HEADER_LEN`] (36) bytes for the
    /// fixed header, then carry exactly `NumSampleLoops × 24` loop bytes
    /// and `SamplerDataLen` trailer bytes. A body whose length does not
    /// match the header's declared loop count and sampler-data length is
    /// an `Error::invalid` — the counts and the body length must agree, so
    /// a mismatch signals truncation or corruption rather than a future
    /// field.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < SMPL_HEADER_LEN {
            return Err(Error::invalid(
                "RIFF: smpl chunk shorter than 36-byte fixed header",
            ));
        }
        let rd = |off: usize| {
            u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
        };
        let manufacturer = rd(0);
        let product = rd(4);
        let sample_period = rd(8);
        let midi_unity_note = rd(12);
        let midi_pitch_fraction = rd(16);
        let smpte_format = rd(20);
        let smpte_offset = rd(24);
        let num_sample_loops = rd(28);
        let sampler_data_len = rd(32);

        // The loop table and SamplerData trailer must fit the body exactly.
        let loops_bytes = (num_sample_loops as usize)
            .checked_mul(SAMPLE_LOOP_LEN)
            .ok_or_else(|| Error::invalid("RIFF: smpl loop table size overflows usize"))?;
        let trailer_off = SMPL_HEADER_LEN
            .checked_add(loops_bytes)
            .ok_or_else(|| Error::invalid("RIFF: smpl body size overflows usize"))?;
        let expected = trailer_off
            .checked_add(sampler_data_len as usize)
            .ok_or_else(|| Error::invalid("RIFF: smpl body size overflows usize"))?;
        if body.len() != expected {
            return Err(Error::invalid(
                "RIFF: smpl body length disagrees with NumSampleLoops / SamplerDataLen",
            ));
        }

        let mut loops = Vec::with_capacity(num_sample_loops as usize);
        let mut off = SMPL_HEADER_LEN;
        for _ in 0..num_sample_loops {
            loops.push(SampleLoop::from_slice(&body[off..off + SAMPLE_LOOP_LEN]));
            off += SAMPLE_LOOP_LEN;
        }
        let sampler_data = body[trailer_off..].to_vec();

        Ok(Smpl {
            manufacturer,
            product,
            sample_period,
            midi_unity_note,
            midi_pitch_fraction,
            smpte_format,
            smpte_offset,
            num_sample_loops,
            sampler_data_len,
            loops,
            sampler_data,
        })
    }

    /// `true` if the chunk declares no SMPTE offset (`SMPTEFormat == 0`),
    /// in which case [`Smpl::smpte_offset`] should be ignored.
    pub fn smpte_none(&self) -> bool {
        self.smpte_format == SMPTE_FORMAT_NONE
    }

    /// The four packed `HH:MM:SS:FF` bytes of `SMPTEOffset`,
    /// most-significant first (hours, minutes, seconds, frames).
    pub fn smpte_offset_parts(&self) -> (u8, u8, u8, u8) {
        let b = self.smpte_offset.to_be_bytes();
        (b[0], b[1], b[2], b[3])
    }

    /// `true` if the chunk carries an opaque manufacturer-specific
    /// `SamplerData` trailer.
    pub fn has_sampler_data(&self) -> bool {
        !self.sampler_data.is_empty()
    }

    /// Serialize this `smpl` chunk *body* — the 36-byte fixed header, the
    /// loop table, then the opaque `SamplerData` trailer.
    ///
    /// To guarantee the emitted body always re-parses, the on-wire
    /// `NumSampleLoops` and `SamplerDataLen` are written from the actual
    /// [`Smpl::loops`] / [`Smpl::sampler_data`] lengths rather than the
    /// stored count fields — so a parsed `smpl` round-trips and a
    /// hand-built one is never internally inconsistent. The other seven
    /// header fields emit verbatim.
    pub fn encode_body(&self) -> Vec<u8> {
        let num_loops = self.loops.len() as u32;
        let data_len = self.sampler_data.len() as u32;
        let mut out = Vec::with_capacity(
            SMPL_HEADER_LEN + self.loops.len() * SAMPLE_LOOP_LEN + data_len as usize,
        );
        for v in [
            self.manufacturer,
            self.product,
            self.sample_period,
            self.midi_unity_note,
            self.midi_pitch_fraction,
            self.smpte_format,
            self.smpte_offset,
            num_loops,
            data_len,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for loop_rec in &self.loops {
            out.extend_from_slice(&loop_rec.encode());
        }
        out.extend_from_slice(&self.sampler_data);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 36-byte `smpl` header from explicit fields.
    #[allow(clippy::too_many_arguments)]
    fn header(
        manufacturer: u32,
        product: u32,
        sample_period: u32,
        midi_unity_note: u32,
        midi_pitch_fraction: u32,
        smpte_format: u32,
        smpte_offset: u32,
        num_sample_loops: u32,
        sampler_data_len: u32,
    ) -> Vec<u8> {
        let mut b = Vec::with_capacity(SMPL_HEADER_LEN);
        for v in [
            manufacturer,
            product,
            sample_period,
            midi_unity_note,
            midi_pitch_fraction,
            smpte_format,
            smpte_offset,
            num_sample_loops,
            sampler_data_len,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn consts_match_spec() {
        assert_eq!(&FOURCC_SMPL, b"smpl");
        assert_eq!(SMPL_HEADER_LEN, 36);
        assert_eq!(SAMPLE_LOOP_LEN, 24);
    }

    #[test]
    fn parse_header_only_no_loops_no_data() {
        // A sample mapped to middle C (60), 22675 ns period (~44.1 kHz),
        // no loops, no sampler data, no SMPTE offset.
        let body = header(0, 0, 22_675, 60, 0, 0, 0, 0, 0);
        let smpl = Smpl::parse(&body).unwrap();
        assert_eq!(smpl.manufacturer, 0);
        assert_eq!(smpl.sample_period, 22_675);
        assert_eq!(smpl.midi_unity_note, 60);
        assert_eq!(smpl.num_sample_loops, 0);
        assert_eq!(smpl.sampler_data_len, 0);
        assert!(smpl.loops.is_empty());
        assert!(!smpl.has_sampler_data());
        assert!(smpl.smpte_none());
    }

    #[test]
    fn parse_one_loop_record_preserved_verbatim() {
        let mut body = header(0, 0, 22_675, 60, 0, 0, 0, 1, 0);
        let loop_bytes: [u8; SAMPLE_LOOP_LEN] = [
            0x00, 0x00, 0x00, 0x00, // cue id
            0x00, 0x00, 0x00, 0x00, // type = forward
            0x10, 0x00, 0x00, 0x00, // start
            0x20, 0x00, 0x00, 0x00, // end
            0x00, 0x00, 0x00, 0x00, // fraction
            0x00, 0x00, 0x00, 0x00, // play count = infinite
        ];
        body.extend_from_slice(&loop_bytes);
        let smpl = Smpl::parse(&body).unwrap();
        assert_eq!(smpl.num_sample_loops, 1);
        assert_eq!(smpl.loops.len(), 1);
        assert_eq!(smpl.loops[0].raw, loop_bytes);
    }

    #[test]
    fn parse_multiple_loops_and_sampler_data() {
        let mut body = header(0x42, 0x07, 20_833, 69, 0x8000_0000, 30, 0, 2, 5);
        body.extend_from_slice(&[0xA0; SAMPLE_LOOP_LEN]);
        body.extend_from_slice(&[0xB1; SAMPLE_LOOP_LEN]);
        body.extend_from_slice(&[1, 2, 3, 4, 5]); // SamplerData (5 bytes)
        let smpl = Smpl::parse(&body).unwrap();
        assert_eq!(smpl.manufacturer, 0x42);
        assert_eq!(smpl.product, 0x07);
        assert_eq!(smpl.midi_unity_note, 69);
        assert_eq!(smpl.midi_pitch_fraction, 0x8000_0000);
        assert_eq!(smpl.smpte_format, 30);
        assert!(!smpl.smpte_none());
        assert_eq!(smpl.loops.len(), 2);
        assert_eq!(smpl.loops[0].raw, [0xA0; SAMPLE_LOOP_LEN]);
        assert_eq!(smpl.loops[1].raw, [0xB1; SAMPLE_LOOP_LEN]);
        assert!(smpl.has_sampler_data());
        assert_eq!(smpl.sampler_data, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn smpte_offset_parts_unpacks_hhmmssff() {
        // 01:30:25:12 packed big-endian: 0x011A190C.
        let body = header(0, 0, 0, 60, 0, 25, 0x011A_190C, 0, 0);
        let smpl = Smpl::parse(&body).unwrap();
        assert_eq!(smpl.smpte_offset_parts(), (0x01, 0x1A, 0x19, 0x0C));
    }

    #[test]
    fn short_header_is_rejected() {
        let err = Smpl::parse(&[0u8; 35]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 36-byte fixed header"));
    }

    #[test]
    fn loop_count_disagreeing_with_body_is_rejected() {
        // Header declares 1 loop but no loop bytes follow.
        let body = header(0, 0, 0, 60, 0, 0, 0, 1, 0);
        let err = Smpl::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with NumSampleLoops"));
    }

    #[test]
    fn sampler_data_len_disagreeing_with_body_is_rejected() {
        // Header declares 4 bytes of SamplerData but none follow.
        let body = header(0, 0, 0, 60, 0, 0, 0, 0, 4);
        let err = Smpl::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with NumSampleLoops"));
    }

    #[test]
    fn overflowing_loop_count_is_rejected() {
        // A loop count that would overflow the body size computation must
        // be rejected, not panic.
        let body = header(0, 0, 0, 60, 0, 0, 0, u32::MAX, 0);
        let err = Smpl::parse(&body).unwrap_err();
        // Either the overflow guard or the length-mismatch guard fires;
        // both are Error::invalid.
        let msg = format!("{err}");
        assert!(msg.contains("smpl"));
    }

    #[test]
    fn default_is_empty() {
        let smpl = Smpl::default();
        assert_eq!(smpl.num_sample_loops, 0);
        assert!(smpl.loops.is_empty());
        assert!(smpl.sampler_data.is_empty());
        assert!(smpl.smpte_none());
    }

    #[test]
    fn encode_body_header_only_round_trips() {
        let body = header(0, 0, 22_675, 60, 0, 0, 0, 0, 0);
        let smpl = Smpl::parse(&body).unwrap();
        let encoded = smpl.encode_body();
        assert_eq!(encoded, body);
        assert_eq!(Smpl::parse(&encoded).unwrap(), smpl);
    }

    #[test]
    fn encode_body_loops_and_data_round_trips() {
        let mut body = header(0x42, 0x07, 20_833, 69, 0x8000_0000, 30, 0, 2, 5);
        body.extend_from_slice(&[0xA0; SAMPLE_LOOP_LEN]);
        body.extend_from_slice(&[0xB1; SAMPLE_LOOP_LEN]);
        body.extend_from_slice(&[1, 2, 3, 4, 5]);
        let smpl = Smpl::parse(&body).unwrap();
        let encoded = smpl.encode_body();
        assert_eq!(encoded, body);
        assert_eq!(Smpl::parse(&encoded).unwrap(), smpl);
    }

    #[test]
    fn encode_derives_counts_from_actual_lengths() {
        // A hand-built Smpl with stale count fields re-encodes to a
        // self-consistent body that re-parses cleanly.
        let smpl = Smpl {
            midi_unity_note: 60,
            num_sample_loops: 99, // stale — ignored on encode
            sampler_data_len: 99, // stale — ignored on encode
            loops: vec![SampleLoop {
                raw: [0x33; SAMPLE_LOOP_LEN],
            }],
            sampler_data: vec![7, 8, 9],
            ..Smpl::default()
        };
        let encoded = smpl.encode_body();
        let reparsed = Smpl::parse(&encoded).unwrap();
        assert_eq!(reparsed.num_sample_loops, 1);
        assert_eq!(reparsed.sampler_data_len, 3);
        assert_eq!(reparsed.loops.len(), 1);
        assert_eq!(reparsed.sampler_data, vec![7, 8, 9]);
    }
}
