//! Typed decoder for the WAV / RIFF `fact` chunk.
//!
//! The `fact` chunk records facts about the contents of a WAVE file that
//! cannot be derived cheaply from the `data` chunk alone. Its single
//! mandatory field is the number of *samples* (per channel) in the
//! waveform — the value a player needs to compute exact duration when
//! the audio is compressed (the `data` byte count no longer maps to a
//! sample count by a fixed `wBlockAlign`) or when the samples are
//! scattered across a `wavl` LIST of `data` / `slnt` chunks.
//!
//! ```text
//! fact ( <ckSize:u32 LE>
//!   <dwSampleLength:u32 LE>   // number of samples per channel
//!   <future fields> ...       // optional; sized by ckSize
//! )
//! ```
//!
//! The 1991 RIFF MCI spec names the first field `<dwFileSize>` in the
//! grammar line but annotates it "Number of samples"; the field carries
//! the per-channel sample count, and that is the name later WAV
//! references settled on (`dwSampleLength`). The spec explicitly reserves
//! the right to append further fields after it, instructing applications
//! to use the chunk-size field to discover which are present — so this
//! decoder reads the leading sample count and retains any trailing bytes
//! verbatim rather than rejecting a longer-than-4-byte body.
//!
//! The chunk is **required** when the waveform data lives in a `wavl`
//! LIST chunk and for every compressed (non-PCM) format; it is optional
//! for a plain PCM `data` chunk.
//!
//! ## RF64 / BW64 deferral
//!
//! In an RF64 / BW64 (MBWF) file whose sample count exceeds the 32-bit
//! range, `dwSampleLength` carries the [`crate::ds64::SIZE_SENTINEL`]
//! (`0xFFFFFFFF`) placeholder and the true 64-bit count lives in the `ds64` chunk's
//! `sampleCount` field. [`Fact::is_deferred`] reports that case so a
//! caller knows to resolve the real value from a parsed
//! [`crate::ds64::DataSize64`] instead of trusting the 32-bit field.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "FACT Chunk" (the `<fact-ck>` grammar, the "Number of samples"
//!   annotation, the required-for-compressed/`wavl` rule, and the
//!   forward-compatibility note that trailing fields are size-discovered).
//! - `docs/container/riff/metadata/ebu-tech3306-v1.pdf` §A — the RF64
//!   `ds64` `sampleCount` field that "replaces the sample count value in
//!   the 'fact' chunk", establishing the `0xFFFFFFFF` deferral.

use crate::ds64::is_sentinel;
use crate::error::{Error, Result};

/// FourCC of the fact chunk.
pub const FOURCC_FACT: [u8; 4] = *b"fact";

/// Minimum on-wire body length: the single mandatory `dwSampleLength`
/// DWORD. Conforming bodies are at least this long; longer bodies carry
/// reserved trailing fields the spec sizes by the chunk header.
pub const FACT_MIN_LEN: usize = 4;

/// A decoded WAV `fact` chunk.
///
/// Carries the mandatory per-channel sample count plus any reserved
/// trailing bytes the writer appended (the spec lets future WAVE formats
/// extend the chunk and tells readers to size the extension by the chunk
/// header). The trailing bytes are preserved verbatim so a non-empty
/// `extra` round-trips and an unrecognised extension is never silently
/// dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fact {
    /// `dwSampleLength` — the number of samples per channel in the
    /// waveform. When this equals [`crate::ds64::SIZE_SENTINEL`] the file
    /// is an RF64 / BW64 variant deferring the true count to `ds64`; see
    /// [`Fact::is_deferred`].
    pub sample_length: u32,
    /// Reserved trailing bytes after `dwSampleLength` (empty for the
    /// classic 4-byte chunk). Retained verbatim for forward
    /// compatibility; their meaning is defined by whichever WAVE format
    /// extension wrote them.
    pub extra: Vec<u8>,
}

