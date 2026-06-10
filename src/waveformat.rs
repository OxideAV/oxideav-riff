//! Typed decoder for the WAV `fmt ` chunk body.
//!
//! The `fmt ` chunk of a RIFF/WAVE file carries a `WAVEFORMATEX`
//! (or, for modern surround / >16-bit / float / non-PCM content, a
//! `WAVEFORMATEXTENSIBLE`) descriptor. This module decodes that body
//! into the typed [`WaveFormat`] struct. It does **not** read the
//! chunk header — the caller pulls the `fmt ` body bytes out of the
//! [`crate::Walker`] and hands the slice here.
//!
//! ## Wire layout
//!
//! The four wave-format structures (`WAVEFORMAT`, `PCMWAVEFORMAT`,
//! `WAVEFORMATEX`, `WAVEFORMATEXTENSIBLE`) all begin with the same
//! five little-endian fields, the canonical 16-byte `WAVEFORMAT`
//! prefix:
//!
//! ```text
//! +0   wFormatTag      : u16 LE   codec format tag (RFC 2361 registry)
//! +2   nChannels       : u16 LE   interleaved channel count
//! +4   nSamplesPerSec  : u32 LE   sample rate (Hz)
//! +8   nAvgBytesPerSec : u32 LE   nominal byte rate (for buffer sizing)
//! +12  nBlockAlign     : u16 LE   bytes per sample-frame (all channels)
//! +14  wBitsPerSample  : u16 LE   container bits per sample
//! ```
//!
//! `WAVEFORMATEX` (the 18-byte form) appends a 2-byte `cbSize`
//! counting the extension bytes that follow:
//!
//! ```text
//! +16  cbSize          : u16 LE   length of the trailing extension
//! ```
//!
//! When `wFormatTag == WAVE_FORMAT_EXTENSIBLE (0xFFFE)` the 22-byte
//! extension is a `WAVEFORMATEXTENSIBLE` tail (`cbSize == 22`):
//!
//! ```text
//! +18  wValidBitsPerSample / wSamplesPerBlock / wReserved : u16 LE (union)
//! +20  dwChannelMask   : u32 LE   speaker-position bitmap
//! +24  SubFormat       : GUID     16-byte codec identifier
//! ```
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 — the
//!   1991 IBM + Microsoft base WAVE `fmt ` layout.
//! - `docs/container/riff/rfc2361-wav.txt` — the `wFormatTag`
//!   registry (PCM/IEEE_FLOAT/ALAW/MULAW values).
//! - `docs/container/riff/waveformatextensible/` — Microsoft Learn
//!   *WAVEFORMATEXTENSIBLE structure* + *Extensible Wave-Format
//!   Descriptors* + *Converting Between Format Tags and Subformat
//!   GUIDs* (the `DEFINE_WAVEFORMATEX_GUID` base template).

use crate::error::{Error, Result};

/// `wFormatTag` = uncompressed integer PCM. (`WAVE_FORMAT_PCM`.)
pub const WAVE_FORMAT_PCM: u16 = 0x0001;
/// `wFormatTag` = Microsoft ADPCM. (`WAVE_FORMAT_ADPCM`.)
pub const WAVE_FORMAT_ADPCM: u16 = 0x0002;
/// `wFormatTag` = IEEE floating-point PCM (32- or 64-bit).
/// (`WAVE_FORMAT_IEEE_FLOAT`.)
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
/// `wFormatTag` = ITU G.711 A-law companded PCM. (`WAVE_FORMAT_ALAW`.)
pub const WAVE_FORMAT_ALAW: u16 = 0x0006;
/// `wFormatTag` = ITU G.711 μ-law companded PCM. (`WAVE_FORMAT_MULAW`.)
pub const WAVE_FORMAT_MULAW: u16 = 0x0007;
/// `wFormatTag` = the `WAVEFORMATEXTENSIBLE` escape hatch; the actual
/// codec is named by the [`ExtensibleFields::sub_format`] GUID.
/// (`WAVE_FORMAT_EXTENSIBLE`.)
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// A 128-bit Microsoft `GUID` as it appears on the RIFF wire.
///
/// The serialized form is the classic mixed-endian Microsoft GUID
/// layout: `Data1` (u32) and `Data2` / `Data3` (u16) are
/// little-endian, while `Data4` (the 8-byte node + clock-seq tail) is
/// stored in big-endian byte order. [`Guid::from_le_wire`] performs
/// that decode; the canonical string form is rendered by [`Guid::to_hyphenated`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Guid {
    /// First 32-bit component (little-endian on the wire). For a
    /// `DEFINE_WAVEFORMATEX_GUID`-derived subtype this low word is the
    /// legacy 16-bit `wFormatTag`.
    pub data1: u32,
    /// Second 16-bit component (little-endian on the wire).
    pub data2: u16,
    /// Third 16-bit component (little-endian on the wire).
    pub data3: u16,
    /// Final 8 bytes (big-endian / verbatim on the wire).
    pub data4: [u8; 8],
}

