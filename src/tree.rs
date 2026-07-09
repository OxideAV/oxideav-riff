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

use crate::chunk::{
    encode_chunk, read_body_bounded, write_chunk_header, FOURCC_LIST, FOURCC_RIFF, FOURCC_RIFX,
};
use crate::error::{Error, Result};

/// Integer byte order of a RIFF file's length fields.
///
/// The 1991 spec (§2 "Notation Conventions") defines `RIFX` as the
/// Motorola big-endian counterpart of the Intel little-endian `RIFF`:
/// "A RIFX file is the same as a RIFF file, except that the first four
/// bytes are 'RIFX' instead of 'RIFF', and integer byte ordering is
/// represented in Motorola format." The choice affects every `ckSize`
/// length word (and any integer body fields the consumer interprets);
/// the 4-byte FourCCs are ASCII and order-independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteOrder {
    /// Intel little-endian — the `RIFF` magic. The overwhelming common
    /// case (WAV, AVI, WebP, …).
    LittleEndian,
    /// Motorola big-endian — the `RIFX` magic.
    BigEndian,
}

impl ByteOrder {
    /// The outer-wrapper FourCC this order serialises with
    /// (`RIFF` / `RIFX`).
    pub const fn magic(self) -> [u8; 4] {
        match self {
            ByteOrder::LittleEndian => FOURCC_RIFF,
            ByteOrder::BigEndian => FOURCC_RIFX,
        }
    }

    /// Decode a 4-byte length word in this order.
    const fn read_u32(self, b: [u8; 4]) -> u32 {
        match self {
            ByteOrder::LittleEndian => u32::from_le_bytes(b),
            ByteOrder::BigEndian => u32::from_be_bytes(b),
        }
    }

