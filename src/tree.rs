//! Recursive RIFF chunk-tree model.
//!
//! The [`crate::walk::Walker`] is **non-recursive** by design — it
//! yields the immediate children of one parent group chunk and leaves
//! the descent into nested `LIST` / `RIFF` sub-trees to the consumer.
//! That's the right primitive for a streaming demuxer that knows
//! exactly which group ckIDs it cares about and wants to skip the rest
//! without materialising them.
//!
//! For the *whole-file edit-and-rewrite* use-case — read a WAV / BWF /
//! AVI file into memory, mutate or reorder a metadata chunk, write it
//! back without dropping anything the editor didn't understand — the
//! more convenient shape is an **owned tree**: every chunk in the file
//! reified as a [`RiffChunk`] node, with the `RIFF` / `LIST` groups
//! holding their children recursively. [`RiffTree`] is that model.
//!
//! ## Grammar (§2 of the 1991 RIFF spec)
//!
//! The spec gives the recursive grammar directly:
//!
//! ```text
//! RIFF ( <formType> <ck>... )
//! LIST ( <listType> <ck>... )
//! <ck> -> <ckID> ( <ckData> )
//! ```
//!
//! A `RIFF` or `LIST` chunk's body is a 4-byte form-type / list-type
//! FourCC followed by zero or more child chunks; every other chunk is a
//! leaf carrying `ckSize` opaque payload bytes. [`RiffChunk::Group`]
//! captures the former, [`RiffChunk::Leaf`] the latter.
//!
//! ## Round-trip guarantee
//!
//! [`RiffTree::parse`] over a byte buffer followed by [`RiffTree::encode`]
//! reproduces the input **byte-for-byte** for any well-formed 32-bit
//! RIFF file (`RIFF` magic; the 64-bit `RF64` / `BW64` forms with a
//! `ds64` size table are handled by the [`crate::walk`] entry points and
//! are out of scope for this in-memory tree). Unknown chunk FourCCs are
//! preserved verbatim as leaves, so an editor that rewrites one chunk
//! never silently corrupts the chunks it didn't touch — the
//! "treat unknown chunks as opaque-but-preserved" rule from the metadata
//! spec's §"On using these specs".
//!
//! ```
//! use oxideav_riff::tree::RiffTree;
//!
//! // RIFF/WAVE { fmt(4) data(3, padded) }
//! let bytes: &[u8] = &[
//!     b'R', b'I', b'F', b'F', 0x1c, 0x00, 0x00, 0x00, // RIFF ckSize=28
//!     b'W', b'A', b'V', b'E',
//!     b'f', b'm', b't', b' ', 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00,
//!     b'd', b'a', b't', b'a', 0x03, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0x00,
//! ];
//! let tree = RiffTree::parse(bytes).unwrap();
//! assert_eq!(&tree.form_type, b"WAVE");
//! assert_eq!(tree.children.len(), 2);
//! // Byte-exact round-trip.
//! assert_eq!(tree.encode().unwrap(), bytes);
//! ```

use crate::chunk::{encode_chunk, write_chunk_header, FOURCC_LIST, FOURCC_RIFF};
use crate::error::{Error, Result};

/// Maximum nesting depth [`RiffTree::parse`] will descend before
/// rejecting the input.
///
/// Real RIFF files are shallow: `RIFF / LIST hdrl / LIST strl` is three
/// deep, the deepest `LIST adtl` / `LIST INFO` trees a couple more. A
/// file claiming hundreds of `LIST`-within-`LIST` levels is either
/// adversarial (a decompression-bomb-style stack exhaustion) or
/// corrupt; the bound keeps the recursive parser's stack usage finite.
pub const MAX_DEPTH: usize = 64;

