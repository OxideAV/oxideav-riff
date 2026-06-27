//! End-to-end **mux → walk → decode** round-trip fixtures.
//!
//! These tests exercise the crate's write-side surface as a whole: they
//! assemble a complete RIFF/WAVE (and a BW64/RF64) file purely from the
//! per-chunk `encode_*` helpers + the shared `encode_chunk` framing, then
//! parse it straight back with the public [`Walker`] and the typed chunk
//! decoders, asserting every decoded struct equals the one that was
//! encoded. This proves the encoders and decoders are exact inverses at
//! the *file* level, not just per chunk — the milestone the round drove
//! toward.
//!
//! No bytes are read from disk; the fixture is built in memory from the
//! public API, so the test doubles as a worked example of muxing a WAV
//! file.

use std::io::Cursor;

use std::io::Read;

use oxideav_riff::{
    encode_chunk, fourcc_to_string, is_rf64_magic, read_chunk_header, read_form_type, Acid,
    AdtlEntry, AdtlList, ChannelAllocation, CueChunk, CuePoint, DataSize64, Fact, InfoList,
    InfoTag, Inst, Junk, RiffChunk, RiffTree, Smpl, Walker, WaveDataList, WaveFile, WaveFormat,
};

/// Wrap an already-serialized LIST *body* (the form-type word + children)
/// in a fresh `LIST` chunk and collect it with a nested walker; returns
/// the decoded [`InfoList`].
fn collect_info_from_blob(list_chunk: &[u8]) -> InfoList {
    let mut cur = Cursor::new(list_chunk);
    let header = read_chunk_header(&mut cur).unwrap().unwrap();
    let mut walker = Walker::open_within(&mut cur, &header).unwrap();
    let mut list = InfoList::new();
    while let Some(child) = walker.read_next().unwrap() {
        let body = walker.read_body(&child).unwrap();
        list.push_chunk(child.id, &body);
    }
    list
}

/// As [`collect_info_from_blob`] but for a `LIST adtl` chunk.
fn collect_adtl_from_blob(list_chunk: &[u8]) -> AdtlList {
    let mut cur = Cursor::new(list_chunk);
    let header = read_chunk_header(&mut cur).unwrap().unwrap();
    let mut walker = Walker::open_within(&mut cur, &header).unwrap();
    let mut list = AdtlList::new();
    while let Some(child) = walker.read_next().unwrap() {
        let body = walker.read_body(&child).unwrap();
        list.push_chunk(child.id, &body).unwrap();
    }
    list
}

/// Re-assemble the full on-wire bytes of a leaf chunk from its
/// [`oxideav_riff::ChunkRef`] + body (header + body + pad). Used to feed a
/// `LIST` chunk back into a nested walker.
fn rebuild_chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    encode_chunk(&mut out, id, body).unwrap();
    out
}

