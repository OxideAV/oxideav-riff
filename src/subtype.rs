//! Named `KSDATAFORMAT_SUBTYPE_*` GUID catalogue.
//!
//! A [`crate::waveformat::WAVEFORMATEXTENSIBLE`] descriptor names its
//! codec by a 128-bit `SubFormat` GUID rather than the legacy 16-bit
//! `wFormatTag`. The [`crate::waveformat`] module already resolves the
//! `DEFINE_WAVEFORMATEX_GUID`-template subtypes (whose `Data1` low word
//! recovers the legacy tag) back to that tag. This module sits on top
//! and gives those GUIDs their **symbolic identity**: it recognises the
//! published `KSDATAFORMAT_SUBTYPE_*` constants by full 128-bit value,
//! returning a [`KsSubtype`] that carries the symbolic name, the codec
//! it denotes, and — for the IEC 61937 compressed-passthrough family —
//! its CEA-861 stream-type index.
//!
//! ## Two GUID families
//!
//! 1. **`WAVEFORMATEX`-derived** — built by `DEFINE_WAVEFORMATEX_GUID(x)`
//!    from the
//!    `00000000-0000-0010-8000-00aa00389b71` base template, with the
//!    legacy `wFormatTag` substituted into the `Data1` low word. The
//!    tail `-0000-0010-8000-00aa00389b71` is fixed.
//! 2. **IEC 61937 compressed-passthrough** (Windows 7+) — for
//!    S/PDIF / HDMI bitstream passthrough. The `Data2` field carries
//!    the discriminator `0x0cea` (CEA, for CEA-861) instead of
//!    `0x0000`; `Data1`'s low word is then a CEA-861 *stream-type*
//!    index, **not** a `wFormatTag`. A few of the earliest passthrough
//!    GUIDs (PCM, AC-3, DTS) instead alias into the `WAVEFORMATEX`
//!    family (`Data2 == 0x0000`).
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/waveformatextensible/ksdataformat-subtype-guids.md`
//!   — consolidated `KSDATAFORMAT_SUBTYPE_*` catalogue (the row table
//!   transcribed below).
//! - `docs/container/riff/waveformatextensible/ms-subformat-guids-compressed-audio.md`
//!   — Microsoft Learn *Subformat GUIDs for Compressed Audio Formats*
//!   (the CEA-861 stream-type ↔ GUID table for the IEC 61937 family).
//! - `docs/container/riff/waveformatextensible/ms-converting-format-tags-and-subformat-guids.md`
//!   — *Converting Between Format Tags and Subformat GUIDs* (the
//!   `DEFINE_WAVEFORMATEX_GUID` base template + the
//!   `WAVE_FORMAT_DOLBY_AC3_SPDIF` worked example).

use crate::waveformat::{Guid, KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE};

/// The `0x0cea` `Data2` discriminator that marks the IEC 61937
/// compressed-passthrough GUID family (CEA-861). When a `SubFormat`
/// GUID carries this value, its `Data1` low word is a CEA-861
/// stream-type index, **not** a `WAVE_FORMAT_*` tag.
pub const IEC61937_DATA2: u16 = 0x0cea;

/// Build a `WAVEFORMATEX`-derived subtype GUID for a legacy
/// `wFormatTag`: the `DEFINE_WAVEFORMATEX_GUID(tag)` expansion.
///
/// The tag occupies the `Data1` low word; everything else equals the
/// [`KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE`] template.
pub const fn waveformatex_guid(tag: u16) -> Guid {
    Guid {
        data1: tag as u32,
        data2: KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data2,
        data3: KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data3,
        data4: KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data4,
    }
}

/// Build an IEC 61937 native passthrough subtype GUID for a CEA-861
/// stream-type index: the `xxxxxxxx-0cea-0010-8000-00aa00389b71`
/// template with the index in the `Data1` low word and the
/// [`IEC61937_DATA2`] discriminator in `Data2`.
pub const fn iec61937_guid(stream_type_index: u16) -> Guid {
    Guid {
        data1: stream_type_index as u32,
        data2: IEC61937_DATA2,
        data3: KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data3,
        data4: KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data4,
    }
}

/// A recognised `KSDATAFORMAT_SUBTYPE_*` GUID.
///
/// Returned by [`KsSubtype::resolve`]. The variants split into the two
/// families described in the module docs: the [`KsSubtype::WaveFormatEx`]
/// family (which folds back to a legacy `wFormatTag`) and the
/// [`KsSubtype::Iec61937`] passthrough family (which folds to a CEA-861
/// stream-type index). [`KsSubtype::Other`] preserves the raw GUID for a
/// value that follows neither template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsSubtype {
    /// A `WAVEFORMATEX`-derived subtype (tail matches the base
    /// template, `Data2 == 0x0000`). The legacy `wFormatTag` is the
    /// `Data1` low word; the symbolic name + codec come from
    /// [`waveformatex_name`].
    WaveFormatEx {
        /// The legacy 16-bit `WAVE_FORMAT_*` tag.
        tag: u16,
    },
    /// An IEC 61937 native compressed-passthrough subtype
    /// (`Data2 == 0x0cea`). The `Data1` low word is the CEA-861
    /// stream-type index.
    Iec61937 {
        /// CEA-861 stream-type index (`0x03` = MPEG-1, …).
        cea861_type: u16,
    },
    /// A GUID that matches neither template — a vendor/proprietary root
    /// GUID. The raw value is preserved for full-128-bit matching by
    /// the caller.
    Other(Guid),
}

impl KsSubtype {
    /// Classify a decoded `SubFormat` [`Guid`] into its family.
    ///
    /// - tail matches the base template and `Data2 == 0x0000` →
    ///   [`KsSubtype::WaveFormatEx`] (the `Data1` low word is the legacy
    ///   tag);
    /// - tail matches the base template and `Data2 == 0x0cea` →
    ///   [`KsSubtype::Iec61937`] (the `Data1` low word is the CEA-861
    ///   stream-type index);
    /// - otherwise → [`KsSubtype::Other`] carrying the raw GUID.
    pub fn resolve(guid: &Guid) -> Self {
        let tail_matches = guid.data3 == KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data3
            && guid.data4 == KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data4
            && (guid.data1 >> 16) == 0;
        if tail_matches && guid.data2 == KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE.data2 {
            KsSubtype::WaveFormatEx {
                tag: guid.data1 as u16,
            }
        } else if tail_matches && guid.data2 == IEC61937_DATA2 {
            KsSubtype::Iec61937 {
                cea861_type: guid.data1 as u16,
            }
        } else {
            KsSubtype::Other(*guid)
        }
    }

    /// The published symbolic `KSDATAFORMAT_SUBTYPE_*` constant name for
    /// this subtype, or `None` if the GUID is a `WaveFormatEx` tag with
    /// no catalogued symbolic name, or an `Other` vendor GUID.
    pub fn symbolic_name(&self) -> Option<&'static str> {
        match self {
            KsSubtype::WaveFormatEx { tag } => waveformatex_name(*tag).map(|(name, _)| name),
            KsSubtype::Iec61937 { cea861_type } => {
                iec61937_name(*cea861_type).map(|(name, _)| name)
            }
            KsSubtype::Other(_) => None,
        }
    }

    /// A short human-readable codec / meaning string for this subtype,
    /// or `None` if uncatalogued.
    pub fn description(&self) -> Option<&'static str> {
        match self {
            KsSubtype::WaveFormatEx { tag } => waveformatex_name(*tag).map(|(_, desc)| desc),
            KsSubtype::Iec61937 { cea861_type } => {
                iec61937_name(*cea861_type).map(|(_, desc)| desc)
            }
            KsSubtype::Other(_) => None,
        }
    }
}

/// Symbolic name + codec description for a catalogued
/// `WAVEFORMATEX`-derived subtype tag, or `None` if the tag is not in
/// the staged catalogue's Family-1 table.
///
/// Only the tags the catalogue's Family-1 table lists by symbolic name
/// are returned; any other valid `wFormatTag` still resolves to a
/// `WaveFormatEx` subtype via [`KsSubtype::resolve`], it just has no
/// catalogued `KSDATAFORMAT_SUBTYPE_*` symbol here.
pub fn waveformatex_name(tag: u16) -> Option<(&'static str, &'static str)> {
    Some(match tag {
        0x0000 => ("KSDATAFORMAT_SUBTYPE_WAVEFORMATEX", "Generic base template"),
        0x0001 => ("KSDATAFORMAT_SUBTYPE_PCM", "Linear PCM"),
        0x0002 => ("KSDATAFORMAT_SUBTYPE_ADPCM", "Microsoft ADPCM"),
        0x0003 => (
            "KSDATAFORMAT_SUBTYPE_IEEE_FLOAT",
            "IEEE 32-/64-bit float PCM",
        ),
        0x0006 => ("KSDATAFORMAT_SUBTYPE_ALAW", "A-law companded"),
        0x0007 => ("KSDATAFORMAT_SUBTYPE_MULAW", "Mu-law companded"),
        0x0008 => ("KSDATAFORMAT_SUBTYPE_DTS", "DTS (in WAVEFORMATEX framing)"),
        0x0009 => ("KSDATAFORMAT_SUBTYPE_DRM", "DRM-protected audio"),
        0x0050 => ("KSDATAFORMAT_SUBTYPE_MPEG", "MPEG-1 audio (Layer 1/2)"),
        0x0092 => (
            "KSDATAFORMAT_SUBTYPE_DOLBY_AC3_SPDIF",
            "Dolby AC-3 over S/PDIF",
        ),
        _ => return None,
    })
}

/// Symbolic name + codec description for a catalogued IEC 61937
/// passthrough subtype, keyed by CEA-861 stream-type index, or `None`
/// for an uncatalogued / reserved index.
pub fn iec61937_name(cea861_type: u16) -> Option<(&'static str, &'static str)> {
    Some(match cea861_type {
        0x03 => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_MPEG1",
            "MPEG-1 (Layer 1 & 2)",
        ),
        0x05 => ("KSDATAFORMAT_SUBTYPE_IEC61937_MPEG3", "MPEG (Layer 3)"),
        0x04 => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_MPEG2",
            "MPEG-2 (multichannel)",
        ),
        0x06 => ("KSDATAFORMAT_SUBTYPE_IEC61937_AAC", "MPEG-2/4 AAC in ADTS"),
        0x08 => ("KSDATAFORMAT_SUBTYPE_IEC61937_ATRAC", "Sony ATRAC"),
        0x09 => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_ONE_BIT_AUDIO",
            "One-bit audio",
        ),
        0x0a => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS",
            "Dolby Digital Plus (E-AC-3)",
        ),
        0x0b => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD",
            "DTS-HD (24-bit / 96 kHz)",
        ),
        0x0c => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MLP",
            "MAT (MLP) — Dolby TrueHD",
        ),
        0x0d => (
            "KSDATAFORMAT_SUBTYPE_IEC61937_DST",
            "Direct Stream Transport",
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveformat::{
        WAVE_FORMAT_ALAW, WAVE_FORMAT_IEEE_FLOAT, WAVE_FORMAT_MULAW, WAVE_FORMAT_PCM,
    };

    #[test]
    fn waveformatex_guid_builds_template() {
        let g = waveformatex_guid(WAVE_FORMAT_PCM);
        assert_eq!(g.to_hyphenated(), "00000001-0000-0010-8000-00aa00389b71");
        // The base template recovers tag 0.
        assert_eq!(
            waveformatex_guid(0).to_hyphenated(),
            "00000000-0000-0010-8000-00aa00389b71"
        );
        // Worked example from the *Converting…* page.
        assert_eq!(
            waveformatex_guid(0x0092).to_hyphenated(),
            "00000092-0000-0010-8000-00aa00389b71"
        );
    }

    #[test]
    fn iec61937_guid_builds_template() {
        // CEA-861 type 0x03 = MPEG-1 → 00000003-0cea-…
        assert_eq!(
            iec61937_guid(0x03).to_hyphenated(),
            "00000003-0cea-0010-8000-00aa00389b71"
        );
        // Dolby Digital Plus index 0x0a.
        assert_eq!(
            iec61937_guid(0x0a).to_hyphenated(),
            "0000000a-0cea-0010-8000-00aa00389b71"
        );
    }

    #[test]
    fn resolve_waveformatex_pcm() {
        let g = waveformatex_guid(WAVE_FORMAT_PCM);
        let s = KsSubtype::resolve(&g);
        assert_eq!(
            s,
            KsSubtype::WaveFormatEx {
                tag: WAVE_FORMAT_PCM
            }
        );
        assert_eq!(s.symbolic_name(), Some("KSDATAFORMAT_SUBTYPE_PCM"));
        assert_eq!(s.description(), Some("Linear PCM"));
    }

    #[test]
    fn resolve_waveformatex_float_alaw_mulaw() {
        assert_eq!(
            KsSubtype::resolve(&waveformatex_guid(WAVE_FORMAT_IEEE_FLOAT)).symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_IEEE_FLOAT")
        );
        assert_eq!(
            KsSubtype::resolve(&waveformatex_guid(WAVE_FORMAT_ALAW)).symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_ALAW")
        );
        assert_eq!(
            KsSubtype::resolve(&waveformatex_guid(WAVE_FORMAT_MULAW)).symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_MULAW")
        );
    }

    #[test]
    fn resolve_ac3_spdif_worked_example() {
        let g = waveformatex_guid(0x0092);
        let s = KsSubtype::resolve(&g);
        assert_eq!(s, KsSubtype::WaveFormatEx { tag: 0x0092 });
        assert_eq!(
            s.symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_DOLBY_AC3_SPDIF")
        );
    }

    #[test]
    fn resolve_iec61937_native_family() {
        // 00000003-0cea-… = IEC61937_MPEG1, CEA-861 type 0x03.
        let g = iec61937_guid(0x03);
        let s = KsSubtype::resolve(&g);
        assert_eq!(s, KsSubtype::Iec61937 { cea861_type: 0x03 });
        assert_eq!(
            s.symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_IEC61937_MPEG1")
        );
        assert_eq!(s.description(), Some("MPEG-1 (Layer 1 & 2)"));

        // Dolby Digital Plus, index 0x0a.
        assert_eq!(
            KsSubtype::resolve(&iec61937_guid(0x0a)).symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS")
        );
        // DTS-HD high-bit-rate, index 0x0b.
        assert_eq!(
            KsSubtype::resolve(&iec61937_guid(0x0b)).symbolic_name(),
            Some("KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD")
        );
    }

    #[test]
    fn iec61937_data2_discriminates_from_waveformatex() {
        // Same Data1 low word (0x03) but the 0x0cea Data2 routes it to
        // the IEC 61937 family, NOT WAVEFORMATEX/IEEE_FLOAT (tag 0x03).
        let wfx = KsSubtype::resolve(&waveformatex_guid(0x03));
        let iec = KsSubtype::resolve(&iec61937_guid(0x03));
        assert_eq!(wfx, KsSubtype::WaveFormatEx { tag: 0x03 });
        assert_eq!(iec, KsSubtype::Iec61937 { cea861_type: 0x03 });
        assert_ne!(wfx.symbolic_name(), iec.symbolic_name());
    }

    #[test]
    fn waveformatex_tag_without_catalogued_name() {
        // MP3 tag 0x0055 is a valid WAVEFORMATEX-derived GUID but is not
        // in the Family-1 named table — it resolves as WaveFormatEx with
        // no symbolic name.
        let g = waveformatex_guid(0x0055);
        let s = KsSubtype::resolve(&g);
        assert_eq!(s, KsSubtype::WaveFormatEx { tag: 0x0055 });
        assert_eq!(s.symbolic_name(), None);
        assert_eq!(s.description(), None);
    }

    #[test]
    fn resolve_non_template_guid_is_other() {
        // A fabricated vendor GUID with a different Data4 tail.
        let g = Guid {
            data1: 0x1234_5678,
            data2: 0xABCD,
            data3: 0xEF01,
            data4: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        };
        let s = KsSubtype::resolve(&g);
        assert_eq!(s, KsSubtype::Other(g));
        assert_eq!(s.symbolic_name(), None);
        assert_eq!(s.description(), None);
    }

    #[test]
    fn iec61937_reserved_index_has_no_name() {
        // CEA-861 0x00 = "refer to the stream", 0x0f = reserved.
        assert_eq!(
            KsSubtype::resolve(&iec61937_guid(0x00)).symbolic_name(),
            None
        );
        assert_eq!(
            KsSubtype::resolve(&iec61937_guid(0x0f)).symbolic_name(),
            None
        );
    }
}