    /// Encode a 4-byte length word in this order.
    const fn write_u32(self, v: u32) -> [u8; 4] {
        match self {
            ByteOrder::LittleEndian => v.to_le_bytes(),
            ByteOrder::BigEndian => v.to_be_bytes(),
        }
    }
}

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

    /// Append this chunk (header + body/children + pad) to `out` in the
    /// given byte order.
    fn encode_into(&self, out: &mut Vec<u8>, order: ByteOrder) -> Result<()> {
        match self {
            RiffChunk::Leaf { id, body } => {
                if order == ByteOrder::LittleEndian {
                    return encode_chunk(out, id, body);
                }
                // RIFX: same framing, big-endian ckSize.
                let size = u32::try_from(body.len())
                    .map_err(|_| Error::invalid("RIFF: chunk body exceeds 32-bit ckSize range"))?;
                out.extend_from_slice(id);
                out.extend_from_slice(&order.write_u32(size));
                out.extend_from_slice(body);
                if size & 1 == 1 {
                    out.push(0);
                }
                Ok(())
            }
            RiffChunk::Group {
                id,
                form_type,
                children,
            } => {
                let ck = self.ck_size();
                let size = u32::try_from(ck)
                    .map_err(|_| Error::invalid("RIFF: group ckSize exceeds 32-bit range"))?;
                if order == ByteOrder::LittleEndian {
                    write_chunk_header(out, id, size);
                } else {
                    out.extend_from_slice(id);
                    out.extend_from_slice(&order.write_u32(size));
                }
                out.extend_from_slice(form_type);
                for child in children {
                    child.encode_into(out, order)?;
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

    /// Collect every descendant (depth-first, pre-order) whose FourCC
    /// equals `id` into `out`. A matched group is itself recorded *and*
    /// still descended (a `LIST` matching the id could in principle
    /// contain a deeper match).
    fn collect_into<'a>(&'a self, id: &[u8; 4], out: &mut Vec<&'a RiffChunk>) {
        if &self.id() == id {
            out.push(self);
        }
        if let RiffChunk::Group { children, .. } = self {
            for child in children {
                child.collect_into(id, out);
            }
        }
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

    /// Mutable access to a group's children for in-place editing
    /// (reorder, insert, remove). `None` for a leaf.
    pub fn children_mut(&mut self) -> Option<&mut Vec<RiffChunk>> {
        match self {
            RiffChunk::Group { children, .. } => Some(children),
            RiffChunk::Leaf { .. } => None,
        }
    }

    /// Mutable variant of [`RiffChunk::find`] — the first descendant
    /// (depth-first, pre-order) whose FourCC equals `id`. Handy for an
    /// editor that wants to mutate a `Leaf { body, .. }` in place.
    pub fn find_mut(&mut self, id: &[u8; 4]) -> Option<&mut RiffChunk> {
        if let RiffChunk::Group { children, .. } = self {
            for child in children {
                if &child.id() == id {
                    return Some(child);
                }
                if let Some(found) = child.find_mut(id) {
                    return Some(found);
                }
            }
        }
        None
    }
}

/// An owned, recursively-parsed RIFF (or RIFX) file.
///
/// The outermost `RIFF` / `RIFX` chunk is unwrapped into its `form_type`
/// (the file's form, e.g. `WAVE` / `AVI ` / `WEBP`), its `byte_order`
/// (little-endian for `RIFF`, big-endian for `RIFX`), plus its top-level
/// `children`. Construct one with [`RiffTree::parse`]; serialise it
/// back with [`RiffTree::encode`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiffTree {
    /// The outer form type (`WAVE`, `AVI `, `WEBP`, …).
    pub form_type: [u8; 4],
    /// Integer byte order of the length fields — little-endian for the
    /// `RIFF` magic, big-endian for `RIFX`.
    pub byte_order: ByteOrder,
    /// Top-level children of the outer chunk, in file order.
    pub children: Vec<RiffChunk>,
}

impl RiffTree {
    /// Parse a complete in-memory `RIFF` or `RIFX` file into an owned
    /// tree.
    ///
    /// `bytes` must begin with the `RIFF` (little-endian) or `RIFX`
    /// (big-endian) magic; the detected [`ByteOrder`] governs how every
    /// `ckSize` length word is decoded. The outer `ckSize` is validated
    /// against the buffer length (it may be smaller — trailing bytes
    /// after the outer chunk are ignored, matching readers that tolerate
    /// a `RIFF` embedded in a larger container — but never larger than
    /// what's present). Every nested `LIST` / `RIFF` group is descended
    /// recursively up to [`MAX_DEPTH`].
    ///
    /// Errors on: a magic that is neither `RIFF` nor `RIFX` (use
    /// [`crate::walk::Walker::open_rf64`] for the 64-bit forms), an outer
    /// `ckSize` that overruns the buffer, a child whose body overflows
    /// its parent's budget, a truncated header, or nesting deeper than
    /// [`MAX_DEPTH`].
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::invalid(
                "RIFF: input shorter than the 12-byte outer prologue",
            ));
        }
        let id = [bytes[0], bytes[1], bytes[2], bytes[3]];
        let byte_order = if id == FOURCC_RIFF {
            ByteOrder::LittleEndian
        } else if id == FOURCC_RIFX {
            ByteOrder::BigEndian
        } else {
            return Err(Error::invalid(
                "RIFF: outer chunk is neither 'RIFF' nor 'RIFX'",
            ));
        };
        let ck_size = byte_order.read_u32([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
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
        let children = parse_children(&bytes[12..payload_end], 1, byte_order)?;
        Ok(RiffTree {
            form_type,
            byte_order,
            children,
        })
    }

    /// Read a `RIFF` / `RIFX` file from a seekable source into an owned
    /// tree.
    ///
    /// Convenience for callers that hold a file handle rather than a
    /// slurped `&[u8]`: it reads the 8-byte outer header to learn the
    /// outer `ckSize`, then pulls exactly the outer chunk
    /// (`8 + ckSize` bytes) into a buffer and delegates to
    /// [`RiffTree::parse`]. Bytes after the outer chunk are not read.
    ///
    /// The tree is an in-memory model by construction, so this is a
    /// read-into-`Vec` followed by `parse` — there's no streaming-tree
    /// variant. For genuinely streaming consumption that skips chunks
    /// without materialising them, use [`crate::walk::Walker`] instead.
    pub fn from_reader<R: std::io::Read + std::io::Seek + ?Sized>(r: &mut R) -> Result<Self> {
        r.seek(std::io::SeekFrom::Start(0))?;
        let mut header = [0u8; 8];
        r.read_exact(&mut header)?;
        let id = [header[0], header[1], header[2], header[3]];
        let byte_order = if id == FOURCC_RIFF {
            ByteOrder::LittleEndian
        } else if id == FOURCC_RIFX {
            ByteOrder::BigEndian
        } else {
            return Err(Error::invalid(
                "RIFF: outer chunk is neither 'RIFF' nor 'RIFX'",
            ));
        };
        let ck_size = byte_order.read_u32([header[4], header[5], header[6], header[7]]) as usize;
        // Read the outer payload through the bounded helper rather than
        // `vec![0u8; 8 + ck_size]` so a hostile outer `ckSize` (up to
        // 4 GiB) on a short reader is rejected as a typed truncation error
        // instead of triggering a multi-gigabyte speculative allocation.
        let mut buf = Vec::with_capacity(8 + ck_size.min(crate::chunk::READ_BODY_INITIAL_CAP));
        buf.extend_from_slice(&header);
        buf.extend_from_slice(&read_body_bounded(r, ck_size)?);
        Self::parse(&buf)
    }

    /// Serialise the tree back to a byte buffer in its [`ByteOrder`].
    ///
    /// The inverse of [`RiffTree::parse`]: for any tree produced by
    /// `parse` the output equals the original input byte-for-byte
    /// (`RIFF` or `RIFX` magic preserved).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let ck = 4 + self
            .children
            .iter()
            .map(RiffChunk::padded_outer_size)
            .sum::<u64>();
        let size = u32::try_from(ck)
            .map_err(|_| Error::invalid("RIFF: outer ckSize exceeds 32-bit range"))?;
        let mut out = Vec::with_capacity(8 + ck as usize);
        out.extend_from_slice(&self.byte_order.magic());
        out.extend_from_slice(&self.byte_order.write_u32(size));
        out.extend_from_slice(&self.form_type);
        for child in &self.children {
            child.encode_into(&mut out, self.byte_order)?;
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

    /// Collect every descendant chunk (depth-first, pre-order) with the
    /// given FourCC across the whole tree.
    ///
    /// Unlike [`RiffTree::find`] (first match only) this is the right
    /// helper when a FourCC may legitimately repeat — e.g. several
    /// top-level `LIST` chunks, or multiple `data` segments — and the
    /// editor needs all of them.
    pub fn find_all(&self, id: &[u8; 4]) -> Vec<&RiffChunk> {
        let mut out = Vec::new();
        for child in &self.children {
            child.collect_into(id, &mut out);
        }
        out
    }

    /// Mutable variant of [`RiffTree::find`] — the first descendant
    /// whose FourCC equals `id`, for in-place editing followed by a
    /// byte-exact [`RiffTree::encode`].
    pub fn find_mut(&mut self, id: &[u8; 4]) -> Option<&mut RiffChunk> {
        for child in &mut self.children {
            if &child.id() == id {
                return Some(child);
            }
            if let Some(found) = child.find_mut(id) {
                return Some(found);
            }
        }
        None
    }
}

/// Parse a run of sibling chunks out of `body` (the payload of a group,
/// after its form-type word has been consumed), decoding length words in
/// `order`.
fn parse_children(body: &[u8], depth: usize, order: ByteOrder) -> Result<Vec<RiffChunk>> {
    if depth > MAX_DEPTH {
        return Err(Error::invalid("RIFF: nesting exceeds MAX_DEPTH"));
    }
    let mut children = Vec::new();
    let mut pos = 0usize;
    while pos < body.len() {
        // Each iteration consumes a full child (header + body + the
        // odd-length pad byte, accounted inline via `ck_size & 1`), so a
        // well-formed body lands `pos` exactly on `body.len()`. Any
        // 1..=7 residual bytes mean the parent's payload is truncated —
        // there isn't room for the next 8-byte header.
        if body.len() - pos < 8 {
            return Err(Error::invalid(
                "RIFF: trailing bytes too short for a chunk header",
            ));
        }
        let id = [body[pos], body[pos + 1], body[pos + 2], body[pos + 3]];
        let ck_size =
            order.read_u32([body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]]) as usize;
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
            let nested = parse_children(&chunk_body[4..], depth + 1, order)?;
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
        assert!(format!("{err}").contains("neither 'RIFF' nor 'RIFX'"));
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
            child
                .encode_into(&mut buf, ByteOrder::LittleEndian)
                .unwrap();
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

    #[test]
    fn find_mut_edits_a_deep_leaf_in_place() {
        let bytes = sample_wave();
        let mut tree = RiffTree::parse(&bytes).unwrap();
        // Mutate the nested INAM leaf body via find_mut.
        match tree.find_mut(b"INAM").unwrap() {
            RiffChunk::Leaf { body, .. } => *body = b"Bye!".to_vec(),
            _ => panic!(),
        }
        let out = tree.encode().unwrap();
        let re = RiffTree::parse(&out).unwrap();
        match re.find(b"INAM").unwrap() {
            RiffChunk::Leaf { body, .. } => assert_eq!(body, b"Bye!"),
            _ => panic!(),
        }
        // Other chunks unaffected.
        assert_eq!(re.find(b"fmt ").unwrap().id(), *b"fmt ");
    }

    #[test]
    fn children_mut_inserts_and_reorders() {
        let bytes = sample_wave();
        let mut tree = RiffTree::parse(&bytes).unwrap();
        // Append a new leaf at the top level.
        tree.children.push(RiffChunk::Leaf {
            id: *b"new ",
            body: vec![0xFF, 0xEE],
        });
        // children_mut on the INFO list: append a tag.
        let list = tree.find_mut(b"LIST").unwrap();
        list.children_mut().unwrap().push(RiffChunk::Leaf {
            id: *b"IART",
            body: vec![b'X', 0],
        });
        // Re-encode + re-parse keeps everything consistent.
        let out = tree.encode().unwrap();
        let re = RiffTree::parse(&out).unwrap();
        assert!(re.find(b"new ").is_some());
        assert_eq!(re.find_list(b"INFO").unwrap().children().len(), 2);
    }

    #[test]
    fn odd_then_even_child_pad_accounting() {
        // RIFF/WAVE { odd(3→pad) even(2) } — the pad byte after the odd
        // first child must be consumed so the second child's header lands
        // on the 2-byte boundary.
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"odd ", &[1, 2, 3]).unwrap(); // 3-byte → pad
        encode_chunk(&mut body, b"evn ", &[4, 5]).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        assert_eq!(tree.children.len(), 2);
        match &tree.children[0] {
            RiffChunk::Leaf { id, body } => {
                assert_eq!(id, b"odd ");
                assert_eq!(body, &[1, 2, 3]); // body is the un-padded 3 bytes
            }
            _ => panic!(),
        }
        match &tree.children[1] {
            RiffChunk::Leaf { id, body } => {
                assert_eq!(id, b"evn ");
                assert_eq!(body, &[4, 5]);
            }
            _ => panic!(),
        }
        assert_eq!(tree.encode().unwrap(), bytes);
    }

    #[test]
    fn truncated_residual_bytes_rejected() {
        // A parent whose payload ends 3 bytes into what should be the
        // next 8-byte header → truncated.
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"data", &[0; 4]).unwrap();
        body.extend_from_slice(&[1, 2, 3]); // 3 stray bytes, not a header
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let err = RiffTree::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("too short for a chunk header"));
    }

    #[test]
    fn from_reader_matches_parse() {
        use std::io::Cursor;
        let bytes = sample_wave();
        let from_slice = RiffTree::parse(&bytes).unwrap();
        let mut cur = Cursor::new(&bytes);
        let from_rdr = RiffTree::from_reader(&mut cur).unwrap();
        assert_eq!(from_rdr, from_slice);
        assert_eq!(from_rdr.encode().unwrap(), bytes);
    }

    #[test]
    fn from_reader_ignores_trailing_bytes() {
        use std::io::Cursor;
        let mut bytes = sample_wave();
        let outer_len = bytes.len();
        bytes.extend_from_slice(b"TRAILINGSLOP");
        let mut cur = Cursor::new(&bytes);
        let tree = RiffTree::from_reader(&mut cur).unwrap();
        // Re-encode reproduces only the outer chunk.
        assert_eq!(tree.encode().unwrap(), &bytes[..outer_len]);
    }

    #[test]
    fn from_reader_rejects_outer_size_lie_without_huge_alloc() {
        use std::io::Cursor;
        // A 12-byte "file" whose outer ckSize claims ~1 GiB. `from_reader`
        // must reject it as a typed truncation error rather than trying to
        // allocate the gigabyte the header claims.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0x4000_0000u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        let mut cur = Cursor::new(bytes);
        let err = RiffTree::from_reader(&mut cur).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("truncated"), "{msg}");
    }

    #[test]
    fn from_reader_rejects_non_riff() {
        use std::io::Cursor;
        let mut bytes = sample_wave();
        bytes[0] = b'Z';
        let mut cur = Cursor::new(&bytes);
        let err = RiffTree::from_reader(&mut cur).unwrap_err();
        assert!(format!("{err}").contains("neither 'RIFF' nor 'RIFX'"));
    }

    #[test]
    fn find_all_collects_repeated_fourccs() {
        // RIFF/WAVE { LIST(INFO){ INAM IART } LIST(adtl){} }
        let mut info = Vec::new();
        info.extend_from_slice(b"INFO");
        encode_chunk(&mut info, b"INAM", b"Hi").unwrap();
        encode_chunk(&mut info, b"IART", b"X").unwrap();
        let mut adtl = Vec::new();
        adtl.extend_from_slice(b"adtl");
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"LIST", &info).unwrap();
        encode_chunk(&mut body, b"LIST", &adtl).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();

        let tree = RiffTree::parse(&bytes).unwrap();
        // Two LIST chunks at the top level.
        let lists = tree.find_all(b"LIST");
        assert_eq!(lists.len(), 2);
        // INAM + IART are deeper leaves; one each.
        assert_eq!(tree.find_all(b"INAM").len(), 1);
        assert_eq!(tree.find_all(b"IART").len(), 1);
        // No match → empty.
        assert!(tree.find_all(b"data").is_empty());
    }

    /// Hand-build a `RIFX`/`WAVE { fmt(4) data(3→pad) }` with big-endian
    /// `ckSize` words so the parser must read Motorola order.
    fn sample_rifx() -> Vec<u8> {
        let mut v = Vec::new();
        // outer ckSize (big-endian): WAVE(4) + fmt hdr(8)+body(4)
        //   + data hdr(8)+body(3)+pad(1) = 28
        v.extend_from_slice(b"RIFX");
        v.extend_from_slice(&28u32.to_be_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&4u32.to_be_bytes());
        v.extend_from_slice(&[0x01, 0x00, 0x02, 0x00]);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&3u32.to_be_bytes());
        v.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        v.push(0); // pad
        v
    }

    #[test]
    fn rifx_parses_big_endian_sizes() {
        let bytes = sample_rifx();
        let tree = RiffTree::parse(&bytes).unwrap();
        assert_eq!(tree.byte_order, ByteOrder::BigEndian);
        assert_eq!(&tree.form_type, b"WAVE");
        assert_eq!(tree.children.len(), 2);
        match tree.find(b"data").unwrap() {
            RiffChunk::Leaf { body, .. } => assert_eq!(body, &[0xAA, 0xBB, 0xCC]),
            _ => panic!(),
        }
    }

    #[test]
    fn rifx_round_trips_byte_exact() {
        let bytes = sample_rifx();
        let tree = RiffTree::parse(&bytes).unwrap();
        // Re-encode must preserve the RIFX magic + big-endian sizes.
        assert_eq!(tree.encode().unwrap(), bytes);
    }

    #[test]
    fn riff_byte_order_is_little_endian() {
        let bytes = sample_wave();
        let tree = RiffTree::parse(&bytes).unwrap();
        assert_eq!(tree.byte_order, ByteOrder::LittleEndian);
        assert_eq!(tree.byte_order.magic(), *b"RIFF");
    }

    #[test]
    fn byte_order_magic_pairs() {
        assert_eq!(ByteOrder::LittleEndian.magic(), *b"RIFF");
        assert_eq!(ByteOrder::BigEndian.magic(), *b"RIFX");
    }
}