/// The `xxxxxxxx-0000-0010-8000-00aa00389b71` base template GUID that
/// every `DEFINE_WAVEFORMATEX_GUID(tag)` subtype is built from (per
/// `ms-converting-format-tags-and-subformat-guids.md`). The low word
/// of `Data1` is the legacy `wFormatTag`.
pub const KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE: Guid = Guid {
    data1: 0x0000_0000,
    data2: 0x0000,
    data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};

impl Guid {
    /// Decode a 16-byte GUID from its on-wire (mixed-endian) bytes.
    pub const fn from_le_wire(b: &[u8; 16]) -> Self {
        Guid {
            data1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            data2: u16::from_le_bytes([b[4], b[5]]),
            data3: u16::from_le_bytes([b[6], b[7]]),
            data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        }
    }

    /// `true` if this GUID matches the `DEFINE_WAVEFORMATEX_GUID` base
    /// template (i.e. every field except `Data1` equals the
    /// [`KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE`] template).
    ///
    /// When this holds, [`Guid::data1`]'s low 16 bits recover the
    /// legacy `wFormatTag` via [`Guid::waveformatex_tag`].
    pub fn is_waveformatex_derived(&self) -> bool {
        self.data2 == KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data2
            && self.data3 == KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data3
            && self.data4 == KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data4
            // The template's high word of Data1 is always zero.
            && (self.data1 >> 16) == 0
    }

    /// Recover the legacy 16-bit `wFormatTag` from a
    /// `DEFINE_WAVEFORMATEX_GUID`-derived subtype, or `None` if this
    /// GUID does not follow that template
    /// (e.g. Dolby AC-3 / DTS, which have their own root GUIDs).
    pub fn waveformatex_tag(&self) -> Option<u16> {
        if self.is_waveformatex_derived() {
            Some(self.data1 as u16)
        } else {
            None
        }
    }

    /// Render the canonical hyphenated lower-case GUID string
    /// (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
    pub fn to_hyphenated(&self) -> String {
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7],
        )
    }
}

/// The `WAVEFORMATEXTENSIBLE` tail fields, present only when the
/// `fmt ` body carries the 0xFFFE extensible descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensibleFields {
    /// The `Samples` union: number of valid bits per sample
    /// (`wValidBitsPerSample`) for PCM/float, OR samples-per-block
    /// (`wSamplesPerBlock`) for packed codecs, OR a reserved zero.
    /// The active interpretation depends on the [`Guid`] sub-format;
    /// the raw 16-bit value is preserved here.
    pub samples: u16,
    /// Speaker-position bitmap (`SPEAKER_FRONT_LEFT` … `SPEAKER_ALL`).
    pub channel_mask: u32,
    /// 128-bit codec-identifying sub-format GUID.
    pub sub_format: Guid,
}

