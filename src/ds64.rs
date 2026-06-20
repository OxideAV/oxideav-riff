//! Typed decoder for the RF64 / BW64 `ds64` (data-size-64) chunk.
//!
//! A standard RIFF/WAVE file stores every chunk size — and the outer
//! file size — in a 32-bit field, capping the format at 4 GiB. The
//! MBWF / RF64 extension lifts that cap: when a file would overflow,
//! the outer `RIFF` FourCC becomes `RF64` (or `BW64` for an
//! ADM-carrying file), the affected 32-bit size fields are set to the
//! sentinel `0xFFFFFFFF`, and a mandatory `ds64` chunk — which must be
//! the first chunk after the form type — supplies the real 64-bit
//! values.
//!
//! The `ds64` chunk body is laid out as:
//!
//! ```text
//! ds64 ( <ckSize:u32 LE>
//!   riffSizeLow    : u32 LE  } 64-bit size of the RF64 block
//!   riffSizeHigh   : u32 LE  }   (replaces the outer RF64 ckSize)
//!   dataSizeLow    : u32 LE  } 64-bit size of the 'data' chunk
//!   dataSizeHigh   : u32 LE  }
//!   sampleCountLow : u32 LE  } 64-bit sample count
//!   sampleCountHigh: u32 LE  }   (replaces the 'fact' chunk count)
//!   tableLength    : u32 LE      number of <chunkSize64> table entries
//!   table          : tableLength records, 12 bytes each
//! )
//!
//! chunkSize64 := struct {
//!     FOURCC chunkId;          // ID of an oversized non-'data' chunk
//!     DWORD  chunkSizeLow;     // low 32 bits of its 64-bit size
//!     DWORD  chunkSizeHigh;    // high 32 bits
//! }
//! ```
//!
//! Every multi-byte field is little-endian. The three mandatory 64-bit
//! values each replace a 32-bit field elsewhere in the file (the outer
//! `RF64`/`BW64` ckSize, the `data` chunk's ckSize, and the `fact`
//! chunk's sample count); the optional table carries 64-bit size
//! overrides for any *other* chunk that exceeds 4 GiB (the spec notes
//! that in practice this is rare).
//!
//! A field elsewhere in the file uses its real 32-bit value whenever
//! that value is *not* the sentinel; only a `0xFFFFFFFF` field defers to
//! the corresponding `ds64` value. [`is_sentinel`] tests for that
//! sentinel so a consumer can decide, per field, whether to use the
//! 32-bit value as-is or look it up here.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/ebu-tech3306-v1.pdf` §A.2 — "New
//!   Chunks and Structs in the RF64/WAVE (MBWF) format" (the
//!   `RF64Chunk` / `DataSize64Chunk` / `ChunkSize64` struct layouts and
//!   the `0xFFFFFFFF` deferral rule).
//! - `docs/container/riff/metadata/ebu-tech3306-v2.pdf` §1 — the `BW64`
//!   FourCC variant carrying ADM payloads.

use crate::error::{Error, Result};

/// FourCC of the RF64 outer wrapper (replaces `RIFF` when the file
/// exceeds the 32-bit size cap).
pub const FOURCC_RF64: [u8; 4] = *b"RF64";

/// FourCC of the BW64 outer wrapper — the ADM-carrying RF64 variant
/// (EBU Tech 3306 v2 / ITU-R BS.2088), structurally identical to
/// `RF64` but signalling an Audio Definition Model payload.
pub const FOURCC_BW64: [u8; 4] = *b"BW64";

/// FourCC of the data-size-64 chunk.
pub const FOURCC_DS64: [u8; 4] = *b"ds64";

/// Size in bytes of one on-wire `<chunkSize64>` table record (a 4-byte
/// FourCC plus a 64-bit little-endian size).
pub const CHUNK_SIZE64_LEN: usize = 12;

/// Size in bytes of the fixed `ds64` prefix that precedes the optional
/// size-override table: three 64-bit values plus the 32-bit table
/// length.
pub const DS64_PREFIX_LEN: usize = 28;

/// The 32-bit sentinel value (`0xFFFFFFFF`) a size field carries when
/// its real value lives in the `ds64` chunk.
pub const SIZE_SENTINEL: u32 = 0xFFFF_FFFF;