#[test]
fn full_wave_file_muxes_and_round_trips() {
    // --- Build the typed source structures. ---
    let fmt = WaveFormat {
        format_tag: 1,
        channels: 2,
        sample_rate: 44_100,
        avg_bytes_per_sec: 176_400,
        block_align: 4,
        bits_per_sample: 16,
        extension: Vec::new(),
        extensible: None,
    };
    let fact = Fact {
        sample_length: 4,
        extra: Vec::new(),
    };
    let data: Vec<u8> = (0u8..16).collect();

    let cue = CueChunk::from_points(vec![
        CuePoint {
            name: 1,
            position: 0,
            fcc_chunk: *b"data",
            chunk_start: 0,
            block_start: 0,
            sample_offset: 0,
        },
        CuePoint {
            name: 2,
            position: 2,
            fcc_chunk: *b"data",
            chunk_start: 0,
            block_start: 0,
            sample_offset: 2,
        },
    ]);

    let info = InfoList::new()
        .push(InfoTag::INAM, "Two Trees")
        .push(InfoTag::IART, "Jane Doe") // odd ZSTR → exercises a pad byte
        .push(InfoTag::ISFT, "oxideav")
        .push(InfoTag::IENC, "oxideav-riff") // extended namespace tag
        .push(InfoTag::ITRK, "7");

    // An alignment-filler chunk with an odd body, to exercise JUNK
    // passthrough + the trailing RIFF pad byte mid-file.
    let junk = Junk::from_body(&[0xCA, 0xFE, 0xBA]);

    let adtl = AdtlList::new()
        .push(AdtlEntry::Label {
            name: 1,
            text: "Start".to_string(),
        })
        .push(AdtlEntry::Note {
            name: 2,
            text: "midpoint".to_string(),
        });

    let smpl = Smpl {
        sample_period: 22_675,
        midi_unity_note: 60,
        loops: Vec::new(),
        sampler_data: Vec::new(),
        ..Smpl::default()
    };
    let inst = Inst {
        unshifted_note: 60,
        low_note: 0,
        high_note: 127,
        low_velocity: 0,
        high_velocity: 127,
        ..Inst::default()
    };
    let acid = Acid {
        flags: 0x02 | 0x04, // root-note-set | stretch
        root_note: 0x3C,
        unknown1: 0x8000,
        num_beats: 8,
        meter_denominator: 4,
        meter_numerator: 4,
        tempo: 138.0,
        ..Acid::default()
    };

    // --- Mux: assemble RIFF/WAVE { fmt fact data cue LIST(INFO) LIST(adtl)
    //     smpl inst acid }. ---
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    encode_chunk(&mut body, b"fmt ", &fmt.encode_body()).unwrap();
    encode_chunk(&mut body, b"fact", &fact.encode_body()).unwrap();
    encode_chunk(&mut body, b"data", &data).unwrap();
    body.extend_from_slice(&junk.encode_chunk().unwrap());
    encode_chunk(&mut body, b"cue ", &cue.encode_body()).unwrap();
    body.extend_from_slice(&info.encode_chunk().unwrap());
    body.extend_from_slice(&adtl.encode_chunk().unwrap());
    encode_chunk(&mut body, b"smpl", &smpl.encode_body()).unwrap();
    encode_chunk(&mut body, b"inst", &inst.encode_body()).unwrap();
    encode_chunk(&mut body, b"acid", &acid.encode_body()).unwrap();
    let mut file = Vec::new();
    encode_chunk(&mut file, b"RIFF", &body).unwrap();

    // --- Walk it back and decode each chunk. ---
    let mut cur = Cursor::new(&file[..]);
    let mut walker = Walker::open_root(&mut cur).unwrap();
    assert_eq!(&walker.form_type(), b"WAVE");

    let mut seen = Vec::new();
    let mut got_fmt = None;
    let mut got_fact = None;
    let mut got_data = None;
    let mut got_cue = None;
    let mut got_info = None;
    let mut got_adtl = None;
    let mut got_smpl = None;
    let mut got_inst = None;
    let mut got_acid = None;
    let mut got_junk = None;

    while let Some(chunk) = walker.read_next().unwrap() {
        seen.push(fourcc_to_string(&chunk.id));
        let raw = walker.read_body(&chunk).unwrap();
        match &chunk.id {
            b"fmt " => got_fmt = Some(WaveFormat::parse(&raw).unwrap()),
            b"fact" => got_fact = Some(Fact::parse(&raw).unwrap()),
            b"data" => got_data = Some(raw),
            b"JUNK" => got_junk = Some(Junk::from_body(&raw)),
            b"cue " => got_cue = Some(CueChunk::parse(&raw).unwrap()),
            b"smpl" => got_smpl = Some(Smpl::parse(&raw).unwrap()),
            b"inst" => got_inst = Some(Inst::parse(&raw).unwrap()),
            b"acid" => got_acid = Some(Acid::parse(&raw).unwrap()),
            b"LIST" => {
                // `raw` is the LIST body (form-type word + children).
                // Rebuild the LIST chunk and dispatch by list-type.
                let list_chunk = rebuild_chunk(b"LIST", &raw);
                if &raw[0..4] == b"INFO" {
                    got_info = Some(collect_info_from_blob(&list_chunk));
                } else if &raw[0..4] == b"adtl" {
                    got_adtl = Some(collect_adtl_from_blob(&list_chunk));
                } else {
                    panic!("unexpected LIST type {:?}", &raw[0..4]);
                }
            }
            other => panic!("unexpected chunk {other:?}"),
        }
    }

    assert_eq!(
        seen,
        vec!["fmt ", "fact", "data", "JUNK", "cue ", "LIST", "LIST", "smpl", "inst", "acid"]
    );
    assert_eq!(got_fmt.unwrap(), fmt);
    assert_eq!(got_fact.unwrap(), fact);
    assert_eq!(got_data.unwrap(), data);
    assert_eq!(got_junk.unwrap(), junk);
    assert_eq!(got_cue.unwrap(), cue);
    let got_info = got_info.unwrap();
    assert_eq!(got_info, info);
    // The extended-namespace tags survive the read→mux→read cycle.
    assert_eq!(got_info.get(InfoTag::IENC), Some("oxideav-riff"));
    assert_eq!(got_info.get(InfoTag::ITRK), Some("7"));
    assert!(InfoTag::IENC.is_extended());
    assert_eq!(got_adtl.unwrap(), adtl);
    assert_eq!(got_smpl.unwrap(), smpl);
    assert_eq!(got_inst.unwrap(), inst);
    assert_eq!(got_acid.unwrap(), acid);
}

