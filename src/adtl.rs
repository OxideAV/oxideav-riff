//! Typed decoder for the WAV / RIFF `LIST adtl` associated-data list.
//!
//! An `adtl` (associated-data) list attaches information — labels,
//! comments, length-bounded text, and embedded media files — to the cue
//! points carried by a `cue ` chunk (see [`crate::cue`]). It is a `LIST`
//! chunk whose list-type FourCC is `adtl`; each child chunk references a
//! cue point by its `dwName` and carries one piece of associated data.
//!
//! The 1991 RIFF MCI spec defines the list as:
//!
//! ```text
//! LIST( <ckSize:u32 LE>
//!   'adtl'                       // list-type FourCC
//!   <labl-ck>                    // Label
//!   <note-ck>                    // Note
//!   <ltxt-ck>                    // Text with data length
//!   <file-ck>                    // Media file
//! )
//!
//! labl( <dwName:DWORD> <data:ZSTR> )
//! note( <dwName:DWORD> <data:ZSTR> )
//!
//! ltxt( <dwName:DWORD>
//!       <dwSampleLength:DWORD>
//!       <dwPurpose:DWORD>
//!       <wCountry:WORD>
//!       <wLanguage:WORD>
//!       <wDialect:WORD>
//!       <wCodePage:WORD>
//!       <data:BYTE>... )
//!
//! file( <dwName:DWORD>
//!       <dwMedType:DWORD>
//!       <fileData:BYTE>... )
//! ```
//!
//! Every multi-byte field is little-endian. The child chunks may appear
//! in any order and any may be absent or repeated, so this module
//! decodes the list into an ordered sequence of [`AdtlEntry`] values
//! rather than a fixed record set; an unrecognised child FourCC is
//! preserved verbatim (the spec instructs applications to ignore — but
//! not reject — chunk IDs they do not understand).
//!
//! Like [`crate::info::InfoList`], this module does not walk the outer
//! chunk tree: the caller locates the `LIST adtl` group with the
//! [`crate::Walker`], reads its `adtl` list-type with
//! [`crate::Walker::read_inner_form_type`], and hands the resulting
//! sub-walker to [`AdtlList::collect_from`].
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "Associated Data Chunk" (the `<assoc-data-list>` grammar and the
//!   `labl` / `note` / `ltxt` / `file` per-field descriptions).

use crate::error::{Error, Result};

/// FourCC of the associated-data list-type word (`LIST` `adtl`).
pub const FOURCC_ADTL: [u8; 4] = *b"adtl";
/// FourCC of a label chunk.
pub const FOURCC_LABL: [u8; 4] = *b"labl";
/// FourCC of a note chunk.
pub const FOURCC_NOTE: [u8; 4] = *b"note";
/// FourCC of a labelled-text chunk.
pub const FOURCC_LTXT: [u8; 4] = *b"ltxt";
/// FourCC of an embedded-file chunk.
pub const FOURCC_FILE: [u8; 4] = *b"file";

/// Size in bytes of the fixed `ltxt` prefix that precedes the optional
/// trailing text (`dwName` + `dwSampleLength` + `dwPurpose` are 4 bytes
/// each; `wCountry` / `wLanguage` / `wDialect` / `wCodePage` are 2 bytes
/// each).
pub const LTXT_PREFIX_LEN: usize = 20;

/// Size in bytes of the fixed `file` prefix that precedes the opaque
/// trailing media bytes (`dwName` + `dwMedType`, 4 bytes each).
pub const FILE_PREFIX_LEN: usize = 8;

fn dw(raw: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]])
}

fn w(raw: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([raw[off], raw[off + 1]])
}

