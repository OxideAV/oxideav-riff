//! Typed carrier for the RIFF `id3 ` / `ID3 ` chunk (embedded ID3v2 tag).
//!
//! A RIFF/WAVE (or AVI) file may carry an ID3v2 metadata tag inside an
//! `id3 ` chunk (lower-case, the form most readers emit) or the
//! historically-equivalent upper-case `ID3 ` chunk. The chunk **body is a
//! complete, self-contained ID3v2 tag** — the same byte sequence that
//! would prefix a bare `.mp3` file. The RIFF layer's only job is to
//! recognise the chunk and hand the embedded tag to an ID3 decoder
//! (`oxideav-id3`); the per-frame semantics of the tag are emphatically
//! **not** parsed here (that would be codec/tag work, not container
//! work). This module therefore preserves the tag body verbatim for a
//! byte-exact mux round-trip and exposes a *lightweight* recognizer over
//! the 10-byte ID3v2 header — magic, version, flags, and the declared
//! sync-safe tag size — so a caller can validate the carriage and route
//! to the tag decoder without this crate growing an ID3 frame parser.
//!
//! ## ID3v2 header (the 10 bytes the recognizer reads)
//!
//! ```text
//! +0  magic           : 3 bytes  "ID3"
//! +3  version major   : u8       2 / 3 / 4 (the "v2.X" minor)
//! +4  version revision: u8       usually 0
//! +5  flags           : u8       bit7 unsynchronisation, bit6 extended
//!                                 header, bit5 experimental, bit4 footer
//! +6  size            : u32      sync-safe (bit 7 of each byte = 0),
//!                                 28 usable bits, size of the tag *after*
//!                                 the 10-byte header (excludes the header,
//!                                 includes a footer if present)
//! ```
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/README.md` — "id3 chunk —
//!   ID3v2-in-RIFF (Microsoft-allowed namespacing for legacy MP3-tagging
//!   compatibility)".
//! - `docs/container/riff/metadata/exiftool-riff-tags.html` — the
//!   `'id3 '` / `'ID3 '` RIFF chunk → ID3 Tags mapping.
//! - `docs/container/id3/README.md` — the 10-byte ID3v2 header layout
//!   (magic + version + flags + sync-safe size) common to all v2
//!   versions, and the sync-safe integer definition. Only the *header*
//!   shape is used here; frame decoding lives in `oxideav-id3`.

use crate::error::Result;

/// The lower-case `id3 ` chunk FourCC (the form most writers emit).
pub const FOURCC_ID3: [u8; 4] = *b"id3 ";

/// The upper-case `ID3 ` chunk FourCC (historically-equivalent variant).
pub const FOURCC_ID3_UPPER: [u8; 4] = *b"ID3 ";

/// The 3-byte ID3v2 tag magic that opens the chunk body.
pub const ID3V2_MAGIC: [u8; 3] = *b"ID3";

/// The fixed ID3v2 header length (magic + version + flags + size).
pub const ID3V2_HEADER_LEN: usize = 10;

/// `true` if `fourcc` is one of the two ID3-tag chunk identifiers.
pub const fn is_id3_fourcc(fourcc: &[u8; 4]) -> bool {
    matches!(fourcc, b"id3 " | b"ID3 ")
}

/// The recognised 10-byte ID3v2 tag header, decoded from the front of an
/// `id3 ` chunk body. The flags byte's bits gate the optional tag
/// regions; the `size` is the sync-safe tag length *after* the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id3v2Header {
    /// The "v2.X" major version (`2`, `3`, or `4` in practice).
    pub version_major: u8,
    /// The version revision byte (almost always `0`).
    pub version_revision: u8,
    /// The raw flags byte.
    pub flags: u8,
    /// The declared tag size (sync-safe-decoded): the number of bytes
    /// that follow the 10-byte header (extended header + frames + padding
    /// + optional footer).
    pub size: u32,
}

/// Flag bit 7 — unsynchronisation applied to the whole tag.
pub const ID3_FLAG_UNSYNCHRONISATION: u8 = 0b1000_0000;
/// Flag bit 6 — an extended header follows the main header (v2.3+).
pub const ID3_FLAG_EXTENDED_HEADER: u8 = 0b0100_0000;
/// Flag bit 5 — the tag is experimental.
pub const ID3_FLAG_EXPERIMENTAL: u8 = 0b0010_0000;
/// Flag bit 4 — a footer is present (v2.4 only).
pub const ID3_FLAG_FOOTER: u8 = 0b0001_0000;

impl Id3v2Header {
    /// `true` if the unsynchronisation flag (bit 7) is set.
    pub const fn unsynchronisation(self) -> bool {
        self.flags & ID3_FLAG_UNSYNCHRONISATION != 0
    }

    /// `true` if an extended header is present (bit 6).
    pub const fn has_extended_header(self) -> bool {
        self.flags & ID3_FLAG_EXTENDED_HEADER != 0
    }

    /// `true` if the experimental flag (bit 5) is set.
    pub const fn experimental(self) -> bool {
        self.flags & ID3_FLAG_EXPERIMENTAL != 0
    }

    /// `true` if a footer is present (bit 4, v2.4 only).
    pub const fn has_footer(self) -> bool {
        self.flags & ID3_FLAG_FOOTER != 0
    }

    /// The total tag length including the 10-byte header (and a 10-byte
    /// footer if [`Id3v2Header::has_footer`]). This is the number of body
    /// bytes the tag *claims* to occupy; a conformant `id3 ` chunk's
    /// `ckSize` is at least this. Returns `None` on the (practically
    /// impossible) overflow of `u32`.
    pub fn total_tag_len(self) -> Option<u32> {
        let footer = if self.has_footer() { 10u32 } else { 0 };
        self.size
            .checked_add(ID3V2_HEADER_LEN as u32)
            .and_then(|n| n.checked_add(footer))
    }
}

/// A decoded RIFF `id3 ` / `ID3 ` chunk: the embedded ID3v2 tag,
/// preserved verbatim, plus a parsed [`Id3v2Header`] when the body opens
/// with a valid ID3v2 header.
///
/// The `tag` bytes are exactly the chunk body — pass them straight to an
/// ID3v2 decoder. This crate does **not** decode frames; the optional
/// [`Id3Chunk::header`] is a recognition convenience only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Id3Chunk {
    /// The complete embedded ID3v2 tag (the chunk body verbatim).
    pub tag: Vec<u8>,
    /// The parsed 10-byte ID3v2 header, present when the body opens with
    /// the `"ID3"` magic and a plausible header. `None` when the body is
    /// shorter than 10 bytes or does not begin with the magic (the bytes
    /// are still preserved in [`Id3Chunk::tag`]).
    pub header: Option<Id3v2Header>,
}

/// Sync-safe decode: read four bytes whose bit 7 is each zero into a
/// 28-bit integer (`xxxxxxx0` × 4 → 28 usable bits). Any high bit set is
/// tolerated (masked off) the way a lenient reader treats it.
fn syncsafe_u32(b: &[u8; 4]) -> u32 {
    ((b[0] as u32 & 0x7F) << 21)
        | ((b[1] as u32 & 0x7F) << 14)
        | ((b[2] as u32 & 0x7F) << 7)
        | (b[3] as u32 & 0x7F)
}

/// Sync-safe encode: the inverse of [`syncsafe_u32`] — split a 28-bit
/// value across four bytes with bit 7 of each forced to zero. Values
/// larger than 28 bits are truncated to the low 28 bits (the maximum a
/// sync-safe field can represent).
fn syncsafe_bytes(v: u32) -> [u8; 4] {
    [
        ((v >> 21) & 0x7F) as u8,
        ((v >> 14) & 0x7F) as u8,
        ((v >> 7) & 0x7F) as u8,
        (v & 0x7F) as u8,
    ]
}

impl Id3Chunk {
    /// Wrap an `id3 ` chunk body verbatim, recognising the ID3v2 header
    /// if one is present at the front.
    pub fn from_body(body: &[u8]) -> Self {
        let header = parse_id3v2_header(body);
        Self {
            tag: body.to_vec(),
            header,
        }
    }

    /// Build an `id3 ` chunk from a complete ID3v2 tag the caller already
    /// has (e.g. from `oxideav-id3`'s encoder). The bytes are carried
    /// verbatim; the header is re-recognised from them.
    pub fn from_tag(tag: &[u8]) -> Self {
        Self::from_body(tag)
    }

    /// The embedded tag bytes (identical to [`Id3Chunk::tag`]) — ready to
    /// hand to an ID3v2 decoder.
    pub fn tag_bytes(&self) -> &[u8] {
        &self.tag
    }

    /// `true` if the body was recognised as opening with a valid ID3v2
    /// header.
    pub fn is_recognised(&self) -> bool {
        self.header.is_some()
    }

    /// The body length in bytes (the chunk's `ckSize`).
    pub fn len(&self) -> usize {
        self.tag.len()
    }

    /// `true` if the chunk has a zero-length body (degenerate, but
    /// preserved rather than rejected).
    pub fn is_empty(&self) -> bool {
        self.tag.is_empty()
    }

    /// The `id3 ` *body* bytes — identical to [`Id3Chunk::tag`], provided
    /// for symmetry with the other typed encoders' `encode_body`.
    pub fn encode_body(&self) -> Vec<u8> {
        self.tag.clone()
    }

    /// Serialize the whole chunk under the lower-case `id3 ` FourCC,
    /// framed with [`crate::chunk::encode_chunk`] (an odd-length tag gets
    /// its trailing RIFF pad byte).
    pub fn encode_chunk(&self) -> Result<Vec<u8>> {
        self.encode_chunk_with(&FOURCC_ID3)
    }

    /// Serialize the whole chunk under a caller-chosen FourCC — pass
    /// [`FOURCC_ID3`] (lower-case) or [`FOURCC_ID3_UPPER`] to preserve the
    /// exact spelling a source file used for a byte-exact round-trip.
    pub fn encode_chunk_with(&self, fourcc: &[u8; 4]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        crate::chunk::encode_chunk(&mut out, fourcc, &self.tag)?;
        Ok(out)
    }
}

/// Parse the 10-byte ID3v2 header from the front of `body`, if present.
/// Returns `None` if the body is shorter than 10 bytes or does not open
/// with the `"ID3"` magic.
pub fn parse_id3v2_header(body: &[u8]) -> Option<Id3v2Header> {
    if body.len() < ID3V2_HEADER_LEN || body[0..3] != ID3V2_MAGIC {
        return None;
    }
    let size = syncsafe_u32(&[body[6], body[7], body[8], body[9]]);
    Some(Id3v2Header {
        version_major: body[3],
        version_revision: body[4],
        flags: body[5],
        size,
    })
}

/// Build a minimal valid 10-byte ID3v2 header (magic + version + flags +
/// sync-safe size). Useful for tests and for a writer assembling a tag
/// from frame bytes. The `size` is the post-header tag length.
pub fn build_id3v2_header(
    version_major: u8,
    version_revision: u8,
    flags: u8,
    size: u32,
) -> [u8; 10] {
    let s = syncsafe_bytes(size);
    [
        ID3V2_MAGIC[0],
        ID3V2_MAGIC[1],
        ID3V2_MAGIC[2],
        version_major,
        version_revision,
        flags,
        s[0],
        s[1],
        s[2],
        s[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{read_chunk_header, FOURCC_LIST, FOURCC_RIFF};
    use std::io::Cursor;

    fn sample_tag() -> Vec<u8> {
        // A v2.3 header (flags = 0) claiming 0x1FF bytes follow, then some
        // opaque "frame" bytes (we do NOT decode them).
        let mut t = build_id3v2_header(3, 0, 0, 0x1FF).to_vec();
        t.extend_from_slice(&[b'T', b'I', b'T', b'2', 0, 0, 0, 5, 0, 0]);
        t
    }

    #[test]
    fn fourcc_recognition() {
        assert!(is_id3_fourcc(b"id3 "));
        assert!(is_id3_fourcc(b"ID3 "));
        assert!(!is_id3_fourcc(b"data"));
        assert!(!is_id3_fourcc(b"ID3\0"));
        assert_ne!(FOURCC_ID3, FOURCC_RIFF);
        assert_ne!(FOURCC_ID3, FOURCC_LIST);
    }

    #[test]
    fn syncsafe_round_trips() {
        for v in [0u32, 1, 0x7F, 0x80, 0x1FF, 0x0FFF_FFFF] {
            assert_eq!(syncsafe_u32(&syncsafe_bytes(v)), v, "v = {v:#x}");
        }
        // The encode forces bit 7 of every byte to zero.
        for b in syncsafe_bytes(0x0FFF_FFFF) {
            assert_eq!(b & 0x80, 0);
        }
        // Values past 28 bits truncate to the low 28.
        assert_eq!(syncsafe_u32(&syncsafe_bytes(0xFFFF_FFFF)), 0x0FFF_FFFF);
    }

    #[test]
    fn parses_a_v23_header() {
        let body = sample_tag();
        let h = parse_id3v2_header(&body).unwrap();
        assert_eq!(h.version_major, 3);
        assert_eq!(h.version_revision, 0);
        assert_eq!(h.flags, 0);
        assert_eq!(h.size, 0x1FF);
        assert!(!h.unsynchronisation());
        assert!(!h.has_extended_header());
        assert!(!h.has_footer());
        assert_eq!(h.total_tag_len(), Some(0x1FF + 10));
    }

    #[test]
    fn flag_bits_decode() {
        let flags = ID3_FLAG_UNSYNCHRONISATION
            | ID3_FLAG_EXTENDED_HEADER
            | ID3_FLAG_EXPERIMENTAL
            | ID3_FLAG_FOOTER;
        let body = build_id3v2_header(4, 0, flags, 100).to_vec();
        let h = parse_id3v2_header(&body).unwrap();
        assert!(h.unsynchronisation());
        assert!(h.has_extended_header());
        assert!(h.experimental());
        assert!(h.has_footer());
        // Footer adds a second 10-byte block to the total.
        assert_eq!(h.total_tag_len(), Some(100 + 10 + 10));
    }

    #[test]
    fn unrecognised_body_is_preserved_not_rejected() {
        // A body that doesn't open with the ID3 magic — kept verbatim,
        // header is None.
        let body = [0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4, 5, 6, 7, 8];
        let c = Id3Chunk::from_body(&body);
        assert!(!c.is_recognised());
        assert_eq!(c.tag, body);
        assert_eq!(c.len(), 12);

        // A body too short to hold a header.
        let short = Id3Chunk::from_body(b"ID3");
        assert!(!short.is_recognised());
        assert_eq!(short.tag, b"ID3");
    }

    #[test]
    fn from_body_recognises_and_preserves() {
        let body = sample_tag();
        let c = Id3Chunk::from_body(&body);
        assert!(c.is_recognised());
        assert_eq!(c.tag_bytes(), &body[..]);
        assert_eq!(c.header.unwrap().version_major, 3);
    }

    #[test]
    fn encode_chunk_round_trips_lowercase() {
        let body = sample_tag();
        let c = Id3Chunk::from_body(&body);
        let chunk = c.encode_chunk().unwrap();
        assert_eq!(&chunk[0..4], b"id3 ");
        assert_eq!(
            u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            body.len() as u32
        );
        assert_eq!(&chunk[8..8 + body.len()], &body[..]);

        let mut cur = Cursor::new(chunk);
        let header = read_chunk_header(&mut cur).unwrap().unwrap();
        assert_eq!(header.id, FOURCC_ID3);
        assert_eq!(header.size as usize, body.len());
    }

    #[test]
    fn encode_chunk_preserves_uppercase_spelling() {
        let body = sample_tag();
        let c = Id3Chunk::from_body(&body);
        let chunk = c.encode_chunk_with(&FOURCC_ID3_UPPER).unwrap();
        assert_eq!(&chunk[0..4], b"ID3 ");
    }

    #[test]
    fn encode_chunk_pads_odd_body() {
        // An odd-length tag body needs a trailing RIFF pad byte.
        let mut body = build_id3v2_header(4, 0, 0, 3).to_vec();
        body.extend_from_slice(&[1, 2, 3]); // total 13 bytes (odd)
        let c = Id3Chunk::from_body(&body);
        let chunk = c.encode_chunk().unwrap();
        // header(8) + body(13) + pad(1) = 22.
        assert_eq!(chunk.len(), 22);
        assert_eq!(*chunk.last().unwrap(), 0, "missing RIFF pad byte");
        assert_eq!(
            u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            13
        );
    }

    #[test]
    fn empty_body_is_preserved() {
        let c = Id3Chunk::from_body(&[]);
        assert!(c.is_empty());
        assert!(!c.is_recognised());
        assert_eq!(c.encode_body(), Vec::<u8>::new());
    }
}
