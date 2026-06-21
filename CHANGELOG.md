# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Round 358 — WAV `wavl` wave-data-list + `slnt` silence decoder.** The
  alternative *scattered* waveform-storage form the 1991 RIFF MCI spec
  defines alongside the bare `data` chunk: a `LIST` whose list-type FourCC
  is `wavl`, holding a play-ordered sequence of `data` (sample) and `slnt`
  (silence) child chunks. `wavl::Silence` decodes the fixed 4-byte `slnt`
  body (the `dwSamples` silent-sample count), rejecting an off-length body
  (the chunk has no extension mechanism). `wavl::WaveDataList::collect_from`
  walks a `wavl` LIST into an ordered `Vec<WaveSegment>` (`Data` byte runs /
  `Silence` sample counts / `Other` preserved-verbatim vendor segments),
  exposing `total_data_bytes` (u64) and `total_silent_samples` (u64, so a
  long file's many silence runs can't overflow a single `u32` `dwSamples`)
  for cross-checking against `fmt ` / `fact`, and honouring the spec's
  ignore-but-don't-reject rule for unrecognised child FourCCs. The write
  side mirrors it: `Silence::encode_body` / `encode_chunk` and
  `WaveDataList::encode_list_body` / `encode_chunk` (plus `push_data` /
  `push_silence` / `push_segment` / `push_child` builders) emit a `wavl`
  LIST that re-collects equal, with each child framed through
  `chunk::encode_chunk` so an odd-length `data` run gets its RIFF pad byte.
  New public surface: `Silence`, `WaveSegment`, `WaveDataList`,
  `FOURCC_WAVL` / `FOURCC_SLNT` / `FOURCC_DATA`, `SLNT_LEN`. 20 new tests.

- **Round 351 — chunk-encode (mux) path: shared header writer + fixed-width
  body encoders.** The crate's first write-side surface, the byte-exact
  inverse of the existing parsers, so a parsed chunk re-encodes to the same
  bytes a walker would consume. `chunk::write_chunk_header` emits the 8-byte
  FourCC + little-endian `ckSize` header; `chunk::encode_chunk` emits a
  complete leaf chunk (header + body + the `0x00` pad byte for an odd-length
  body, with `ckSize` recording the un-padded length, §1.3) and rejects a
  body beyond the 32-bit `ckSize` range. Body encoders landed for the
  fixed-width chunks: `Fact::encode_body` (the `dwSampleLength` DWORD +
  retained reserved trailer, sentinel-preserving), `Inst::encode_body` (the
  7 one-byte fields), `Acid::encode_body` (the 24-byte Acidizer record, the
  observed-constant `unknown*` fields re-emitted from their retained values),
  `CharacterSet::encode_body` (the four `CSET` `WORD` fields), and the RF64 /
  BW64 `DataSize64::encode_body` (the 28-byte prefix + `<chunkSize64>` table,
  `tableLength` always agreeing with the record count) plus
  `ChunkSize64::encode` and the `DataSize64::new` / `with_override` builder
  so a writer can construct a `ds64` from scratch. New public surface:
  `write_chunk_header` / `encode_chunk`; per-chunk `encode_body` /
  `encode` / `new` / `with_override`. 24 new round-trip tests.

- **Round 351 — chunk-encode (mux) path: variable-width body encoders.**
  Write-side inverses for the count-prefixed and table-bearing chunks.
  `CueChunk::encode_body` / `CuePoint::encode` and `Playlist::encode_body`
  / `PlaySegment::encode` emit the `dwCuePoints` / `dwSegments` count
  prefix (always agreeing with the record count) plus the 24- / 12-byte
  records, with `from_points` / `from_segments` / `push` builders.
  `Smpl::encode_body` / `SampleLoop::encode` emit the 36-byte header, the
  loop table, and the `SamplerData` trailer — deriving `NumSampleLoops` /
  `SamplerDataLen` from the actual list lengths so the body is always
  self-consistent. `BroadcastExtension::encode_body` emits the 602-byte
  `bext` prefix (the 180-byte `Reserved` region normalised to zero, the
  spec-required value) plus the verbatim `CodingHistory`.
  `ChannelAllocation::encode_body` / `AudioId::encode` emit the
  `numTracks` / `numUIDs` preamble plus the 40-byte `audioID` records,
  preserving over-provisioned zeroed slots, with `from_records` / `push`
  builders. 18 new round-trip tests.

