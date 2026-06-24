//! `WAVEFORMATEXTENSIBLE` `dwChannelMask` speaker-position decoding.
//!
//! The extensible `fmt ` descriptor carries a 32-bit `dwChannelMask`
//! speaker-assignment bitmap. Each set bit names a physical speaker
//! position the corresponding interleaved channel feeds, and — crucially
//! — **the channels are stored in the file in the bit order of the mask**
//! (lowest set bit first). This module decomposes a raw mask into the
//! ordered list of [`SpeakerPosition`]s it names, recognises the standard
//! channel layouts, and provides the channel-index ↔ speaker-position
//! mapping a renderer needs to route a `WAVEFORMATEXTENSIBLE` stream to a
//! physical speaker set.
//!
//! This is **container metadata only** — the bit assignments, the
//! channel-ordering rule, and the standard layouts are all part of the
//! `WAVEFORMATEXTENSIBLE` wire contract, not the audio codec carried in
//! the `data` chunk.
//!
//! ## Clean-room source
//!
//! `docs/container/riff/waveformatextensible/README.md` — the
//! `dwChannelMask` bit-assignment table (the 18 standard `SPEAKER_*`
//! positions plus the reserved high bits and `SPEAKER_ALL`), the
//! channel-ordering rule ("stored in the file in the bit order of the
//! mask, lowest set bit first"), and the standard-layout table (Mono /
//! Stereo / 2.1 / Quad / 5.1 / 7.1).

#[cfg(not(feature = "registry"))]
use crate::error::Error;
#[cfg(feature = "registry")]
use oxideav_core::Error;

use crate::Result;

/// One of the named speaker positions a `dwChannelMask` bit assigns.
///
/// The discriminant of each variant is the bit **mask** (`1 << index`)
/// it occupies in `dwChannelMask`, matching the `SPEAKER_*` constants
/// from `mmreg.h` / `ksmedia.h`. The 18 standard positions (bits 0–17)
/// are the assignable speaker locations; [`SpeakerPosition::All`] is the
/// `SPEAKER_ALL` (`0x8000_0000`) catch-all flag rather than a discrete
/// speaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum SpeakerPosition {
    /// `SPEAKER_FRONT_LEFT` — Front Left (FL).
    FrontLeft = 0x0000_0001,
    /// `SPEAKER_FRONT_RIGHT` — Front Right (FR).
    FrontRight = 0x0000_0002,
    /// `SPEAKER_FRONT_CENTER` — Front Center (FC).
    FrontCenter = 0x0000_0004,
    /// `SPEAKER_LOW_FREQUENCY` — Low Frequency Effects (LFE).
    LowFrequency = 0x0000_0008,
    /// `SPEAKER_BACK_LEFT` — Back Left (BL).
    BackLeft = 0x0000_0010,
    /// `SPEAKER_BACK_RIGHT` — Back Right (BR).
    BackRight = 0x0000_0020,
    /// `SPEAKER_FRONT_LEFT_OF_CENTER` — Front Left of Center (FLC).
    FrontLeftOfCenter = 0x0000_0040,
    /// `SPEAKER_FRONT_RIGHT_OF_CENTER` — Front Right of Center (FRC).
    FrontRightOfCenter = 0x0000_0080,
    /// `SPEAKER_BACK_CENTER` — Back Center (BC).
    BackCenter = 0x0000_0100,
    /// `SPEAKER_SIDE_LEFT` — Side Left (SL).
    SideLeft = 0x0000_0200,
    /// `SPEAKER_SIDE_RIGHT` — Side Right (SR).
    SideRight = 0x0000_0400,
    /// `SPEAKER_TOP_CENTER` — Top Center (TC).
    TopCenter = 0x0000_0800,
    /// `SPEAKER_TOP_FRONT_LEFT` — Top Front Left (TFL).
    TopFrontLeft = 0x0000_1000,
    /// `SPEAKER_TOP_FRONT_CENTER` — Top Front Center (TFC).
    TopFrontCenter = 0x0000_2000,
    /// `SPEAKER_TOP_FRONT_RIGHT` — Top Front Right (TFR).
    TopFrontRight = 0x0000_4000,
    /// `SPEAKER_TOP_BACK_LEFT` — Top Back Left (TBL).
    TopBackLeft = 0x0000_8000,
    /// `SPEAKER_TOP_BACK_CENTER` — Top Back Center (TBC).
    TopBackCenter = 0x0001_0000,
    /// `SPEAKER_TOP_BACK_RIGHT` — Top Back Right (TBR).
    TopBackRight = 0x0002_0000,
    /// `SPEAKER_ALL` (`0x8000_0000`) — the catch-all flag a stream sets
    /// to request "all speakers" rather than naming a discrete position.
    All = 0x8000_0000,
}

