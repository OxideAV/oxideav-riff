//! RIFF Bundle (`BND`) compound-file form.
//!
//! The 1991 spec (§3 "Bundle File Format") defines the Bundle form as
//! the canonical user of the [`crate::ctoc`] compound-file structure:
//!
//! ```text
//! <BND-file> -> RIFF('BND ' <CTOC-chunk> <CGRP-chunk>)
//! ```
//!
//! A `BND` file carries a series of standalone multimedia files (each
//! compound-file element "must be capable of standing alone as an
//! independent file") concatenated into the `CGRP` chunk, with the
//! `CTOC` chunk indexing them. [`Bundle`] pairs the parsed [`CtocChunk`]
//! index with the raw `CGRP` body so a consumer can enumerate the
//! bundled elements and slice each one out by its table entry.
//!
//! This sits on top of the generic [`crate::tree::RiffTree`]: build the
//! tree, then [`Bundle::from_tree`] locates the `CTOC` + `CGRP` chunks
//! and decodes the index.

use crate::ctoc::{element_bytes, CtocChunk, CtocEntry, FOURCC_CGRP, FOURCC_CTOC};
use crate::error::{Error, Result};
use crate::tree::{RiffChunk, RiffTree};

/// Form type identifying a RIFF Bundle file (`'BND'` space-padded to a
/// 4-byte FourCC).
pub const FORM_BND: [u8; 4] = *b"BND ";

/// A parsed RIFF Bundle: the `CTOC` index plus the raw `CGRP` element
/// block it indexes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    /// The decoded `CTOC` table of contents.
    pub toc: CtocChunk,
    /// The raw `CGRP` chunk body (every bundled element, concatenated).
    pub group: Vec<u8>,
}

impl Bundle {
    /// Decode a Bundle from a parsed [`RiffTree`].
    ///
    /// Errors if the tree's form type is not `BND `, or if either the
    /// mandatory `CTOC` or `CGRP` chunk is missing, or if the `CTOC`
    /// body fails to decode. The `CGRP` body is taken verbatim (its
    /// elements are sliced lazily by [`Bundle::element`]).
    pub fn from_tree(tree: &RiffTree) -> Result<Self> {
        if tree.form_type != FORM_BND {
            return Err(Error::invalid("BND: tree form type is not 'BND '"));
        }
        let mut toc = None;
        let mut group = None;
        for child in &tree.children {
            if let RiffChunk::Leaf { id, body } = child {
                if *id == FOURCC_CTOC {
                    toc = Some(CtocChunk::parse(body)?);
                } else if *id == FOURCC_CGRP {
                    group = Some(body.clone());
                }
            }
        }
        let toc = toc.ok_or_else(|| Error::invalid("BND: missing mandatory 'CTOC' chunk"))?;
        let group = group.ok_or_else(|| Error::invalid("BND: missing mandatory 'CGRP' chunk"))?;
        Ok(Bundle { toc, group })
    }

    /// The bundled elements' table entries (including any deleted /
    /// unused slots; check [`CtocEntry::flags`]).
    pub fn entries(&self) -> &[CtocEntry] {
        &self.toc.entries
    }

    /// Slice the bytes of the element indexed by `entry` out of the
    /// `CGRP` body. Rejects deleted / unused entries and out-of-range
    /// spans (see [`crate::ctoc::element_bytes`]).
    pub fn element(&self, entry: &CtocEntry) -> Result<&[u8]> {
        element_bytes(&self.group, entry)
    }