- **Round 351 — chunk-encode (mux) path: LIST + descriptor encoders.**
  Write-side inverses for the remaining chunk families.
  `WaveFormat::encode_body` (+ `Guid::to_le_wire`) re-emits the `fmt `
  descriptor across all three forms — `WAVEFORMAT` / `WAVEFORMATEX` /
  `WAVEFORMATEXTENSIBLE` — rebuilding the 22-byte extensible tail from the
  typed `ExtensibleFields` and preserving any trailing extension bytes.
  `InfoList::encode_list_body` / `encode_chunk` (+ `push`) and
  `AdtlList::encode_list_body` / `encode_chunk` (+ `push`) emit the
  `LIST INFO` / `LIST adtl` groups, framing each child with
  `chunk::encode_chunk` (ZSTR terminators re-added for `INFO` / `labl` /
  `note`; per-entry `AdtlEntry::encode_body` / `fourcc`, plus
  `LabeledText::encode_body` / `EmbeddedFile::encode_body`).
  `AxmlChunk` / `BxmlChunk` / `SxmlChunk` gained `encode_body`, with the
  `sxml` encoder deriving `subXMLCkTbSize` and each `subXMLChunkSize` from
  the actual payload lengths so the body is always self-consistent. 23 new
  round-trip tests (parse↔encode and, for the LIST chunks, full
  encode→walk→collect round-trips).

- **Round 351 — mux fixture round-trip (`tests/mux_roundtrip.rs`).** The
  milestone capstone: file-level proof that the new encoders and the
  existing decoders are exact inverses. `full_wave_file_muxes_and_round_trips`
  assembles a complete `RIFF` / `WAVE` file — `fmt ` / `fact` / `data` /
  `cue ` / `LIST INFO` / `LIST adtl` / `smpl` / `inst` / `acid` — purely
  from the `encode_*` helpers, walks it back with the public `Walker`, and
  asserts every decoded struct equals the encoded source plus the chunk
  order. `waveformatextensible_file_round_trips` drives a 5.1 / 24-in-32
  PCM file through the extensible `fmt ` encoder.
  `bw64_rf64_file_with_ds64_chna_round_trips` muxes a `BW64` file with a
  `ds64` 64-bit size table and a `chna` ADM allocation table and re-parses
  it via the low-level `read_chunk_header` path (the `Walker` is strict on
  the `RIFF` outer FourCC). 3 fixture tests. **Followup:** a dedicated
  `Walker::open_rf64` / `open_bw64` constructor would let the 64-bit outer
  wrappers be walked through the high-level API instead of the manual
  header loop.

- **Round 340 — BW64 ADM XML-carrier decoders (`axml` / `bxml` / `sxml`).**
  Three typed readers for the *Audio Definition Model* metadata document
  that a BW64 (ADM-carrying) WAV file pairs with the binary `chna` table,
  per ITU-R BS.2088 §5-§7. `AxmlChunk::parse` keeps the uncompressed XML
  body verbatim (`xml_str()` UTF-8 accessor). `BxmlChunk::parse` splits the
  2-byte `fmtType` compression selector (`0x0000` uncompressed / `0x0001`
  gzip) from the verbatim compressed payload (`is_gzip` / `is_uncompressed`;
  a sub-2-byte body is rejected). `SxmlChunk::parse` walks the structured
  carrier: the `fmtType` + 64-bit `subXMLCkTbSize` prefix, the `SubXMLChunk`
  table binding each XML span to its `nSamplesSubDataChunk` audio-sample
  count, and the optional sample-accurate `AlignmentPoint` seek table
  (64-bit byte offset + timeline sample count). The table-size field
  (counting its own `nSubXMLChunks` field) is range-checked against the
  body, each `SubXMLChunk` record is checked against the declared table
  region, and the trailing `nAlignmentPoints` count must consume exactly
  the remaining body — so a truncated or over-long chunk is rejected with
  `Error::invalid` rather than parsed past its bounds. Decompression /
  XML interpretation stays the caller's concern, above the container
  layer. `total_samples()` sums the sub-chunk spans. New public surface:
  `AxmlChunk` / `BxmlChunk` / `SxmlChunk` / `SubXmlChunk` / `AlignmentPoint`
  plus `FOURCC_AXML` / `FOURCC_BXML` / `FOURCC_SXML` / `FMT_TYPE_*` /
  `SUB_XML_HEADER_LEN` / `ALIGNMENT_POINT_LEN`. 13 new tests.