/// The bitmask of every standard discrete `SPEAKER_*` position (bits
/// 0–17). The two high bits this crate models — `SPEAKER_RESERVED`
/// (everything between bit 18 and bit 30) and [`SpeakerPosition::All`]
/// (bit 31) — are **not** in this set.
pub const SPEAKER_STANDARD_MASK: u32 = 0x0003_FFFF;

/// `SPEAKER_ALL` — the catch-all "all speakers" flag (bit 31).
pub const SPEAKER_ALL: u32 = 0x8000_0000;

/// The reserved-bit region (bits 18..=30): no `SPEAKER_*` constant is
/// assigned here in the published table, but a stream may set them and a
/// reader must not reject them.
pub const SPEAKER_RESERVED_MASK: u32 = 0x7FFC_0000;

impl SpeakerPosition {
    /// The 18 standard discrete positions, in ascending bit order — which
    /// is also the in-file channel order a mask using them implies.
    pub const STANDARD: [SpeakerPosition; 18] = [
        SpeakerPosition::FrontLeft,
        SpeakerPosition::FrontRight,
        SpeakerPosition::FrontCenter,
        SpeakerPosition::LowFrequency,
        SpeakerPosition::BackLeft,
        SpeakerPosition::BackRight,
        SpeakerPosition::FrontLeftOfCenter,
        SpeakerPosition::FrontRightOfCenter,
        SpeakerPosition::BackCenter,
        SpeakerPosition::SideLeft,
        SpeakerPosition::SideRight,
        SpeakerPosition::TopCenter,
        SpeakerPosition::TopFrontLeft,
        SpeakerPosition::TopFrontCenter,
        SpeakerPosition::TopFrontRight,
        SpeakerPosition::TopBackLeft,
        SpeakerPosition::TopBackCenter,
        SpeakerPosition::TopBackRight,
    ];

    /// The single-bit mask this position occupies in `dwChannelMask`.
    pub const fn bit(self) -> u32 {
        self as u32
    }

    /// Resolve a single-bit mask to its position, if it names one of the
    /// 18 standard positions or `SPEAKER_ALL`. Returns `None` for a
    /// reserved bit, a zero mask, or a multi-bit value.
    pub const fn from_bit(bit: u32) -> Option<SpeakerPosition> {
        match bit {
            0x0000_0001 => Some(SpeakerPosition::FrontLeft),
            0x0000_0002 => Some(SpeakerPosition::FrontRight),
            0x0000_0004 => Some(SpeakerPosition::FrontCenter),
            0x0000_0008 => Some(SpeakerPosition::LowFrequency),
            0x0000_0010 => Some(SpeakerPosition::BackLeft),
            0x0000_0020 => Some(SpeakerPosition::BackRight),
            0x0000_0040 => Some(SpeakerPosition::FrontLeftOfCenter),
            0x0000_0080 => Some(SpeakerPosition::FrontRightOfCenter),
            0x0000_0100 => Some(SpeakerPosition::BackCenter),
            0x0000_0200 => Some(SpeakerPosition::SideLeft),
            0x0000_0400 => Some(SpeakerPosition::SideRight),
            0x0000_0800 => Some(SpeakerPosition::TopCenter),
            0x0000_1000 => Some(SpeakerPosition::TopFrontLeft),
            0x0000_2000 => Some(SpeakerPosition::TopFrontCenter),
            0x0000_4000 => Some(SpeakerPosition::TopFrontRight),
            0x0000_8000 => Some(SpeakerPosition::TopBackLeft),
            0x0001_0000 => Some(SpeakerPosition::TopBackCenter),
            0x0002_0000 => Some(SpeakerPosition::TopBackRight),
            0x8000_0000 => Some(SpeakerPosition::All),
            _ => None,
        }
    }