/// A decoded WAV `fmt ` chunk body.
///
/// Covers the `WAVEFORMAT` (16-byte) / `WAVEFORMATEX` (18-byte +
/// extension) / `WAVEFORMATEXTENSIBLE` (40-byte) forms. The five base
/// fields are always populated; [`WaveFormat::extension`] holds the
/// raw `cbSize`-counted extension bytes (if any), and
/// [`WaveFormat::extensible`] holds the parsed extensible tail when
/// `format_tag == WAVE_FORMAT_EXTENSIBLE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaveFormat {
    /// `wFormatTag` — codec format tag (RFC 2361 registry).
    pub format_tag: u16,
    /// `nChannels` — interleaved channel count.
    pub channels: u16,
    /// `nSamplesPerSec` — sample rate (Hz).
    pub sample_rate: u32,
    /// `nAvgBytesPerSec` — nominal byte rate.
    pub avg_bytes_per_sec: u32,
    /// `nBlockAlign` — bytes per sample-frame (all channels).
    pub block_align: u16,
    /// `wBitsPerSample` — container bits per sample.
    pub bits_per_sample: u16,
    /// Raw `cbSize`-counted extension bytes that follow the 18-byte
    /// `WAVEFORMATEX` header. Empty for a bare 16-byte `WAVEFORMAT`
    /// body or an 18-byte `WAVEFORMATEX` with `cbSize == 0`.
    pub extension: Vec<u8>,
    /// Parsed `WAVEFORMATEXTENSIBLE` tail, present only when
    /// `format_tag == WAVE_FORMAT_EXTENSIBLE` and the extension is at
    /// least 22 bytes.
    pub extensible: Option<ExtensibleFields>,
}

impl WaveFormat {
    /// Parse a `fmt ` chunk body.
    ///
    /// Accepts the bare 16-byte `WAVEFORMAT` prefix, the 18-byte
    /// `WAVEFORMATEX` form, and any `cbSize`-counted extension
    /// (including the 22-byte `WAVEFORMATEXTENSIBLE` tail).
    ///
    /// Errors:
    /// - body shorter than the 16-byte base prefix;
    /// - body declares a `WAVEFORMATEX` header (≥ 18 bytes) whose
    ///   `cbSize` over-runs the bytes actually present;
    /// - `format_tag == 0xFFFE` but fewer than 22 extension bytes are
    ///   present (a malformed extensible descriptor).
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 16 {
            return Err(Error::invalid(
                "RIFF fmt: body shorter than the 16-byte WAVEFORMAT prefix",
            ));
        }
        let format_tag = u16::from_le_bytes([body[0], body[1]]);
        let channels = u16::from_le_bytes([body[2], body[3]]);
        let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
        let avg_bytes_per_sec = u32::from_le_bytes([body[8], body[9], body[10], body[11]]);
        let block_align = u16::from_le_bytes([body[12], body[13]]);
        let bits_per_sample = u16::from_le_bytes([body[14], body[15]]);

        // The optional WAVEFORMATEX cbSize field starts at +16.
        let extension = if body.len() >= 18 {
            let cb_size = u16::from_le_bytes([body[16], body[17]]) as usize;
            let avail = body.len() - 18;
            if cb_size > avail {
                return Err(Error::invalid(
                    "RIFF fmt: cbSize over-runs the fmt body length",
                ));
            }
            body[18..18 + cb_size].to_vec()
        } else {
            // A 16- or 17-byte body is a bare WAVEFORMAT prefix with no
            // cbSize field; treat it as a zero-length extension.
            Vec::new()
        };

        let extensible = if format_tag == WAVE_FORMAT_EXTENSIBLE {
            if extension.len() < 22 {
                return Err(Error::invalid(
                    "RIFF fmt: WAVE_FORMAT_EXTENSIBLE needs a 22-byte extension tail",
                ));
            }
            let samples = u16::from_le_bytes([extension[0], extension[1]]);
            let channel_mask =
                u32::from_le_bytes([extension[2], extension[3], extension[4], extension[5]]);
            let mut guid = [0u8; 16];
            guid.copy_from_slice(&extension[6..22]);
            Some(ExtensibleFields {
                samples,
                channel_mask,
                sub_format: Guid::from_le_wire(&guid),
            })
        } else {
            None
        };