/// A decoded `ltxt` (labelled-text) record.
///
/// `ltxt` associates a block of text with a length-bounded segment of
/// the waveform starting at a cue point. The text body is kept raw
/// (`text`) rather than decoded as a ZSTR, because its character set is
/// governed by the record's own `code_page` field rather than the file's
/// global `CSET` chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabeledText {
    /// `dwName` — the cue point this text segment is associated with;
    /// must match a `dwName` in the `cue ` cue-point table.
    pub name: u32,
    /// `dwSampleLength` — the number of samples in the segment of
    /// waveform data this text describes.
    pub sample_length: u32,
    /// `dwPurpose` — the type or purpose of the text, carried as a
    /// FourCC (e.g. `scrp` for script text, `capt` for close-caption).
    /// A zero value (no specific purpose) reads as `[0; 4]`.
    pub purpose: [u8; 4],
    /// `wCountry` — the country code for the text (the Chapter 2 country
    /// code table).
    pub country: u16,
    /// `wLanguage` — the language code for the text (the Chapter 2
    /// language/dialect table).
    pub language: u16,
    /// `wDialect` — the dialect code for the text, paired with
    /// `language`.
    pub dialect: u16,
    /// `wCodePage` — the code page the `text` bytes are encoded in.
    pub code_page: u16,
    /// The trailing text bytes (chunk body minus the 20-byte prefix),
    /// kept raw because their character set is `code_page`-dependent.
    pub text: Vec<u8>,
}

impl LabeledText {
    /// Decode an `ltxt` chunk body (already pad-stripped by the walker).
    ///
    /// The body is the 20-byte fixed prefix followed by zero or more
    /// trailing text bytes. A body shorter than [`LTXT_PREFIX_LEN`] is
    /// rejected as truncated.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < LTXT_PREFIX_LEN {
            return Err(Error::invalid(
                "RIFF: ltxt chunk shorter than 20-byte prefix",
            ));
        }
        let mut purpose = [0u8; 4];
        purpose.copy_from_slice(&body[8..12]);
        Ok(LabeledText {
            name: dw(body, 0),
            sample_length: dw(body, 4),
            purpose,
            country: w(body, 12),
            language: w(body, 14),
            dialect: w(body, 16),
            code_page: w(body, 18),
            text: body[LTXT_PREFIX_LEN..].to_vec(),
        })
    }
}

/// A decoded `file` (embedded-media) record.
///
/// `file` embeds another file's contents — for example an `RDIB` image
/// or an ASCII text file — and binds it to a cue point. The embedded
/// payload is kept opaque (`data`); this module does not recurse into
/// it, since `dwMedType` may name an arbitrary external format.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmbeddedFile {
    /// `dwName` — the cue point this media file is associated with; must
    /// match a `dwName` in the `cue ` cue-point table.
    pub name: u32,
    /// `dwMedType` — the file type contained in `data`. When the
    /// payload is itself a RIFF form, this equals that form's type; a
    /// zero value means unspecified.
    pub med_type: u32,
    /// The raw embedded media bytes (chunk body minus the 8-byte
    /// prefix).
    pub data: Vec<u8>,
}

impl EmbeddedFile {
    /// Decode a `file` chunk body (already pad-stripped by the walker).
    ///
    /// The body is the 8-byte fixed prefix followed by the opaque media
    /// payload. A body shorter than [`FILE_PREFIX_LEN`] is rejected as
    /// truncated.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < FILE_PREFIX_LEN {
            return Err(Error::invalid(
                "RIFF: file chunk shorter than 8-byte prefix",
            ));
        }
        Ok(EmbeddedFile {
            name: dw(body, 0),
            med_type: dw(body, 4),
            data: body[FILE_PREFIX_LEN..].to_vec(),
        })
    }
}

/// One decoded child of a `LIST adtl` block.
///
/// The four spec-defined child kinds plus an `Other` arm for any
/// unrecognised four-character code, whose body is preserved verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdtlEntry {
    /// A `labl` label: title text for a cue point.
    Label {
        /// `dwName` — the cue point this label is attached to.
        name: u32,
        /// The label text (ZSTR body decoded to a [`String`]).
        text: String,
    },
    /// A `note` comment: free-text annotation for a cue point.
    Note {
        /// `dwName` — the cue point this note is attached to.
        name: u32,
        /// The comment text (ZSTR body decoded to a [`String`]).
        text: String,
    },
    /// An `ltxt` labelled-text segment.
    LabeledText(LabeledText),
    /// A `file` embedded media file.
    File(EmbeddedFile),
    /// An unrecognised child chunk, preserved verbatim.
    Other {
        /// The raw four-character code of the chunk.
        fourcc: [u8; 4],
        /// The raw (pad-stripped) chunk body.
        body: Vec<u8>,
    },
}