/// `true` if `value` is the `0xFFFFFFFF` deferral sentinel — i.e. the
/// real 64-bit value for this field is in the `ds64` chunk rather than
/// in the 32-bit field itself.
#[must_use]
pub fn is_sentinel(value: u32) -> bool {
    value == SIZE_SENTINEL
}

/// `true` if `magic` is an RF64-family outer wrapper FourCC (`RF64` or
/// `BW64`) — i.e. the file uses 64-bit sizes and carries a `ds64` chunk.
#[must_use]
pub fn is_rf64_magic(magic: &[u8; 4]) -> bool {
    magic == &FOURCC_RF64 || magic == &FOURCC_BW64
}

/// One `<chunkSize64>` table entry: a 64-bit size override for a single
/// oversized non-`data` chunk, keyed by its FourCC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkSize64 {
    /// `chunkId` — the FourCC of the chunk whose real 64-bit size this
    /// entry supplies. Kept as a raw `[u8; 4]` so any FourCC round-trips.
    pub chunk_id: [u8; 4],
    /// The real 64-bit size of that chunk, reassembled from the
    /// little-endian low/high halves.
    pub size: u64,
}

impl ChunkSize64 {
    /// Decode one 12-byte `<chunkSize64>` record from `raw`.
    ///
    /// `raw` must be exactly [`CHUNK_SIZE64_LEN`] bytes; otherwise an
    /// `Error::invalid` is returned.
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() != CHUNK_SIZE64_LEN {
            return Err(Error::invalid("RIFF: ds64 table record is not 12 bytes"));
        }
        let mut id = [0u8; 4];
        id.copy_from_slice(&raw[0..4]);
        let low = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as u64;
        let high = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as u64;
        Ok(ChunkSize64 {
            chunk_id: id,
            size: (high << 32) | low,
        })
    }

    /// Serialize this entry as a [`CHUNK_SIZE64_LEN`]-byte `<chunkSize64>`
    /// record — the FourCC followed by the 64-bit size split into its
    /// little-endian low/high halves. Exact inverse of
    /// [`ChunkSize64::parse`].
    pub fn encode(&self) -> [u8; CHUNK_SIZE64_LEN] {
        let mut b = [0u8; CHUNK_SIZE64_LEN];
        b[0..4].copy_from_slice(&self.chunk_id);
        b[4..8].copy_from_slice(&((self.size & 0xFFFF_FFFF) as u32).to_le_bytes());
        b[8..12].copy_from_slice(&((self.size >> 32) as u32).to_le_bytes());
        b
    }
}

/// A decoded `ds64` chunk: the three mandatory 64-bit values plus the
/// optional per-chunk size-override table.
///
/// The fixed prefix is [`DS64_PREFIX_LEN`] bytes; the body must then
/// carry exactly `tableLength` records of [`CHUNK_SIZE64_LEN`] bytes —
/// a short or over-long table is rejected rather than silently
/// truncated, so a malformed chunk never yields a partially-populated
/// table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataSize64 {
    /// `riffSize` — the real 64-bit size of the RF64/BW64 block, used in
    /// place of the outer wrapper's `0xFFFFFFFF` ckSize.
    pub riff_size: u64,
    /// `dataSize` — the real 64-bit size of the `data` chunk, used in
    /// place of that chunk's `0xFFFFFFFF` ckSize.
    pub data_size: u64,
    /// `sampleCount` — the real 64-bit per-channel sample count, used in
    /// place of the `fact` chunk's `0xFFFFFFFF` count.
    pub sample_count: u64,
    /// The size-override table: one entry per oversized non-`data` chunk.
    /// Empty in the common case (only `data` exceeds 4 GiB).
    table: Vec<ChunkSize64>,
}

