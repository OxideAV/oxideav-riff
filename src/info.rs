//! Typed decoder for the WAV / RIFF `LIST INFO` metadata namespace.
//!
//! The `INFO` list is a registered global `LIST` form-type the 1991
//! RIFF MCI spec defines for storing identification metadata —
//! copyright, comments, artist, creation date, and so on — that helps
//! identify the contents of a file without affecting how a program
//! interprets it. An `INFO` list is a `LIST` chunk whose list-type
//! FourCC is `INFO`; each child chunk's body is a **ZSTR**, a
//! NULL-terminated ASCII text string.
//!
//! This module decodes that sub-tree into a typed [`InfoList`]. It does
//! **not** walk the outer chunk tree itself — the caller uses the
//! [`crate::Walker`] to locate the `LIST INFO` group, descends into it
//! with a nested walker, and feeds each child's
//! `(FourCC, body)` pair to [`InfoList`] (or hands the whole sub-walker
//! to [`InfoList::collect_from`]).
//!
//! ## Wire layout
//!
//! ```text
//! LIST( <ckSize:u32 LE>
//!   'INFO'                       // list-type FourCC
//!   Ixxx( <ckSize:u32 LE> <ZSTR> )   // one tag chunk …
//!   Iyyy( <ckSize:u32 LE> <ZSTR> )   // … repeated
//! )
//! ```
//!
//! Each tag chunk's FourCC is a four-character code from the registered
//! `INFO` namespace (`INAM`, `IART`, `ICOP`, …). Its body is a ZSTR:
//! ASCII characters terminated by a `0x00` byte. RIFF pads any
//! odd-length body with a trailing `0x00` so the next header lands on a
//! 2-byte boundary; the walker strips that pad before the body reaches
//! this module, and [`zstr_value`] additionally tolerates trailing
//! `NUL` padding inside the declared body length.
//!
//! ## What this module decodes
//!
//! The 23 **baseline** `INFO` sub-IDs the 1991 spec registers, each
//! exposed as an [`InfoTag`] FourCC constant with its spec semantics in
//! the doc comment, plus the 38 **extended** sub-IDs ExifTool's RIFF
//! Tags table catalogues for production WAV / AVI files (the audio-
//! stream-language `IAS1`–`IAS9` set, the editorial credits
//! `ICNM` / `ICDS` / `IMUS` / `ISTR` / …, the `IDIT` digitization time,
//! `ITRK` track number, `ISMP` time-code, and so on). An `InfoList`
//! preserves the on-wire order of the tags it collected and keeps any
//! unrecognised four-character codes verbatim (the spec explicitly
//! allows new chunk IDs and instructs an application to ignore — but not
//! reject — IDs it does not understand).
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "INFO List Chunk" (the registered global `INFO` form-type and the
//!   baseline tag table) + "NULL-Terminated String (ZSTR) Format".
//! - `docs/container/riff/metadata/exiftool-riff-tags.html` — the
//!   field-name catalogue (a DATA table) for the extended `INFO` tags
//!   beyond the 1991 baseline.

use crate::error::{Error, Result};

/// A registered four-character `INFO` sub-ID.
///
/// The associated constants are the 23 baseline tags the 1991 RIFF MCI
/// spec registers for the `INFO` list ([`InfoTag::BASELINE`]) plus the
/// 38 extended tags ExifTool's RIFF Tags table catalogues for
/// production WAV / AVI files ([`InfoTag::EXTENDED`]). The wrapped
/// `[u8; 4]` is the raw FourCC; [`InfoTag::label`] maps any registered
/// code to its short human-readable name and returns `None` for any
/// other (vendor / unknown) code. Use [`InfoTag::is_baseline`] /
/// [`InfoTag::is_extended`] / [`InfoTag::is_registered`] to classify a
/// code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InfoTag(pub [u8; 4]);

impl InfoTag {
    /// Archival Location. Indicates where the subject of the file is
    /// archived.
    pub const IARL: InfoTag = InfoTag(*b"IARL");
    /// Artist. Lists the artist of the original subject of the file.
    pub const IART: InfoTag = InfoTag(*b"IART");
    /// Commissioned. Lists the name of the person or organization that
    /// commissioned the subject of the file.
    pub const ICMS: InfoTag = InfoTag(*b"ICMS");
    /// Comments. Provides general comments about the file or its
    /// subject.
    pub const ICMT: InfoTag = InfoTag(*b"ICMT");
    /// Copyright. Records the copyright information for the file.
    pub const ICOP: InfoTag = InfoTag(*b"ICOP");
    /// Creation date. The date the subject of the file was created, in
    /// year-month-day form (e.g. `1553-05-03`).
    pub const ICRD: InfoTag = InfoTag(*b"ICRD");
    /// Cropped. Describes whether (and how) an image has been cropped.
    pub const ICRP: InfoTag = InfoTag(*b"ICRP");
    /// Dimensions. The size of the original subject of the file.
    pub const IDIM: InfoTag = InfoTag(*b"IDIM");
    /// Dots Per Inch. The DPI setting of the digitizer used to produce
    /// the file.
    pub const IDPI: InfoTag = InfoTag(*b"IDPI");
    /// Engineer. The name of the engineer who worked on the file.
    pub const IENG: InfoTag = InfoTag(*b"IENG");
    /// Genre. Describes the original work (e.g. `landscape`,
    /// `portrait`).
    pub const IGNR: InfoTag = InfoTag(*b"IGNR");
    /// Keywords. A list of keywords referring to the file or subject.
    pub const IKEY: InfoTag = InfoTag(*b"IKEY");
    /// Lightness. The lightness-setting changes on the digitizer
    /// required to produce the file.
    pub const ILGT: InfoTag = InfoTag(*b"ILGT");
    /// Medium. Describes the original subject of the file (e.g.
    /// `computer image`, `drawing`).
    pub const IMED: InfoTag = InfoTag(*b"IMED");
    /// Name. The title of the subject of the file.
    pub const INAM: InfoTag = InfoTag(*b"INAM");
    /// Palette Setting. The number of colors requested when digitizing
    /// an image.
    pub const IPLT: InfoTag = InfoTag(*b"IPLT");
    /// Product. The name of the title the file was originally intended
    /// for.
    pub const IPRD: InfoTag = InfoTag(*b"IPRD");
    /// Subject. Describes the contents of the file.
    pub const ISBJ: InfoTag = InfoTag(*b"ISBJ");
    /// Software. The name of the software package used to create the
    /// file.
    pub const ISFT: InfoTag = InfoTag(*b"ISFT");
    /// Sharpness. The sharpness-setting changes for the digitizer
    /// required to produce the file.
    pub const ISHP: InfoTag = InfoTag(*b"ISHP");
    /// Source. The name of the person or organization who supplied the
    /// original subject of the file.
    pub const ISRC: InfoTag = InfoTag(*b"ISRC");
    /// Source Form. The original form of the digitized material (e.g.
    /// `slide`, `paper`, `map`).
    pub const ISRF: InfoTag = InfoTag(*b"ISRF");
    /// Technician. The technician who digitized the subject file.
    pub const ITCH: InfoTag = InfoTag(*b"ITCH");

    // ---------------------------------------------------------------
    // Extended `INFO` namespace.
    //
    // Tags beyond the 23 the 1991 RIFF MCI spec registers, seen widely
    // in production WAV / AVI files and catalogued in ExifTool's RIFF
    // Tags table (`docs/container/riff/metadata/exiftool-riff-tags.html`).
    // These are *not* in the 1991 baseline; [`InfoTag::is_baseline`]
    // returns `false` for them while [`InfoTag::is_extended`] returns
    // `true`. Their bodies follow the same ZSTR convention as the
    // baseline tags (with the documented exception of `ICCP`, whose body
    // is a raw ICC profile blob rather than text — the label still
    // resolves but a consumer should not ZSTR-decode it).
    // ---------------------------------------------------------------

    /// First Language. The primary audio-stream language (`IAS1`–`IAS9`
    /// enumerate up to nine parallel-stream languages).
    pub const IAS1: InfoTag = InfoTag(*b"IAS1");
    /// Second Language. See [`InfoTag::IAS1`].
    pub const IAS2: InfoTag = InfoTag(*b"IAS2");
    /// Third Language. See [`InfoTag::IAS1`].
    pub const IAS3: InfoTag = InfoTag(*b"IAS3");
    /// Fourth Language. See [`InfoTag::IAS1`].
    pub const IAS4: InfoTag = InfoTag(*b"IAS4");
    /// Fifth Language. See [`InfoTag::IAS1`].
    pub const IAS5: InfoTag = InfoTag(*b"IAS5");
    /// Sixth Language. See [`InfoTag::IAS1`].
    pub const IAS6: InfoTag = InfoTag(*b"IAS6");
    /// Seventh Language. See [`InfoTag::IAS1`].
    pub const IAS7: InfoTag = InfoTag(*b"IAS7");
    /// Eighth Language. See [`InfoTag::IAS1`].
    pub const IAS8: InfoTag = InfoTag(*b"IAS8");
    /// Ninth Language. See [`InfoTag::IAS1`].
    pub const IAS9: InfoTag = InfoTag(*b"IAS9");
    /// Base URL. The base address for the file's More-Info links.
    pub const IBSU: InfoTag = InfoTag(*b"IBSU");
    /// Default Audio Stream. Which audio stream plays by default.
    pub const ICAS: InfoTag = InfoTag(*b"ICAS");
    /// ICC Profile. A raw embedded ICC colour profile (binary body, not
    /// a ZSTR).
    pub const ICCP: InfoTag = InfoTag(*b"ICCP");
    /// Costume Designer.
    pub const ICDS: InfoTag = InfoTag(*b"ICDS");
    /// Cinematographer.
    pub const ICNM: InfoTag = InfoTag(*b"ICNM");
    /// Country. The country of origin.
    pub const ICNT: InfoTag = InfoTag(*b"ICNT");
    /// Date/Time Original. The original digitization date-time.
    pub const IDIT: InfoTag = InfoTag(*b"IDIT");
    /// Distributed By.
    pub const IDST: InfoTag = InfoTag(*b"IDST");
    /// Edited By.
    pub const IEDT: InfoTag = InfoTag(*b"IEDT");
    /// Encoded By. The name of the person or tool that encoded the file.
    pub const IENC: InfoTag = InfoTag(*b"IENC");
    /// Logo URL.
    pub const ILGU: InfoTag = InfoTag(*b"ILGU");
    /// Logo Icon URL.
    pub const ILIU: InfoTag = InfoTag(*b"ILIU");
    /// Language. The content language.
    pub const ILNG: InfoTag = InfoTag(*b"ILNG");
    /// More-Info Banner Image.
    pub const IMBI: InfoTag = InfoTag(*b"IMBI");
    /// More-Info Banner URL.
    pub const IMBU: InfoTag = InfoTag(*b"IMBU");
    /// More-Info Text.
    pub const IMIT: InfoTag = InfoTag(*b"IMIT");
    /// More-Info URL.
    pub const IMIU: InfoTag = InfoTag(*b"IMIU");
    /// Music By.
    pub const IMUS: InfoTag = InfoTag(*b"IMUS");
    /// Production Designer.
    pub const IPDS: InfoTag = InfoTag(*b"IPDS");
    /// Produced By.
    pub const IPRO: InfoTag = InfoTag(*b"IPRO");
    /// Ripped By.
    pub const IRIP: InfoTag = InfoTag(*b"IRIP");
    /// Rating.
    pub const IRTD: InfoTag = InfoTag(*b"IRTD");
    /// Secondary Genre.
    pub const ISGN: InfoTag = InfoTag(*b"ISGN");
    /// Time Code. An SMPTE time code string.
    pub const ISMP: InfoTag = InfoTag(*b"ISMP");
    /// Production Studio.
    pub const ISTD: InfoTag = InfoTag(*b"ISTD");
    /// Starring. The featured performers.
    pub const ISTR: InfoTag = InfoTag(*b"ISTR");
    /// Track Number.
    pub const ITRK: InfoTag = InfoTag(*b"ITRK");
    /// Watermark URL.
    pub const IWMU: InfoTag = InfoTag(*b"IWMU");
    /// Written By.
    pub const IWRI: InfoTag = InfoTag(*b"IWRI");

    /// All 23 baseline tags the 1991 RIFF MCI spec registers, in the
    /// order the spec lists them.
    pub const BASELINE: [InfoTag; 23] = [
        Self::IARL,
        Self::IART,
        Self::ICMS,
        Self::ICMT,
        Self::ICOP,
        Self::ICRD,
        Self::ICRP,
        Self::IDIM,
        Self::IDPI,
        Self::IENG,
        Self::IGNR,
        Self::IKEY,
        Self::ILGT,
        Self::IMED,
        Self::INAM,
        Self::IPLT,
        Self::IPRD,
        Self::ISBJ,
        Self::ISFT,
        Self::ISHP,
        Self::ISRC,
        Self::ISRF,
        Self::ITCH,
    ];

    /// The extended `INFO` tags catalogued by ExifTool's RIFF Tags
    /// table — tags seen in production WAV / AVI files that are *not*
    /// part of the 1991 baseline. Listed in ascending FourCC order.
    pub const EXTENDED: [InfoTag; 38] = [
        Self::IAS1,
        Self::IAS2,
        Self::IAS3,
        Self::IAS4,
        Self::IAS5,
        Self::IAS6,
        Self::IAS7,
        Self::IAS8,
        Self::IAS9,
        Self::IBSU,
        Self::ICAS,
        Self::ICCP,
        Self::ICDS,
        Self::ICNM,
        Self::ICNT,
        Self::IDIT,
        Self::IDST,
        Self::IEDT,
        Self::IENC,
        Self::ILGU,
        Self::ILIU,
        Self::ILNG,
        Self::IMBI,
        Self::IMBU,
        Self::IMIT,
        Self::IMIU,
        Self::IMUS,
        Self::IPDS,
        Self::IPRO,
        Self::IRIP,
        Self::IRTD,
        Self::ISGN,
        Self::ISMP,
        Self::ISTD,
        Self::ISTR,
        Self::ITRK,
        Self::IWMU,
        Self::IWRI,
    ];

    /// The raw four-character code of this tag.
    pub const fn fourcc(&self) -> [u8; 4] {
        self.0
    }

    /// `true` if this is one of the 23 baseline `INFO` sub-IDs the
    /// 1991 spec registers.
    pub fn is_baseline(&self) -> bool {
        Self::BASELINE.contains(self)
    }

    /// `true` if this is one of the extended (non-baseline) `INFO` tags
    /// catalogued in ExifTool's RIFF Tags table.
    pub fn is_extended(&self) -> bool {
        Self::EXTENDED.contains(self)
    }

    /// `true` if this tag is registered — either a 1991 baseline tag or
    /// a catalogued extended tag. A `false` result means the FourCC is
    /// a vendor / unknown code (still preserved verbatim by
    /// [`InfoList`], but with no [`InfoTag::label`]).
    pub fn is_registered(&self) -> bool {
        self.is_baseline() || self.is_extended()
    }

    /// Short human-readable label for a registered tag, or `None` for a
    /// vendor / unknown four-character code.
    ///
    /// The baseline labels are the registered field names from the 1991
    /// RIFF MCI "INFO List Chunk" table; the extended labels are the
    /// field names ExifTool's RIFF Tags table records for the
    /// production-seen tags outside that baseline.
    pub fn label(&self) -> Option<&'static str> {
        Some(match &self.0 {
            // 1991 RIFF MCI baseline.
            b"IARL" => "Archival Location",
            b"IART" => "Artist",
            b"ICMS" => "Commissioned",
            b"ICMT" => "Comments",
            b"ICOP" => "Copyright",
            b"ICRD" => "Creation Date",
            b"ICRP" => "Cropped",
            b"IDIM" => "Dimensions",
            b"IDPI" => "Dots Per Inch",
            b"IENG" => "Engineer",
            b"IGNR" => "Genre",
            b"IKEY" => "Keywords",
            b"ILGT" => "Lightness",
            b"IMED" => "Medium",
            b"INAM" => "Name",
            b"IPLT" => "Palette Setting",
            b"IPRD" => "Product",
            b"ISBJ" => "Subject",
            b"ISFT" => "Software",
            b"ISHP" => "Sharpness",
            b"ISRC" => "Source",
            b"ISRF" => "Source Form",
            b"ITCH" => "Technician",
            // Extended namespace (ExifTool RIFF Tags table).
            b"IAS1" => "First Language",
            b"IAS2" => "Second Language",
            b"IAS3" => "Third Language",
            b"IAS4" => "Fourth Language",
            b"IAS5" => "Fifth Language",
            b"IAS6" => "Sixth Language",
            b"IAS7" => "Seventh Language",
            b"IAS8" => "Eighth Language",
            b"IAS9" => "Ninth Language",
            b"IBSU" => "Base URL",
            b"ICAS" => "Default Audio Stream",
            b"ICCP" => "ICC Profile",
            b"ICDS" => "Costume Designer",
            b"ICNM" => "Cinematographer",
            b"ICNT" => "Country",
            b"IDIT" => "Date/Time Original",
            b"IDST" => "Distributed By",
            b"IEDT" => "Edited By",
            b"IENC" => "Encoded By",
            b"ILGU" => "Logo URL",
            b"ILIU" => "Logo Icon URL",
            b"ILNG" => "Language",
            b"IMBI" => "More Info Banner Image",
            b"IMBU" => "More Info Banner URL",
            b"IMIT" => "More Info Text",
            b"IMIU" => "More Info URL",
            b"IMUS" => "Music By",
            b"IPDS" => "Production Designer",
            b"IPRO" => "Produced By",
            b"IRIP" => "Ripped By",
            b"IRTD" => "Rating",
            b"ISGN" => "Secondary Genre",
            b"ISMP" => "Time Code",
            b"ISTD" => "Production Studio",
            b"ISTR" => "Starring",
            b"ITRK" => "Track Number",
            b"IWMU" => "Watermark URL",
            b"IWRI" => "Written By",
            _ => return None,
        })
    }
}