impl AdtlEntry {
    /// Decode one `adtl` child chunk from its FourCC and (pad-stripped)
    /// body.
    ///
    /// `labl` / `note` bodies are a 4-byte `dwName` followed by a ZSTR
    /// (a body shorter than the 4-byte name is rejected). `ltxt` /
    /// `file` delegate to [`LabeledText::parse`] / [`EmbeddedFile::parse`].
    /// Any other FourCC yields [`AdtlEntry::Other`].
    pub fn parse(fourcc: [u8; 4], body: &[u8]) -> Result<Self> {
        match fourcc {
            FOURCC_LABL | FOURCC_NOTE => {
                if body.len() < 4 {
                    return Err(Error::invalid(
                        "RIFF: labl/note chunk shorter than 4-byte dwName",
                    ));
                }
                let name = dw(body, 0);
                let text = crate::info::zstr_value(&body[4..]);
                Ok(if fourcc == FOURCC_LABL {
                    AdtlEntry::Label { name, text }
                } else {
                    AdtlEntry::Note { name, text }
                })
            }
            FOURCC_LTXT => Ok(AdtlEntry::LabeledText(LabeledText::parse(body)?)),
            FOURCC_FILE => Ok(AdtlEntry::File(EmbeddedFile::parse(body)?)),
            _ => Ok(AdtlEntry::Other {
                fourcc,
                body: body.to_vec(),
            }),
        }
    }

    /// The cue point name (`dwName`) this entry references, or `None`
    /// for an [`AdtlEntry::Other`] (whose layout is unknown).
    pub fn cue_name(&self) -> Option<u32> {
        match self {
            AdtlEntry::Label { name, .. } | AdtlEntry::Note { name, .. } => Some(*name),
            AdtlEntry::LabeledText(l) => Some(l.name),
            AdtlEntry::File(f) => Some(f.name),
            AdtlEntry::Other { .. } => None,
        }
    }
}

/// A decoded `LIST adtl` block: the associated-data entries in on-wire
/// order.
///
/// Order is preserved; any child kind may be absent, repeated, or
/// interleaved, so the entries are kept as a flat ordered list rather
/// than grouped by kind. The cross-reference to a `cue ` chunk is
/// recorded (each entry's `dwName`) but not resolved, since this decoder
/// has no view of the surrounding chunk tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdtlList {
    entries: Vec<AdtlEntry>,
}

impl AdtlList {
    /// A new, empty associated-data list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one child chunk and append it to the list.
    pub fn push_chunk(&mut self, fourcc: [u8; 4], body: &[u8]) -> Result<()> {
        self.entries.push(AdtlEntry::parse(fourcc, body)?);
        Ok(())
    }

    /// The entries in on-wire order.
    pub fn entries(&self) -> &[AdtlEntry] {
        &self.entries
    }

    /// Number of entries collected.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if the list has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries that reference cue point `name`, in on-wire order.
    ///
    /// An `adtl` list may attach several pieces of data (a label, a
    /// note, several `ltxt` segments) to the same cue point, so this
    /// returns every match rather than the first. [`AdtlEntry::Other`]
    /// entries (unknown layout) are never matched.
    pub fn by_cue_name(&self, name: u32) -> impl Iterator<Item = &AdtlEntry> {
        self.entries
            .iter()
            .filter(move |e| e.cue_name() == Some(name))
    }

    /// First `labl` text attached to cue point `name`, or `None`.
    pub fn label(&self, name: u32) -> Option<&str> {
        self.entries.iter().find_map(|e| match e {
            AdtlEntry::Label { name: n, text } if *n == name => Some(text.as_str()),
            _ => None,
        })
    }

    /// First `note` text attached to cue point `name`, or `None`.
    pub fn note(&self, name: u32) -> Option<&str> {
        self.entries.iter().find_map(|e| match e {
            AdtlEntry::Note { name: n, text } if *n == name => Some(text.as_str()),
            _ => None,
        })
    }