    /// The `SPEAKER_*` symbolic constant name.
    pub const fn symbolic_name(self) -> &'static str {
        match self {
            SpeakerPosition::FrontLeft => "SPEAKER_FRONT_LEFT",
            SpeakerPosition::FrontRight => "SPEAKER_FRONT_RIGHT",
            SpeakerPosition::FrontCenter => "SPEAKER_FRONT_CENTER",
            SpeakerPosition::LowFrequency => "SPEAKER_LOW_FREQUENCY",
            SpeakerPosition::BackLeft => "SPEAKER_BACK_LEFT",
            SpeakerPosition::BackRight => "SPEAKER_BACK_RIGHT",
            SpeakerPosition::FrontLeftOfCenter => "SPEAKER_FRONT_LEFT_OF_CENTER",
            SpeakerPosition::FrontRightOfCenter => "SPEAKER_FRONT_RIGHT_OF_CENTER",
            SpeakerPosition::BackCenter => "SPEAKER_BACK_CENTER",
            SpeakerPosition::SideLeft => "SPEAKER_SIDE_LEFT",
            SpeakerPosition::SideRight => "SPEAKER_SIDE_RIGHT",
            SpeakerPosition::TopCenter => "SPEAKER_TOP_CENTER",
            SpeakerPosition::TopFrontLeft => "SPEAKER_TOP_FRONT_LEFT",
            SpeakerPosition::TopFrontCenter => "SPEAKER_TOP_FRONT_CENTER",
            SpeakerPosition::TopFrontRight => "SPEAKER_TOP_FRONT_RIGHT",
            SpeakerPosition::TopBackLeft => "SPEAKER_TOP_BACK_LEFT",
            SpeakerPosition::TopBackCenter => "SPEAKER_TOP_BACK_CENTER",
            SpeakerPosition::TopBackRight => "SPEAKER_TOP_BACK_RIGHT",
            SpeakerPosition::All => "SPEAKER_ALL",
        }
    }

    /// The short label from the spec's table (`FL`, `FR`, `LFE`, …).
    /// [`SpeakerPosition::All`] has no discrete short label and reports
    /// the symbolic `"ALL"`.
    pub const fn short_label(self) -> &'static str {
        match self {
            SpeakerPosition::FrontLeft => "FL",
            SpeakerPosition::FrontRight => "FR",
            SpeakerPosition::FrontCenter => "FC",
            SpeakerPosition::LowFrequency => "LFE",
            SpeakerPosition::BackLeft => "BL",
            SpeakerPosition::BackRight => "BR",
            SpeakerPosition::FrontLeftOfCenter => "FLC",
            SpeakerPosition::FrontRightOfCenter => "FRC",
            SpeakerPosition::BackCenter => "BC",
            SpeakerPosition::SideLeft => "SL",
            SpeakerPosition::SideRight => "SR",
            SpeakerPosition::TopCenter => "TC",
            SpeakerPosition::TopFrontLeft => "TFL",
            SpeakerPosition::TopFrontCenter => "TFC",
            SpeakerPosition::TopFrontRight => "TFR",
            SpeakerPosition::TopBackLeft => "TBL",
            SpeakerPosition::TopBackCenter => "TBC",
            SpeakerPosition::TopBackRight => "TBR",
            SpeakerPosition::All => "ALL",
        }
    }
}