/// One node in a [`RiffTree`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiffChunk {
    /// A leaf chunk: a 4-byte FourCC and its opaque `ckSize` payload
    /// bytes (the pad byte, if any, is implied by `body.len()` being
    /// odd and is **not** stored).
    Leaf {
        /// 4-byte chunk FourCC.
        id: [u8; 4],
        /// `ckSize` payload bytes, verbatim.
        body: Vec<u8>,
    },
    /// A group chunk (`RIFF` nested form, or — far more commonly — a
    /// `LIST`): a 4-byte form/list-type FourCC followed by recursively
    /// parsed children.
    Group {
        /// `RIFF` or `LIST`.
        id: [u8; 4],
        /// 4-byte form-type (`WAVE`, `AVI `, …) / list-type (`INFO`,
        /// `adtl`, `hdrl`, …) word that prefixes the children.
        form_type: [u8; 4],
        /// Recursively parsed child chunks, in file order.
        children: Vec<RiffChunk>,
    },
}

impl RiffChunk {
    /// The chunk's FourCC (`ckID`).
    pub const fn id(&self) -> [u8; 4] {
        match self {
            RiffChunk::Leaf { id, .. } | RiffChunk::Group { id, .. } => *id,
        }
    }

    /// `true` if this node is a `RIFF` / `LIST` group.
    pub const fn is_group(&self) -> bool {
        matches!(self, RiffChunk::Group { .. })
    }

    /// The un-padded `ckSize` this node serialises to: for a leaf, the
    /// body length; for a group, `4` (the form-type word) plus the
    /// padded size of every child.
    pub fn ck_size(&self) -> u64 {
        match self {
            RiffChunk::Leaf { body, .. } => body.len() as u64,
            RiffChunk::Group { children, .. } => {
                4 + children
                    .iter()
                    .map(RiffChunk::padded_outer_size)
                    .sum::<u64>()
            }
        }
    }

    /// Total bytes this node occupies on the wire including its own
    /// 8-byte header and any trailing pad byte — i.e. what its parent
    /// must budget for it.
    pub fn padded_outer_size(&self) -> u64 {
        let ck = self.ck_size();
        8 + ck + (ck & 1)
    }

