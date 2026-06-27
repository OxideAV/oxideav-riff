//! RIFF Compound File `CTOC` table-of-contents + `CGRP` element-group
//! chunks.
//!
//! The 1991 RIFF spec (§2 "Compound File Structure") defines a generic
//! container-within-a-container: a `RIFF('type' <CTOC> <CGRP>)` file in
//! which the `CGRP` chunk holds a contiguous block of arbitrary
//! "compound file elements" (each potentially itself a RIFF form) and
//! the `CTOC` chunk is the index into it — one table entry per element
//! giving its offset, size, media type, name, and attributes. The
//! Bundle (`BND`) format is the canonical user:
//! `RIFF('BND' <CTOC-chunk> <CGRP-chunk>)`.
//!
//! `CTOC` is a *parameterized* structure: its header declares the size
//! of each table entry (`wEntrySize`), the name-field width
//! (`wNameSize`), and the count of optional "extra" header / entry
//! fields (`wExHdrFields` / `wExEntFields`) whose meaning is given by
//! parallel usage-code arrays. This module decodes the full layout and
//! re-encodes it byte-for-byte.
//!
//! ## Layout (§2)
//!
//! ```text
//! CTOC (
//!   // -- Header information (7 DWORDs) --
//!   dwHeaderSize  dwEntriesTotal  dwEntriesDeleted  dwEntriesUnused
//!   dwBytesTotal  dwBytesDeleted  dwHeaderFlags
//!   // -- Parameter table definition --
//!   wEntrySize  wNameSize  wExHdrFields  wExEntFields
//!   awExHdrFldUsage[wExHdrFields]   awExEntFldUsage[wExEntFields]
//!   // -- Header parameter table --
//!   adwExHdrField[wExHdrFields]   [bHeaderPad...]
//!   // -- CTOC table entries (dwEntriesTotal of them) --
//!   <CTOC-table-entry>...
//! )
//!
//! <CTOC-table-entry> ->
//!   dwOffset  dwSize  dwMedType  dwMedUsage  dwCompressTech
//!   dwUncompressBytes   adwExEntField[wExEntFields]
//!   bEntryFlags(BYTE)   achName[wNameSize]   [bEntryPad...]
//! ```
//!
//! All multi-byte fields are little-endian (the spec's Intel byte
//! order; `RIFX`/Motorola compound files are not defined).

use crate::error::{Error, Result};
use crate::fourcc::fourcc_bytes;

/// FourCC of the table-of-contents chunk.
pub const FOURCC_CTOC: [u8; 4] = fourcc_bytes(b"CTOC");

/// FourCC of the element-group chunk.
pub const FOURCC_CGRP: [u8; 4] = fourcc_bytes(b"CGRP");

/// Fixed size of the CTOC header-information section: seven DWORDs.
pub const CTOC_HEADER_INFO_LEN: usize = 7 * 4;

/// Fixed size of the parameter-table-definition preamble: four WORDs
/// (before the variable-length usage arrays).
pub const CTOC_PARAM_DEF_LEN: usize = 4 * 2;

// --- dwHeaderFlags bits (§2 "Header Information") ---

/// Valid CTOC table entries are arranged in sequential order. If unset,
/// entries may be in arbitrary order.
pub const CTOC_HF_SEQUENTIAL: u32 = 0x0000_0001;
/// The `dwMedUsage` field of each entry contains a FOURCC indicating how
/// the element is used. If unset, `dwMedUsage` is form-type-defined.
pub const CTOC_HF_MEDSUBTYPE: u32 = 0x0000_0002;

// --- bEntryFlags bits (§2 "CTOC Table Entries") ---

/// Compound file element is marked deleted and should not be accessed.
/// Mutually exclusive with [`CTOC_EF_UNUSED`].
pub const CTOC_EF_DELETED: u8 = 0x01;
/// CTOC table entry is unused and refers to no element. Mutually
/// exclusive with [`CTOC_EF_DELETED`].
pub const CTOC_EF_UNUSED: u8 = 0x02;

// --- Usage codes for extra header / entry fields (§2 "Usage Codes") ---