/// A recognised standard channel layout (a named `dwChannelMask` value
/// from the spec's standard-layout table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StandardLayout {
    /// `SPEAKER_FRONT_CENTER` (`0x0004`) — a single front-center channel.
    Mono,
    /// `FL | FR` (`0x0003`) — Stereo.
    Stereo,
    /// `FL | FR | LFE` (`0x000B`) — 2.1.
    TwoPointOne,
    /// `FL | FR | BL | BR` (`0x0033`) — Quad.
    Quad,
    /// `FL | FR | FC | LFE | BL | BR` (`0x003F`) — Microsoft 5.1 (back).
    FivePointOneBack,
    /// `FL | FR | FC | LFE | SL | SR` (`0x060F`) — DVD-style 5.1 (side).
    FivePointOneSide,
    /// `FL | FR | FC | LFE | BL | BR | SL | SR` (`0x063F`) — 7.1.
    SevenPointOne,
}

impl StandardLayout {
    /// The `dwChannelMask` value this layout corresponds to.
    pub const fn channel_mask(self) -> u32 {
        match self {
            StandardLayout::Mono => 0x0000_0004,
            StandardLayout::Stereo => 0x0000_0003,
            StandardLayout::TwoPointOne => 0x0000_000B,
            StandardLayout::Quad => 0x0000_0033,
            StandardLayout::FivePointOneBack => 0x0000_003F,
            StandardLayout::FivePointOneSide => 0x0000_060F,
            StandardLayout::SevenPointOne => 0x0000_063F,
        }
    }

    /// The number of channels (set bits) in this layout.
    pub const fn channel_count(self) -> u32 {
        self.channel_mask().count_ones()
    }

    /// A short human name for the layout (`"Mono"`, `"5.1 (back)"`, …).
    pub const fn name(self) -> &'static str {
        match self {
            StandardLayout::Mono => "Mono",
            StandardLayout::Stereo => "Stereo",
            StandardLayout::TwoPointOne => "2.1",
            StandardLayout::Quad => "Quad",
            StandardLayout::FivePointOneBack => "5.1 (back)",
            StandardLayout::FivePointOneSide => "5.1 (side)",
            StandardLayout::SevenPointOne => "7.1",
        }
    }

    /// Recognise a raw `dwChannelMask` as one of the standard layouts, if
    /// it matches one exactly. Returns `None` for any mask that is not a
    /// listed layout (including the empty mask and vendor masks).
    pub const fn from_mask(mask: u32) -> Option<StandardLayout> {
        // Ordered so a `0x003F` "5.1 (back)" is preferred over the
        // distinct `0x060F` "5.1 (side)" — they are different masks, so
        // the match is exact and order is immaterial; listed for clarity.
        match mask {
            0x0000_0004 => Some(StandardLayout::Mono),
            0x0000_0003 => Some(StandardLayout::Stereo),
            0x0000_000B => Some(StandardLayout::TwoPointOne),
            0x0000_0033 => Some(StandardLayout::Quad),
            0x0000_003F => Some(StandardLayout::FivePointOneBack),
            0x0000_060F => Some(StandardLayout::FivePointOneSide),
            0x0000_063F => Some(StandardLayout::SevenPointOne),
            _ => None,
        }
    }
}

/// A decoded `dwChannelMask`: the raw value plus typed accessors over the
/// speaker positions it names.
///
/// The channel ordering rule is central: the *N*-th channel in the
/// interleaved `data` stream feeds the *N*-th set bit of the mask, in
/// ascending bit order. So [`ChannelMask::positions`] returns the
/// speaker positions in exactly the channel order the file stores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChannelMask {
    raw: u32,
}

impl ChannelMask {
    /// Wrap a raw `dwChannelMask` value.
    pub const fn new(raw: u32) -> Self {
        ChannelMask { raw }
    }

    /// The raw 32-bit mask.
    pub const fn raw(self) -> u32 {
        self.raw
    }

    /// `true` if the mask is empty (`0` — "no speaker assignment", the
    /// spec's "let the renderer decide" sentinel).
    pub const fn is_empty(self) -> bool {
        self.raw == 0
    }