#[test]
fn waveformatextensible_file_round_trips() {
    // A 5.1 / 24-in-32 PCM file driven entirely through the extensible
    // descriptor encoder.
    let fmt = WaveFormat::parse(&{
        let mut v = Vec::new();
        v.extend_from_slice(&0xFFFEu16.to_le_bytes());
        v.extend_from_slice(&6u16.to_le_bytes());
        v.extend_from_slice(&48_000u32.to_le_bytes());
        v.extend_from_slice(&864_000u32.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes());
        v.extend_from_slice(&32u16.to_le_bytes());
        v.extend_from_slice(&22u16.to_le_bytes());
        v.extend_from_slice(&24u16.to_le_bytes());
        v.extend_from_slice(&0x0000_003Fu32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&0x0010u16.to_le_bytes());
        v.extend_from_slice(&[0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
        v
    })
    .unwrap();

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    encode_chunk(&mut body, b"fmt ", &fmt.encode_body()).unwrap();
    encode_chunk(&mut body, b"data", &[0u8; 24]).unwrap();
    let mut file = Vec::new();
    encode_chunk(&mut file, b"RIFF", &body).unwrap();

    let mut cur = Cursor::new(&file[..]);
    let mut walker = Walker::open_root(&mut cur).unwrap();
    let chunk = walker.read_next().unwrap().unwrap();
    assert_eq!(&chunk.id, b"fmt ");
    let back = WaveFormat::parse(&walker.read_body(&chunk).unwrap()).unwrap();
    assert_eq!(back, fmt);
    assert!(back.is_extensible());
    assert_eq!(back.effective_format_tag(), Some(1)); // PCM
}

#[test]
fn bw64_rf64_file_with_ds64_chna_round_trips() {
    // A BW64 (ADM) file: BW64 form WAVE, ds64 first (mandatory), then the
    // chna ADM channel-allocation table. The public Walker::open_root is
    // strict on 'RIFF', so the 64-bit outer wrapper is parsed manually via
    // read_chunk_header + Walker::open_within (the documented path until a
    // dedicated RF64 constructor lands).
    let ds64 = DataSize64::new(
        5_368_709_120 + 1024, // riffSize
        5_368_709_120,        // dataSize (~5 GiB)
        1_342_177_280,        // sampleCount
        Vec::new(),
    );
    let chna = ChannelAllocation::from_records(
        2,
        2,
        vec![
            mk_audio_id(1, b"ATU_00000001", b"AT_00010001_01", b"AP_00010002"),
            mk_audio_id(2, b"ATU_00000002", b"AT_00010002_01", b"AP_00010002"),
        ],
    );

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    encode_chunk(&mut body, b"ds64", &ds64.encode_body()).unwrap();
    encode_chunk(&mut body, b"chna", &chna.encode_body()).unwrap();
    let mut file = Vec::new();
    encode_chunk(&mut file, b"BW64", &body).unwrap();

    // Read the outer BW64 header + form type manually, then iterate the
    // child chunk headers. (Walker::open_root/open_within are strict on the
    // RIFF/LIST group bit, so a BW64 outer wrapper is walked with the
    // low-level read_chunk_header API until a dedicated RF64 constructor
    // lands.)
    let mut cur = Cursor::new(&file[..]);
    let outer = read_chunk_header(&mut cur).unwrap().unwrap();
    assert!(is_rf64_magic(&outer.id));
    let form = read_form_type(&mut cur).unwrap();
    assert_eq!(&form, b"WAVE");

    let mut got_ds64 = None;
    let mut got_chna = None;
    while let Some(child) = read_chunk_header(&mut cur).unwrap() {
        let mut raw = vec![0u8; child.size as usize];
        cur.read_exact(&mut raw).unwrap();
        if child.size & 1 == 1 {
            let mut pad = [0u8; 1];
            cur.read_exact(&mut pad).unwrap();
        }
        match &child.id {
            b"ds64" => got_ds64 = Some(DataSize64::parse(&raw).unwrap()),
            b"chna" => got_chna = Some(ChannelAllocation::parse(&raw).unwrap()),
            other => panic!("unexpected chunk {other:?}"),
        }
    }
    assert_eq!(got_ds64.unwrap(), ds64);
    let chna_back = got_chna.unwrap();
    assert_eq!(chna_back, chna);
    assert_eq!(chna_back.active_count(), 2);
    assert_eq!(
        chna_back.by_track_index(1).unwrap().uid_str(),
        Some("ATU_00000001")
    );
}

#[test]
fn bw64_file_walks_through_the_open_bw64_constructor() {
    // A self-consistent BW64 file: the outer 32-bit ckSize carries the
    // 0xFFFFFFFF sentinel and the mandatory ds64 chunk's riffSize equals
    // the real body length, so the high-level Walker::open_bw64 resolves
    // the 64-bit size and yields the post-ds64 chunks directly. This is
    // the constructor that replaces the manual read_chunk_header loop the
    // sibling fixture documents.
    let fmt = WaveFormat {
        format_tag: 1,
        channels: 2,
        sample_rate: 48_000,
        avg_bytes_per_sec: 192_000,
        block_align: 4,
        bits_per_sample: 16,
        extension: Vec::new(),
        extensible: None,
    };
    let fmt_chunk = {
        let mut v = Vec::new();
        encode_chunk(&mut v, b"fmt ", &fmt.encode_body()).unwrap();
        v
    };
    let chna = ChannelAllocation::from_records(
        1,
        1,
        vec![mk_audio_id(
            1,
            b"ATU_00000001",
            b"AT_00010001_01",
            b"AP_00010002",
        )],
    );
    let chna_chunk = {
        let mut v = Vec::new();
        encode_chunk(&mut v, b"chna", &chna.encode_body()).unwrap();
        v
    };
    let data = [0x11u8, 0x22, 0x33, 0x44];
    let data_chunk = {
        let mut v = Vec::new();
        encode_chunk(&mut v, b"data", &data).unwrap();
        v
    };

    // Compute the real body size = WAVE(4) + ds64 chunk + fmt + chna + data.
    // ds64 is a fixed 28-byte body → 8 + 28 = 36 framed bytes.
    let body_len = 4 + (8 + 28) + fmt_chunk.len() + chna_chunk.len() + data_chunk.len();
    let ds64 = DataSize64::new(body_len as u64, data.len() as u64, 1, Vec::new());

    // Assemble the BW64 file with the outer sentinel size.
    let mut file = Vec::new();
    file.extend_from_slice(b"BW64");
    file.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // sentinel
    file.extend_from_slice(b"WAVE");
    encode_chunk(&mut file, b"ds64", &ds64.encode_body()).unwrap();
    file.extend_from_slice(&fmt_chunk);
    file.extend_from_slice(&chna_chunk);
    file.extend_from_slice(&data_chunk);

    // Walk it through the high-level constructor.
    let mut cur = Cursor::new(&file[..]);
    let mut walker = Walker::open_bw64(&mut cur).unwrap();
    assert_eq!(&walker.form_type(), b"WAVE");
    // The constructor resolved the 64-bit size from ds64.
    assert_eq!(walker.data_size_64().unwrap().riff_size, body_len as u64);

    // ds64 is already consumed; the first yielded chunk is fmt.
    let c = walker.read_next().unwrap().unwrap();
    assert_eq!(&c.id, b"fmt ");
    let got_fmt = WaveFormat::parse(&walker.read_body(&c).unwrap()).unwrap();
    assert_eq!(got_fmt, fmt);

    let c = walker.read_next().unwrap().unwrap();
    assert_eq!(&c.id, b"chna");
    let got_chna = ChannelAllocation::parse(&walker.read_body(&c).unwrap()).unwrap();
    assert_eq!(got_chna, chna);

    let c = walker.read_next().unwrap().unwrap();
    assert_eq!(&c.id, b"data");
    assert_eq!(walker.read_body(&c).unwrap(), data);

    // Budget exactly satisfied.
    assert!(walker.read_next().unwrap().is_none());
}

/// As [`collect_info_from_blob`] but for a `wavl` wave-data-list chunk.
fn collect_wavl_from_blob(list_chunk: &[u8]) -> WaveDataList {
    let mut cur = Cursor::new(list_chunk);
    let header = read_chunk_header(&mut cur).unwrap().unwrap();
    let mut walker = Walker::open_within(&mut cur, &header).unwrap();
    WaveDataList::collect_from(&mut walker).unwrap()
}

#[test]
fn wavl_wave_data_list_file_muxes_and_round_trips() {
    // A WAV file whose waveform is stored as a `wavl` LIST of alternating
    // `data` (sample) and `slnt` (silence) chunks instead of a single bare
    // `data` chunk. The spec requires a `fact` chunk in this form; its
    // `dwSampleLength` carries the total sample count the reader cannot
    // derive cheaply from the scattered runs and silence counts.
    let fmt = WaveFormat {
        format_tag: 1,
        channels: 1,
        sample_rate: 48_000,
        avg_bytes_per_sec: 96_000,
        block_align: 2,
        bits_per_sample: 16,
        extension: Vec::new(),
        extensible: None,
    };

    // 6 sample bytes + 1000 silent samples + 5 sample bytes (odd → pad).
    let mut wavl = WaveDataList::new();
    wavl.push_data(vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    wavl.push_silence(1_000);
    wavl.push_data(vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE]); // odd-length run
    assert_eq!(wavl.total_data_bytes(), 11);
    assert_eq!(wavl.total_silent_samples(), 1_000);

    // 3 + 1000 + 2 = 1005 mono 16-bit samples total (sample_length is the
    // per-channel sample count: 11 bytes / 2 bytes-per-sample rounds the
    // PCM runs to 3 + 2 samples here for the fixture).
    let fact = Fact {
        sample_length: 1_005,
        extra: Vec::new(),
    };

    // --- Mux: RIFF/WAVE { fmt fact LIST(wavl) }. ---
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    encode_chunk(&mut body, b"fmt ", &fmt.encode_body()).unwrap();
    encode_chunk(&mut body, b"fact", &fact.encode_body()).unwrap();
    body.extend_from_slice(&wavl.encode_chunk().unwrap());
    let mut file = Vec::new();
    encode_chunk(&mut file, b"RIFF", &body).unwrap();

    // --- Walk it back and decode each chunk. ---
    let mut cur = Cursor::new(&file[..]);
    let mut walker = Walker::open_root(&mut cur).unwrap();
    assert_eq!(&walker.form_type(), b"WAVE");

    let mut seen = Vec::new();
    let mut got_fmt = None;
    let mut got_fact = None;
    let mut got_wavl = None;

    while let Some(chunk) = walker.read_next().unwrap() {
        seen.push(fourcc_to_string(&chunk.id));
        let raw = walker.read_body(&chunk).unwrap();
        match &chunk.id {
            b"fmt " => got_fmt = Some(WaveFormat::parse(&raw).unwrap()),
            b"fact" => got_fact = Some(Fact::parse(&raw).unwrap()),
            b"LIST" => {
                assert_eq!(&raw[0..4], b"wavl");
                let list_chunk = rebuild_chunk(b"LIST", &raw);
                got_wavl = Some(collect_wavl_from_blob(&list_chunk));
            }
            other => panic!("unexpected chunk {other:?}"),
        }
    }

    assert_eq!(seen, vec!["fmt ", "fact", "LIST"]);
    assert_eq!(got_fmt.unwrap(), fmt);
    assert_eq!(got_fact.unwrap(), fact);
    let wavl_back = got_wavl.unwrap();
    assert_eq!(wavl_back, wavl);
    // The decoded list re-tallies to the same totals (so the odd-length
    // data run survived its RIFF pad byte intact).
    assert_eq!(wavl_back.total_data_bytes(), 11);
    assert_eq!(wavl_back.total_silent_samples(), 1_000);
    assert_eq!(wavl_back.len(), 3);
}

/// Build an `AudioId` with NUL-padded fixed-width identifier fields.
fn mk_audio_id(
    track_index: u16,
    uid: &[u8],
    track_ref: &[u8],
    pack_ref: &[u8],
) -> oxideav_riff::AudioId {
    let mut uid_f = [0u8; 12];
    uid_f[..uid.len()].copy_from_slice(uid);
    let mut tr_f = [0u8; 14];
    tr_f[..track_ref.len()].copy_from_slice(track_ref);
    let mut pr_f = [0u8; 11];
    pr_f[..pack_ref.len()].copy_from_slice(pack_ref);
    oxideav_riff::AudioId {
        track_index,
        uid: uid_f,
        track_ref: tr_f,
        pack_ref: pr_f,
        pad: 0,
    }
}

/// Build a full RIFF/WAVE file, parse it into the owned [`RiffTree`],
/// decode the typed [`WaveFile`] view, and confirm both the typed
/// fields and the byte-exact tree round-trip — the round-376 capstone
/// tying the recursive tree + typed-view layers to the per-chunk
/// decoders.
#[test]
fn tree_and_wavefile_view_round_trip_a_full_wave() {
    // fmt : 16-byte WAVEFORMAT, PCM stereo 44.1 kHz 16-bit.
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&2u16.to_le_bytes()); // channels
    fmt.extend_from_slice(&44_100u32.to_le_bytes());
    fmt.extend_from_slice(&176_400u32.to_le_bytes());
    fmt.extend_from_slice(&4u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&16u16.to_le_bytes()); // bits

    // fact: 4-byte sample length.
    let fact = 2048u32.to_le_bytes().to_vec();

    // LIST INFO { INAM "Title" IART "Artist" }
    let mut info = Vec::new();
    info.extend_from_slice(b"INFO");
    encode_chunk(&mut info, b"INAM", b"Title\0").unwrap();
    encode_chunk(&mut info, b"IART", b"Artist\0").unwrap();

    // LIST adtl { labl: id + "marker\0" }
    let mut labl = Vec::new();
    labl.extend_from_slice(&1u32.to_le_bytes());
    labl.extend_from_slice(b"marker\0");
    let mut adtl = Vec::new();
    adtl.extend_from_slice(b"adtl");
    encode_chunk(&mut adtl, b"labl", &labl).unwrap();

    // A vendor chunk the WaveFile view does NOT model — must survive.
    let vendor = b"vendor-private-blob".to_vec();

    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    encode_chunk(&mut body, b"fmt ", &fmt).unwrap();
    encode_chunk(&mut body, b"fact", &fact).unwrap();
    encode_chunk(&mut body, b"data", &[0u8; 64]).unwrap();
    encode_chunk(&mut body, b"LIST", &info).unwrap();
    encode_chunk(&mut body, b"LIST", &adtl).unwrap();
    encode_chunk(&mut body, b"vnd ", &vendor).unwrap();
    let mut bytes = Vec::new();
    encode_chunk(&mut bytes, b"RIFF", &body).unwrap();

    // Parse into the owned tree; byte-exact re-encode.
    let tree = RiffTree::parse(&bytes).unwrap();
    assert_eq!(&tree.form_type, b"WAVE");
    assert_eq!(tree.encode().unwrap(), bytes);

    // Typed view decodes the modelled chunks.
    let wav = WaveFile::from_tree(&tree).unwrap();
    let f = wav.format.as_ref().unwrap();
    assert_eq!(f.channels, 2);
    assert_eq!(f.sample_rate, 44_100);
    assert_eq!(wav.fact.as_ref().unwrap().sample_length, 2048);
    assert_eq!(wav.info.as_ref().unwrap().len(), 2);
    assert_eq!(wav.adtl.as_ref().unwrap().entries().len(), 1);

    // The vendor chunk the view ignores is still in the tree, verbatim.
    match tree.find(b"vnd ").unwrap() {
        RiffChunk::Leaf { body, .. } => assert_eq!(body, b"vendor-private-blob"),
        _ => panic!("vnd should be a leaf"),
    }

    // Reading the same bytes through from_reader yields an equal tree.
    let mut cur = Cursor::new(&bytes);
    let from_rdr = RiffTree::from_reader(&mut cur).unwrap();
    assert_eq!(from_rdr, tree);
}
