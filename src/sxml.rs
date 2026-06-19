//! Typed decoders for the BW64 ADM **XML-carrier** chunks — `axml`,
//! `bxml`, and `sxml`.
//!
//! A BW64 (ADM-carrying) WAV file pairs the binary `chna` channel-
//! allocation table (decoded in [`crate::chna`]) with the *Audio
//! Definition Model* metadata document itself, which travels in one of
//! three XML-carrier chunks:
//!
//! - **`axml`** (§5) — a header followed by uncompressed XML text.
//! - **`bxml`** (§6) — a header carrying a 2-byte `fmtType` compression
//!   selector followed by the XML compressed by that method.
//! - **`sxml`** (§7) — a *structured* carrier: a `fmtType` selector and
//!   a 64-bit table size, then an array of `SubXMLChunk` records (each
//!   binding a span of audio samples to its own XML payload), then an
//!   optional `AlignmentPoint` table giving sample-accurate seek points
//!   into those sub-chunks. Used for time-variant metadata (a serial
//!   ADM per Rec. ITU-R BS.2125).
//!
//! The `axml` and `bxml` bodies are opaque payloads — this decoder
//! exposes the framing (`fmtType` for `bxml`) and hands the XML bytes
//! back verbatim, since interpreting the XML / decompressing the gzip
//! stream is the caller's concern, above the container layer. The
//! `sxml` body has real internal structure, which this module walks
//! into typed [`SubXmlChunk`] / [`AlignmentPoint`] records.
//!
//! Every multi-byte field is little-endian.
//!
//! ## Compression selector (`fmtType`)
//!
//! `fmtType` is a 2-byte value shared by `bxml` and `sxml`:
//! `0x0000` = uncompressed XML, `0x0001` = gzip (IETF RFC 1952). Other
//! values are reserved; the raw `u16` is preserved so an unrecognised
//! method is never silently dropped.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/R-REC-BS.2088.pdf` §5 (`axml`), §6
//!   (`bxml`), §7 (`sxml`) — the `struct axml_chunk` / `bxml_chunk` /
//!   `sxml_chunk` layouts, the `SubXMLChunk` and `AlignmentPoint`
//!   record definitions, the `subXMLCkTbSize` 64-bit table-size field
//!   (which counts the 4-byte `nSubXMLChunks` field plus the
//!   `SubXMLChunk` table), and the §7.2 element tables.
//! - `docs/container/riff/metadata/R-REC-BS.2088.pdf` §2-§4 +
//!   `docs/container/riff/metadata/bs2088-chna-chunk-layout.md` — the
//!   surrounding BW64 chunk model (little-endian, word-aligned
//!   subchunks) these XML-carrier chunks sit inside.

use crate::error::{Error, Result};

/// FourCC of the `axml` chunk — uncompressed ADM XML.
pub const FOURCC_AXML: [u8; 4] = *b"axml";

/// FourCC of the `bxml` chunk — compressed ADM XML (per `fmtType`).
pub const FOURCC_BXML: [u8; 4] = *b"bxml";

/// FourCC of the `sxml` chunk — structured, time-segmented ADM XML.
pub const FOURCC_SXML: [u8; 4] = *b"sxml";

/// `fmtType` value for uncompressed XML data (`0x0000`).
pub const FMT_TYPE_UNCOMPRESSED: u16 = 0x0000;

/// `fmtType` value for gzip-compressed XML data (`0x0001`, IETF
/// RFC 1952).
pub const FMT_TYPE_GZIP: u16 = 0x0001;

/// On-wire size of one `sxml` `SubXMLChunk` fixed header: the 4-byte
/// `subXMLChunkSize` plus the 4-byte `nSamplesSubDataChunk`, before the
/// variable `xmlData` payload.
pub const SUB_XML_HEADER_LEN: usize = 8;