        Ok(WaveFormat {
            format_tag,
            channels,
            sample_rate,
            avg_bytes_per_sec,
            block_align,
            bits_per_sample,
            extension,
            extensible,
        })
    }

    /// `true` if this descriptor is the `WAVEFORMATEXTENSIBLE` form
    /// (`format_tag == 0xFFFE` with a parsed [`ExtensibleFields`] tail).
    pub const fn is_extensible(&self) -> bool {
        self.extensible.is_some()
    }

    /// The codec's *effective* format tag.
    ///
    /// For a plain descriptor this is [`WaveFormat::format_tag`]; for a
    /// `WAVEFORMATEXTENSIBLE` whose `SubFormat` follows the
    /// `DEFINE_WAVEFORMATEX_GUID` template, this resolves the GUID back
    /// to its legacy 16-bit tag (so `0xFFFE` PCM reports `0x0001`).
    /// Returns `None` for an extensible descriptor whose `SubFormat`
    /// is a non-template GUID (Dolby AC-3, DTS, …).
    pub fn effective_format_tag(&self) -> Option<u16> {
        match &self.extensible {
            Some(ext) => ext.sub_format.waveformatex_tag(),
            None => Some(self.format_tag),
        }
    }

    /// Number of speakers set in the `dwChannelMask` (only meaningful
    /// for an extensible descriptor). Returns `None` for non-extensible
    /// formats. The spec recommends this equal
    /// [`WaveFormat::channels`].
    pub fn channel_mask_count(&self) -> Option<u32> {
        self.extensible
            .as_ref()
            .map(|e| e.channel_mask.count_ones())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16-bit / 44.1 kHz / stereo PCM as a bare 16-byte WAVEFORMAT.
    fn pcm16_waveformat() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&WAVE_FORMAT_PCM.to_le_bytes()); // wFormatTag
        v.extend_from_slice(&2u16.to_le_bytes()); // nChannels
        v.extend_from_slice(&44_100u32.to_le_bytes()); // nSamplesPerSec
        v.extend_from_slice(&176_400u32.to_le_bytes()); // nAvgBytesPerSec
        v.extend_from_slice(&4u16.to_le_bytes()); // nBlockAlign
        v.extend_from_slice(&16u16.to_le_bytes()); // wBitsPerSample
        v
    }

    #[test]
    fn parse_bare_waveformat_prefix() {
        let f = WaveFormat::parse(&pcm16_waveformat()).unwrap();
        assert_eq!(f.format_tag, WAVE_FORMAT_PCM);
        assert_eq!(f.channels, 2);
        assert_eq!(f.sample_rate, 44_100);
        assert_eq!(f.avg_bytes_per_sec, 176_400);
        assert_eq!(f.block_align, 4);
        assert_eq!(f.bits_per_sample, 16);
        assert!(f.extension.is_empty());
        assert!(!f.is_extensible());
        assert_eq!(f.effective_format_tag(), Some(WAVE_FORMAT_PCM));
        assert_eq!(f.channel_mask_count(), None);
    }

    #[test]
    fn parse_waveformatex_with_zero_cbsize() {
        let mut v = pcm16_waveformat();
        v.extend_from_slice(&0u16.to_le_bytes()); // cbSize = 0
        let f = WaveFormat::parse(&v).unwrap();
        assert_eq!(f.format_tag, WAVE_FORMAT_PCM);
        assert!(f.extension.is_empty());
        assert!(!f.is_extensible());
    }

    #[test]
    fn parse_waveformatex_with_extension_bytes() {
        // e.g. an ADPCM fmt body with a 4-byte codec-private extension.
        let mut v = pcm16_waveformat();
        v[0..2].copy_from_slice(&WAVE_FORMAT_ADPCM.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes()); // cbSize = 4
        v.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let f = WaveFormat::parse(&v).unwrap();
        assert_eq!(f.format_tag, WAVE_FORMAT_ADPCM);
        assert_eq!(f.extension, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(!f.is_extensible());
    }

    #[test]
    fn parse_waveformatextensible_pcm() {
        // 24-bit-in-32-bit PCM, 5.1 (Microsoft) layout = 0x3F.
        let mut v = Vec::new();
        v.extend_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        v.extend_from_slice(&6u16.to_le_bytes()); // nChannels = 6
        v.extend_from_slice(&48_000u32.to_le_bytes());
        v.extend_from_slice(&864_000u32.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes()); // nBlockAlign
        v.extend_from_slice(&32u16.to_le_bytes()); // wBitsPerSample (container)
        v.extend_from_slice(&22u16.to_le_bytes()); // cbSize = 22
        v.extend_from_slice(&24u16.to_le_bytes()); // wValidBitsPerSample = 24
        v.extend_from_slice(&0x0000_003Fu32.to_le_bytes()); // dwChannelMask = 5.1
                                                            // SubFormat = KSDATAFORMAT_SUBTYPE_PCM
                                                            // (00000001-0000-0010-8000-00aa00389b71)
        v.extend_from_slice(&1u32.to_le_bytes()); // Data1 = 1 (LE)
        v.extend_from_slice(&0u16.to_le_bytes()); // Data2
        v.extend_from_slice(&0x0010u16.to_le_bytes()); // Data3
        v.extend_from_slice(&[0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]); // Data4

        let f = WaveFormat::parse(&v).unwrap();
        assert_eq!(f.format_tag, WAVE_FORMAT_EXTENSIBLE);
        assert_eq!(f.channels, 6);
        assert_eq!(f.bits_per_sample, 32);
        assert!(f.is_extensible());
        let ext = f.extensible.as_ref().unwrap();
        assert_eq!(ext.samples, 24);
        assert_eq!(ext.channel_mask, 0x3F);
        assert_eq!(f.channel_mask_count(), Some(6));
        // The PCM subtype resolves back to the legacy WAVE_FORMAT_PCM tag.
        assert_eq!(ext.sub_format.waveformatex_tag(), Some(WAVE_FORMAT_PCM));
        assert_eq!(f.effective_format_tag(), Some(WAVE_FORMAT_PCM));
        assert_eq!(
            ext.sub_format.to_hyphenated(),
            "00000001-0000-0010-8000-00aa00389b71"
        );
    }

    #[test]
    fn extensible_with_non_template_subformat_has_no_legacy_tag() {
        let mut v = Vec::new();
        v.extend_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&48_000u32.to_le_bytes());
        v.extend_from_slice(&192_000u32.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(&22u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes()); // wValidBitsPerSample
        v.extend_from_slice(&0x0000_0003u32.to_le_bytes()); // stereo
                                                            // A fabricated non-template GUID (different Data4 tail).
        v.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        v.extend_from_slice(&0xABCDu16.to_le_bytes());
        v.extend_from_slice(&0xEF01u16.to_le_bytes());
        v.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);

        let f = WaveFormat::parse(&v).unwrap();
        let ext = f.extensible.as_ref().unwrap();
        assert!(!ext.sub_format.is_waveformatex_derived());
        assert_eq!(ext.sub_format.waveformatex_tag(), None);
        assert_eq!(f.effective_format_tag(), None);
        assert_eq!(
            ext.sub_format.to_hyphenated(),
            "12345678-abcd-ef01-1122-334455667788"
        );
    }

    #[test]
    fn parse_rejects_short_body() {
        let err = WaveFormat::parse(&[0u8; 15]).unwrap_err();
        assert!(format!("{err}").contains("16-byte WAVEFORMAT prefix"));
    }

    #[test]
    fn parse_rejects_cbsize_overrun() {
        let mut v = pcm16_waveformat();
        v.extend_from_slice(&10u16.to_le_bytes()); // cbSize claims 10
        v.extend_from_slice(&[0xAA, 0xBB]); // but only 2 present
        let err = WaveFormat::parse(&v).unwrap_err();
        assert!(format!("{err}").contains("cbSize over-runs"));
    }

    #[test]
    fn parse_rejects_truncated_extensible_tail() {
        let mut v = pcm16_waveformat();
        v[0..2].copy_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes()); // cbSize = 4 (< 22)
        v.extend_from_slice(&[0, 0, 0, 0]);
        let err = WaveFormat::parse(&v).unwrap_err();
        assert!(format!("{err}").contains("22-byte extension tail"));
    }

    #[test]
    fn guid_from_wire_decodes_mixed_endian() {
        // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT on the wire.
        let wire = [
            0x03, 0x00, 0x00, 0x00, // Data1 = 3 (LE)
            0x00, 0x00, // Data2 = 0
            0x10, 0x00, // Data3 = 0x10
            0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71, // Data4 (BE)
        ];
        let g = Guid::from_le_wire(&wire);
        assert_eq!(g.data1, 3);
        assert_eq!(g.data3, 0x0010);
        assert_eq!(g.waveformatex_tag(), Some(WAVE_FORMAT_IEEE_FLOAT));
        assert_eq!(g.to_hyphenated(), "00000003-0000-0010-8000-00aa00389b71");
    }

    #[test]
    fn waveformatex_base_template_recognised() {
        assert!(KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.is_waveformatex_derived());
        assert_eq!(
            KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.waveformatex_tag(),
            Some(0)
        );
    }
}