    /// `true` if the `SPEAKER_ALL` catch-all flag (bit 31) is set.
    pub const fn is_all(self) -> bool {
        self.raw & SPEAKER_ALL != 0
    }

    /// `true` if any reserved bit (18..=30) is set. A conformant writer
    /// leaves these clear, but a reader tolerates them.
    pub const fn has_reserved_bits(self) -> bool {
        self.raw & SPEAKER_RESERVED_MASK != 0
    }

    /// The number of **discrete standard** speaker positions named (set
    /// bits in 0..=17). This is the channel count the assignment implies,
    /// excluding the `SPEAKER_ALL` flag and any reserved bits.
    pub const fn channel_count(self) -> u32 {
        (self.raw & SPEAKER_STANDARD_MASK).count_ones()
    }

    /// The discrete standard speaker positions named, in ascending bit
    /// order — i.e. the in-file channel order. The `SPEAKER_ALL` flag and
    /// any reserved bits are not included.
    pub fn positions(self) -> Vec<SpeakerPosition> {
        SpeakerPosition::STANDARD
            .iter()
            .copied()
            .filter(|p| self.raw & p.bit() != 0)
            .collect()
    }

    /// The speaker position the interleaved channel at `index` feeds, by
    /// the bit-order channel rule. Returns `None` if `index` is past the
    /// number of discrete positions named.
    pub fn position_for_channel(self, index: usize) -> Option<SpeakerPosition> {
        SpeakerPosition::STANDARD
            .iter()
            .copied()
            .filter(|p| self.raw & p.bit() != 0)
            .nth(index)
    }

    /// The interleaved channel index that feeds `position`, by the
    /// bit-order channel rule. Returns `None` if the mask does not name
    /// that position.
    pub fn channel_for_position(self, position: SpeakerPosition) -> Option<usize> {
        if self.raw & position.bit() == 0 {
            return None;
        }
        SpeakerPosition::STANDARD
            .iter()
            .copied()
            .filter(|p| self.raw & p.bit() != 0)
            .position(|p| p == position)
    }

    /// `true` if `position` is named by the mask.
    pub const fn contains(self, position: SpeakerPosition) -> bool {
        self.raw & position.bit() != 0
    }

    /// Recognise this mask as a [`StandardLayout`], if it matches one
    /// exactly (the raw mask must equal the layout's mask, with no extra
    /// or reserved bits).
    pub const fn standard_layout(self) -> Option<StandardLayout> {
        StandardLayout::from_mask(self.raw)
    }

    /// Cross-check the mask's discrete-channel count against a declared
    /// `nChannels`. The spec recommends `dwChannelMask` name exactly
    /// `nChannels` positions; this reports whether they agree (an empty
    /// "renderer decides" mask is treated as *not* a mismatch — it makes
    /// no claim about channel positions). The `SPEAKER_ALL` flag is also
    /// treated as consistent, since it names no discrete count.
    pub const fn is_consistent_with_channels(self, channels: u16) -> bool {
        if self.is_empty() || self.is_all() {
            return true;
        }
        self.channel_count() == channels as u32
    }

    /// Build a mask from an iterator of positions, OR-ing each bit. The
    /// `SPEAKER_ALL` flag may be included; reserved bits cannot be set
    /// this way (the enum has no reserved variants).
    pub fn from_positions<I: IntoIterator<Item = SpeakerPosition>>(positions: I) -> Self {
        let raw = positions.into_iter().fold(0u32, |acc, p| acc | p.bit());
        ChannelMask { raw }
    }

    /// Validate a mask intended for an `N`-channel stream: the discrete
    /// channel count named must equal `channels` (an empty or
    /// `SPEAKER_ALL` mask is rejected here, since this is the *strict*
    /// authoring check — use [`ChannelMask::is_consistent_with_channels`]
    /// for the lenient read-side check). Reserved bits are rejected.
    pub fn validate_for_channels(self, channels: u16) -> Result<()> {
        if self.has_reserved_bits() {
            return Err(Error::invalid("dwChannelMask: reserved bits (18..=30) set"));
        }
        if self.is_empty() {
            return Err(Error::invalid(
                "dwChannelMask: empty mask names no channel positions",
            ));
        }
        if self.is_all() {
            return Err(Error::invalid(
                "dwChannelMask: SPEAKER_ALL names no discrete positions",
            ));
        }
        if self.channel_count() != channels as u32 {
            return Err(Error::invalid(
                "dwChannelMask: named-position count != nChannels",
            ));
        }
        Ok(())
    }
}