impl Fact {
    /// Decode a `fact` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be at least [`FACT_MIN_LEN`] bytes — the mandatory
    /// `dwSampleLength` DWORD. Any bytes beyond the first four are kept
    /// in [`Fact::extra`] rather than rejected, honouring the spec's
    /// forward-compatibility rule. A body shorter than four bytes is an
    /// `Error::invalid`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < FACT_MIN_LEN {
            return Err(Error::invalid(
                "RIFF: fact chunk shorter than 4-byte dwSampleLength",
            ));
        }
        let sample_length = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        Ok(Fact {
            sample_length,
            extra: body[FACT_MIN_LEN..].to_vec(),
        })
    }

    /// `true` if `dwSampleLength` is the RF64 / BW64 deferral sentinel
    /// (`0xFFFFFFFF`), meaning the real 64-bit sample count lives in the
    /// `ds64` chunk's `sampleCount` field and this field must not be
    /// trusted as a literal count.
    pub fn is_deferred(&self) -> bool {
        is_sentinel(self.sample_length)
    }

    /// The literal `dwSampleLength`, or `None` when it is the RF64 /
    /// BW64 deferral sentinel.
    ///
    /// Use this when a deferred 32-bit field should read as "unknown
    /// here" rather than the `0xFFFFFFFF` placeholder; resolve the true
    /// value from the `ds64` chunk in that case.
    pub fn sample_length(&self) -> Option<u32> {
        if self.is_deferred() {
            None
        } else {
            Some(self.sample_length)
        }
    }

    /// `true` if the chunk carries reserved trailing fields beyond the
    /// mandatory `dwSampleLength` (a future-format extension).
    pub fn has_extra(&self) -> bool {
        !self.extra.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ds64::SIZE_SENTINEL;

    /// Build a minimal 4-byte `fact` body from a sample count.
    fn body(sample_length: u32) -> Vec<u8> {
        sample_length.to_le_bytes().to_vec()
    }

    #[test]
    fn min_len_is_four() {
        assert_eq!(FACT_MIN_LEN, 4);
        assert_eq!(&FOURCC_FACT, b"fact");
    }

    #[test]
    fn parse_classic_pcm_sample_count() {
        let fact = Fact::parse(&body(44_100)).unwrap();
        assert_eq!(fact.sample_length, 44_100);
        assert_eq!(fact.sample_length(), Some(44_100));
        assert!(!fact.is_deferred());
        assert!(!fact.has_extra());
        assert!(fact.extra.is_empty());
    }

    #[test]
    fn parse_zero_sample_count() {
        let fact = Fact::parse(&body(0)).unwrap();
        assert_eq!(fact.sample_length, 0);
        assert_eq!(fact.sample_length(), Some(0));
        assert!(!fact.is_deferred());
    }

    #[test]
    fn rf64_sentinel_is_deferred() {
        let fact = Fact::parse(&body(SIZE_SENTINEL)).unwrap();
        assert!(fact.is_deferred());
        assert_eq!(fact.sample_length, SIZE_SENTINEL);
        assert_eq!(fact.sample_length(), None);
    }

    #[test]
    fn trailing_reserved_fields_are_retained() {
        // 4-byte count + a 4-byte reserved future field.
        let mut b = body(1_000);
        b.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let fact = Fact::parse(&b).unwrap();
        assert_eq!(fact.sample_length, 1_000);
        assert!(fact.has_extra());
        assert_eq!(fact.extra, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn short_body_is_rejected() {
        let err = Fact::parse(&[0u8; 3]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 4-byte dwSampleLength"));
    }

    #[test]
    fn empty_body_is_rejected() {
        let err = Fact::parse(&[]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 4-byte dwSampleLength"));
    }

    #[test]
    fn default_is_zero_no_extra() {
        let fact = Fact::default();
        assert_eq!(fact.sample_length, 0);
        assert!(fact.extra.is_empty());
        assert!(!fact.is_deferred());
    }
}
