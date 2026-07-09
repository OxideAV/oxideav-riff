//! Dependency-free micro-benchmark of the RIFF hot paths.
//!
//! Registered with `harness = false`, so this is a plain `main` timing
//! loop rather than a `libtest` / external-harness bench — it runs on
//! stable and pulls in no third-party crate. It measures the four
//! operations a RIFF-family demuxer or editor spends its time in:
//!
//! - **streaming walk (skip)** — [`Walker`] over a large file, skipping
//!   every child body: the demuxer-that-seeks-past-uninteresting-chunks
//!   path.
//! - **streaming walk (read)** — the same walk but pulling every body
//!   into a `Vec` via the bounded reader.
//! - **tree parse** — [`RiffTree::parse`], the whole-file
//!   edit-and-rewrite read path.
//! - **tree encode** — [`RiffTree::encode`], the write-back path.
//!
//! Run with `cargo bench -p oxideav-riff`. Absolute numbers are
//! machine-dependent; the value is tracking relative regressions across
//! rounds.

use std::hint::black_box;
use std::io::Cursor;
use std::time::Instant;

use oxideav_riff::{encode_chunk, RiffTree, Walker};

/// Build a RIFF/WAVE file carrying `n_chunks` mixed leaf chunks and a
/// handful of nested `LIST INFO` groups, so the walk and tree paths see a
/// realistic mixture of leaves, groups, and odd-length pad bytes.
fn build_file(n_chunks: usize) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");

    // A representative fmt chunk.
    encode_chunk(
        &mut body,
        b"fmt ",
        &[0x01, 0x00, 0x02, 0x00, 0x44, 0xAC, 0, 0],
    )
    .unwrap();

    // Many data-ish leaves, alternating even/odd length to exercise pad
    // handling on the hot loop.
    for i in 0..n_chunks {
        let len = 32 + (i % 7); // some odd, some even
        let payload: Vec<u8> = (0..len).map(|b| (b as u8).wrapping_mul(31)).collect();
        encode_chunk(&mut body, b"data", &payload).unwrap();

        // Sprinkle in a nested LIST INFO every 16 leaves.
        if i % 16 == 0 {
            let mut info = Vec::new();
            info.extend_from_slice(b"INFO");
            encode_chunk(&mut info, b"INAM", b"benchmark title\0").unwrap();
            encode_chunk(&mut info, b"IART", b"artist\0").unwrap();
            encode_chunk(&mut body, b"LIST", &info).unwrap();
        }
    }

    let mut out = Vec::new();
    encode_chunk(&mut out, b"RIFF", &body).unwrap();
    out
}

/// Time `iters` runs of `f` and report ns/op plus throughput over
/// `bytes` input bytes per op.
fn bench(label: &str, iters: u32, bytes: usize, mut f: impl FnMut()) {
    // Warm-up (let the allocator / caches settle).
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed.as_secs_f64() / iters as f64;
    let mb_s = (bytes as f64) / per_op / (1024.0 * 1024.0);
    println!(
        "{label:<26} {:>10.0} ns/op   {:>8.1} MiB/s   ({} B input)",
        per_op * 1e9,
        mb_s,
        bytes,
    );
}

fn main() {
    let file = build_file(4096);
    let len = file.len();
    println!("RIFF hot-path micro-benchmark — {len} B synthetic WAVE\n");

    bench("walk_skip", 2_000, len, || {
        let mut cur = Cursor::new(&file);
        let mut w = Walker::open_root(&mut cur).unwrap();
        while let Some(c) = w.read_next().unwrap() {
            w.skip(&c).unwrap();
            black_box(&c);
        }
    });

    bench("walk_read_body", 2_000, len, || {
        let mut cur = Cursor::new(&file);
        let mut w = Walker::open_root(&mut cur).unwrap();
        while let Some(c) = w.read_next().unwrap() {
            let body = w.read_body(&c).unwrap();
            black_box(&body);
        }
    });

    bench("tree_parse", 2_000, len, || {
        let tree = RiffTree::parse(&file).unwrap();
        black_box(&tree);
    });

    let tree = RiffTree::parse(&file).unwrap();
    bench("tree_encode", 2_000, len, || {
        let out = tree.encode().unwrap();
        black_box(&out);
    });
}