    /// Append this chunk (header + body/children + pad) to `out`.
    fn encode_into(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            RiffChunk::Leaf { id, body } => encode_chunk(out, id, body),
            RiffChunk::Group {
                id,
                form_type,
                children,
            } => {
                let ck = self.ck_size();
                let size = u32::try_from(ck)
                    .map_err(|_| Error::invalid("RIFF: group ckSize exceeds 32-bit range"))?;
                write_chunk_header(out, id, size);
                out.extend_from_slice(form_type);
                for child in children {
                    child.encode_into(out)?;
                }
                // A group's ckSize is `4 + sum(padded children)`; each
                // child already self-pads, so the group total is even
                // iff `ck` is even. Emit the group's own pad byte when
                // odd (only reachable via an odd-length leaf with no
                // following sibling, which the children loop already
                // padded — so this is belt-and-braces for hand-built
                // trees with an odd form-type contribution, which can't
                // actually happen since 4 + even = even; kept for the
                // invariant's symmetry with `encode_chunk`).
                if size & 1 == 1 {
                    out.push(0);
                }
                Ok(())
            }
        }
    }

    /// Find the first descendant (depth-first, pre-order) whose FourCC
    /// equals `id`. Searches this node's children recursively. Returns
    /// `None` if no match. The receiver node itself is not tested.
    pub fn find(&self, id: &[u8; 4]) -> Option<&RiffChunk> {
        if let RiffChunk::Group { children, .. } = self {
            for child in children {
                if &child.id() == id {
                    return Some(child);
                }
                if let Some(found) = child.find(id) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Find the first child `LIST` whose list-type equals `list_type`
    /// (e.g. `b"INFO"`, `b"adtl"`, `b"hdrl"`). Only the immediate
    /// children of this node are considered. Returns the matching
    /// [`RiffChunk::Group`].
    pub fn find_list(&self, list_type: &[u8; 4]) -> Option<&RiffChunk> {
        if let RiffChunk::Group { children, .. } = self {
            children.iter().find(|c| {
                matches!(
                    c,
                    RiffChunk::Group { id, form_type, .. }
                        if *id == FOURCC_LIST && form_type == list_type
                )
            })
        } else {
            None
        }
    }

    /// The immediate children of this node, or an empty slice for a
    /// leaf.
    pub fn children(&self) -> &[RiffChunk] {
        match self {
            RiffChunk::Group { children, .. } => children,
            RiffChunk::Leaf { .. } => &[],
        }
    }
}

/// An owned, recursively-parsed RIFF file.
///
/// The outermost `RIFF` chunk is unwrapped into its `form_type` (the
/// file's form, e.g. `WAVE` / `AVI ` / `WEBP`) plus its top-level
/// `children`. Construct one with [`RiffTree::parse`]; serialise it
/// back with [`RiffTree::encode`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiffTree {
    /// The outer `RIFF` form type (`WAVE`, `AVI `, `WEBP`, …).
    pub form_type: [u8; 4],
    /// Top-level children of the outer `RIFF` chunk, in file order.
    pub children: Vec<RiffChunk>,
}

impl RiffTree {
    /// Parse a complete in-memory `RIFF` file into an owned tree.
    ///
    /// `bytes` must begin with the `RIFF` magic. The outer `ckSize` is
    /// validated against the buffer length (it may be smaller — trailing
    /// bytes after the outer chunk are ignored, matching readers that
    /// tolerate a `RIFF` embedded in a larger container — but never
    /// larger than what's present). Every nested `LIST` / `RIFF` group
    /// is descended recursively up to [`MAX_DEPTH`].
    ///
    /// Errors on: a non-`RIFF` magic (use [`crate::walk::Walker::open_rf64`]
    /// for the 64-bit forms), an outer `ckSize` that overruns the buffer,
    /// a child whose body overflows its parent's budget, a truncated
    /// header, or nesting deeper than [`MAX_DEPTH`].
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::invalid(
                "RIFF: input shorter than the 12-byte outer prologue",
            ));
        }
        let id = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if id != FOURCC_RIFF {
            return Err(Error::invalid("RIFF: outer chunk is not 'RIFF'"));
        }
        let ck_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if ck_size < 4 {
            return Err(Error::invalid(
                "RIFF: outer ckSize < 4 — no room for form type",
            ));
        }
        // The outer payload spans bytes[8 .. 8+ck_size]; it must fit.
        let payload_end = 8usize
            .checked_add(ck_size)
            .ok_or_else(|| Error::invalid("RIFF: outer ckSize overflows usize"))?;
        if payload_end > bytes.len() {
            return Err(Error::invalid(
                "RIFF: outer ckSize overruns the input buffer",
            ));
        }
        let form_type = [bytes[8], bytes[9], bytes[10], bytes[11]];
        // Children occupy bytes[12 .. payload_end].
        let children = parse_children(&bytes[12..payload_end], 1)?;
        Ok(RiffTree {
            form_type,
            children,
        })
    }

    /// Serialise the tree back to a `RIFF` byte buffer.
    ///
    /// The inverse of [`RiffTree::parse`]: for any tree produced by
    /// `parse` the output equals the original input byte-for-byte.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let ck = 4 + self
            .children
            .iter()
            .map(RiffChunk::padded_outer_size)
            .sum::<u64>();
        let size = u32::try_from(ck)
            .map_err(|_| Error::invalid("RIFF: outer ckSize exceeds 32-bit range"))?;
        let mut out = Vec::with_capacity(8 + ck as usize);
        write_chunk_header(&mut out, &FOURCC_RIFF, size);
        out.extend_from_slice(&self.form_type);
        for child in &self.children {
            child.encode_into(&mut out)?;
        }
        Ok(out)
    }

    /// Find the first descendant chunk (depth-first, pre-order) with the
    /// given FourCC across the whole tree.
    pub fn find(&self, id: &[u8; 4]) -> Option<&RiffChunk> {
        for child in &self.children {
            if &child.id() == id {
                return Some(child);
            }
            if let Some(found) = child.find(id) {
                return Some(found);
            }
        }
        None
    }

    /// Find the first top-level `LIST` whose list-type equals
    /// `list_type` (e.g. `b"INFO"`).
    pub fn find_list(&self, list_type: &[u8; 4]) -> Option<&RiffChunk> {
        self.children.iter().find(|c| {
            matches!(
                c,
                RiffChunk::Group { id, form_type, .. }
                    if *id == FOURCC_LIST && form_type == list_type
            )
        })
    }
}