- **Round 337 — `inst` instrument + `smpl` sampler decoders.** Two typed
  readers for the WAV sampler-instrument chunk pair. `Inst::parse` decodes
  the fixed 7-byte `inst` instrument record — `UnshiftedNote`, `FineTune`,
  `Gain`, `LowNote`, `HighNote`, `LowVelocity`, `HighVelocity`, one byte
  each — exposing the signed `fine_tune()` / `gain()` offsets (raw bytes
  retained), the `note_range()` / `velocity_range()` key + velocity zones,
  and `covers_note` / `covers_velocity` membership tests. `Smpl::parse`
  decodes the `smpl` sampler chunk: the 36-byte fixed header
  (`Manufacturer` / `Product` / `SamplePeriod` / `MIDIUnityNote` /
  `MIDIPitchFraction` / `SMPTEFormat` / `SMPTEOffset` / `NumSampleLoops` /
  `SamplerDataLen`), the `NumSampleLoops` table of 24-byte `<sample-loop>`
  records, and the opaque `SamplerData` trailer; the body length must
  agree with the declared loop count and sampler-data length (overflow-safe
  size arithmetic rejects a corrupt count instead of panicking). Because
  the per-loop field breakdown is not in the in-tree clean-room material,
  each loop record is preserved verbatim as a 24-byte `SampleLoop`;
  `smpte_offset_parts()` unpacks the packed `HH:MM:SS:FF` offset and
  `smpte_none()` / `has_sampler_data()` are convenience predicates. Both
  reject off-length / mismatched bodies rather than treating them as
  future fields. Adds `Inst`, `FOURCC_INST`, `INST_LEN`, `Smpl`,
  `SampleLoop`, `FOURCC_SMPL`, `SMPL_HEADER_LEN`, `SAMPLE_LOOP_LEN`, and
  `SMPTE_FORMAT_NONE` to the public surface. Sourced from
  `docs/container/riff/metadata/exiftool-riff-tags.html` (RIFF Instrument
  Tags + RIFF Sampler Tags) and `docs/container/riff/metadata/README.md`
  (the `inst` 7-byte / `smpl` 36-byte + N × 24 loop-record size summary).

- **Round 333 — `CSET` character-set decoder + country / language /
  dialect code lookups.** A typed reader for the file-wide `CSET`
  (character set) chunk — the 8-byte body of four 16-bit fields
  (`wCodePage` / `wCountryCode` / `wLanguageCode` / `wDialect`) — plus
  the two numeric-code tables the spec's §2 registers: the 29-entry
  *Country Codes* table (`001` USA … `358` Finland) via `country_name`,
  and the 44-entry *Language and Dialect Codes* table via `language_name`,
  keyed on the (`wLanguage`, `wDialect`) pair so one language code
  resolves to its dialect-specific name (e.g. `12`/`1..4` French / Belgian
  / Canadian / Swiss French; `4`/`1` Traditional vs `4`/`2` Simplified
  Chinese). The spec's zero-field defaulting (country `0` → USA,
  language/dialect `(0, 0)` → US English) is exposed via the
  `country_name_defaulted` / `language_name_defaulted` free functions and
  the `CharacterSet::country` / `language` accessors; `CharacterSet::parse`
  rejects any body that is not exactly 8 bytes. These same codes appear in
  the `LIST adtl` `ltxt` record, so `LabeledText` gains `country_name` /
  `language_name` accessors that resolve its previously-raw `wCountry` /
  `wLanguage` / `wDialect` `u16` values. Adds `CharacterSet`,
  `FOURCC_CSET`, `CSET_LEN`, the `DEFAULT_COUNTRY` / `DEFAULT_LANGUAGE` /
  `DEFAULT_DIALECT` constants, and the four name-lookup functions to the
  public surface. Sourced from `docs/container/riff/metadata/microsoft-riffmci.pdf`
  §2 (CSET chunk, Country Codes, Language and Dialect Codes).
- **Round 328 — `acid` (Acidizer) loop-metadata decoder.** A typed
  reader for the Sonic Foundry / Sony ACID `acid` chunk, the loop
  metadata "ACIDized" WAV files carry so a host can pitch- and
  time-stretch a loop to the project tempo. `Acid::parse` decodes the
  fixed 24-byte body: the `flags` property bitfield, the MIDI `rootNote`,
  two observed-constant `unknown` fields, `numBeats`, the
  meter denominator/numerator, and the IEEE-754 single-precision `tempo`
  (BPM). The `flags` bits are exposed through `is_one_shot` / `is_loop`
  (bit 0 type discriminator), `root_note_set` (bit 1, which gates the
  `root_note()` accessor), `stretch_enabled` (bit 2), `disk_based`
  (bit 3) and `high_octave` (bit 4) accessors plus the matching
  `FLAG_*` masks. The body length is required to be exactly 24 bytes —
  this is a fixed-width vendor record with no documented extension
  mechanism, so an off-length body is rejected rather than treated as a
  future field. The two `unknown` fields are kept verbatim so an
  atypical writer's values round-trip. Adds the `FOURCC_ACID` /
  `ACID_LEN` constants and the `Acid` type to the public surface.
  Sourced from the clean-room field spec `docs/container/riff/acid-chunk.md`.