/// On-wire size of one `sxml` `AlignmentPoint` record: two 64-bit
/// little-endian values (byte offset + sample count), each split into a
/// low / high 32-bit half.
pub const ALIGNMENT_POINT_LEN: usize = 16;

/// Size of the `sxml` body prefix preceding the `SubXMLChunk` table:
/// the 2-byte `fmtType` plus the 64-bit `subXMLCkTbSize`
/// (low + high halves).
const SXML_PREFIX_LEN: usize = 2 + 8;

/// A decoded `axml` chunk — uncompressed ADM XML.
///
/// The XML bytes are kept verbatim. The chunk has no internal framing
/// beyond the RIFF header, so any non-empty body is accepted; an empty
/// body is permitted (it round-trips to an empty document).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AxmlChunk {
    /// The raw XML text bytes (UTF-8 per XML 1.0, but kept as bytes so a
    /// declared non-UTF-8 encoding is preserved for the caller).
    pub xml: Vec<u8>,
}

impl AxmlChunk {
    /// Decode an `axml` chunk body (already pad-stripped by the walker).
    /// The whole body is the XML payload.
    pub fn parse(body: &[u8]) -> Self {
        AxmlChunk { xml: body.to_vec() }
    }

    /// The XML payload as a `&str`, if it is valid UTF-8.
    pub fn xml_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.xml).ok()
    }
}

/// A decoded `bxml` chunk — compressed ADM XML.
///
/// The 2-byte `fmtType` selects the compression method; the remaining
/// body is the compressed payload, handed back verbatim. This decoder
/// does **not** decompress — the gzip / other stream is interpreted by
/// the caller above the container layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BxmlChunk {
    /// Compression method selector (`0x0000` uncompressed, `0x0001`
    /// gzip; other values reserved, preserved verbatim).
    pub fmt_type: u16,
    /// The (possibly compressed) XML payload bytes.
    pub xml: Vec<u8>,
}

impl BxmlChunk {
    /// Decode a `bxml` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be at least 2 bytes (the `fmtType`); a shorter body
    /// is an `Error::invalid`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 2 {
            return Err(Error::invalid(
                "RIFF: bxml chunk body is shorter than the 2-byte fmtType",
            ));
        }
        let fmt_type = u16::from_le_bytes([body[0], body[1]]);
        Ok(BxmlChunk {
            fmt_type,
            xml: body[2..].to_vec(),
        })
    }

    /// `true` if `fmt_type` is the gzip selector (`0x0001`).
    pub fn is_gzip(&self) -> bool {
        self.fmt_type == FMT_TYPE_GZIP
    }

    /// `true` if `fmt_type` marks the payload as uncompressed (`0x0000`).
    pub fn is_uncompressed(&self) -> bool {
        self.fmt_type == FMT_TYPE_UNCOMPRESSED
    }
}

/// One `SubXMLChunk` record from an `sxml` chunk: an XML payload bound
/// to a span of audio samples.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubXmlChunk {
    /// `nSamplesSubDataChunk` — number of audio samples per channel
    /// associated with this sub-chunk. Consecutive sub-chunks describe
    /// contiguous, non-overlapping sample spans.
    pub n_samples: u32,
    /// The XML payload for this span, compressed per the parent chunk's
    /// `fmt_type`. Length equals the record's `subXMLChunkSize` minus
    /// the 4-byte `nSamplesSubDataChunk` field; preserved verbatim.
    pub xml: Vec<u8>,
}

/// One `AlignmentPoint` record from an `sxml` chunk: a sample-accurate
/// seek point binding a byte offset into the `sxml` body to a timeline
/// sample count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlignmentPoint {
    /// 64-bit byte offset of a `SubXMLChunk` from the start of the
    /// `sxml` body (i.e. excluding the 8-byte `ckID` + `ckSize`).
    pub byte_offset: u64,
    /// 64-bit timeline position of the alignment point, in audio samples
    /// per channel from the start of the `data` chunk.
    pub n_samples: u64,
}

