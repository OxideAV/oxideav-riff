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
//! the doc comment. An `InfoList` preserves the on-wire order of the
//! tags it collected and keeps any unrecognised four-character codes
//! verbatim (the spec explicitly allows new chunk IDs and instructs an
//! application to ignore — but not reject — IDs it does not understand).
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "INFO List Chunk" (the registered global `INFO` form-type and the
//!   baseline tag table) + "NULL-Terminated String (ZSTR) Format".

use crate::error::{Error, Result};

/// A registered four-character `INFO` sub-ID.
///
/// The associated constants are the 23 baseline tags the 1991 RIFF MCI
/// spec registers for the `INFO` list. The wrapped `[u8; 4]` is the
/// raw FourCC; [`InfoTag::label`] maps the registered ones to their
/// short human-readable name and returns `None` for any other code.
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

    /// The raw four-character code of this tag.
    pub const fn fourcc(&self) -> [u8; 4] {
        self.0
    }

    /// `true` if this is one of the 23 baseline `INFO` sub-IDs the
    /// 1991 spec registers.
    pub fn is_baseline(&self) -> bool {
        Self::BASELINE.contains(self)
    }

    /// Short human-readable label for a baseline tag, or `None` for a
    /// vendor / unknown four-character code.
    ///
    /// The labels are the registered field names from the 1991 RIFF
    /// MCI "INFO List Chunk" table.
    pub fn label(&self) -> Option<&'static str> {
        Some(match self.0 {
            b if b == *b"IARL" => "Archival Location",
            b if b == *b"IART" => "Artist",
            b if b == *b"ICMS" => "Commissioned",
            b if b == *b"ICMT" => "Comments",
            b if b == *b"ICOP" => "Copyright",
            b if b == *b"ICRD" => "Creation Date",
            b if b == *b"ICRP" => "Cropped",
            b if b == *b"IDIM" => "Dimensions",
            b if b == *b"IDPI" => "Dots Per Inch",
            b if b == *b"IENG" => "Engineer",
            b if b == *b"IGNR" => "Genre",
            b if b == *b"IKEY" => "Keywords",
            b if b == *b"ILGT" => "Lightness",
            b if b == *b"IMED" => "Medium",
            b if b == *b"INAM" => "Name",
            b if b == *b"IPLT" => "Palette Setting",
            b if b == *b"IPRD" => "Product",
            b if b == *b"ISBJ" => "Subject",
            b if b == *b"ISFT" => "Software",
            b if b == *b"ISHP" => "Sharpness",
            b if b == *b"ISRC" => "Source",
            b if b == *b"ISRF" => "Source Form",
            b if b == *b"ITCH" => "Technician",
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
        assert!(t.label().is_none());
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