- **Round 323 — BW64 `chna` ADM channel-allocation decoder.** A typed
  reader for the *Audio Definition Model* `chna` chunk, sourced from
  ITU-R BS.2088-2 §8 (the BW64 file format, the binary `struct
  chna_chunk` / `struct audioID` layout that BS.2076 ADM and EBU Tech
  3285s5 reference but defer). `ChannelAllocation::parse` decodes the
  4-byte `numTracks` / `numUIDs` preamble followed by an array of fixed
  40-byte `audioID` records — each a `trackIndex` (u16 LE, 1-based into
  the `data` interleave) plus the three fixed-width non-NUL-terminated
  ADM identifier references (`UID` `ATU_…`, `trackRef` `AT_…` or
  `AC_…`, `packRef` `AP_…` or all-NUL). The record count is derived
  from the body length (`N = (ckSize − 4) / 40`) with a multiple-of-40
  cross-check, and the `N ≥ numUIDs` over-provisioning convention is
  honoured: zeroed spare records (`trackIndex == 0`) are retained but
  skipped by `active()` / `by_track_index()`, with `uid_count_consistent`
  reporting whether the used-record count matches the declared
  `numUIDs`. Identifier fields are exposed as raw byte arrays plus
  trimmed-UTF-8 `uid_str` / `track_ref_str` / `pack_ref_str` accessors.
  Adds the `FOURCC_CHNA` / `AUDIO_ID_LEN` / `CHNA_PREFIX_LEN` /
  `UID_LEN` / `TRACK_REF_LEN` / `PACK_REF_LEN` constants and the
  `AudioId` record type.