impl From<u32> for ChannelMask {
    fn from(raw: u32) -> Self {
        ChannelMask::new(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_table_is_bit_ordered() {
        // Each entry's bit is strictly greater than the previous, and the
        // ordinal index maps to bit `1 << i`.
        for (i, p) in SpeakerPosition::STANDARD.iter().enumerate() {
            assert_eq!(p.bit(), 1u32 << i, "{} at index {i}", p.symbolic_name());
        }
        assert_eq!(SpeakerPosition::STANDARD.len(), 18);
    }

    #[test]
    fn from_bit_round_trips_each_standard_position() {
        for p in SpeakerPosition::STANDARD {
            assert_eq!(SpeakerPosition::from_bit(p.bit()), Some(p));
        }
        assert_eq!(
            SpeakerPosition::from_bit(SPEAKER_ALL),
            Some(SpeakerPosition::All)
        );
    }

    #[test]
    fn from_bit_rejects_reserved_and_multibit() {
        // A reserved bit (bit 18).
        assert_eq!(SpeakerPosition::from_bit(1 << 18), None);
        // A multi-bit value.
        assert_eq!(SpeakerPosition::from_bit(0x3), None);
        // Zero.
        assert_eq!(SpeakerPosition::from_bit(0), None);
    }

    #[test]
    fn positions_are_in_channel_order() {
        // 5.1 (back): FL FR FC LFE BL BR.
        let m = ChannelMask::new(0x3F);
        use SpeakerPosition::*;
        assert_eq!(
            m.positions(),
            vec![
                FrontLeft,
                FrontRight,
                FrontCenter,
                LowFrequency,
                BackLeft,
                BackRight
            ]
        );
        assert_eq!(m.channel_count(), 6);
    }

    #[test]
    fn channel_position_mapping_is_bidirectional() {
        let m = ChannelMask::new(0x60F); // 5.1 (side): FL FR FC LFE SL SR.
        use SpeakerPosition::*;
        assert_eq!(m.position_for_channel(0), Some(FrontLeft));
        assert_eq!(m.position_for_channel(4), Some(SideLeft));
        assert_eq!(m.position_for_channel(5), Some(SideRight));
        assert_eq!(m.position_for_channel(6), None);
        assert_eq!(m.channel_for_position(SideLeft), Some(4));
        assert_eq!(m.channel_for_position(FrontLeft), Some(0));
        // A position the mask doesn't name.
        assert_eq!(m.channel_for_position(BackLeft), None);
    }

    #[test]
    fn standard_layouts_resolve() {
        assert_eq!(
            ChannelMask::new(0x04).standard_layout(),
            Some(StandardLayout::Mono)
        );
        assert_eq!(
            ChannelMask::new(0x03).standard_layout(),
            Some(StandardLayout::Stereo)
        );
        assert_eq!(
            ChannelMask::new(0x0B).standard_layout(),
            Some(StandardLayout::TwoPointOne)
        );
        assert_eq!(
            ChannelMask::new(0x33).standard_layout(),
            Some(StandardLayout::Quad)
        );
        assert_eq!(
            ChannelMask::new(0x3F).standard_layout(),
            Some(StandardLayout::FivePointOneBack)
        );
        assert_eq!(
            ChannelMask::new(0x60F).standard_layout(),
            Some(StandardLayout::FivePointOneSide)
        );
        assert_eq!(
            ChannelMask::new(0x63F).standard_layout(),
            Some(StandardLayout::SevenPointOne)
        );
        // A non-standard mask (FL | FR | TC).
        assert_eq!(ChannelMask::new(0x803).standard_layout(), None);
    }

    #[test]
    fn standard_layout_channel_counts() {
        assert_eq!(StandardLayout::Mono.channel_count(), 1);
        assert_eq!(StandardLayout::Stereo.channel_count(), 2);
        assert_eq!(StandardLayout::TwoPointOne.channel_count(), 3);
        assert_eq!(StandardLayout::Quad.channel_count(), 4);
        assert_eq!(StandardLayout::FivePointOneBack.channel_count(), 6);
        assert_eq!(StandardLayout::FivePointOneSide.channel_count(), 6);
        assert_eq!(StandardLayout::SevenPointOne.channel_count(), 8);
    }

    #[test]
    fn all_flag_and_reserved_bits() {
        let m = ChannelMask::new(SPEAKER_ALL);
        assert!(m.is_all());
        assert!(!m.is_empty());
        assert!(!m.has_reserved_bits());
        assert_eq!(m.channel_count(), 0);
        assert_eq!(m.positions(), vec![]);

        let r = ChannelMask::new(1 << 20);
        assert!(r.has_reserved_bits());
        assert!(!r.is_all());
        assert_eq!(r.channel_count(), 0);

        // A standard mask with a stray reserved bit still reports its
        // discrete positions but flags the reserved bit.
        let mixed = ChannelMask::new(0x3F | (1 << 19));
        assert!(mixed.has_reserved_bits());
        assert_eq!(mixed.channel_count(), 6);
        assert_eq!(mixed.standard_layout(), None); // not an exact 5.1 mask
    }

    #[test]
    fn empty_mask_is_renderer_decides() {
        let m = ChannelMask::new(0);
        assert!(m.is_empty());
        assert_eq!(m.channel_count(), 0);
        assert!(m.positions().is_empty());
        assert_eq!(m.position_for_channel(0), None);
        assert_eq!(m.standard_layout(), None);
    }

    #[test]
    fn from_positions_builds_mask() {
        use SpeakerPosition::*;
        let m = ChannelMask::from_positions([
            FrontLeft,
            FrontRight,
            FrontCenter,
            LowFrequency,
            BackLeft,
            BackRight,
        ]);
        assert_eq!(m.raw(), 0x3F);
        assert_eq!(m.standard_layout(), Some(StandardLayout::FivePointOneBack));
    }

    #[test]
    fn consistency_with_channels() {
        let stereo = ChannelMask::new(0x03);
        assert!(stereo.is_consistent_with_channels(2));
        assert!(!stereo.is_consistent_with_channels(3));
        // Empty / ALL make no claim → consistent with anything.
        assert!(ChannelMask::new(0).is_consistent_with_channels(8));
        assert!(ChannelMask::new(SPEAKER_ALL).is_consistent_with_channels(8));
    }

    #[test]
    fn validate_for_channels_strict() {
        assert!(ChannelMask::new(0x3F).validate_for_channels(6).is_ok());
        // Count mismatch.
        assert!(ChannelMask::new(0x3F).validate_for_channels(5).is_err());
        // Empty and ALL are rejected by the strict authoring check.
        assert!(ChannelMask::new(0).validate_for_channels(0).is_err());
        assert!(ChannelMask::new(SPEAKER_ALL)
            .validate_for_channels(0)
            .is_err());
        // Reserved bit.
        assert!(ChannelMask::new(0x3F | (1 << 19))
            .validate_for_channels(6)
            .is_err());
    }

    #[test]
    fn contains_and_short_labels() {
        let m = ChannelMask::new(0x0B); // 2.1: FL FR LFE.
        assert!(m.contains(SpeakerPosition::FrontLeft));
        assert!(m.contains(SpeakerPosition::LowFrequency));
        assert!(!m.contains(SpeakerPosition::FrontCenter));
        assert_eq!(SpeakerPosition::LowFrequency.short_label(), "LFE");
        assert_eq!(SpeakerPosition::TopBackRight.short_label(), "TBR");
    }
}