/// A decoded `sxml` chunk — structured, time-segmented ADM XML.
///
/// The fixed prefix carries the `fmt_type` compression selector and the
/// 64-bit `subXMLCkTbSize` (the size of the `nSubXMLChunks` count field
/// plus the `SubXMLChunk` table). After the table comes an optional
/// `AlignmentPoint` table (a count plus the records).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SxmlChunk {
    /// Compression method selector applied to every sub-chunk's
    /// `xmlData` (`0x0000` uncompressed, `0x0001` gzip; preserved
    /// verbatim).
    pub fmt_type: u16,
    /// The `SubXMLChunk` table — one record per contiguous sample span.
    pub sub_chunks: Vec<SubXmlChunk>,
    /// The optional `AlignmentPoint` table — sample-accurate seek points
    /// into the sub-chunks (empty when `nAlignmentPoints == 0`).
    pub alignment_points: Vec<AlignmentPoint>,
}

impl SxmlChunk {
    /// Decode an `sxml` chunk body (already pad-stripped by the walker).
    ///
    /// Rejects (with `Error::invalid`) any body that is shorter than the
    /// fixed prefix, whose `subXMLCkTbSize` overruns the body, whose
    /// `SubXMLChunk` table is internally inconsistent (a record size that
    /// overruns the declared table region), or whose `AlignmentPoint`
    /// table count disagrees with the bytes remaining.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < SXML_PREFIX_LEN + 4 {
            // prefix (10) + at least the nSubXMLChunks count (4)
            return Err(Error::invalid(
                "RIFF: sxml chunk body is too short for its fixed prefix",
            ));
        }
        let fmt_type = u16::from_le_bytes([body[0], body[1]]);
        let tb_size_low = u32::from_le_bytes([body[2], body[3], body[4], body[5]]);
        let tb_size_high = u32::from_le_bytes([body[6], body[7], body[8], body[9]]);
        let tb_size = (u64::from(tb_size_high) << 32) | u64::from(tb_size_low);
        let tb_size = usize::try_from(tb_size).map_err(|_| {
            Error::invalid("RIFF: sxml subXMLCkTbSize exceeds the addressable range")
        })?;

        // The table region begins at SXML_PREFIX_LEN and spans tb_size
        // bytes: the 4-byte nSubXMLChunks field plus the SubXMLChunk
        // table itself.
        if tb_size < 4 {
            return Err(Error::invalid(
                "RIFF: sxml subXMLCkTbSize is smaller than its own nSubXMLChunks field",
            ));
        }
        let table_end = SXML_PREFIX_LEN
            .checked_add(tb_size)
            .ok_or_else(|| Error::invalid("RIFF: sxml table region size overflow"))?;
        if table_end > body.len() {
            return Err(Error::invalid(
                "RIFF: sxml subXMLCkTbSize overruns the chunk body",
            ));
        }

        let n_sub = u32::from_le_bytes([
            body[SXML_PREFIX_LEN],
            body[SXML_PREFIX_LEN + 1],
            body[SXML_PREFIX_LEN + 2],
            body[SXML_PREFIX_LEN + 3],
        ]) as usize;

        let mut sub_chunks = Vec::with_capacity(n_sub.min(1024));
        let mut pos = SXML_PREFIX_LEN + 4;
        for _ in 0..n_sub {
            if pos + SUB_XML_HEADER_LEN > table_end {
                return Err(Error::invalid(
                    "RIFF: sxml SubXMLChunk header overruns the declared table region",
                ));
            }
            // subXMLChunkSize counts the data section AND the 4-byte
            // nSamplesSubDataChunk field but NOT subXMLChunkSize itself.
            let sub_size =
                u32::from_le_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]])
                    as usize;
            if sub_size < 4 {
                return Err(Error::invalid(
                    "RIFF: sxml SubXMLChunk size is smaller than its nSamplesSubDataChunk field",
                ));
            }
            let n_samples =
                u32::from_le_bytes([body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]]);
            let xml_len = sub_size - 4;
            let xml_start = pos + SUB_XML_HEADER_LEN;
            let xml_end = xml_start
                .checked_add(xml_len)
                .ok_or_else(|| Error::invalid("RIFF: sxml SubXMLChunk payload size overflow"))?;
            if xml_end > table_end {
                return Err(Error::invalid(
                    "RIFF: sxml SubXMLChunk payload overruns the declared table region",
                ));
            }
            sub_chunks.push(SubXmlChunk {
                n_samples,
                xml: body[xml_start..xml_end].to_vec(),
            });
            pos = xml_end;
        }

        // After the table region comes the optional AlignmentPoint table:
        // a 4-byte count followed by ALIGNMENT_POINT_LEN-byte records.
        let mut alignment_points = Vec::new();
        if table_end < body.len() {
            if table_end + 4 > body.len() {
                return Err(Error::invalid(
                    "RIFF: sxml trailing bytes too short for the nAlignmentPoints count",
                ));
            }
            let n_align = u32::from_le_bytes([
                body[table_end],
                body[table_end + 1],
                body[table_end + 2],
                body[table_end + 3],
            ]) as usize;
            let align_start = table_end + 4;
            let needed = n_align
                .checked_mul(ALIGNMENT_POINT_LEN)
                .ok_or_else(|| Error::invalid("RIFF: sxml alignment-point table size overflow"))?;
            let align_end = align_start
                .checked_add(needed)
                .ok_or_else(|| Error::invalid("RIFF: sxml alignment-point table size overflow"))?;
            if align_end != body.len() {
                return Err(Error::invalid(
                    "RIFF: sxml nAlignmentPoints disagrees with the remaining body length",
                ));
            }
            alignment_points.reserve(n_align);
            let mut ap = align_start;
            for _ in 0..n_align {
                let off_low =
                    u32::from_le_bytes([body[ap], body[ap + 1], body[ap + 2], body[ap + 3]]);
                let off_high =
                    u32::from_le_bytes([body[ap + 4], body[ap + 5], body[ap + 6], body[ap + 7]]);
                let s_low =
                    u32::from_le_bytes([body[ap + 8], body[ap + 9], body[ap + 10], body[ap + 11]]);
                let s_high = u32::from_le_bytes([
                    body[ap + 12],
                    body[ap + 13],
                    body[ap + 14],
                    body[ap + 15],
                ]);
                alignment_points.push(AlignmentPoint {
                    byte_offset: (u64::from(off_high) << 32) | u64::from(off_low),
                    n_samples: (u64::from(s_high) << 32) | u64::from(s_low),
                });
                ap += ALIGNMENT_POINT_LEN;
            }
        }

        Ok(SxmlChunk {
            fmt_type,
            sub_chunks,
            alignment_points,
        })
    }

    /// `true` if `fmt_type` is the gzip selector (`0x0001`).
    pub fn is_gzip(&self) -> bool {
        self.fmt_type == FMT_TYPE_GZIP
    }

    /// `true` if `fmt_type` marks the sub-chunk payloads as uncompressed
    /// (`0x0000`).
    pub fn is_uncompressed(&self) -> bool {
        self.fmt_type == FMT_TYPE_UNCOMPRESSED
    }

    /// Total number of audio samples (per channel) spanned by the
    /// `SubXMLChunk` table — the sum of each record's `n_samples`.
    pub fn total_samples(&self) -> u64 {
        self.sub_chunks.iter().map(|s| u64::from(s.n_samples)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourccs_and_consts_match_spec() {
        assert_eq!(&FOURCC_AXML, b"axml");
        assert_eq!(&FOURCC_BXML, b"bxml");
        assert_eq!(&FOURCC_SXML, b"sxml");
        assert_eq!(FMT_TYPE_UNCOMPRESSED, 0x0000);
        assert_eq!(FMT_TYPE_GZIP, 0x0001);
        assert_eq!(SUB_XML_HEADER_LEN, 8);
        assert_eq!(ALIGNMENT_POINT_LEN, 16);
    }

    #[test]
    fn axml_keeps_xml_verbatim() {
        let xml = b"<audioFormatExtended></audioFormatExtended>";
        let c = AxmlChunk::parse(xml);
        assert_eq!(c.xml, xml);
        assert_eq!(
            c.xml_str(),
            Some("<audioFormatExtended></audioFormatExtended>")
        );
    }

    #[test]
    fn axml_empty_body_is_empty_document() {
        let c = AxmlChunk::parse(&[]);
        assert!(c.xml.is_empty());
        assert_eq!(c.xml_str(), Some(""));
    }

    #[test]
    fn bxml_splits_fmt_type_from_payload() {
        let mut body = Vec::new();
        body.extend_from_slice(&FMT_TYPE_GZIP.to_le_bytes());
        body.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]); // gzip magic-ish
        let c = BxmlChunk::parse(&body).unwrap();
        assert_eq!(c.fmt_type, FMT_TYPE_GZIP);
        assert!(c.is_gzip());
        assert!(!c.is_uncompressed());
        assert_eq!(c.xml, &[0x1f, 0x8b, 0x08, 0x00]);
    }

    #[test]
    fn bxml_uncompressed_selector() {
        let mut body = Vec::new();
        body.extend_from_slice(&FMT_TYPE_UNCOMPRESSED.to_le_bytes());
        body.extend_from_slice(b"<adm/>");
        let c = BxmlChunk::parse(&body).unwrap();
        assert!(c.is_uncompressed());
        assert_eq!(c.xml, b"<adm/>");
    }

    #[test]
    fn bxml_too_short_is_rejected() {
        let err = BxmlChunk::parse(&[0x01]).unwrap_err();
        assert!(format!("{err}").contains("shorter than the 2-byte fmtType"));
    }

    /// Build one SubXMLChunk record: subXMLChunkSize (= 4 + xml.len),
    /// nSamplesSubDataChunk, then the xml payload.
    fn sub_record(n_samples: u32, xml: &[u8]) -> Vec<u8> {
        let mut r = Vec::new();
        let sub_size = (4 + xml.len()) as u32;
        r.extend_from_slice(&sub_size.to_le_bytes());
        r.extend_from_slice(&n_samples.to_le_bytes());
        r.extend_from_slice(xml);
        r
    }

    /// Assemble an sxml body: fmtType + 64-bit tbSize + nSubXMLChunks +
    /// records, then optionally an alignment-point count + records.
    fn sxml_body(fmt_type: u16, records: &[Vec<u8>], aligns: &[AlignmentPoint]) -> Vec<u8> {
        let mut table = Vec::new();
        table.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for r in records {
            table.extend_from_slice(r);
        }
        let mut body = Vec::new();
        body.extend_from_slice(&fmt_type.to_le_bytes());
        let tb_size = table.len() as u64;
        body.extend_from_slice(&(tb_size as u32).to_le_bytes());
        body.extend_from_slice(&((tb_size >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&table);
        if !aligns.is_empty() {
            body.extend_from_slice(&(aligns.len() as u32).to_le_bytes());
            for a in aligns {
                body.extend_from_slice(&(a.byte_offset as u32).to_le_bytes());
                body.extend_from_slice(&((a.byte_offset >> 32) as u32).to_le_bytes());
                body.extend_from_slice(&(a.n_samples as u32).to_le_bytes());
                body.extend_from_slice(&((a.n_samples >> 32) as u32).to_le_bytes());
            }
        }
        body
    }

    #[test]
    fn sxml_two_sub_chunks_no_alignment() {
        let recs = vec![sub_record(48_000, b"<a/>"), sub_record(24_000, b"<bb/>")];
        let body = sxml_body(FMT_TYPE_UNCOMPRESSED, &recs, &[]);
        let c = SxmlChunk::parse(&body).unwrap();
        assert!(c.is_uncompressed());
        assert_eq!(c.sub_chunks.len(), 2);
        assert_eq!(c.sub_chunks[0].n_samples, 48_000);
        assert_eq!(c.sub_chunks[0].xml, b"<a/>");
        assert_eq!(c.sub_chunks[1].n_samples, 24_000);
        assert_eq!(c.sub_chunks[1].xml, b"<bb/>");
        assert!(c.alignment_points.is_empty());
        assert_eq!(c.total_samples(), 72_000);
    }

    #[test]
    fn sxml_with_alignment_points() {
        let recs = vec![sub_record(100, b"<x/>")];
        let aligns = vec![
            AlignmentPoint {
                byte_offset: 14,
                n_samples: 0,
            },
            AlignmentPoint {
                byte_offset: 0x1_0000_0002,
                n_samples: 0x2_0000_0003,
            },
        ];
        let body = sxml_body(FMT_TYPE_GZIP, &recs, &aligns);
        let c = SxmlChunk::parse(&body).unwrap();
        assert!(c.is_gzip());
        assert_eq!(c.sub_chunks.len(), 1);
        assert_eq!(c.alignment_points.len(), 2);
        assert_eq!(c.alignment_points[0].byte_offset, 14);
        assert_eq!(c.alignment_points[1].byte_offset, 0x1_0000_0002);
        assert_eq!(c.alignment_points[1].n_samples, 0x2_0000_0003);
    }

    #[test]
    fn sxml_zero_sub_chunks() {
        let body = sxml_body(FMT_TYPE_UNCOMPRESSED, &[], &[]);
        let c = SxmlChunk::parse(&body).unwrap();
        assert!(c.sub_chunks.is_empty());
        assert!(c.alignment_points.is_empty());
        assert_eq!(c.total_samples(), 0);
    }

    #[test]
    fn sxml_too_short_prefix_rejected() {
        let err = SxmlChunk::parse(&[0u8; 8]).unwrap_err();
        assert!(format!("{err}").contains("too short for its fixed prefix"));
    }

    #[test]
    fn sxml_table_size_overruns_body_rejected() {
        // fmtType + tbSize claiming 0xFFFF bytes but a tiny body.
        let mut body = Vec::new();
        body.extend_from_slice(&FMT_TYPE_UNCOMPRESSED.to_le_bytes());
        body.extend_from_slice(&0xFFFFu32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // nSubXMLChunks = 0
        let err = SxmlChunk::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("overruns the chunk body"));
    }

    #[test]
    fn sxml_sub_record_overruns_table_rejected() {
        // One record claiming a payload bigger than the declared table.
        let mut table = Vec::new();
        table.extend_from_slice(&1u32.to_le_bytes()); // nSubXMLChunks = 1
        table.extend_from_slice(&0xFFu32.to_le_bytes()); // subXMLChunkSize huge
        table.extend_from_slice(&0u32.to_le_bytes()); // nSamples
        table.extend_from_slice(b"x"); // truncated payload
        let mut body = Vec::new();
        body.extend_from_slice(&FMT_TYPE_UNCOMPRESSED.to_le_bytes());
        let tb = table.len() as u64;
        body.extend_from_slice(&(tb as u32).to_le_bytes());
        body.extend_from_slice(&((tb >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&table);
        let err = SxmlChunk::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("overruns the declared table region"));
    }

    #[test]
    fn sxml_alignment_count_mismatch_rejected() {
        let recs = vec![sub_record(10, b"<y/>")];
        let mut body = sxml_body(FMT_TYPE_UNCOMPRESSED, &recs, &[]);
        // Append a bogus alignment count of 2 but no records.
        body.extend_from_slice(&2u32.to_le_bytes());
        let err = SxmlChunk::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with the remaining body length"));
    }
}