- **Round 319 — WAV `fact` chunk decoder.** A typed reader for the
  WAVE `fact` chunk, sourced from the 1991 RIFF MCI spec §2 ("FACT
  Chunk"). `Fact::parse` decodes the mandatory `dwSampleLength` — the
  per-channel sample count the chunk records so a player can derive
  exact duration when the audio is compressed (the `data` byte count no
  longer maps to samples by a fixed `wBlockAlign`) or scattered across
  a `wavl` LIST. Per the spec's forward-compatibility rule the decoder
  keeps any reserved trailing fields (sized by the chunk header) verbatim
  in `Fact::extra` rather than rejecting a longer body, and recognises
  the RF64 / BW64 `0xFFFFFFFF` sentinel (EBU Tech 3306 §A, whose `ds64`
  `sampleCount` "replaces the sample count value in the 'fact' chunk")
  via `is_deferred` / `sample_length()`, returning `None` for the
  deferred case so callers resolve the real 64-bit count from a parsed
  `ds64` chunk. Adds the `FOURCC_FACT` / `FACT_MIN_LEN` constants.

- **Round 314 — RF64 / BW64 `ds64` data-size-64 decoder.** A typed
  reader for the MBWF / RF64 64-bit size-extension chunk, sourced from
  EBU Tech 3306 §A.2 ("New Chunks and Structs in the RF64/WAVE (MBWF)
  format") and the v2 `BW64` ADM variant. `DataSize64::parse` decodes
  the 28-byte fixed prefix — the three mandatory 64-bit values
  (`riffSize` / `dataSize` / `sampleCount`) that replace the file's
  `0xFFFFFFFF`-sentinel 32-bit fields — plus the optional
  `<chunkSize64>` table of per-chunk 64-bit size overrides, with the
  same body-length ↔ count cross-check the other decoders use. Adds
  `is_sentinel` / `is_rf64_magic` predicates, the `FOURCC_RF64` /
  `FOURCC_BW64` / `FOURCC_DS64` constants, and
  `DataSize64::resolve` / `size_for` to look a chunk's real size up by
  FourCC.

## [0.0.2](https://github.com/OxideAV/oxideav-riff/compare/v0.0.1...v0.0.2) - 2026-06-15

### Added

- *(info)* LIST INFO metadata decoder (round 275)

### Other

- round 310 — LIST adtl associated-data decoder
- typed playlist-chunk decoder (round 307)
- round 301 — cue cue-points chunk decoder
- named KSDATAFORMAT_SUBTYPE_* GUID catalogue (round 295)
- add BWF `bext` Broadcast Audio Extension decoder (EBU Tech 3285 v2)
- typed WAV `fmt ` chunk decoder — WAVEFORMATEX(TENSIBLE) + GUID resolver
- neutralise enumerated-denial paragraph in CHANGELOG (r257 brief-inheritance scrub-in-place)

### Added

- **Round 310 — `LIST adtl` associated-data decoder.** A typed reader
  for the WAV / RIFF associated-data list, sourced from the "Associated
  Data Chunk" section of `microsoft-riffmci.pdf`. It attaches labels,
  comments, length-bounded text, and embedded media files to the cue
  points of a `cue ` chunk, completing the cue-points triad with the
  round-301 `cue ` and round-307 `plst` decoders.

  - `adtl::AdtlList::collect_from(walker)` walks a `LIST adtl` sub-tree
    (built after reading the `adtl` list-type with
    `Walker::read_inner_form_type`) into an ordered list of entries; a
    non-`adtl` list-type is rejected.
  - `adtl::AdtlEntry` — one decoded child: `Label { name, text }` /
    `Note { name, text }` (a `dwName` `u32` + ZSTR text), `LabeledText`
    (the `ltxt` 20-byte prefix — `name` / `sample_length` / `purpose`
    FourCC / `country` / `language` / `dialect` / `code_page` — plus raw
    trailing text), `File` (the `file` 8-byte `name` / `med_type` prefix
    plus an opaque payload), and `Other { fourcc, body }` for an
    unrecognised child FourCC (preserved verbatim).
  - Length invariants: a `labl` / `note` body shorter than the 4-byte
    `dwName`, an `ltxt` body shorter than its 20-byte prefix, or a `file`
    body shorter than its 8-byte prefix is rejected (`Error::invalid`).
  - Cue cross-reference recorded but not resolved: `AdtlEntry::cue_name()`,
    `AdtlList::by_cue_name(name)` (all entries for a cue point),
    `label(name)` / `note(name)` (first text for a cue point), plus
    `entries()` / `len()` / `is_empty()` and the `FOURCC_ADTL` /
    `FOURCC_LABL` / `FOURCC_NOTE` / `FOURCC_LTXT` / `FOURCC_FILE` /
    `LTXT_PREFIX_LEN` / `FILE_PREFIX_LEN` constants.
  - 16 unit tests covering each child kind's parse, the ZSTR
    missing-terminator path, the `ltxt` / `file` prefix-only (empty
    trailing data) cases, the three short-body rejections, the unknown
    child arm, the `collect_from` happy path with cue cross-reference,
    odd-length-body pad re-sync, the non-`adtl` list-type rejection, and
    the empty list.
  - Re-exported at the crate root: `AdtlEntry`, `AdtlList`,
    `EmbeddedFile`, `LabeledText`, and the FourCC + prefix-length
    constants.

- **Round 307 — `plst` playlist chunk decoder.** A typed body decoder
  for the WAV / RIFF playlist chunk, sourced from the "Playlist Chunk"
  section of `microsoft-riffmci.pdf`. It orders the cue points of a
  `cue ` chunk into a play sequence.

  - `plst::Playlist::parse(body)` decodes the `dwSegments` count prefix
    followed by that many 12-byte `<play-segment>` records. The declared
    count must account for exactly the remaining body length; a short or
    over-long body is rejected (`Error::invalid`) rather than silently
    truncated.
  - `plst::PlaySegment` exposes the three little-endian fields (`name`,
    referencing a `cue ` `dwName`; `length`, the section length in
    samples; `loops`, the play-repeat count). The cue reference is
    recorded but not resolved (the decoder has no view of the surrounding
    chunk tree).
  - `Playlist::segments()` / `len()` / `is_empty()` / `by_name(name)`
    plus `FOURCC_PLST` and `PLAY_SEGMENT_LEN` constants. Unlike a cue
    `dwName`, a playlist may reference the same cue point more than once,
    so `by_name` returns the first match in play order.
  - 9 unit tests covering single / multi-segment ordering, repeated cue
    references, `by_name` lookup, the empty chunk, and the short /
    count-mismatch / over-long rejection paths.

- **Round 301 — `cue ` cue-points chunk decoder.** A typed body decoder
  for the WAV / RIFF cue-points chunk, sourced from the "Cue-Points
  Chunk" section of `microsoft-riffmci.pdf`.

  - `cue::CueChunk::parse(body)` decodes the `dwCuePoints` count prefix
    followed by that many 24-byte `<cue-point>` records. The declared
    count must account for exactly the remaining body length; a short or
    over-long body is rejected (`Error::invalid`) rather than silently
    truncated.
  - `cue::CuePoint` exposes the six little-endian fields (`name` /
    `position` / `fcc_chunk` / `chunk_start` / `block_start` /
    `sample_offset`), with `is_data()` / `is_silent()` helpers over the
    raw `fccChunk` FourCC. The raw offset fields are preserved without
    interpretation (their meaning depends on the surrounding `wavl` /
    single-`data`, PCM / compressed layout that this decoder cannot see).
  - `CueChunk::points()` / `len()` / `is_empty()` / `by_name(name)` plus
    `FOURCC_CUE` and `CUE_POINT_LEN` constants.
  - 10 unit tests covering single-point PCM, multi-point ordering,
    `slnt` / non-`data` FourCCs, `by_name` lookup, the empty chunk, and
    the short / count-mismatch / over-long rejection paths.

- **Round 295 — named `KSDATAFORMAT_SUBTYPE_*` GUID catalogue.** A
  classifier layer on top of the round-267 `Guid` decoder, sourced from
  the staged `ksdataformat-subtype-guids.md` catalogue +
  `ms-subformat-guids-compressed-audio.md` (CEA-861 IEC 61937 table) +
  `ms-converting-format-tags-and-subformat-guids.md`
  (`DEFINE_WAVEFORMATEX_GUID` macro).

  - `subtype::KsSubtype::resolve(&Guid)` classifies a `SubFormat` GUID
    into `WaveFormatEx { tag }` (base template, `Data2 == 0x0000`, the
    `Data1` low word is the legacy `WAVE_FORMAT_*` tag), `Iec61937
    { cea861_type }` (the Windows-7+ passthrough family, discriminated
    by the `0x0cea` `Data2` marker, the `Data1` low word being a CEA-861
    stream-type index), or `Other(Guid)` (a vendor/proprietary root
    GUID preserved verbatim).
  - `KsSubtype::symbolic_name()` / `description()` return the
    `KSDATAFORMAT_SUBTYPE_*` constant name + a short codec string.
    Family-1 covers `…_WAVEFORMATEX` / `…_PCM` / `…_ADPCM` /
    `…_IEEE_FLOAT` / `…_ALAW` / `…_MULAW` / `…_DTS` / `…_DRM` /
    `…_MPEG` / `…_DOLBY_AC3_SPDIF`; Family-2 covers `…_IEC61937_MPEG1`
    / `…_MPEG2` / `…_MPEG3` / `…_AAC` / `…_ATRAC` / `…_ONE_BIT_AUDIO` /
    `…_DOLBY_DIGITAL_PLUS` / `…_DTS_HD` / `…_DOLBY_MLP` / `…_DST`.
  - `subtype::waveformatex_guid(tag)` / `iec61937_guid(index)` build a
    template GUID; `waveformatex_name` / `iec61937_name` expose the
    lookup tables; `IEC61937_DATA2` names the `0x0cea` discriminator.
  - 11 new unit tests covering both template builders, the
    WAVEFORMATEX-family resolve (PCM / float / A-law / mu-law / the
    AC-3 worked example), the IEC 61937 family resolve, the
    `0x0cea`-vs-`0x0000` discrimination on a shared `Data1` low word, an
    uncatalogued-but-valid tag (MP3 0x0055), a non-template `Other`
    GUID, and reserved CEA-861 indices.
  - Re-exported at the crate root: `KsSubtype`, `waveformatex_guid`,
    `iec61937_guid`, `waveformatex_name`, `iec61937_name`,
    `IEC61937_DATA2`.

- **Round 289 — BWF `bext` broadcast-extension decoder.** A typed
  reader for the Broadcast Audio Extension chunk, per EBU Tech 3285 v2
  (`broadcast_audio_extension` struct + per-field descriptions + §1.1
  "Version compatibility").

  - `bext::BroadcastExtension::parse(body)` decodes the 602-byte fixed
    prefix — `Description` / `Originator` / `OriginatorReference`
    (NUL-padded ASCII), `OriginationDate` (`"yyyy-mm-dd"`) /
    `OriginationTime` (`"hh-mm-ss"`), the 64-bit `TimeReference`
    reassembled from its low/high words, the `Version` word, the 64-byte
    SMPTE 330M `UMID`, and the five 16-bit-signed loudness fields — plus
    the trailing variable-length `CodingHistory` (chunk size − 602). A
    body shorter than the 602-byte prefix is rejected as truncated.
  - Version gating per §1.1: `umid()` returns the UMID only when
    `version >= 1`; `loudness()` returns the `bext::Loudness`
    measurements only when `version >= 2` (the bytes are reserved in
    earlier versions). Raw bytes (`umid_bytes`, the `*_x100` fields)
    stay reachable unconditionally.
  - `bext::Loudness` — `value` / `range` / `max_true_peak` /
    `max_momentary` / `max_short_term`, each a `round(100 × …)` integer,
    with `_x100` raw accessors and `_lufs` / `_lu` / `_dbtp`
    natural-unit accessors.
  - String accessors (`description()`, `originator()`, …) trim at the
    first NUL and lossily decode to `String`; `coding_history()`
    additionally strips trailing NUL padding.
  - 10 new unit tests covering the prefix-length invariant, short-body
    rejection, NUL-trimmed string fields, 64-bit TimeReference
    reassembly, UMID + loudness version gating with scaling, the
    CodingHistory trailing field + its NUL-padding trim, and lossy
    non-ASCII decode.
  - Re-exported at the crate root: `BroadcastExtension`, `Loudness`,
    `BEXT_PREFIX_LEN`, and the field-length constants.

- **Round 275 — `LIST INFO` metadata decoder.** A typed reader for
  the registered `INFO` identification-metadata namespace, per the
  1991 RIFF MCI §2 "INFO List Chunk" + "NULL-Terminated String
  (ZSTR) Format".

  - `info::InfoTag` — the 23 baseline `INFO` sub-IDs the spec
    registers, exposed as associated constants (`INAM`, `IART`,
    `ICOP`, …) with `InfoTag::label()` mapping each to its spec field
    name, `InfoTag::is_baseline()`, and the `InfoTag::BASELINE`
    ordered table. Unknown / vendor four-character codes are
    preserved verbatim (the spec instructs applications to ignore,
    not reject, unrecognised IDs).
  - `info::zstr_bytes` / `info::zstr_value` — ZSTR body decode: bytes
    up to the first `0x00`, with tolerance for bodies that rely only
    on the RIFF pad byte (no embedded terminator). `zstr_value`
    lossily decodes to `String`.
  - `info::InfoList` — an ordered `(InfoTag, String)` collection.
    `collect_from(&mut Walker)` walks a `LIST INFO` sub-tree (built
    after reading the `INFO` list-type with
    `Walker::read_inner_form_type`) into the list; `get(tag)` returns
    the first value, `entries()` exposes all (order + duplicates
    preserved). A non-`INFO` list-type is rejected.
  - 12 new unit tests covering the baseline table, label mapping,
    ZSTR edge cases (missing terminator, embedded NUL, invalid
    UTF-8), order/duplicate preservation, odd-length-body pad
    re-sync, unknown-tag retention, and the non-INFO rejection.
  - Re-exported at the crate root: `InfoList`, `InfoTag`,
    `zstr_bytes`, `zstr_value`.

- **Round 267 — `fmt ` chunk decoder.** First typed chunk-body
  primitive: `waveformat::WaveFormat::parse(body)` decodes a WAV
  `fmt ` chunk body (the bytes the walker yields) into a typed
  descriptor, per the 1991 RIFF MCI §2 base layout + the Microsoft
  Learn `WAVEFORMATEXTENSIBLE` references.

  - Base `WAVEFORMAT` prefix — `format_tag`, `channels`,
    `sample_rate`, `avg_bytes_per_sec`, `block_align`,
    `bits_per_sample` (all little-endian).
  - `WAVEFORMATEX` extension — the optional `cbSize` at +16 and its
    counted trailing bytes, exposed raw as `extension`; a `cbSize`
    that over-runs the body length is rejected.
  - `WAVEFORMATEXTENSIBLE` tail (`ExtensibleFields`) — parsed when
    `format_tag == WAVE_FORMAT_EXTENSIBLE (0xFFFE)`: the `Samples`
    union (`samples`), `dwChannelMask` (`channel_mask`), and the
    16-byte `SubFormat` GUID. A `0xFFFE` tag with fewer than 22
    extension bytes is rejected.
  - `Guid` — Microsoft mixed-endian GUID (`from_le_wire`,
    `to_hyphenated`) with `is_waveformatex_derived` /
    `waveformatex_tag` recovering the legacy 16-bit `wFormatTag`
    from a `DEFINE_WAVEFORMATEX_GUID`-template subtype, plus the
    `KSDATAFORMAT_SUBTYPE_WAVEFORMATEX_BASE` template constant.
  - `WaveFormat::is_extensible` / `effective_format_tag` /
    `channel_mask_count` convenience accessors.
  - `WAVE_FORMAT_PCM` / `_ADPCM` / `_IEEE_FLOAT` / `_ALAW` /
    `_MULAW` / `_EXTENSIBLE` `wFormatTag` constants.
  - 12 new unit tests covering the bare-`WAVEFORMAT`,
    `WAVEFORMATEX`-with-extension, extensible-PCM, non-template
    `SubFormat`, mixed-endian GUID decode, and the short-body /
    `cbSize`-overrun / truncated-extensible rejection paths.

- **Round 257 — bootstrap.** Initial release of the `oxideav-riff`
  crate: a shared, clean-room **RIFF chunk-walker** that every
  RIFF-family parser (WAV, AVI, WebP, AMV, ANI, …) can plug into.
  Implements the 1991 IBM + Microsoft *Multimedia Programming
  Interface and Data Specifications 1.0* §1.3 wire format:

  - `ChunkHeader { id: [u8; 4], size: u32 }` — 8-byte header decode
    via `read_chunk_header(r)`, returning `Ok(None)` at clean EOF
    and `Err(Error::invalid)` on a partial header (parent
    `ckSize` lied).
  - `Walker::open_root(r)` — opens the outer `RIFF` chunk at offset
    0, validates the FourCC + minimum `ckSize >= 4` (room for the
    form-type word), and positions just past the form type so the
    first `.read_next()` yields the first top-level child. Strict on
    `RIFF` — the `RF64` / `BW64` 64-bit-extended variants
    (EBU Tech 3306 §4) are deferred to a later round.
  - `Walker::open_within(r, header)` — wrap an already-located
    group chunk (`RIFF` or `LIST`) so the caller can descend into
    nested sub-trees without re-reading the outer header.
  - `Walker::read_next()` — yields the next `ChunkRef { id, size,
    body_offset }`. Enforces parent budget: a child whose body +
    pad would overflow the parent's `ckSize` is rejected with
    `Error::invalid("RIFF: chunk overflows parent")`; a clean EOF
    before the parent budget is satisfied surfaces as
    `Error::invalid("RIFF: truncated parent — …")`.
  - `Walker::read_body(chunk)` / `Walker::skip(chunk)` — consume
    the body + pad byte, advancing both the underlying reader and
    the walker's parent-budget counter.
  - `Walker::read_inner_form_type(chunk)` — for `RIFF` / `LIST`
    children, reads the 4-byte form-type / list-type tag and
    charges 4 bytes against the parent walker's budget, leaving
    the reader positioned at the first nested child.
  - `ChunkHeader::padded_size()` and `ChunkRef::padded_size()` /
    `end_offset()` — pre-computed wire-byte counts (body + pad)
    for callers that want to seek past a chunk without reading
    the body.
  - `FOURCC_RIFF` / `FOURCC_LIST` constants.
  - `fourcc::fourcc_bytes(b"RIFF")` `const` helper for compile-
    time tag literals.
  - `fourcc::fourcc_to_string()` — debug-safe rendering, escapes
    non-printable bytes as `\xNN` so debug dumps of malformed
    files stay readable.
  - `fourcc::is_printable_fourcc()` — `const` predicate for
    cheap rejection of obvious garbage (e.g. a JPEG SOI marker
    mis-fed into a RIFF parser).

- **Default-on `registry` feature.** With `registry` enabled the
  crate re-exports `oxideav_core::Error` / `oxideav_core::Result`
  so the walker plugs cleanly into framework consumers. Drop
  `default-features = false` to use the standalone in-tree
  `Error` enum (`Invalid(String)` + `Io(std::io::Error)`) and
  remove the framework dependency entirely.

- **24 unit tests** covering:
  - `ChunkHeader` constants + `is_group` + `padded_size` (incl.
    the `u32::MAX` odd-size edge).
  - `read_chunk_header` LE decode + clean-EOF + truncated-header
    paths.
  - `Walker::open_root` happy path, non-`RIFF` rejection,
    `ckSize < 4` rejection.
  - `Walker::read_next` round-trip + parent-budget enforcement (child
    overflow rejected) + truncated-parent detection.
  - `Walker::skip` advancing past body + pad.
  - `Walker::read_inner_form_type` reading the nested form-type
    word for `LIST` descent.
  - `fourcc_bytes` / `fourcc_to_string` printable + escaped
    rendering paths + `is_printable_fourcc` boundary checks.

### Known gaps (deferred to later rounds)

- `RF64` / `BW64` 64-bit-extended outer wrapper + `ds64`
  side-table (EBU Tech 3306 §4).
- The Media-Foundation `MFAudioFormat_*` parallel namespace and the
  MAT 2.0 Atmos IEC 61937 variants (the round-295 `KsSubtype` catalogue
  covers the WAVEFORMATEX-derived + base IEC 61937 families).
- WAV metadata-bearing chunks: the `LIST INFO` vendor / iTunes-era
  sub-IDs beyond the 23-entry baseline (RecordingBlogs + ExifTool
  catalog), BWF `iXML` / `qlty` / `mext`, `cue ` / `plst` /
  `LIST adtl`, `smpl` / `inst`, ADM `axml` / `chna`, `id3 ` chunk.
- Higher-level recursive walker (`walk_tree`) for callers that
  want one-shot enumeration of every nested chunk.
- Streaming writer (begin/finish reservation pattern) — currently
  out of scope; the AVI / WebP crates carry their own form-
  specific writers.

### Clean-room provenance

All wire-format details are sourced from `docs/container/riff/`:

- `metadata/microsoft-riffmci.pdf` §1-2 (1991 IBM + Microsoft
  base spec).
- `metadata/ms-xaudio2-riff.html` (modern Microsoft Learn
  reformulation).
- `avi-riff-file-reference.md` (DirectShow AVI RIFF File
  Reference — cross-check that FourCC + size encoding matches
  across forms).
- `rfc2361-wav.txt` (the `wFormatTag` registry values).
- `waveformatextensible/` — Microsoft Learn *WAVEFORMATEXTENSIBLE
  structure*, *Extensible Wave-Format Descriptors*, and
  *Converting Between Format Tags and Subformat GUIDs* (the
  `DEFINE_WAVEFORMATEX_GUID` base-template macro).

Clean-room implementation. The sibling `oxideav-avi` crate's own
internal `riff.rs` was referenced as a clean-room precedent (same
project, same provenance), but the new walker is a fresh write-up
against the spec.