/// Parse a run of sibling chunks out of `body` (the payload of a group,
/// after its form-type word has been consumed).
fn parse_children(body: &[u8], depth: usize) -> Result<Vec<RiffChunk>> {
    if depth > MAX_DEPTH {
        return Err(Error::invalid("RIFF: nesting exceeds MAX_DEPTH"));
    }
    let mut children = Vec::new();
    let mut pos = 0usize;
    while pos < body.len() {
        // A bare pad byte at the very end is tolerated only if it's the
        // single trailing byte left; otherwise we need a full header.
        if body.len() - pos < 8 {
            return Err(Error::invalid(
                "RIFF: trailing bytes too short for a chunk header",
            ));
        }
        let id = [body[pos], body[pos + 1], body[pos + 2], body[pos + 3]];
        let ck_size =
            u32::from_le_bytes([body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]])
                as usize;
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(ck_size)
            .ok_or_else(|| Error::invalid("RIFF: child ckSize overflows usize"))?;
        if body_end > body.len() {
            return Err(Error::invalid("RIFF: child chunk overflows parent"));
        }
        let chunk_body = &body[body_start..body_end];
        let node = if id == FOURCC_RIFF || id == FOURCC_LIST {
            if ck_size < 4 {
                return Err(Error::invalid(
                    "RIFF: group ckSize < 4 — no room for list type",
                ));
            }
            let form_type = [chunk_body[0], chunk_body[1], chunk_body[2], chunk_body[3]];
            let nested = parse_children(&chunk_body[4..], depth + 1)?;
            RiffChunk::Group {
                id,
                form_type,
                children: nested,
            }
        } else {
            RiffChunk::Leaf {
                id,
                body: chunk_body.to_vec(),
            }
        };
        children.push(node);
        // Advance past the body and the implicit pad byte for odd sizes.
        pos = body_end + (ck_size & 1);
    }
    Ok(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RIFF/WAVE { fmt(4) data(3→pad) LIST(INFO){ INAM("Hi") } }
    fn sample_wave() -> Vec<u8> {
        let mut info = Vec::new();
        // LIST body = "INFO" + INAM chunk
        info.extend_from_slice(b"INFO");
        encode_chunk(&mut info, b"INAM", b"Hi").unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"fmt ", &[0x01, 0x00, 0x02, 0x00]).unwrap();
        encode_chunk(&mut body, b"data", &[0xAA, 0xBB, 0xCC]).unwrap();
        encode_chunk(&mut body, b"LIST", &info).unwrap();
        let mut out = Vec::new();
        encode_chunk(&mut out, b"RIFF", &body).unwrap();
        out
    }

    #[test]
    fn parse_then_encode_is_byte_exact() {
        let bytes = sample_wave();
        let tree = RiffTree::parse(&bytes).unwrap();
        assert_eq!(&tree.form_type, b"WAVE");
        assert_eq!(tree.children.len(), 3);
        let round = tree.encode().unwrap();
        assert_eq!(round, bytes);
    }

    #[test]
    fn nested_list_is_descended_recursively() {
        let bytes = sample_wave();
        let tree = RiffTree::parse(&bytes).unwrap();
        let list = tree.find_list(b"INFO").expect("INFO list present");
        match list {
            RiffChunk::Group {
                id,
                form_type,
                children,
            } => {
                assert_eq!(id, b"LIST");
                assert_eq!(form_type, b"INFO");
                assert_eq!(children.len(), 1);
                match &children[0] {
                    RiffChunk::Leaf { id, body } => {
                        assert_eq!(id, b"INAM");
                        assert_eq!(body, b"Hi");
                    }
                    _ => panic!("INAM should be a leaf"),
                }
            }
            _ => panic!("INFO list should be a group"),
        }
    }

    #[test]
    fn find_locates_a_deep_leaf() {
        let bytes = sample_wave();
        let tree = RiffTree::parse(&bytes).unwrap();
        // INAM is two levels deep (RIFF > LIST > INAM).
        let inam = tree.find(b"INAM").expect("INAM found");
        assert_eq!(inam.id(), *b"INAM");
        assert_eq!(inam.children(), &[]);
        // A leaf at the top level.
        let fmt = tree.find(b"fmt ").unwrap();
        assert_eq!(fmt.id(), *b"fmt ");
    }

    #[test]
    fn editing_a_leaf_and_re_encoding_keeps_other_chunks() {
        let bytes = sample_wave();
        let mut tree = RiffTree::parse(&bytes).unwrap();
        // Replace the data chunk body with a longer one.
        for child in &mut tree.children {
            if let RiffChunk::Leaf { id, body } = child {
                if id == b"data" {
                    *body = vec![0x11, 0x22, 0x33, 0x44];
                }
            }
        }
        let out = tree.encode().unwrap();
        // Re-parse and confirm the untouched fmt + INFO survived.
        let re = RiffTree::parse(&out).unwrap();
        assert_eq!(re.find(b"fmt ").unwrap().id(), *b"fmt ");
        assert_eq!(re.find_list(b"INFO").unwrap().id(), *b"LIST");
        match re.find(b"data").unwrap() {
            RiffChunk::Leaf { body, .. } => assert_eq!(body, &[0x11, 0x22, 0x33, 0x44]),
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_non_riff_magic() {
        let mut bytes = sample_wave();
        bytes[0] = b'X';
        let err = RiffTree::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("not 'RIFF'"));
    }

    #[test]
    fn rejects_outer_size_overrun() {
        let mut bytes = sample_wave();
        // Bump outer ckSize past the buffer.
        let huge = (bytes.len() as u32).to_le_bytes();
        bytes[4..8].copy_from_slice(&huge); // ckSize == full len (too big: includes hdr)
        let err = RiffTree::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("overruns"));
    }

    #[test]
    fn rejects_child_overflowing_parent() {
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"data");
        body.extend_from_slice(&1_000_000u32.to_le_bytes()); // lies
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let err = RiffTree::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("overflows parent"));
    }

    #[test]
    fn tolerates_trailing_bytes_after_outer_chunk() {
        let mut bytes = sample_wave();
        bytes.extend_from_slice(b"junkjunk"); // trailing slop
        let tree = RiffTree::parse(&bytes).unwrap();
        // The re-encode is the canonical chunk WITHOUT the slop.
        let round = tree.encode().unwrap();
        assert_eq!(round, &bytes[..round.len()]);
    }

    #[test]
    fn ck_size_and_padded_size_match_encode() {
        let bytes = sample_wave();
        let tree = RiffTree::parse(&bytes).unwrap();
        for child in &tree.children {
            let mut buf = Vec::new();
            child.encode_into(&mut buf).unwrap();
            assert_eq!(buf.len() as u64, child.padded_outer_size());
        }
    }

    #[test]
    fn empty_group_round_trips() {
        // RIFF/WAVE { LIST(INFO){} } — an empty INFO list.
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"LIST", b"INFO").unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        let list = tree.find_list(b"INFO").unwrap();
        assert_eq!(list.children().len(), 0);
        assert_eq!(tree.encode().unwrap(), bytes);
    }

    #[test]
    fn max_depth_guard_rejects_pathological_nesting() {
        // Build MAX_DEPTH+2 nested LISTs to trip the guard.
        let mut inner = Vec::new();
        inner.extend_from_slice(b"sub ");
        for _ in 0..(MAX_DEPTH + 2) {
            let mut wrap = Vec::new();
            wrap.extend_from_slice(b"sub ");
            encode_chunk(&mut wrap, b"LIST", &inner).unwrap();
            inner = wrap;
        }
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &inner).unwrap();
        let err = RiffTree::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("MAX_DEPTH"));
    }
}
