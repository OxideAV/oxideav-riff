//! Clean-room **RIFF** (Resource Interchange File Format) primitives.
//!
//! RIFF is the chunked little-endian container format Microsoft and IBM
//! published in 1991 as part of the *Multimedia Programming Interface and
//! Data Specifications 1.0*. It is the carrier for WAV (RIFF/WAVE), AVI
//! (RIFF/AVI ), WebP (RIFF/WEBP), AMV (RIFF-like, big-endian sizes), the
//! Windows ANI cursor format (RIFF/ACON), and several others.
//!
//! This crate scopes itself to the **shared chunk-walking primitives**
//! that every RIFF-family parser needs: a `ChunkHeader` decoder, an
//! `Iter`-style walker over the top-level RIFF/LIST tree, FourCC
//! helpers, and a `Reader` wrapper that tracks the current chunk's
//! body length so consumers don't accidentally read past it. Codec-
//! specific chunks (`fmt `, `data`, `LIST INFO`, BWF `bext`, RF64
//! `ds64`, the AVI `idx1` index, the WebP `VP8 ` / `VP8L` payloads,
//! …) are intentionally **not** parsed here — they live in their own
//! codec crates which call into the walker.
//!
//! Round 257 lands the chunk-walker as the foundation; round 267
//! stacks the WAV `fmt`-chunk decoder ([`waveformat::WaveFormat`])
//! with its `WAVEFORMATEX` + `WAVEFORMATEXTENSIBLE` sub-fields and the
//! `DEFINE_WAVEFORMATEX_GUID` sub-format resolver on top; round 275 adds
//! the `LIST INFO` metadata decoder ([`info::InfoList`]); round 289 adds
//! the BWF `bext` broadcast-extension decoder
//! ([`bext::BroadcastExtension`]); round 295 adds the named
//! `KSDATAFORMAT_SUBTYPE_*` GUID catalogue
//! ([`subtype::KsSubtype`]) that classifies a `SubFormat` GUID into its
//! `WAVEFORMATEX`-derived or IEC 61937 passthrough family and recovers
//! its symbolic name; round 301 adds the `cue ` cue-points decoder
//! ([`cue::CueChunk`]) that parses the cue-point table (the `dwCuePoints`
//! count plus the 24-byte `<cue-point>` records); round 307 adds the
//! `plst` playlist decoder ([`plst::Playlist`]) that parses the
//! play-segment table (the `dwSegments` count plus the 12-byte
//! `<play-segment>` records) ordering cue points into a play sequence;
//! round 310 adds the `LIST adtl` associated-data decoder
//! ([`adtl::AdtlList`]) that collects the `labl` / `note` / `ltxt` /
//! `file` child chunks attaching labels, comments, length-bounded text,
//! and embedded media files to cue points; round 314 adds the RF64 /
//! BW64 `ds64` data-size-64 decoder ([`ds64::DataSize64`]) that lifts
//! the 4 GiB size cap — the three mandatory 64-bit values (`riffSize` /
//! `dataSize` / `sampleCount`) plus the optional `<chunkSize64>`
//! size-override table — and the `0xFFFFFFFF` deferral-sentinel helpers;
//! round 319 adds the WAV `fact` chunk decoder ([`fact::Fact`]) that
//! decodes the mandatory `dwSampleLength` per-channel sample count
//! (required for compressed and `wavl`-LIST waveforms), retains any
//! reserved trailing future-format fields verbatim, and recognises the
//! RF64 / BW64 `0xFFFFFFFF` sentinel that defers the true count to the
//! `ds64` chunk's `sampleCount`; round 323 adds the BW64 `chna` ADM
//! channel-allocation decoder ([`chna::ChannelAllocation`]) that parses
//! the `numTracks` / `numUIDs` preamble plus the array of fixed 40-byte
//! `audioID` records (`trackIndex` + `UID` / `trackRef` / `packRef`
//! ADM identifier references), honouring the `N >= numUIDs`
//! over-provisioning convention by skipping zero-`trackIndex` slots;
//! round 328 adds the `acid` (Acidizer) loop-metadata decoder
//! ([`acid::Acid`]) that parses the fixed 24-byte Sonic Foundry / Sony
//! ACID record — the `flags` property bitfield (one-shot/loop,
//! root-note-set, stretch, disk-based, high-octave), the MIDI `rootNote`,
//! `numBeats`, the time signature, and the IEEE-754 `tempo` (BPM) —
//! preserving the two observed-constant `unknown` fields verbatim;
//! round 333 adds the `CSET` character-set decoder ([`cset::CharacterSet`])
//! and the typed country / language / dialect code lookups
//! ([`cset::country_name`] / [`cset::language_name`]) the §2 numeric-code
//! tables register, wiring them into the `LIST adtl` `ltxt` record so its
//! `wCountry` / `wLanguage` / `wDialect` raw codes resolve to names;
//! round 337 adds the sampler-instrument pair — the `inst` instrument
//! decoder ([`inst::Inst`]) for the fixed 7-byte MIDI playback-hint record
//! (unshifted note, signed fine-tune / gain, key + velocity zones) and the
//! `smpl` sampler decoder ([`smpl::Smpl`]) for the 36-byte fixed header
//! (manufacturer / product / sample period / MIDI unity note + pitch
//! fraction / SMPTE format + offset) plus the `NumSampleLoops` table of
//! 24-byte loop records (preserved verbatim) and the opaque
//! `SamplerData` trailer; round 340 adds the BW64 ADM XML-carrier
//! decoders ([`sxml::AxmlChunk`] / [`sxml::BxmlChunk`] /
//! [`sxml::SxmlChunk`], ITU-R BS.2088 §5-§7) — `axml` (uncompressed XML
//! verbatim), `bxml` (a `fmtType` compression selector + verbatim
//! payload), and the structured `sxml` (the `fmtType` + 64-bit
//! `subXMLCkTbSize` prefix, the `SubXMLChunk` table binding XML spans to
//! sample counts, and the optional sample-accurate `AlignmentPoint`
//! seek table) with full table-bound cross-checks; round 351 adds the
//! write-side **mux** path — a shared chunk-header writer
//! ([`chunk::write_chunk_header`] / [`chunk::encode_chunk`]) plus a
//! byte-exact `encode_body` / `encode` / `encode_chunk` inverse for every
//! typed decoder above, capped by the `tests/mux_roundtrip.rs` file-level
//! build→walk→decode fixture for a full RIFF/WAVE and a BW64/RF64 file;
//! round 358 adds the WAV `wavl` wave-data-list decoder
//! ([`wavl::WaveDataList`]) and its `slnt` silence segment
//! ([`wavl::Silence`]) — the alternative scattered storage form in which
//! the waveform is a `LIST` of `data` (sample) and `slnt` (silent-sample
//! count) child chunks in play order, with on-wire-order preservation,
//! `data`-byte / silent-sample totals, verbatim retention of unrecognised
//! child FourCCs, and a byte-exact `encode_chunk` inverse; round 361
//! widens the `LIST INFO` namespace ([`info::InfoTag`]) from the 23
//! baseline 1991 sub-IDs to also cover the 38 extended production tags
//! catalogued by ExifTool (`IAS1`–`IAS9` stream languages, the editorial
//! credits, `IDIT` / `ITRK` / `ISMP`, …) with `is_extended` /
//! `is_registered` classifiers, and adds the `JUNK` filler-chunk decoder
//! ([`padding::Junk`], RIFF MCI §2) — verbatim body preservation, the
//! alignment / zero-fill constructors, the `encode_chunk` inverse, and
//! the BS.2088-2 §2.5 `ds64`-reservation classifier
//! ([`padding::Junk::is_ds64_reservation`]); round 365 adds the
//! `WAVEFORMATEXTENSIBLE` `dwChannelMask` speaker-position decoder
//! ([`channels::ChannelMask`] / [`channels::SpeakerPosition`] /
//! [`channels::StandardLayout`]) — the named-position decomposition, the
//! in-file channel-order mapping, and standard-layout recognition — plus
//! the `id3 ` / `ID3 ` embedded-ID3v2-tag carrier ([`id3::Id3Chunk`],
//! verbatim tag preservation + a lightweight [`id3::Id3v2Header`]
//! recognizer, no frame decoding) and the `PAD ` padding FourCC
//! ([`padding::FOURCC_PAD`]); round 365 also adds the high-level
//! 64-bit walker constructors [`walk::Walker::open_rf64`] /
//! [`walk::Walker::open_bw64`], which read the mandatory `ds64` chunk to
//! resolve the `0xFFFFFFFF`-sentinel outer size and walk an `RF64` /
//! `BW64` file through the public `Walker` API
//! ([`walk::Walker::data_size_64`] exposes the parsed `ds64`).
//!
//! ## Wire format (§1.3 of the 1991 spec)
//!
//! Every RIFF chunk is laid out as:
//!
//! ```text
//! +0  ckID    : 4 bytes  ASCII FourCC, padded with trailing 0x20 if shorter
//! +4  ckSize  : u32 LE   payload length in bytes, NOT including ckID/ckSize
//!                        and NOT including the trailing pad byte
//! +8  ckData  : ckSize bytes payload
//! +8+ckSize   : pad      0x00 pad byte if ckSize is odd (so the next
//!                        chunk header starts at a 2-byte boundary)
//! ```
//!
//! Two reserved ckIDs introduce **list chunks** whose payload starts
//! with an additional 4-byte FourCC ("form type") followed by zero or
//! more nested child chunks:
//!
//! | ckID   | Role                                                       |
//! |--------|------------------------------------------------------------|
//! | `RIFF` | Outermost wrapper; appears exactly once at file offset 0   |
//! | `LIST` | Nested grouping inside a `RIFF` (or another `LIST`)        |
//!
//! All multi-byte fields are little-endian (a deliberate choice that
//! distinguishes RIFF from IFF-85, which is big-endian and uses
//! `FORM`/`LIST`/`CAT ` as its group IDs). The total file size is
//! ck-size of the outer `RIFF` + 8 (header) bytes.
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §1-2 — IBM
//!   + Microsoft, *Multimedia Programming Interface and Data
//!   Specifications 1.0*, August 1991.
//! - `docs/container/riff/metadata/ms-xaudio2-riff.html` — Microsoft
//!   Learn, modern reformulation of the same wire layout for the
//!   Win32 XAudio2 reference.
//! - `docs/container/riff/avi-riff-file-reference.md` — DirectShow
//!   AVI RIFF File Reference; the AVI form's use of the base RIFF
//!   primitives, useful as a cross-check that the FourCC + size
//!   encoding matches across forms.
//!
//! ## Standalone build
//!
//! `oxideav-core` is gated behind the default-on `registry` feature.
//! Drop the framework dependency entirely with:
//!
//! ```toml
//! oxideav-riff = { version = "0.0", default-features = false }
//! ```
//!
//! Without `registry`, the crate exposes its own [`Error`] /
//! [`Result`] aliases (defined in [`error`]); with `registry`, those
//! aliases re-export [`oxideav_core::Error`] / [`oxideav_core::Result`]
//! so the walker plugs cleanly into framework consumers.
//!
//! ## Quick start
//!
//! ```
//! use std::io::Cursor;
//! use oxideav_riff::{ChunkHeader, FOURCC_RIFF, read_chunk_header, read_form_type};
//!
//! // Minimal RIFF/WAVE skeleton: just the outer header + form type.
//! let bytes: &[u8] = &[
//!     b'R', b'I', b'F', b'F',
//!     0x04, 0x00, 0x00, 0x00,   // ckSize = 4 (just the form type)
//!     b'W', b'A', b'V', b'E',
//! ];
//! let mut cur = Cursor::new(bytes);
//! let outer: ChunkHeader = read_chunk_header(&mut cur).unwrap().unwrap();
//! assert_eq!(outer.id, FOURCC_RIFF);
//! assert_eq!(outer.size, 4);
//! assert!(outer.is_group());
//!
//! let form = read_form_type(&mut cur).unwrap();
//! assert_eq!(&form, b"WAVE");
//! ```

