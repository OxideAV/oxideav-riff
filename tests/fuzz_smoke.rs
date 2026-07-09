//! Deterministic generative fuzz-smoke harness.
//!
//! RIFF is a container format that untrusted files are routinely fed
//! into, so the walker, the tree model, and every typed chunk-body
//! decoder must treat *any* byte string as input and fail with a typed
//! `Error` rather than panicking (a slice-index out of bounds, a
//! subtract overflow, an `unwrap` on a short body, …). This harness is
//! the panic-safety guard: a small seeded xorshift PRNG generates
//! hostile inputs — purely random buffers, RIFF-shaped buffers, and
//! bit-flip mutations of valid files — and drives them through the
//! whole public parsing surface. A panic anywhere fails the test.
//!
//! It is intentionally dependency-free and deterministic: the same seed
//! always produces the same corpus, so a discovered crash is
//! reproducible from the iteration index alone. It is not a replacement
//! for coverage-guided fuzzing; it is a fast, always-on regression net
//! that runs in CI on every push.

use std::io::Cursor;

use oxideav_riff::{
    Acid, AdtlEntry, AudioId, AxmlChunk, BroadcastExtension, Bundle, BxmlChunk, ChannelAllocation,
    CharacterSet, ChunkSize64, CtocChunk, CueChunk, CuePoint, DataSize64, EmbeddedFile, Fact, Inst,
    LabeledText, PlaySegment, Playlist, RiffTree, Silence, Smpl, SxmlChunk, Walker, WaveFile,
    WaveFormat,
};

/// Minimal deterministic xorshift64 PRNG. Not cryptographic; only used
/// to enumerate a reproducible hostile-input corpus.
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        // Avoid the zero fixed point.
        XorShift64(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 11) as u32
    }

    /// A byte biased toward FourCC-alphabet and structural values so the
    /// generated buffers exercise real decode paths rather than bouncing
    /// off the first length check.
    fn next_byte(&mut self) -> u8 {
        let r = self.next_u64();
        match r & 0x7 {
            0 | 1 => b"RIFFLISTWAVEfmt data"[(r >> 3) as usize % 20],
            2 => 0,
            3 => 0xFF,
            _ => (r >> 8) as u8,
        }
    }

    fn random_bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = (self.next_u32() as usize) % (max_len + 1);
        (0..len).map(|_| self.next_byte()).collect()
    }
}

/// Feed one arbitrary chunk-body buffer to every typed decoder that
/// accepts a raw body. Every call must return without panicking; the
/// `Ok`/`Err` outcome is irrelevant here — only the absence of a panic
/// matters.
fn feed_body_decoders(body: &[u8]) {
    let _ = Acid::parse(body);
    let _ = BroadcastExtension::parse(body);
    let _ = ChannelAllocation::parse(body);
    let _ = AudioId::parse(body);
    let _ = CharacterSet::parse(body);
    let _ = CtocChunk::parse(body);
    let _ = CueChunk::parse(body);
    let _ = CuePoint::parse(body);
    let _ = DataSize64::parse(body);
    let _ = ChunkSize64::parse(body);
    let _ = Fact::parse(body);
    let _ = Playlist::parse(body);
    let _ = PlaySegment::parse(body);
    let _ = Inst::parse(body);
    let _ = AxmlChunk::parse(body);
    let _ = BxmlChunk::parse(body);
    let _ = SxmlChunk::parse(body);
    let _ = Smpl::parse(body);
    let _ = Silence::parse(body);
    let _ = WaveFormat::parse(body);
    let _ = LabeledText::parse(body);
    let _ = EmbeddedFile::parse(body);
    // The FourCC-parameterised adtl entry decoder: fuzz the tag too.
    let tag = [
        body.first().copied().unwrap_or(b'?'),
        body.get(1).copied().unwrap_or(b'?'),
        body.get(2).copied().unwrap_or(b'?'),
        body.get(3).copied().unwrap_or(b'?'),
    ];
    let _ = AdtlEntry::parse(tag, body);
}

/// Drive a whole-file buffer through both entry points that consume a
/// reader: the streaming [`Walker`] and the owned [`RiffTree`]. A
/// successful tree parse additionally exercises the round-trip
/// idempotence invariant and the typed file views, which fan out into
/// the `collect_from` / `from_tree` decoders.
fn feed_whole_file(bytes: &[u8]) {
    // Streaming walker: read every child body to completion.
    {
        let mut cur = Cursor::new(bytes);
        if let Ok(mut w) = Walker::open_root(&mut cur) {
            drain_walker(&mut w);
        }
    }
    // 64-bit walker entry point over the same bytes.
    {
        let mut cur = Cursor::new(bytes);
        if let Ok(mut w) = Walker::open_rf64(&mut cur) {
            drain_walker(&mut w);
        }
    }
    // Owned tree.
    if let Ok(tree) = RiffTree::parse(bytes) {
        // Round-trip idempotence: encode -> parse must reproduce the
        // identical tree, and a second encode must be byte-identical.
        if let Ok(enc) = tree.encode() {
            let reparsed = RiffTree::parse(&enc).expect("re-parse of own encoding must succeed");
            assert_eq!(reparsed, tree, "parse/encode is not idempotent");
            assert_eq!(
                reparsed.encode().expect("re-encode must succeed"),
                enc,
                "encode is not deterministic"
            );
        }
        // Typed file views fan out into the per-chunk decoders.
        let _ = WaveFile::from_tree(&tree);
        let _ = Bundle::from_tree(&tree);
    }
}