/// The field is unused (may logically delete a header field).
pub const CTOC_EFU_UNUSED: u16 = 0x00;
/// Last-modified time, seconds since the Unix epoch (GMT).
pub const CTOC_EFU_LASTMODTIME: u16 = 0x01;
/// Code page + country code for the `achName` field; overrides `CSET`.
pub const CTOC_EFU_CODEPAGE: u16 = 0x02;
/// Language + dialect for the `achName` field; overrides `CSET`.
pub const CTOC_EFU_LANGUAGE: u16 = 0x03;
/// First compression-parameter usage code. The spec lists the
/// compression-parameter range as `CTOC_EFU_COMPRESSPARAM0` (`0x05`)
/// "through" `CTOC_EFU_COMPRESSPARAM9` (`0x14`); each specifies a
/// compression parameter (see §2 "Compression of Compound File
/// Elements").
pub const CTOC_EFU_COMPRESSPARAM0: u16 = 0x05;
/// Last compression-parameter usage code (`0x14`).
pub const CTOC_EFU_COMPRESSPARAM9: u16 = 0x14;

/// One `<CTOC-table-entry>` indexing a single compound-file element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtocEntry {
    /// Byte offset of the element from the start of the CGRP data
    /// portion (after the CGRP chunk's 8-byte header — see the spec's
    /// worked `+8` example).
    pub offset: u32,
    /// Size of the element in bytes.
    pub size: u32,
    /// FOURCC media element type (`0` if the element is not a standalone
    /// file; equals the RIFF form type when the element is a RIFF form).
    pub med_type: u32,
    /// Extra usage info; a FOURCC when [`CTOC_HF_MEDSUBTYPE`] is set in
    /// the header flags, otherwise form-type-defined.
    pub med_usage: u32,
    /// FOURCC compression-technique id (`0` = uncompressed).
    pub compress_tech: u32,
    /// In-memory size after decompression (equals `size` when
    /// uncompressed).
    pub uncompress_bytes: u32,
    /// The `adwExEntField` extra-field DWORDs (length = `wExEntFields`).
    pub ex_ent_fields: Vec<u32>,
    /// `bEntryFlags` ([`CTOC_EF_DELETED`] / [`CTOC_EF_UNUSED`]).
    pub flags: u8,
    /// The `achName` field verbatim, including its NUL padding/terminator
    /// (length = `wNameSize`).
    pub name: Vec<u8>,
}

impl CtocEntry {
    /// The element name as a `&str`, trimmed at the first NUL.
    /// Returns `None` if the bytes before the first NUL are not UTF-8.
    pub fn name_str(&self) -> Option<&str> {
        let end = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..end]).ok()
    }
}

/// A fully decoded `CTOC` chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtocChunk {
    /// `dwHeaderSize` — combined size of the header-information,
    /// parameter-table-definition, and header-parameter-table sections;
    /// locates the first table entry.
    pub header_size: u32,
    /// `dwEntriesTotal` — total entries, including unused + deleted.
    pub entries_total: u32,
    /// `dwEntriesDeleted`.
    pub entries_deleted: u32,
    /// `dwEntriesUnused`.
    pub entries_unused: u32,
    /// `dwBytesTotal` — combined size of all CGRP elements incl.
    /// deleted.
    pub bytes_total: u32,
    /// `dwBytesDeleted`.
    pub bytes_deleted: u32,
    /// `dwHeaderFlags` ([`CTOC_HF_SEQUENTIAL`] / [`CTOC_HF_MEDSUBTYPE`]).
    pub header_flags: u32,
    /// `wEntrySize` — size of each table entry including pad bytes.
    pub entry_size: u16,
    /// `wNameSize` — width of each entry's `achName` field.
    pub name_size: u16,
    /// `awExHdrFldUsage` — usage code per extra-header field
    /// (length = `wExHdrFields`).
    pub ex_hdr_usage: Vec<u16>,
    /// `awExEntFldUsage` — usage code per extra-entry field
    /// (length = `wExEntFields`).
    pub ex_ent_usage: Vec<u16>,
    /// `adwExHdrField` — the header parameter table
    /// (length = `wExHdrFields`).
    pub ex_hdr_fields: Vec<u32>,
    /// `bHeaderPad` — NUL bytes after the header parameter table that
    /// pad the CTOC header (offsets `header_size`) to an even length,
    /// preserved verbatim.
    pub header_pad: Vec<u8>,
    /// The decoded table entries (length = `entries_total`).
    pub entries: Vec<CtocEntry>,
}

fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn rd_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