    /// Iterate the *live* elements (skipping deleted / unused entries) as
    /// `(entry, bytes)` pairs. A malformed entry span surfaces as an
    /// error item.
    pub fn live_elements(&self) -> impl Iterator<Item = Result<(&CtocEntry, &[u8])>> {
        self.toc
            .entries
            .iter()
            .filter(|e| e.flags & (crate::ctoc::CTOC_EF_DELETED | crate::ctoc::CTOC_EF_UNUSED) == 0)
            .map(move |e| element_bytes(&self.group, e).map(|b| (e, b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::encode_chunk;
    use crate::ctoc::{CtocEntry, CTOC_EF_DELETED};

    /// Build a BND tree carrying two CGRP elements indexed by a CTOC.
    fn sample_bundle_bytes() -> Vec<u8> {
        // Two elements: "FIRST" (5 bytes) then "SECONDEL" (8 bytes).
        let el0 = b"FIRST".to_vec();
        let el1 = b"SECONDEL".to_vec();
        let mut cgrp = Vec::new();
        cgrp.extend_from_slice(&el0);
        cgrp.extend_from_slice(&el1);

        let name = |s: &[u8]| {
            let mut v = s.to_vec();
            v.resize(4, 0);
            v
        };
        let mk = |offset: u32, size: u32, nm: &[u8]| CtocEntry {
            offset,
            size,
            med_type: 0,
            med_usage: 0,
            compress_tech: 0,
            uncompress_bytes: size,
            ex_ent_fields: vec![],
            flags: 0,
            name: name(nm),
        };
        let toc = CtocChunk {
            header_size: 36,
            entries_total: 2,
            entries_deleted: 0,
            entries_unused: 0,
            bytes_total: cgrp.len() as u32,
            bytes_deleted: 0,
            header_flags: 0,
            entry_size: 30,
            name_size: 4,
            ex_hdr_usage: vec![],
            ex_ent_usage: vec![],
            ex_hdr_fields: vec![],
            header_pad: vec![],
            entries: vec![mk(0, 5, b"a"), mk(5, 8, b"b")],
        };

        let mut body = Vec::new();
        body.extend_from_slice(b"BND ");
        encode_chunk(&mut body, b"CTOC", &toc.encode_body().unwrap()).unwrap();
        encode_chunk(&mut body, b"CGRP", &cgrp).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        bytes
    }

    #[test]
    fn from_tree_decodes_ctoc_and_cgrp() {
        let bytes = sample_bundle_bytes();
        let tree = RiffTree::parse(&bytes).unwrap();
        let bundle = Bundle::from_tree(&tree).unwrap();
        assert_eq!(bundle.entries().len(), 2);
        assert_eq!(bundle.element(&bundle.toc.entries[0]).unwrap(), b"FIRST");
        assert_eq!(bundle.element(&bundle.toc.entries[1]).unwrap(), b"SECONDEL");
    }

    #[test]
    fn live_elements_skips_deleted() {
        let bytes = sample_bundle_bytes();
        let tree = RiffTree::parse(&bytes).unwrap();
        let mut bundle = Bundle::from_tree(&tree).unwrap();
        bundle.toc.entries[0].flags = CTOC_EF_DELETED;
        let live: Vec<_> = bundle.live_elements().map(|r| r.unwrap()).collect();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].1, b"SECONDEL");
    }

    #[test]
    fn rejects_non_bnd_form() {
        // RIFF/WAVE tree, not a bundle.
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"fmt ", &[0u8; 16]).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        let err = Bundle::from_tree(&tree).unwrap_err();
        assert!(format!("{err}").contains("not 'BND '"));
    }

    #[test]
    fn rejects_missing_cgrp() {
        // BND tree with CTOC but no CGRP.
        let toc = CtocChunk {
            header_size: 36,
            entries_total: 0,
            entries_deleted: 0,
            entries_unused: 0,
            bytes_total: 0,
            bytes_deleted: 0,
            header_flags: 0,
            entry_size: 30,
            name_size: 4,
            ex_hdr_usage: vec![],
            ex_ent_usage: vec![],
            ex_hdr_fields: vec![],
            header_pad: vec![],
            entries: vec![],
        };
        let mut body = Vec::new();
        body.extend_from_slice(b"BND ");
        encode_chunk(&mut body, b"CTOC", &toc.encode_body().unwrap()).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        let err = Bundle::from_tree(&tree).unwrap_err();
        assert!(format!("{err}").contains("missing mandatory 'CGRP'"));
    }
}