/// Decode a ZSTR `INFO` chunk body into its text value.
///
/// Per the 1991 spec, an `INFO` chunk body is a ZSTR: ASCII characters
/// terminated by a `0x00` NULL byte. This function returns the bytes
/// up to (and excluding) the first `0x00`; a body without an explicit
/// terminator (some encoders rely solely on the RIFF pad byte) yields
/// the whole body. Any further trailing bytes — the terminator plus
/// the optional RIFF pad — are discarded.
///
/// The text is returned as a borrowed `&[u8]` so the caller chooses how
/// to interpret the code page (the spec leaves the character set to the
/// `CSET` chunk; default is plain ASCII / Windows-1252). No UTF-8
/// validation is performed here.
pub fn zstr_bytes(body: &[u8]) -> &[u8] {
    match body.iter().position(|&b| b == 0) {
        Some(nul) => &body[..nul],
        None => body,
    }
}

/// Decode a ZSTR `INFO` chunk body to an owned [`String`], replacing
/// any non-UTF-8 bytes with the Unicode replacement character.
///
/// Convenience wrapper over [`zstr_bytes`] for the common ASCII case.
pub fn zstr_value(body: &[u8]) -> String {
    String::from_utf8_lossy(zstr_bytes(body)).into_owned()
}

/// A decoded `LIST INFO` block: an ordered list of `(tag, value)`
/// pairs.
///
/// Order is preserved exactly as the tags appear on the wire (the spec
/// imposes no ordering, and round-tripping a file should not reshuffle
/// its metadata). Duplicate tags are kept — the spec does not forbid
/// them — so [`InfoList::get`] returns the *first* occurrence while
/// [`InfoList::entries`] exposes all of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfoList {
    entries: Vec<(InfoTag, String)>,
}