impl CtocChunk {
    /// Parse a `CTOC` chunk body (everything after its 8-byte chunk
    /// header).
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < CTOC_HEADER_INFO_LEN + CTOC_PARAM_DEF_LEN {
            return Err(Error::invalid(
                "CTOC: body too short for header + parameter-table definition",
            ));
        }
        let header_size = rd_u32(body, 0);
        let entries_total = rd_u32(body, 4);
        let entries_deleted = rd_u32(body, 8);
        let entries_unused = rd_u32(body, 12);
        let bytes_total = rd_u32(body, 16);
        let bytes_deleted = rd_u32(body, 20);
        let header_flags = rd_u32(body, 24);

        let mut pos = CTOC_HEADER_INFO_LEN;
        let entry_size = rd_u16(body, pos);
        let name_size = rd_u16(body, pos + 2);
        let ex_hdr_fields_n = rd_u16(body, pos + 4) as usize;
        let ex_ent_fields_n = rd_u16(body, pos + 6) as usize;
        pos += CTOC_PARAM_DEF_LEN;

        // awExHdrFldUsage[wExHdrFields] (WORDs)
        let need_usage = ex_hdr_fields_n
            .checked_add(ex_ent_fields_n)
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| Error::invalid("CTOC: usage-array size overflow"))?;
        if pos + need_usage > body.len() {
            return Err(Error::invalid(
                "CTOC: body truncated inside the usage arrays",
            ));
        }
        let mut ex_hdr_usage = Vec::with_capacity(ex_hdr_fields_n);
        for _ in 0..ex_hdr_fields_n {
            ex_hdr_usage.push(rd_u16(body, pos));
            pos += 2;
        }
        let mut ex_ent_usage = Vec::with_capacity(ex_ent_fields_n);
        for _ in 0..ex_ent_fields_n {
            ex_ent_usage.push(rd_u16(body, pos));
            pos += 2;
        }

        // adwExHdrField[wExHdrFields] (DWORDs)
        let need_hdr = ex_hdr_fields_n
            .checked_mul(4)
            .ok_or_else(|| Error::invalid("CTOC: header-field size overflow"))?;
        if pos + need_hdr > body.len() {
            return Err(Error::invalid(
                "CTOC: body truncated inside the header parameter table",
            ));
        }
        let mut ex_hdr_fields = Vec::with_capacity(ex_hdr_fields_n);
        for _ in 0..ex_hdr_fields_n {
            ex_hdr_fields.push(rd_u32(body, pos));
            pos += 4;
        }

        // bHeaderPad: everything between here and dwHeaderSize.
        let header_size_usize = header_size as usize;
        if header_size_usize < pos || header_size_usize > body.len() {
            return Err(Error::invalid(
                "CTOC: dwHeaderSize is inconsistent with the parsed header layout",
            ));
        }
        let header_pad = body[pos..header_size_usize].to_vec();
        pos = header_size_usize;

        // CTOC table entries.
        let entry_size_usize = entry_size as usize;
        let name_size_usize = name_size as usize;
        // Minimum entry footprint the spec mandates: 6 DWORDs + the
        // extra-entry DWORDs + 1 flag byte + wNameSize name bytes.
        let min_entry = 24 + ex_ent_fields_n * 4 + 1 + name_size_usize;
        if entry_size_usize < min_entry {
            return Err(Error::invalid(
                "CTOC: wEntrySize smaller than the mandatory entry fields",
            ));
        }
        let mut entries = Vec::with_capacity(entries_total as usize);
        for _ in 0..entries_total {
            if pos + entry_size_usize > body.len() {
                return Err(Error::invalid(
                    "CTOC: body truncated inside the table entries",
                ));
            }
            let e = &body[pos..pos + entry_size_usize];
            let mut ep = 0usize;
            let offset = rd_u32(e, ep);
            let size = rd_u32(e, ep + 4);
            let med_type = rd_u32(e, ep + 8);
            let med_usage = rd_u32(e, ep + 12);
            let compress_tech = rd_u32(e, ep + 16);
            let uncompress_bytes = rd_u32(e, ep + 20);
            ep += 24;
            let mut ex_ent_fields = Vec::with_capacity(ex_ent_fields_n);
            for _ in 0..ex_ent_fields_n {
                ex_ent_fields.push(rd_u32(e, ep));
                ep += 4;
            }
            let flags = e[ep];
            ep += 1;
            let name = e[ep..ep + name_size_usize].to_vec();
            entries.push(CtocEntry {
                offset,
                size,
                med_type,
                med_usage,
                compress_tech,
                uncompress_bytes,
                ex_ent_fields,
                flags,
                name,
            });
            pos += entry_size_usize;
        }

        Ok(CtocChunk {
            header_size,
            entries_total,
            entries_deleted,
            entries_unused,
            bytes_total,
            bytes_deleted,
            header_flags,
            entry_size,
            name_size,
            ex_hdr_usage,
            ex_ent_usage,
            ex_hdr_fields,
            header_pad,
            entries,
        })
    }

    /// Serialise the chunk body (the bytes after the 8-byte header).
    ///
    /// The byte-exact inverse of [`CtocChunk::parse`]: for a chunk
    /// produced by `parse` the output equals the original body. Validates
    /// internal consistency (the usage arrays match the entries' extra
    /// fields, the name lengths match `wNameSize`, the entries serialise
    /// to exactly `wEntrySize`).
    pub fn encode_body(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.header_size.to_le_bytes());
        out.extend_from_slice(&self.entries_total.to_le_bytes());
        out.extend_from_slice(&self.entries_deleted.to_le_bytes());
        out.extend_from_slice(&self.entries_unused.to_le_bytes());
        out.extend_from_slice(&self.bytes_total.to_le_bytes());
        out.extend_from_slice(&self.bytes_deleted.to_le_bytes());
        out.extend_from_slice(&self.header_flags.to_le_bytes());

        out.extend_from_slice(&self.entry_size.to_le_bytes());
        out.extend_from_slice(&self.name_size.to_le_bytes());
        let ex_hdr_n = self.ex_hdr_usage.len();
        let ex_ent_n = self.ex_ent_usage.len();
        if ex_hdr_n != self.ex_hdr_fields.len() {
            return Err(Error::invalid(
                "CTOC: awExHdrFldUsage length != adwExHdrField length",
            ));
        }
        out.extend_from_slice(&(ex_hdr_n as u16).to_le_bytes());
        out.extend_from_slice(&(ex_ent_n as u16).to_le_bytes());
        for u in &self.ex_hdr_usage {
            out.extend_from_slice(&u.to_le_bytes());
        }
        for u in &self.ex_ent_usage {
            out.extend_from_slice(&u.to_le_bytes());
        }
        for f in &self.ex_hdr_fields {
            out.extend_from_slice(&f.to_le_bytes());
        }
        // bHeaderPad → reach dwHeaderSize.
        let written = out.len();
        let header_size_usize = self.header_size as usize;
        if header_size_usize < written {
            return Err(Error::invalid(
                "CTOC: dwHeaderSize smaller than the encoded header fields",
            ));
        }
        if header_size_usize - written != self.header_pad.len() {
            return Err(Error::invalid(
                "CTOC: header_pad length inconsistent with dwHeaderSize",
            ));
        }
        out.extend_from_slice(&self.header_pad);

        let entry_size_usize = self.entry_size as usize;
        let name_size_usize = self.name_size as usize;
        for e in &self.entries {
            if e.ex_ent_fields.len() != ex_ent_n {
                return Err(Error::invalid(
                    "CTOC: entry adwExEntField length != awExEntFldUsage length",
                ));
            }
            if e.name.len() != name_size_usize {
                return Err(Error::invalid("CTOC: entry achName length != wNameSize"));
            }
            let start = out.len();
            out.extend_from_slice(&e.offset.to_le_bytes());
            out.extend_from_slice(&e.size.to_le_bytes());
            out.extend_from_slice(&e.med_type.to_le_bytes());
            out.extend_from_slice(&e.med_usage.to_le_bytes());
            out.extend_from_slice(&e.compress_tech.to_le_bytes());
            out.extend_from_slice(&e.uncompress_bytes.to_le_bytes());
            for f in &e.ex_ent_fields {
                out.extend_from_slice(&f.to_le_bytes());
            }
            out.push(e.flags);
            out.extend_from_slice(&e.name);
            let body_len = out.len() - start;
            if body_len > entry_size_usize {
                return Err(Error::invalid("CTOC: encoded entry exceeds wEntrySize"));
            }
            // bEntryPad → reach wEntrySize.
            out.resize(start + entry_size_usize, 0);
        }
        Ok(out)
    }
}