impl DataSize64 {
    /// Decode a `ds64` chunk body (already pad-stripped by the walker).
    ///
    /// The body is the [`DS64_PREFIX_LEN`]-byte fixed prefix
    /// (`riffSize` / `dataSize` / `sampleCount` / `tableLength`)
    /// followed by `tableLength` records of [`CHUNK_SIZE64_LEN`] bytes.
    /// A body shorter than the prefix, or whose remaining length is not
    /// exactly `tableLength * 12`, is an `Error::invalid`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < DS64_PREFIX_LEN {
            return Err(Error::invalid(
                "RIFF: ds64 chunk shorter than 28-byte prefix",
            ));
        }
        let u64_at = |off: usize| -> u64 {
            let low =
                u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as u64;
            let high =
                u32::from_le_bytes([body[off + 4], body[off + 5], body[off + 6], body[off + 7]])
                    as u64;
            (high << 32) | low
        };
        let riff_size = u64_at(0);
        let data_size = u64_at(8);
        let sample_count = u64_at(16);
        let table_length = u32::from_le_bytes([body[24], body[25], body[26], body[27]]) as usize;

        let records = &body[DS64_PREFIX_LEN..];
        let expected = table_length
            .checked_mul(CHUNK_SIZE64_LEN)
            .ok_or_else(|| Error::invalid("RIFF: ds64 table length overflows"))?;
        if records.len() != expected {
            return Err(Error::invalid(
                "RIFF: ds64 body length disagrees with tableLength",
            ));
        }
        let mut table = Vec::with_capacity(table_length);
        for i in 0..table_length {
            let off = i * CHUNK_SIZE64_LEN;
            table.push(ChunkSize64::parse(&records[off..off + CHUNK_SIZE64_LEN])?);
        }
        Ok(DataSize64 {
            riff_size,
            data_size,
            sample_count,
            table,
        })
    }

    /// Build a `ds64` from the three mandatory 64-bit values and a
    /// size-override table.
    ///
    /// The write-side counterpart of [`DataSize64::parse`]: callers
    /// constructing an RF64 / BW64 file populate the mandatory
    /// `riffSize` / `dataSize` / `sampleCount` plus any per-chunk
    /// overrides, then emit the body with [`DataSize64::encode_body`].
    pub fn new(riff_size: u64, data_size: u64, sample_count: u64, table: Vec<ChunkSize64>) -> Self {
        DataSize64 {
            riff_size,
            data_size,
            sample_count,
            table,
        }
    }

    /// Append a `<chunkSize64>` size override for an oversized non-`data`
    /// chunk to the table, returning `self` for chaining.
    pub fn with_override(mut self, chunk_id: [u8; 4], size: u64) -> Self {
        self.table.push(ChunkSize64 { chunk_id, size });
        self
    }

    /// The size-override table entries in on-wire order.
    pub fn table(&self) -> &[ChunkSize64] {
        &self.table
    }

    /// Look up the 64-bit size override for a chunk by FourCC.
    ///
    /// Returns the first table entry whose `chunk_id` matches, or `None`
    /// when the chunk has no override (i.e. its 32-bit ckSize is its
    /// real size). The `data` chunk is intentionally *not* served from
    /// here — its size lives in the dedicated [`DataSize64::data_size`]
    /// field, never in the table.
    pub fn size_for(&self, chunk_id: &[u8; 4]) -> Option<u64> {
        self.table
            .iter()
            .find(|e| &e.chunk_id == chunk_id)
            .map(|e| e.size)
    }

    /// Resolve a chunk's real 64-bit size given the 32-bit ckSize read
    /// from its header.
    ///
    /// If `ck_size` is not the [`SIZE_SENTINEL`] it is returned verbatim
    /// (widened to 64 bits). If it *is* the sentinel, the table is
    /// consulted for `chunk_id`; a sentinel with no matching table entry
    /// is a malformed file and yields `None`.
    pub fn resolve(&self, chunk_id: &[u8; 4], ck_size: u32) -> Option<u64> {
        if is_sentinel(ck_size) {
            self.size_for(chunk_id)
        } else {
            Some(u64::from(ck_size))
        }
    }

    /// Serialize this `ds64` chunk *body* — the [`DS64_PREFIX_LEN`]-byte
    /// fixed prefix (the three mandatory 64-bit values, each split into
    /// little-endian low/high halves, plus the 32-bit `tableLength`)
    /// followed by the size-override table records.
    ///
    /// Exact inverse of [`DataSize64::parse`]: a parsed `ds64`
    /// re-encodes to the same body bytes, and the emitted `tableLength`
    /// always agrees with the record count it precedes.
    pub fn encode_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DS64_PREFIX_LEN + self.table.len() * CHUNK_SIZE64_LEN);
        for value in [self.riff_size, self.data_size, self.sample_count] {
            out.extend_from_slice(&((value & 0xFFFF_FFFF) as u32).to_le_bytes());
            out.extend_from_slice(&((value >> 32) as u32).to_le_bytes());
        }
        out.extend_from_slice(&(self.table.len() as u32).to_le_bytes());
        for entry in &self.table {
            out.extend_from_slice(&entry.encode());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one 12-byte `<chunkSize64>` table record.
    fn rec(id: &[u8; 4], size: u64) -> Vec<u8> {
        let mut v = Vec::with_capacity(CHUNK_SIZE64_LEN);
        v.extend_from_slice(id);
        v.extend_from_slice(&((size & 0xFFFF_FFFF) as u32).to_le_bytes());
        v.extend_from_slice(&((size >> 32) as u32).to_le_bytes());
        v
    }

    /// Build a `ds64` chunk body from the three mandatory values plus a
    /// table of records.
    fn ds64_body(riff: u64, data: u64, samples: u64, table: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&((riff & 0xFFFF_FFFF) as u32).to_le_bytes());
        body.extend_from_slice(&((riff >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&((data & 0xFFFF_FFFF) as u32).to_le_bytes());
        body.extend_from_slice(&((data >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&((samples & 0xFFFF_FFFF) as u32).to_le_bytes());
        body.extend_from_slice(&((samples >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&(table.len() as u32).to_le_bytes());
        for r in table {
            body.extend_from_slice(r);
        }
        body
    }

    #[test]
    fn prefix_and_record_sizes() {
        assert_eq!(DS64_PREFIX_LEN, 28);
        assert_eq!(CHUNK_SIZE64_LEN, 12);
        assert_eq!(rec(b"big1", 1).len(), 12);
    }

    #[test]
    fn sentinel_and_magic_predicates() {
        assert!(is_sentinel(0xFFFF_FFFF));
        assert!(!is_sentinel(0));
        assert!(!is_sentinel(0xFFFF_FFFE));
        assert!(is_rf64_magic(b"RF64"));
        assert!(is_rf64_magic(b"BW64"));
        assert!(!is_rf64_magic(b"RIFF"));
        assert!(!is_rf64_magic(b"WAVE"));
    }

    #[test]
    fn parse_mandatory_values_no_table() {
        // A file just over 4 GiB: data size ~5 GiB.
        let data = 5_368_709_120u64; // 5 * 2^30
        let riff = data + 1024;
        let samples = data / 4; // 16-bit stereo => 4 bytes/frame
        let body = ds64_body(riff, data, samples, &[]);
        let ds = DataSize64::parse(&body).unwrap();
        assert_eq!(ds.riff_size, riff);
        assert_eq!(ds.data_size, data);
        assert_eq!(ds.sample_count, samples);
        assert!(ds.table().is_empty());
    }

    #[test]
    fn high_word_reassembly() {
        // Exercise a value that lives entirely in the high 32 bits.
        let big = 1u64 << 40;
        let body = ds64_body(big, big, big, &[]);
        let ds = DataSize64::parse(&body).unwrap();
        assert_eq!(ds.riff_size, big);
        assert_eq!(ds.data_size, big);
        assert_eq!(ds.sample_count, big);
    }

    #[test]
    fn parse_with_table_entries() {
        let big_levl = 600_000_000_000u64; // a hypothetical oversized 'levl'
        let body = ds64_body(
            700_000_000_000,
            500_000_000_000,
            120_000_000_000,
            &[rec(b"levl", big_levl), rec(b"junk", 9_000_000_000)],
        );
        let ds = DataSize64::parse(&body).unwrap();
        assert_eq!(ds.table().len(), 2);
        assert_eq!(ds.table()[0].chunk_id, *b"levl");
        assert_eq!(ds.table()[0].size, big_levl);
        assert_eq!(ds.size_for(b"levl"), Some(big_levl));
        assert_eq!(ds.size_for(b"junk"), Some(9_000_000_000));
        assert_eq!(ds.size_for(b"data"), None);
    }

    #[test]
    fn resolve_uses_table_only_for_sentinel() {
        let big = 8_000_000_000u64;
        let body = ds64_body(0, 0, 0, &[rec(b"levl", big)]);
        let ds = DataSize64::parse(&body).unwrap();
        // Non-sentinel ckSize is returned verbatim (widened).
        assert_eq!(ds.resolve(b"levl", 1234), Some(1234));
        // Sentinel ckSize defers to the table.
        assert_eq!(ds.resolve(b"levl", SIZE_SENTINEL), Some(big));
        // Sentinel with no table entry is a malformed file.
        assert_eq!(ds.resolve(b"absx", SIZE_SENTINEL), None);
    }

    #[test]
    fn body_shorter_than_prefix_is_rejected() {
        let err = DataSize64::parse(&[0u8; 27]).unwrap_err();
        assert!(format!("{err}").contains("shorter than 28-byte prefix"));
    }

    #[test]
    fn table_length_disagreeing_with_body_is_rejected() {
        // Declares 2 table entries but supplies only one record.
        let mut body = ds64_body(0, 0, 0, &[]);
        // Overwrite tableLength to 2.
        body[24..28].copy_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&rec(b"levl", 1));
        let err = DataSize64::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with tableLength"));
    }

    #[test]
    fn overlong_table_is_rejected() {
        // Declares 1 entry but supplies two.
        let body = {
            let mut b = ds64_body(0, 0, 0, &[rec(b"levl", 1), rec(b"junk", 2)]);
            b[24..28].copy_from_slice(&1u32.to_le_bytes());
            b
        };
        let err = DataSize64::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("disagrees with tableLength"));
    }

    #[test]
    fn record_parse_rejects_wrong_length() {
        let err = ChunkSize64::parse(&[0u8; 11]).unwrap_err();
        assert!(format!("{err}").contains("not 12 bytes"));
    }

    #[test]
    fn arbitrary_table_fourcc_round_trips() {
        let body = ds64_body(0, 0, 0, &[rec(b"\x00\x01\x02\x03", 42)]);
        let ds = DataSize64::parse(&body).unwrap();
        assert_eq!(ds.table()[0].chunk_id, [0, 1, 2, 3]);
        assert_eq!(ds.size_for(&[0, 1, 2, 3]), Some(42));
    }

    #[test]
    fn record_encode_is_parse_inverse() {
        let big = 600_000_000_000u64;
        let r = ChunkSize64::parse(&rec(b"levl", big)).unwrap();
        let encoded = r.encode();
        assert_eq!(&encoded[..], &rec(b"levl", big)[..]);
        assert_eq!(ChunkSize64::parse(&encoded).unwrap(), r);
    }

    #[test]
    fn encode_body_no_table_round_trips() {
        let data = 5_368_709_120u64;
        let riff = data + 1024;
        let samples = data / 4;
        let body = ds64_body(riff, data, samples, &[]);
        let ds = DataSize64::parse(&body).unwrap();
        let encoded = ds.encode_body();
        assert_eq!(encoded, body);
        assert_eq!(DataSize64::parse(&encoded).unwrap(), ds);
    }

    #[test]
    fn encode_body_with_table_round_trips() {
        let body = ds64_body(
            700_000_000_000,
            500_000_000_000,
            120_000_000_000,
            &[rec(b"levl", 600_000_000_000), rec(b"junk", 9_000_000_000)],
        );
        let ds = DataSize64::parse(&body).unwrap();
        let encoded = ds.encode_body();
        assert_eq!(encoded, body);
        assert_eq!(DataSize64::parse(&encoded).unwrap(), ds);
    }

    #[test]
    fn builder_encode_parse_round_trip() {
        let ds = DataSize64::new(
            700_000_000_000,
            500_000_000_000,
            120_000_000_000,
            Vec::new(),
        )
        .with_override(*b"levl", 600_000_000_000)
        .with_override(*b"junk", 9_000_000_000);
        let encoded = ds.encode_body();
        let reparsed = DataSize64::parse(&encoded).unwrap();
        assert_eq!(reparsed, ds);
        assert_eq!(reparsed.size_for(b"levl"), Some(600_000_000_000));
        // tableLength written into the prefix matches the record count.
        assert_eq!(encoded[24..28], 2u32.to_le_bytes());
    }
}