fn drain_walker<R: std::io::Read + std::io::Seek + ?Sized>(w: &mut Walker<'_, R>) {
    // Read every child body until the parent budget is satisfied or a
    // typed error stops iteration. Neither outcome may panic.
    loop {
        match w.read_next() {
            Ok(Some(chunk)) => {
                if w.read_body(&chunk).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

/// Prefix a random payload with a `RIFF`/`RIFX`/`RF64` outer header
/// whose `ckSize` is drawn from a small biased set (exact, off-by-a-few,
/// wildly large) so many buffers reach the child-walking code instead of
/// being rejected at the magic check.
fn riff_shaped(rng: &mut XorShift64, payload: &[u8]) -> Vec<u8> {
    let magic: &[u8; 4] = match rng.next_u64() & 0x3 {
        0 => b"RIFF",
        1 => b"RIFX",
        2 => b"RF64",
        _ => b"BW64",
    };
    let form: [u8; 4] = [b'W', b'A', b'V', b'E'];
    let mut out = Vec::new();
    out.extend_from_slice(magic);
    let declared = match rng.next_u64() & 0x7 {
        0 => (4 + payload.len()) as u32,                 // exact
        1 => (4 + payload.len()).wrapping_sub(3) as u32, // short by a few
        2 => (4 + payload.len() + 5) as u32,             // long by a few
        3 => 0xFFFF_FFFF,                                // RF64 sentinel
        _ => rng.next_u32(),                             // arbitrary
    };
    out.extend_from_slice(&declared.to_le_bytes());
    out.extend_from_slice(&form);
    out.extend_from_slice(payload);
    out
}

#[test]
fn fuzz_chunk_body_decoders_never_panic() {
    let mut rng = XorShift64::new(0x0DDC_0FFE_EBAD_F00D);
    for _ in 0..20_000 {
        let body = rng.random_bytes(96);
        feed_body_decoders(&body);
    }
    // A sweep of every short length 0..=40 with a fixed fill exercises the
    // exact fixed-record boundary of each decoder (many are 4/7/12/24/28/40
    // bytes) deterministically.
    for len in 0..=40usize {
        for fill in [0u8, 0xFF, 0x01] {
            feed_body_decoders(&vec![fill; len]);
        }
    }
}

#[test]
fn fuzz_whole_file_walker_and_tree_never_panic() {
    let mut rng = XorShift64::new(0xF00D_FACE_1234_5678);
    for _ in 0..20_000 {
        // Half purely random, half RIFF-shaped so the child walker runs.
        let payload = rng.random_bytes(120);
        if rng.next_u64() & 1 == 0 {
            feed_whole_file(&payload);
        } else {
            let shaped = riff_shaped(&mut rng, &payload);
            feed_whole_file(&shaped);
        }
    }
}

/// Build a random *valid* RIFF tree (bounded depth + child count), then
/// assert `parse(encode(t)) == t` and that a bit-flip of the encoding
/// never panics the parser.
#[test]
fn fuzz_structured_valid_tree_roundtrips_and_survives_mutation() {
    let mut rng = XorShift64::new(0xABCD_EF01_2345_6789);
    for _ in 0..3_000 {
        let tree = random_tree(&mut rng, 0);
        let enc = tree.encode().expect("valid tree must encode");
        let reparsed = RiffTree::parse(&enc).expect("valid encoding must parse");
        assert_eq!(reparsed, tree, "structured round-trip mismatch");
        assert_eq!(
            reparsed.encode().expect("re-encode"),
            enc,
            "structured re-encode not byte-exact"
        );

        // Mutate a handful of bytes and confirm the parser stays panic-free.
        let mut mutated = enc.clone();
        if !mutated.is_empty() {
            let flips = 1 + (rng.next_u32() as usize % 4);
            for _ in 0..flips {
                let idx = rng.next_u32() as usize % mutated.len();
                mutated[idx] ^= rng.next_byte();
            }
            feed_whole_file(&mutated);
        }
    }
}

/// Recursively build a random well-formed [`RiffTree`]. Depth is bounded
/// well under `MAX_DEPTH`; child counts and body lengths are small so the
/// corpus stays fast.
fn random_tree(rng: &mut XorShift64, depth: usize) -> RiffTree {
    use oxideav_riff::RiffChunk;

    fn random_fourcc(rng: &mut XorShift64) -> [u8; 4] {
        // Printable ASCII so the FourCCs read like real tags; never a
        // group id here (groups are emitted explicitly below).
        let mut fc = [0u8; 4];
        for b in &mut fc {
            *b = 0x41 + (rng.next_u32() as u8 % 26);
        }
        fc
    }

    fn random_children(rng: &mut XorShift64, depth: usize) -> Vec<RiffChunk> {
        let n = rng.next_u32() as usize % 4;
        let mut children = Vec::new();
        for _ in 0..n {
            // Occasionally emit a nested LIST, but cap the depth so the
            // recursion terminates well short of MAX_DEPTH.
            if depth < 4 && rng.next_u64() & 0x3 == 0 {
                let form = random_fourcc(rng);
                children.push(RiffChunk::Group {
                    id: *b"LIST",
                    form_type: form,
                    children: random_children(rng, depth + 1),
                });
            } else {
                let len = rng.next_u32() as usize % 20;
                let body = (0..len).map(|_| rng.next_byte()).collect();
                children.push(RiffChunk::Leaf {
                    id: random_fourcc(rng),
                    body,
                });
            }
        }
        children
    }

    RiffTree {
        form_type: *b"WAVE",
        byte_order: if rng.next_u64() & 1 == 0 {
            oxideav_riff::ByteOrder::LittleEndian
        } else {
            oxideav_riff::ByteOrder::BigEndian
        },
        children: random_children(rng, depth),
    }
}