/// `true` if `id` is a compound-file structural FourCC (`CTOC` / `CGRP`).
pub const fn is_compound_fourcc(id: &[u8; 4]) -> bool {
    matches!(*id, FOURCC_CTOC | FOURCC_CGRP)
}

/// Locate a compound-file element's bytes inside a `CGRP` chunk body.
///
/// `cgrp_body` is the payload of the `CGRP` chunk (everything after its
/// 8-byte header). The entry's `offset` is measured "from the beginning
/// of the data portion of the CGRP chunk" — i.e. relative to
/// `cgrp_body`'s first byte. Returns the `entry.size` bytes of the
/// element, or an error if the entry is marked deleted/unused or its
/// span overruns the CGRP body.
pub fn element_bytes<'a>(cgrp_body: &'a [u8], entry: &CtocEntry) -> Result<&'a [u8]> {
    if entry.flags & (CTOC_EF_DELETED | CTOC_EF_UNUSED) != 0 {
        return Err(Error::invalid("CTOC: element is marked deleted or unused"));
    }
    let start = entry.offset as usize;
    let end = start
        .checked_add(entry.size as usize)
        .ok_or_else(|| Error::invalid("CTOC: element span overflows usize"))?;
    if end > cgrp_body.len() {
        return Err(Error::invalid("CTOC: element overruns the CGRP body"));
    }
    Ok(&cgrp_body[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but complete CTOC body: no extra fields, two
    /// entries (`wNameSize = 4`, `wEntrySize = 24 + 0 + 1 + 4 = 29 → 30`
    /// even).
    fn sample_ctoc() -> CtocChunk {
        let name = |s: &[u8]| {
            let mut v = s.to_vec();
            v.resize(4, 0);
            v
        };
        // header = 7*4 (info) + 8 (param def) + 0 usage + 0 hdr fields = 36,
        // already even so header_pad empty.
        CtocChunk {
            header_size: 36,
            entries_total: 2,
            entries_deleted: 0,
            entries_unused: 0,
            bytes_total: 30,
            bytes_deleted: 0,
            header_flags: CTOC_HF_SEQUENTIAL,
            entry_size: 30,
            name_size: 4,
            ex_hdr_usage: vec![],
            ex_ent_usage: vec![],
            ex_hdr_fields: vec![],
            header_pad: vec![],
            entries: vec![
                CtocEntry {
                    offset: 0,
                    size: 10,
                    med_type: u32::from_le_bytes(*b"WAVE"),
                    med_usage: 0,
                    compress_tech: 0,
                    uncompress_bytes: 10,
                    ex_ent_fields: vec![],
                    flags: 0,
                    name: name(b"a"),
                },
                CtocEntry {
                    offset: 10,
                    size: 20,
                    med_type: u32::from_le_bytes(*b"RDIB"),
                    med_usage: 0,
                    compress_tech: 0,
                    uncompress_bytes: 20,
                    ex_ent_fields: vec![],
                    flags: 0,
                    name: name(b"bb"),
                },
            ],
        }
    }

    #[test]
    fn round_trip_is_byte_exact() {
        let ctoc = sample_ctoc();
        let body = ctoc.encode_body().unwrap();
        let parsed = CtocChunk::parse(&body).unwrap();
        assert_eq!(parsed, ctoc);
        assert_eq!(parsed.encode_body().unwrap(), body);
    }

    #[test]
    fn header_size_locates_first_entry() {
        let ctoc = sample_ctoc();
        let body = ctoc.encode_body().unwrap();
        // First entry begins exactly at header_size.
        assert_eq!(&body[36..40], &0u32.to_le_bytes()); // entry[0].offset
        assert_eq!(body.len(), 36 + 2 * 30);
    }

    #[test]
    fn entry_name_str_trims_at_nul() {
        let ctoc = sample_ctoc();
        assert_eq!(ctoc.entries[0].name_str(), Some("a"));
        assert_eq!(ctoc.entries[1].name_str(), Some("bb"));
    }

    #[test]
    fn extra_fields_round_trip() {
        let mut ctoc = sample_ctoc();
        // Add one extra header field + one extra entry field.
        ctoc.ex_hdr_usage = vec![CTOC_EFU_LASTMODTIME];
        ctoc.ex_hdr_fields = vec![0x1234_5678];
        ctoc.ex_ent_usage = vec![CTOC_EFU_CODEPAGE];
        // header = 36 base + 2 (ex_hdr_usage) + 2 (ex_ent_usage)
        //   + 4 (ex_hdr_field) = 44, already even.
        ctoc.header_size = 44;
        // entry grows by 4 (one extra DWORD) → min 28+1+4 = 33 → pad to 34.
        ctoc.entry_size = 34;
        for e in &mut ctoc.entries {
            e.ex_ent_fields = vec![0xAABB_CCDD];
        }
        let body = ctoc.encode_body().unwrap();
        let parsed = CtocChunk::parse(&body).unwrap();
        assert_eq!(parsed, ctoc);
        assert_eq!(parsed.ex_hdr_usage, vec![CTOC_EFU_LASTMODTIME]);
        assert_eq!(parsed.entries[0].ex_ent_fields, vec![0xAABB_CCDD]);
    }

    #[test]
    fn header_pad_is_preserved() {
        let mut ctoc = sample_ctoc();
        // Force an odd-length header that needs a pad byte.
        ctoc.ex_hdr_usage = vec![CTOC_EFU_UNUSED]; // +2
        ctoc.ex_hdr_fields = vec![0]; // +4
        ctoc.header_pad = vec![0, 0]; // arbitrary even-up pad
        ctoc.header_size = 36 + 2 + 4 + 2;
        let body = ctoc.encode_body().unwrap();
        let parsed = CtocChunk::parse(&body).unwrap();
        assert_eq!(parsed.header_pad, vec![0, 0]);
    }

    #[test]
    fn element_bytes_resolves_offset_and_size() {
        let ctoc = sample_ctoc();
        let cgrp = (0u8..30).collect::<Vec<_>>();
        let a = element_bytes(&cgrp, &ctoc.entries[0]).unwrap();
        assert_eq!(a, &cgrp[0..10]);
        let b = element_bytes(&cgrp, &ctoc.entries[1]).unwrap();
        assert_eq!(b, &cgrp[10..30]);
    }

    #[test]
    fn element_bytes_rejects_deleted_entry() {
        let mut ctoc = sample_ctoc();
        ctoc.entries[0].flags = CTOC_EF_DELETED;
        let cgrp = vec![0u8; 30];
        let err = element_bytes(&cgrp, &ctoc.entries[0]).unwrap_err();
        assert!(format!("{err}").contains("deleted"));
    }

    #[test]
    fn element_bytes_rejects_overrun() {
        let ctoc = sample_ctoc();
        let cgrp = vec![0u8; 5]; // too small for the 10-byte element
        let err = element_bytes(&cgrp, &ctoc.entries[0]).unwrap_err();
        assert!(format!("{err}").contains("overruns"));
    }

    #[test]
    fn parse_rejects_short_body() {
        let err = CtocChunk::parse(&[0u8; 10]).unwrap_err();
        assert!(format!("{err}").contains("too short"));
    }

    #[test]
    fn parse_rejects_inconsistent_header_size() {
        let ctoc = sample_ctoc();
        let mut body = ctoc.encode_body().unwrap();
        // Corrupt dwHeaderSize to point past the body.
        body[0..4].copy_from_slice(&9999u32.to_le_bytes());
        let err = CtocChunk::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("dwHeaderSize"));
    }

    #[test]
    fn is_compound_fourcc_matches_ctoc_and_cgrp() {
        assert!(is_compound_fourcc(b"CTOC"));
        assert!(is_compound_fourcc(b"CGRP"));
        assert!(!is_compound_fourcc(b"data"));
    }

    #[test]
    fn usage_code_constants_match_spec() {
        // §2 "Usage Codes" literal values.
        assert_eq!(CTOC_EFU_UNUSED, 0x00);
        assert_eq!(CTOC_EFU_LASTMODTIME, 0x01);
        assert_eq!(CTOC_EFU_CODEPAGE, 0x02);
        assert_eq!(CTOC_EFU_LANGUAGE, 0x03);
        assert_eq!(CTOC_EFU_COMPRESSPARAM0, 0x05);
        assert_eq!(CTOC_EFU_COMPRESSPARAM9, 0x14);
    }
}