#![doc(html_root_url = "https://docs.rs/oxideav-riff/0.0.1")]

pub mod acid;
pub mod adtl;
pub mod bext;
pub mod channels;
pub mod chna;
pub mod chunk;
pub mod cset;
pub mod cue;
pub mod ds64;
pub mod error;
pub mod fact;
pub mod fourcc;
pub mod id3;
pub mod info;
pub mod inst;
pub mod padding;
pub mod plst;
pub mod smpl;
pub mod subtype;
pub mod sxml;
pub mod walk;
pub mod waveformat;
pub mod wavl;

pub use acid::{
    Acid, ACID_LEN, FLAG_DISK_BASED, FLAG_HIGH_OCTAVE, FLAG_ONE_SHOT, FLAG_ROOT_NOTE_SET,
    FLAG_STRETCH, FOURCC_ACID,
};
pub use adtl::{
    AdtlEntry, AdtlList, EmbeddedFile, LabeledText, FILE_PREFIX_LEN, FOURCC_ADTL, FOURCC_FILE,
    FOURCC_LABL, FOURCC_LTXT, FOURCC_NOTE, LTXT_PREFIX_LEN,
};
pub use bext::{
    BroadcastExtension, Loudness, BEXT_PREFIX_LEN, DESCRIPTION_LEN, ORIGINATION_DATE_LEN,
    ORIGINATION_TIME_LEN, ORIGINATOR_LEN, ORIGINATOR_REFERENCE_LEN, RESERVED_LEN, UMID_LEN,
};
pub use channels::{
    ChannelMask, SpeakerPosition, StandardLayout, SPEAKER_ALL, SPEAKER_RESERVED_MASK,
    SPEAKER_STANDARD_MASK,
};
pub use chna::{
    AudioId, ChannelAllocation, AUDIO_ID_LEN, CHNA_PREFIX_LEN, FOURCC_CHNA, PACK_REF_LEN,
    TRACK_REF_LEN, UID_LEN,
};
pub use chunk::{
    encode_chunk, read_chunk_header, read_form_type, skip_chunk, skip_pad, write_chunk_header,
    ChunkHeader, FOURCC_LIST, FOURCC_RIFF,
};
pub use cset::{
    country_name, country_name_defaulted, language_name, language_name_defaulted, CharacterSet,
    CSET_LEN, DEFAULT_COUNTRY, DEFAULT_DIALECT, DEFAULT_LANGUAGE, FOURCC_CSET,
};
pub use cue::{CueChunk, CuePoint, CUE_POINT_LEN, FOURCC_CUE};
pub use ds64::{
    is_rf64_magic, is_sentinel, ChunkSize64, DataSize64, CHUNK_SIZE64_LEN, DS64_PREFIX_LEN,
    FOURCC_BW64, FOURCC_DS64, FOURCC_RF64, SIZE_SENTINEL,
};
pub use error::{Error, Result};
pub use fact::{Fact, FACT_MIN_LEN, FOURCC_FACT};
pub use fourcc::{fourcc_bytes, fourcc_to_string, is_printable_fourcc};
pub use id3::{
    build_id3v2_header, is_id3_fourcc, parse_id3v2_header, Id3Chunk, Id3v2Header, FOURCC_ID3,
    FOURCC_ID3_UPPER, ID3V2_HEADER_LEN, ID3V2_MAGIC, ID3_FLAG_EXPERIMENTAL,
    ID3_FLAG_EXTENDED_HEADER, ID3_FLAG_FOOTER, ID3_FLAG_UNSYNCHRONISATION,
};
pub use info::{zstr_bytes, zstr_value, InfoList, InfoTag};
pub use inst::{Inst, FOURCC_INST, INST_LEN};
pub use padding::{is_padding_fourcc, Junk, DS64_RESERVATION_LEN, FOURCC_JUNK, FOURCC_PAD};
pub use plst::{PlaySegment, Playlist, FOURCC_PLST, PLAY_SEGMENT_LEN};
pub use smpl::{
    SampleLoop, Smpl, FOURCC_SMPL, SAMPLE_LOOP_LEN, SMPL_HEADER_LEN, SMPTE_FORMAT_NONE,
};
pub use subtype::{
    iec61937_guid, iec61937_name, waveformatex_guid, waveformatex_name, KsSubtype, IEC61937_DATA2,
};
pub use sxml::{
    AlignmentPoint, AxmlChunk, BxmlChunk, SubXmlChunk, SxmlChunk, ALIGNMENT_POINT_LEN,
    FMT_TYPE_GZIP, FMT_TYPE_UNCOMPRESSED, FOURCC_AXML, FOURCC_BXML, FOURCC_SXML,
    SUB_XML_HEADER_LEN,
};
pub use walk::{ChunkRef, Walker};
pub use waveformat::{
    ExtensibleFields, Guid, WaveFormat, KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE, WAVE_FORMAT_ADPCM,
    WAVE_FORMAT_ALAW, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IEEE_FLOAT, WAVE_FORMAT_MULAW,
    WAVE_FORMAT_PCM,
};
pub use wavl::{
    Silence, WaveDataList, WaveSegment, FOURCC_DATA, FOURCC_SLNT, FOURCC_WAVL, SLNT_LEN,
};