impl InfoList {
    /// A new, empty `INFO` block.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one decoded tag chunk: its four-character code and its raw
    /// (already pad-stripped) body bytes. The body is decoded as a
    /// ZSTR via [`zstr_value`].
    pub fn push_chunk(&mut self, fourcc: [u8; 4], body: &[u8]) {
        self.entries.push((InfoTag(fourcc), zstr_value(body)));
    }

    /// Number of tag entries collected.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if no tags were collected.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All `(tag, value)` entries in on-wire order.
    pub fn entries(&self) -> &[(InfoTag, String)] {
        &self.entries
    }

    /// First value stored under `tag`, or `None` if absent.
    pub fn get(&self, tag: InfoTag) -> Option<&str> {
        self.entries
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, v)| v.as_str())
    }

    /// Append a `(tag, value)` entry, returning `self` for chaining.
    ///
    /// The value is the text alone (no ZSTR terminator); the terminator
    /// is added on encode. The write-side counterpart of
    /// [`InfoList::push_chunk`].
    pub fn push(mut self, tag: InfoTag, value: impl Into<String>) -> Self {
        self.entries.push((tag, value.into()));
        self
    }

    /// Serialize the `LIST INFO` *body* — the `INFO` list-type word
    /// followed by one `Ixxx` child chunk per entry, in order.
    ///
    /// Each child is a ZSTR `INFO` tag: the value bytes plus a `0x00`
    /// terminator, framed with [`crate::chunk::encode_chunk`] (so an
    /// odd-length body gets its RIFF pad byte). This is the inverse of
    /// [`InfoList::collect_from`] for the common all-ASCII case: a list
    /// built from valid-UTF-8 tag values re-collects to an equal
    /// `InfoList`. (Values are stored decoded, so a non-UTF-8 source byte
    /// that became U+FFFD on read does not survive a read→encode→read
    /// cycle — the normal ASCII / Windows-1252 text path is unaffected.)
    pub fn encode_list_body(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(b"INFO");
        for (tag, value) in &self.entries {
            let mut zstr = value.as_bytes().to_vec();
            zstr.push(0); // ZSTR terminator
            crate::chunk::encode_chunk(&mut out, &tag.0, &zstr)?;
        }
        Ok(out)
    }

    /// Serialize the whole `LIST INFO` chunk — the `LIST` header (with the
    /// computed `ckSize`) wrapping [`InfoList::encode_list_body`]. Append
    /// directly to a RIFF/WAVE file being muxed.
    pub fn encode_chunk(&self) -> Result<Vec<u8>> {
        let body = self.encode_list_body()?;
        let mut out = Vec::new();
        crate::chunk::encode_chunk(&mut out, &crate::chunk::FOURCC_LIST, &body)?;
        Ok(out)
    }

    /// Collect a whole `LIST INFO` sub-tree from a [`crate::Walker`]
    /// already positioned over the `INFO` list body (i.e. constructed
    /// after the caller read the `INFO` list-type with
    /// [`crate::Walker::read_inner_form_type`]).
    ///
    /// Each child chunk is read in full and decoded as a ZSTR `INFO`
    /// tag. The walker's parent-budget enforcement still applies, so a
    /// child overflowing the `LIST` body surfaces as the walker's
    /// existing `Error::invalid` — this function adds no further
    /// structural validation beyond requiring the list-type be `INFO`.
    pub fn collect_from<R: std::io::Read + std::io::Seek + ?Sized>(
        walker: &mut crate::Walker<'_, R>,
    ) -> Result<Self> {
        if &walker.form_type() != b"INFO" {
            return Err(Error::invalid(
                "RIFF: collect_from called on a non-INFO LIST",
            ));
        }
        let mut list = Self::new();
        while let Some(chunk) = walker.read_next()? {
            let body = walker.read_body(&chunk)?;
            list.push_chunk(chunk.id, &body);
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn baseline_table_has_23_unique_tags() {
        assert_eq!(InfoTag::BASELINE.len(), 23);
        // No duplicates.
        for (i, a) in InfoTag::BASELINE.iter().enumerate() {
            for b in &InfoTag::BASELINE[i + 1..] {
                assert_ne!(a, b, "duplicate baseline tag {:?}", a.0);
            }
        }
    }

    #[test]
    fn every_baseline_tag_has_a_label_and_is_baseline() {
        for tag in InfoTag::BASELINE {
            assert!(tag.is_baseline());
            assert!(tag.label().is_some(), "missing label for {:?}", tag.0);
        }
    }

    #[test]
    fn unknown_tag_has_no_label_and_is_not_baseline() {
        let t = InfoTag(*b"IMP3");
        assert!(!t.is_baseline());
        assert!(!t.is_extended());
        assert!(!t.is_registered());
        assert!(t.label().is_none());
    }

    #[test]
    fn extended_table_has_38_unique_tags_disjoint_from_baseline() {
        assert_eq!(InfoTag::EXTENDED.len(), 38);
        // No duplicates within EXTENDED.
        for (i, a) in InfoTag::EXTENDED.iter().enumerate() {
            for b in &InfoTag::EXTENDED[i + 1..] {
                assert_ne!(a, b, "duplicate extended tag {:?}", a.0);
            }
        }
        // No overlap with BASELINE.
        for e in InfoTag::EXTENDED {
            assert!(
                !InfoTag::BASELINE.contains(&e),
                "extended tag {:?} also in baseline",
                e.0
            );
        }
    }

    #[test]
    fn every_extended_tag_has_a_label_and_is_extended_not_baseline() {
        for tag in InfoTag::EXTENDED {
            assert!(tag.is_extended(), "{:?} not is_extended", tag.0);
            assert!(!tag.is_baseline(), "{:?} reported baseline", tag.0);
            assert!(tag.is_registered());
            assert!(tag.label().is_some(), "missing label for {:?}", tag.0);
        }
    }

    #[test]
    fn extended_labels_match_catalogue() {
        assert_eq!(InfoTag::IAS1.label(), Some("First Language"));
        assert_eq!(InfoTag::IDIT.label(), Some("Date/Time Original"));
        assert_eq!(InfoTag::ITRK.label(), Some("Track Number"));
        assert_eq!(InfoTag::ISMP.label(), Some("Time Code"));
        assert_eq!(InfoTag::IENC.label(), Some("Encoded By"));
        assert_eq!(InfoTag::ICCP.label(), Some("ICC Profile"));
    }

    #[test]
    fn extended_tag_decodes_and_round_trips_through_collect() {
        // A LIST INFO carrying an extended tag must decode, classify, and
        // re-collect to an equal list.
        let list = InfoList::new()
            .push(InfoTag::IENC, "OxideAV")
            .push(InfoTag::ITRK, "7")
            .push(InfoTag::IAS1, "eng");
        let body = list.encode_list_body().unwrap();
        let mut blob = Vec::new();
        blob.extend_from_slice(b"LIST");
        blob.extend_from_slice(&(body.len() as u32).to_le_bytes());
        blob.extend_from_slice(&body);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let collected = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(collected, list);
        assert_eq!(collected.get(InfoTag::IENC), Some("OxideAV"));
        assert!(collected.entries()[0].0.is_extended());
    }

    #[test]
    fn well_known_labels_match_spec() {
        assert_eq!(InfoTag::INAM.label(), Some("Name"));
        assert_eq!(InfoTag::ICOP.label(), Some("Copyright"));
        assert_eq!(InfoTag::IART.label(), Some("Artist"));
        assert_eq!(InfoTag::ICRD.label(), Some("Creation Date"));
    }

    #[test]
    fn zstr_stops_at_first_nul() {
        assert_eq!(zstr_bytes(b"Two Trees\0"), b"Two Trees");
        assert_eq!(zstr_value(b"Two Trees\0"), "Two Trees");
        // Embedded NUL truncates (per ZSTR semantics).
        assert_eq!(zstr_value(b"abc\0def\0"), "abc");
    }

    #[test]
    fn zstr_tolerates_missing_terminator() {
        // Some encoders rely on the RIFF pad byte only; no embedded NUL.
        assert_eq!(zstr_value(b"NoNul"), "NoNul");
        assert_eq!(zstr_value(b""), "");
    }

    #[test]
    fn zstr_lossy_on_invalid_utf8() {
        // 0xFF is not valid UTF-8; lossy decode yields U+FFFD.
        let v = zstr_value(&[b'a', 0xFF, b'b', 0x00]);
        assert!(v.starts_with('a') && v.ends_with('b'));
        assert!(v.contains('\u{FFFD}'));
    }

    #[test]
    fn push_chunk_preserves_order_and_duplicates() {
        let mut list = InfoList::new();
        list.push_chunk(*b"INAM", b"Two Trees\0");
        list.push_chunk(*b"ICMT", b"A picture\0");
        list.push_chunk(*b"INAM", b"Second name\0");
        assert_eq!(list.len(), 3);
        assert_eq!(list.get(InfoTag::INAM), Some("Two Trees"));
        assert_eq!(list.get(InfoTag::ICMT), Some("A picture"));
        // entries() exposes the duplicate.
        let names: Vec<_> = list
            .entries()
            .iter()
            .filter(|(t, _)| *t == InfoTag::INAM)
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(names, vec!["Two Trees", "Second name"]);
    }

    #[test]
    fn get_returns_none_for_absent_tag() {
        let mut list = InfoList::new();
        list.push_chunk(*b"INAM", b"X\0");
        assert_eq!(list.get(InfoTag::IART), None);
    }

    /// Build a `LIST INFO` group body: the `INFO` list-type word plus a
    /// sequence of `(fourcc, zstr-body)` child chunks (with pad).
    fn list_info_blob(children: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"INFO");
        for (id, payload) in children {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() & 1 == 1 {
                body.push(0); // RIFF pad
            }
        }
        // Wrap in a LIST chunk header.
        let mut out = Vec::new();
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn collect_from_walks_a_list_info_subtree() {
        // LIST INFO { INAM "Two Trees"Z, ICMT "A picture"Z }
        let blob = list_info_blob(&[(b"INAM", b"Two Trees\0"), (b"ICMT", b"A picture\0")]);
        let mut cur = Cursor::new(blob);
        // Read the LIST header, then open a walker over its body.
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        assert!(header.is_group());
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        assert_eq!(&walker.form_type(), b"INFO");
        let info = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(info.len(), 2);
        assert_eq!(info.get(InfoTag::INAM), Some("Two Trees"));
        assert_eq!(info.get(InfoTag::ICMT), Some("A picture"));
    }

    #[test]
    fn collect_from_handles_odd_length_body_pad() {
        // "Hi" (2) is even; "Odd" (3) needs a pad byte. Ensure the
        // walker re-syncs after the pad.
        let blob = list_info_blob(&[(b"INAM", b"Odd\0"), (b"IART", b"Hi\0")]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let info = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(info.get(InfoTag::INAM), Some("Odd"));
        assert_eq!(info.get(InfoTag::IART), Some("Hi"));
    }

    #[test]
    fn collect_from_keeps_unknown_vendor_tags() {
        let blob = list_info_blob(&[(b"IMP3", b"passthrough\0")]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let info = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(info.len(), 1);
        let (tag, value) = &info.entries()[0];
        assert_eq!(tag.fourcc(), *b"IMP3");
        assert!(!tag.is_baseline());
        assert_eq!(value, "passthrough");
    }

    #[test]
    fn encode_list_body_round_trips_through_collect() {
        let list = InfoList::new()
            .push(InfoTag::INAM, "Two Trees")
            .push(InfoTag::ICMT, "A picture")
            .push(InfoTag(*b"IMP3"), "passthrough");
        let body = list.encode_list_body().unwrap();
        // Wrap the body in a LIST header and re-collect.
        let mut blob = Vec::new();
        blob.extend_from_slice(b"LIST");
        blob.extend_from_slice(&(body.len() as u32).to_le_bytes());
        blob.extend_from_slice(&body);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let collected = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(collected, list);
    }

    #[test]
    fn encode_chunk_emits_list_header_and_pads_odd_children() {
        // "Odd" value → ZSTR "Odd\0" (4 bytes, even); "X" → "X\0" (2, even).
        // Use a 2-char value to force an odd ZSTR body needing a pad.
        let list = InfoList::new().push(InfoTag::INAM, "Hi");
        let chunk = list.encode_chunk().unwrap();
        assert_eq!(&chunk[0..4], b"LIST");
        // Re-parse the whole chunk back.
        let mut cur = Cursor::new(chunk);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        assert!(header.is_group());
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let collected = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(collected.get(InfoTag::INAM), Some("Hi"));
    }

    #[test]
    fn encode_handles_odd_length_zstr_pad() {
        // A 2-char value gives a 3-byte ZSTR ("Odd"=3 → "Odd\0"=4 even),
        // so use a 4-char value → 5-byte ZSTR (odd) to exercise the pad.
        let list = InfoList::new().push(InfoTag::IART, "Jane");
        let body = list.encode_list_body().unwrap();
        // INFO(4) + IART hdr(8) + "Jane\0"(5) + pad(1) = 18.
        assert_eq!(body.len(), 18);
        let mut blob = Vec::new();
        blob.extend_from_slice(b"LIST");
        blob.extend_from_slice(&(body.len() as u32).to_le_bytes());
        blob.extend_from_slice(&body);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let collected = InfoList::collect_from(&mut walker).unwrap();
        assert_eq!(collected.get(InfoTag::IART), Some("Jane"));
    }

    #[test]
    fn collect_from_rejects_non_info_list_type() {
        // A LIST with list-type "adtl" rather than "INFO".
        let mut body = Vec::new();
        body.extend_from_slice(b"adtl");
        let mut out = Vec::new();
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        let mut cur = Cursor::new(out);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let err = InfoList::collect_from(&mut walker).unwrap_err();
        assert!(format!("{err}").contains("non-INFO LIST"));
    }
}