    /// Collect a whole `LIST adtl` sub-tree from a [`crate::Walker`]
    /// already positioned over the `adtl` list body (constructed after
    /// the caller read the `adtl` list-type with
    /// [`crate::Walker::read_inner_form_type`]).
    ///
    /// Each child chunk is read in full and decoded. A non-`adtl`
    /// list-type is rejected; the walker's parent-budget enforcement
    /// still applies to each child.
    pub fn collect_from<R: std::io::Read + std::io::Seek + ?Sized>(
        walker: &mut crate::Walker<'_, R>,
    ) -> Result<Self> {
        if walker.form_type() != FOURCC_ADTL {
            return Err(Error::invalid(
                "RIFF: collect_from called on a non-adtl LIST",
            ));
        }
        let mut list = Self::new();
        while let Some(chunk) = walker.read_next()? {
            let body = walker.read_body(&chunk)?;
            list.push_chunk(chunk.id, &body)?;
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn labl_body(name: u32, text: &[u8]) -> Vec<u8> {
        let mut v = name.to_le_bytes().to_vec();
        v.extend_from_slice(text);
        v
    }

    /// Build an `ltxt` body. `codes` is
    /// `[wCountry, wLanguage, wDialect, wCodePage]`.
    fn ltxt_body(
        name: u32,
        sample_length: u32,
        purpose: &[u8; 4],
        codes: [u16; 4],
        text: &[u8],
    ) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&name.to_le_bytes());
        v.extend_from_slice(&sample_length.to_le_bytes());
        v.extend_from_slice(purpose);
        for c in codes {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(text);
        v
    }

    fn file_body(name: u32, med_type: u32, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&name.to_le_bytes());
        v.extend_from_slice(&med_type.to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    /// Build a `LIST adtl` blob: the `adtl` list-type word plus a
    /// sequence of `(fourcc, body)` child chunks (with RIFF pad).
    fn list_adtl_blob(children: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"adtl");
        for (id, payload) in children {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() & 1 == 1 {
                body.push(0);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn prefix_constants() {
        assert_eq!(LTXT_PREFIX_LEN, 20);
        assert_eq!(FILE_PREFIX_LEN, 8);
    }

    #[test]
    fn parse_labl_and_note() {
        let l = AdtlEntry::parse(FOURCC_LABL, &labl_body(1, b"Intro\0")).unwrap();
        assert_eq!(
            l,
            AdtlEntry::Label {
                name: 1,
                text: "Intro".to_string()
            }
        );
        assert_eq!(l.cue_name(), Some(1));
        let n = AdtlEntry::parse(FOURCC_NOTE, &labl_body(2, b"see take 3\0")).unwrap();
        assert_eq!(
            n,
            AdtlEntry::Note {
                name: 2,
                text: "see take 3".to_string()
            }
        );
    }

    #[test]
    fn labl_without_terminator_uses_pad() {
        // Body relies on the RIFF pad byte only (no embedded NUL).
        let l = AdtlEntry::parse(FOURCC_LABL, &labl_body(5, b"NoNul")).unwrap();
        assert_eq!(
            l,
            AdtlEntry::Label {
                name: 5,
                text: "NoNul".to_string()
            }
        );
    }

    #[test]
    fn labl_too_short_is_rejected() {
        let err = AdtlEntry::parse(FOURCC_LABL, &[0u8; 3]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 4-byte dwName"));
    }

    #[test]
    fn parse_ltxt_full_fields() {
        let body = ltxt_body(7, 4410, b"scrp", [1, 9, 1, 1252], b"Hello");
        let e = AdtlEntry::parse(FOURCC_LTXT, &body).unwrap();
        match e {
            AdtlEntry::LabeledText(l) => {
                assert_eq!(l.name, 7);
                assert_eq!(l.sample_length, 4410);
                assert_eq!(&l.purpose, b"scrp");
                assert_eq!(l.country, 1);
                assert_eq!(l.language, 9);
                assert_eq!(l.dialect, 1);
                assert_eq!(l.code_page, 1252);
                assert_eq!(l.text, b"Hello");
            }
            other => panic!("expected LabeledText, got {other:?}"),
        }
    }

    #[test]
    fn ltxt_prefix_only_has_empty_text() {
        let body = ltxt_body(7, 0, &[0; 4], [0; 4], b"");
        assert_eq!(body.len(), LTXT_PREFIX_LEN);
        let e = AdtlEntry::parse(FOURCC_LTXT, &body).unwrap();
        match e {
            AdtlEntry::LabeledText(l) => {
                assert!(l.text.is_empty());
                assert_eq!(l.purpose, [0; 4]);
            }
            other => panic!("expected LabeledText, got {other:?}"),
        }
    }

    #[test]
    fn ltxt_too_short_is_rejected() {
        let err = AdtlEntry::parse(FOURCC_LTXT, &[0u8; 19]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 20-byte prefix"));
    }

    #[test]
    fn parse_file_with_payload() {
        let body = file_body(3, u32::from_le_bytes(*b"RDIB"), b"\x00\x01\x02\x03");
        let e = AdtlEntry::parse(FOURCC_FILE, &body).unwrap();
        match e {
            AdtlEntry::File(f) => {
                assert_eq!(f.name, 3);
                assert_eq!(f.med_type, u32::from_le_bytes(*b"RDIB"));
                assert_eq!(f.data, b"\x00\x01\x02\x03");
            }
            other => panic!("expected File, got {other:?}"),
        }
    }

    #[test]
    fn file_prefix_only_has_empty_data() {
        let body = file_body(3, 0, b"");
        assert_eq!(body.len(), FILE_PREFIX_LEN);
        let e = AdtlEntry::parse(FOURCC_FILE, &body).unwrap();
        match e {
            AdtlEntry::File(f) => assert!(f.data.is_empty()),
            other => panic!("expected File, got {other:?}"),
        }
    }

    #[test]
    fn file_too_short_is_rejected() {
        let err = AdtlEntry::parse(FOURCC_FILE, &[0u8; 7]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 8-byte prefix"));
    }

    #[test]
    fn unknown_child_is_preserved_verbatim() {
        let e = AdtlEntry::parse(*b"junk", b"\xde\xad\xbe\xef").unwrap();
        assert_eq!(
            e,
            AdtlEntry::Other {
                fourcc: *b"junk",
                body: b"\xde\xad\xbe\xef".to_vec()
            }
        );
        assert_eq!(e.cue_name(), None);
    }

    #[test]
    fn collect_from_walks_an_adtl_subtree() {
        let blob = list_adtl_blob(&[
            (b"labl", labl_body(1, b"Intro\0")),
            (b"note", labl_body(1, b"fade in\0")),
            (b"ltxt", ltxt_body(2, 22050, b"capt", [0; 4], b"caption")),
        ]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        assert!(header.is_group());
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        assert_eq!(walker.form_type(), FOURCC_ADTL);
        let adtl = AdtlList::collect_from(&mut walker).unwrap();
        assert_eq!(adtl.len(), 3);
        assert_eq!(adtl.label(1), Some("Intro"));
        assert_eq!(adtl.note(1), Some("fade in"));
        // by_cue_name returns label + note for cue 1.
        assert_eq!(adtl.by_cue_name(1).count(), 2);
        assert_eq!(adtl.by_cue_name(2).count(), 1);
    }

    #[test]
    fn collect_from_handles_odd_length_pad_resync() {
        // "Odd" (3 bytes after a 4-byte name = 7-byte body) needs a pad.
        let blob = list_adtl_blob(&[
            (b"labl", labl_body(1, b"Odd")),
            (b"labl", labl_body(2, b"Even\0")),
        ]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let adtl = AdtlList::collect_from(&mut walker).unwrap();
        assert_eq!(adtl.label(1), Some("Odd"));
        assert_eq!(adtl.label(2), Some("Even"));
    }

    #[test]
    fn collect_from_keeps_unknown_child() {
        let blob = list_adtl_blob(&[(b"junk", b"\x01\x02\x03\x04".to_vec())]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let adtl = AdtlList::collect_from(&mut walker).unwrap();
        assert_eq!(adtl.len(), 1);
        match &adtl.entries()[0] {
            AdtlEntry::Other { fourcc, .. } => assert_eq!(fourcc, b"junk"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn collect_from_rejects_non_adtl_list_type() {
        let mut body = Vec::new();
        body.extend_from_slice(b"INFO");
        let mut out = Vec::new();
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        let mut cur = Cursor::new(out);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let err = AdtlList::collect_from(&mut walker).unwrap_err();
        assert!(format!("{err}").contains("non-adtl LIST"));
    }

    #[test]
    fn empty_adtl_list() {
        let blob = list_adtl_blob(&[]);
        let mut cur = Cursor::new(blob);
        let header = crate::chunk::read_chunk_header(&mut cur).unwrap().unwrap();
        let mut walker = crate::Walker::open_within(&mut cur, &header).unwrap();
        let adtl = AdtlList::collect_from(&mut walker).unwrap();
        assert!(adtl.is_empty());
        assert_eq!(adtl.label(1), None);
        assert_eq!(adtl.note(1), None);
    }
}
