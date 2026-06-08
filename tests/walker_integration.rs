//! End-to-end walker integration test.
//!
//! Builds a synthetic RIFF/WAVE-shaped file with a top-level
//! `fmt ` chunk, a nested `LIST INFO` containing a single `INAM`
//! tag, and a `data` chunk with an odd payload length (to exercise
//! the pad byte), then walks it end-to-end with the public API.

use std::io::Cursor;

use oxideav_riff::{fourcc_to_string, Walker};

fn synthetic_wav_with_list_info() -> Vec<u8> {
    // Layout (bytes):
    //   0..4   "RIFF"
    //   4..8   outer ckSize (LE u32)
    //   8..12  "WAVE"  (form type — counts against outer ckSize)
    //
    //   "fmt " + ckSize=4 + body=[01 00 02 00]
    //   "LIST" + ckSize=16 + "INFO" + "INAM" + ckSize=4 + "Hi!\0"
    //   "data" + ckSize=5 + 5-byte body + 1 pad byte
    //
    //   outer ckSize accounts for everything after itself = 4 (WAVE)
    //   + 8 (fmt hdr) + 4 (fmt body) + 8 (LIST hdr) + 16 (LIST body)
    //   + 8 (data hdr) + 5 (data body) + 1 (pad) = 54
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&54u32.to_le_bytes());
    v.extend_from_slice(b"WAVE");

    // fmt
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend_from_slice(&[0x01, 0x00, 0x02, 0x00]);

    // LIST INFO { INAM "Hi!\0" }
    v.extend_from_slice(b"LIST");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(b"INFO");
    v.extend_from_slice(b"INAM");
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend_from_slice(b"Hi!\0");

    // data (odd-length body → pad byte)
    v.extend_from_slice(b"data");
    v.extend_from_slice(&5u32.to_le_bytes());
    v.extend_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x50]);
    v.push(0); // pad

    v
}

#[test]
fn end_to_end_walk_lists_every_top_level_chunk_in_order() {
    let bytes = synthetic_wav_with_list_info();
    let mut cur = Cursor::new(&bytes[..]);
    let mut outer = Walker::open_root(&mut cur).unwrap();
    assert_eq!(&outer.form_type(), b"WAVE");

    // 1) fmt — read its body to confirm the LE-decoded ckSize matched
    //    the actual on-wire payload.
    let fmt = outer.read_next().unwrap().unwrap();
    assert_eq!(&fmt.id, b"fmt ");
    assert_eq!(fmt.size, 4);
    let fmt_body = outer.read_body(&fmt).unwrap();
    assert_eq!(fmt_body, vec![0x01, 0x00, 0x02, 0x00]);

    // 2) LIST(INFO) — confirm the group bit + form-type word decode,
    //    then skip the LIST body (the recursive walker that would
    //    descend into the `INAM` sub-child is deferred to a later
    //    round; this test just exercises the top-level walker).
    let list = outer.read_next().unwrap().unwrap();
    assert_eq!(&list.id, b"LIST");
    assert!(list.is_group());
    let info_type = outer.read_inner_form_type(&list).unwrap();
    assert_eq!(&info_type, b"INFO");
    // The remaining LIST body is `list.size - 4` bytes (we already
    // consumed 4 for the form-type word). Build a synthetic ChunkRef
    // with that residual length so we can use Walker::skip() to
    // advance both the reader and the parent-budget counter in one
    // call rather than driving the cursor and the bookkeeping by
    // hand.
    let residual_len = list.size - 4;
    let residual = oxideav_riff::ChunkRef {
        id: *b"____",
        size: residual_len,
        body_offset: 0,
    };
    outer.skip(&residual).unwrap();

    // 3) data — odd-length body, padded_size() reports 6.
    let data = outer.read_next().unwrap().unwrap();
    assert_eq!(&data.id, b"data");
    assert_eq!(data.size, 5);
    assert_eq!(data.padded_size(), 6);
    let data_body = outer.read_body(&data).unwrap();
    assert_eq!(data_body, vec![0x10, 0x20, 0x30, 0x40, 0x50]);

    // Walked the whole parent budget exactly.
    assert!(outer.read_next().unwrap().is_none());
}

#[test]
fn fourcc_helper_round_trips_through_the_walker() {
    let bytes = synthetic_wav_with_list_info();
    let mut cur = Cursor::new(&bytes[..]);
    let mut outer = Walker::open_root(&mut cur).unwrap();

    let mut seen = Vec::new();
    while let Some(child) = outer.read_next().unwrap() {
        seen.push(fourcc_to_string(&child.id));
        outer.skip(&child).unwrap();
    }
    assert_eq!(
        seen,
        vec!["fmt ".to_string(), "LIST".to_string(), "data".to_string()]
    );
}
