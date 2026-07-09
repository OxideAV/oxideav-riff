//! Enumerated hostile-input hardening tests.
//!
//! The generative `fuzz_smoke` harness proves *panic-freedom* over a
//! random corpus; this suite pins the *specific typed-error behaviour* on
//! hand-crafted adversarial structures the fuzzer would only hit rarely:
//! the exact [`MAX_DEPTH`] nesting cutoff, a byte-by-byte truncation
//! sweep of a real multi-chunk file, integer-boundary `ckSize` values,
//! and a nested-group budget overflow driven through the streaming
//! [`Walker`]. Each asserts a *typed* `Error` (never a panic, never a
//! silent partial accept).

use std::io::Cursor;

use oxideav_riff::{encode_chunk, RiffTree, Walker, MAX_DEPTH};

/// Build a RIFF/WAVE file with `n` `LIST`-within-`LIST` groups nested
/// inside the outer `RIFF`. The innermost group is empty (form type
/// only). Mirrors the wire layout the parser descends recursively.
fn nested_lists(n: usize) -> Vec<u8> {
    let mut inner = Vec::new();
    inner.extend_from_slice(b"sub "); // innermost group body = form type only
    for _ in 0..n {
        let mut wrap = Vec::new();
        wrap.extend_from_slice(b"sub ");
        encode_chunk(&mut wrap, b"LIST", &inner).unwrap();
        inner = wrap;
    }
    let mut bytes = Vec::new();
    encode_chunk(&mut bytes, b"RIFF", &inner).unwrap();
    bytes
}

#[test]
fn max_depth_cutoff_is_exact() {
    // A file with `MAX_DEPTH - 1` nested LISTs sits just inside the guard
    // and must parse; one more level trips it. This pins the off-by-one
    // that a bare `> MAX_DEPTH + 2` smoke test leaves unspecified.
    let ok = nested_lists(MAX_DEPTH - 1);
    assert!(
        RiffTree::parse(&ok).is_ok(),
        "MAX_DEPTH-1 nesting must parse"
    );

    let too_deep = nested_lists(MAX_DEPTH);
    let err = RiffTree::parse(&too_deep).unwrap_err();
    assert!(
        format!("{err}").contains("MAX_DEPTH"),
        "MAX_DEPTH nesting must be rejected with a depth error"
    );
}

/// A realistic multi-chunk RIFF/WAVE file used as the truncation base.
fn sample_wave() -> Vec<u8> {
    let mut info = Vec::new();
    info.extend_from_slice(b"INFO");
    encode_chunk(&mut info, b"INAM", b"Title").unwrap(); // odd body → pad
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    encode_chunk(&mut body, b"fmt ", &[0x01, 0x00, 0x02, 0x00]).unwrap();
    encode_chunk(&mut body, b"data", &[0xAA, 0xBB, 0xCC]).unwrap(); // odd → pad
    encode_chunk(&mut body, b"LIST", &info).unwrap();
    let mut out = Vec::new();
    encode_chunk(&mut out, b"RIFF", &body).unwrap();
    out
}

#[test]
fn every_truncation_prefix_is_typed_error_or_clean_parse() {
    // Cut the file at every possible length. The full file parses; every
    // shorter prefix must either parse cleanly (a shorter-but-consistent
    // outer ckSize is legal) or return a typed Error — but must never
    // panic and never over-read. The tree path and the streaming walker
    // path are both exercised.
    let full = sample_wave();
    for cut in 0..full.len() {
        let prefix = &full[..cut];

        // Owned tree: any Ok must round-trip; any failure is a typed Err.
        if let Ok(tree) = RiffTree::parse(prefix) {
            let enc = tree.encode().expect("parsed tree must re-encode");
            // The canonical encoding must be a prefix-consistent reparse.
            let re = RiffTree::parse(&enc).expect("re-encode must parse");
            assert_eq!(re, tree, "truncated-prefix round-trip mismatch at {cut}");
        }

        // Streaming walker: drive it to exhaustion; typed Err or clean end.
        let mut cur = Cursor::new(prefix);
        if let Ok(mut w) = Walker::open_root(&mut cur) {
            // Exits on Ok(None) (clean end) or Err (typed failure).
            while let Ok(Some(c)) = w.read_next() {
                if w.read_body(&c).is_err() {
                    break;
                }
            }
        }
    }

    // The untruncated file must fully parse and round-trip byte-exact.
    let tree = RiffTree::parse(&full).unwrap();
    assert_eq!(tree.encode().unwrap(), full);
}

#[test]
fn child_cksize_u32_max_is_rejected_not_overflowed() {
    // A leaf claiming a u32::MAX body inside a small parent. The size
    // arithmetic must reject it (overflows parent) without wrapping or
    // panicking.
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    body.extend_from_slice(b"data");
    body.extend_from_slice(&u32::MAX.to_le_bytes());
    body.extend_from_slice(&[0u8; 4]); // only 4 real bytes follow
    let mut bytes = Vec::new();
    encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
    let err = RiffTree::parse(&bytes).unwrap_err();
    assert!(
        format!("{err}").contains("overflows parent"),
        "u32::MAX child ckSize must be rejected as a parent overflow"
    );
}

#[test]
fn outer_cksize_of_exactly_four_is_empty_tree() {
    // The minimal legal RIFF: outer ckSize = 4 (just the form type, no
    // children). Must parse to an empty child list and round-trip.
    let bytes = [
        b'R', b'I', b'F', b'F', 0x04, 0x00, 0x00, 0x00, b'W', b'A', b'V', b'E',
    ];
    let tree = RiffTree::parse(&bytes).unwrap();
    assert_eq!(&tree.form_type, b"WAVE");
    assert!(tree.children.is_empty());
    assert_eq!(tree.encode().unwrap(), bytes);
}

#[test]
fn nested_group_overflowing_parent_budget_is_rejected() {
    // A top-level LIST whose declared ckSize exceeds the outer RIFF's
    // remaining budget. The streaming walker must reject the child, not
    // read past the parent.
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    // Outer payload budget = form type (4) + one child header (8) = 12.
    v.extend_from_slice(&12u32.to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"LIST");
    v.extend_from_slice(&1_000_000u32.to_le_bytes()); // lies about its size
    let mut cur = Cursor::new(v);
    let mut w = Walker::open_root(&mut cur).unwrap();
    let err = w.read_next().unwrap_err();
    assert!(
        format!("{err}").contains("overflows parent"),
        "an over-budget nested group must be rejected"
    );
}

#[test]
fn odd_body_missing_its_pad_at_parent_boundary_is_truncation() {
    // A parent whose declared ckSize accounts for an odd-length child's
    // body but stops before its mandatory pad byte. The pad keeps the
    // next header 2-byte aligned; a parent that ends mid-pad lied about
    // its length and must surface a typed truncation error rather than
    // silently dropping the alignment.
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    // odd 3-byte child, then a would-be sibling header the budget can't
    // hold in full.
    body.extend_from_slice(b"odd ");
    body.extend_from_slice(&3u32.to_le_bytes());
    body.extend_from_slice(&[1, 2, 3]);
    body.push(0); // pad
    body.extend_from_slice(b"xyz"); // 3 stray bytes < a header
    let mut bytes = Vec::new();
    encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
    let err = RiffTree::parse(&bytes).unwrap_err();
    assert!(
        format!("{err}").contains("too short for a chunk header"),
        "trailing sub-header bytes must be a typed truncation error"
    );
}
