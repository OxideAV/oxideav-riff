//! Typed `WAVE`-form view over a [`crate::tree::RiffTree`].
//!
//! The [`crate::tree`] module gives a generic, format-agnostic owned
//! tree of every chunk in a `RIFF` file. This module sits one layer up
//! for the specific `RIFF/WAVE` form: [`WaveFile::from_tree`] walks a
//! parsed tree and decodes the chunks a WAV / BWF file commonly carries
//! into their existing typed structs — the `fmt ` format descriptor, the
//! `fact` sample count, the `cue ` / `plst` cue + playlist tables, the
//! `bext` broadcast extension, the `acid` loop block, the `smpl` /
//! `inst` sampler pair, and the `LIST INFO` / `LIST adtl` metadata lists.
//!
//! It is deliberately a **read-side convenience view**: each typed field
//! is optional (a WAV file need only carry `fmt ` + `data`), and the
//! generic tree remains the source of truth for byte-exact rewrite —
//! `WaveFile` borrows the tree it was built from. Chunks this view
//! doesn't model (or a vendor chunk it doesn't recognise) are simply not
//! surfaced here; they survive in the underlying tree untouched, honoring
//! the metadata spec's "treat unknown chunks as opaque-but-preserved"
//! rule.
//!
//! ## Chunk dispatch (§3 "Waveform Audio File Format")
//!
//! The `WAVE` form's chunk set is dispatched by 4-byte FourCC:
//! `fmt ` → [`crate::WaveFormat`], `fact` → [`crate::Fact`],
//! `cue ` → [`crate::CueChunk`], `plst` → [`crate::Playlist`],
//! `bext` → [`crate::BroadcastExtension`], `acid` → [`crate::Acid`],
//! `smpl` → [`crate::Smpl`], `inst` → [`crate::Inst`],
//! `LIST INFO` → [`crate::InfoList`], `LIST adtl` → [`crate::AdtlList`].

use crate::acid::Acid;
use crate::adtl::AdtlList;
use crate::bext::BroadcastExtension;
use crate::cue::CueChunk;
use crate::error::{Error, Result};
use crate::fact::Fact;
use crate::info::InfoList;
use crate::inst::Inst;
use crate::plst::Playlist;
use crate::smpl::Smpl;
use crate::tree::{RiffChunk, RiffTree};
use crate::waveformat::WaveFormat;

/// 4-byte form type identifying a `RIFF/WAVE` file.
pub const FORM_WAVE: [u8; 4] = *b"WAVE";

/// Decoded typed view of the chunks in a `RIFF/WAVE` file.
///
/// Build one with [`WaveFile::from_tree`]. Every field is optional
/// except that construction fails if the tree's form type is not
/// `WAVE`. The view does not own the sample `data` bytes — those (and
/// every other chunk) stay in the [`RiffTree`] the view was decoded
/// from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaveFile {
    /// The `fmt ` format descriptor.
    pub format: Option<WaveFormat>,
    /// The `fact` chunk (sample count; mandatory for compressed/`wavl`
    /// forms).
    pub fact: Option<Fact>,
    /// The `cue ` cue-point table.
    pub cue: Option<CueChunk>,
    /// The `plst` playlist.
    pub playlist: Option<Playlist>,
    /// The BWF `bext` broadcast extension.
    pub bext: Option<BroadcastExtension>,
    /// The Sonic Foundry / Sony `acid` loop block.
    pub acid: Option<Acid>,
    /// The `smpl` sampler chunk.
    pub smpl: Option<Smpl>,
    /// The `inst` instrument chunk.
    pub inst: Option<Inst>,
    /// The `LIST INFO` metadata list.
    pub info: Option<InfoList>,
    /// The `LIST adtl` associated-data list.
    pub adtl: Option<AdtlList>,
}

impl WaveFile {
    /// Decode the typed `WAVE` chunks out of a parsed [`RiffTree`].
    ///
    /// Errors if the tree's form type is not `WAVE`, or if a chunk that
    /// *is* present fails to decode (a malformed `fmt `, a `cue ` whose
    /// declared count overruns its body, …) — the underlying typed
    /// decoder's `Error::invalid` propagates. Chunks that are simply
    /// absent leave their field `None`.
    pub fn from_tree(tree: &RiffTree) -> Result<Self> {
        if tree.form_type != FORM_WAVE {
            return Err(Error::invalid("WAVE: tree form type is not 'WAVE'"));
        }
        let mut wav = WaveFile::default();
        for child in &tree.children {
            match child {
                RiffChunk::Leaf { id, body } => match id {
                    b"fmt " => wav.format = Some(WaveFormat::parse(body)?),
                    b"fact" => wav.fact = Some(Fact::parse(body)?),
                    b"cue " => wav.cue = Some(CueChunk::parse(body)?),
                    b"plst" => wav.playlist = Some(Playlist::parse(body)?),
                    b"bext" => wav.bext = Some(BroadcastExtension::parse(body)?),
                    b"acid" => wav.acid = Some(Acid::parse(body)?),
                    b"smpl" => wav.smpl = Some(Smpl::parse(body)?),
                    b"inst" => wav.inst = Some(Inst::parse(body)?),
                    _ => {}
                },
                RiffChunk::Group {
                    id: [b'L', b'I', b'S', b'T'],
                    form_type,
                    children,
                } => match form_type {
                    b"INFO" => wav.info = Some(decode_info(children)),
                    b"adtl" => wav.adtl = Some(decode_adtl(children)?),
                    _ => {}
                },
                RiffChunk::Group { .. } => {}
            }
        }
        Ok(wav)
    }

    /// `true` if the file carries a `fmt ` descriptor — the one chunk a
    /// usable WAV file cannot omit.
    pub fn has_format(&self) -> bool {
        self.format.is_some()
    }
}

/// Decode a `LIST INFO`'s children into an [`InfoList`].
fn decode_info(children: &[RiffChunk]) -> InfoList {
    let mut info = InfoList::new();
    for child in children {
        if let RiffChunk::Leaf { id, body } = child {
            info.push_chunk(*id, body);
        }
    }
    info
}

/// Decode a `LIST adtl`'s children into an [`AdtlList`].
fn decode_adtl(children: &[RiffChunk]) -> Result<AdtlList> {
    let mut adtl = AdtlList::new();
    for child in children {
        if let RiffChunk::Leaf { id, body } = child {
            adtl.push_chunk(*id, body)?;
        }
    }
    Ok(adtl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::encode_chunk;

    /// Build a RIFF/WAVE tree with fmt + fact + LIST INFO{INAM} +
    /// LIST adtl{labl}.
    fn sample_tree() -> RiffTree {
        // fmt: minimal 16-byte WAVEFORMAT (PCM, mono, 8 kHz, 8-bit).
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes()); // wFormatTag = PCM
        fmt.extend_from_slice(&1u16.to_le_bytes()); // nChannels
        fmt.extend_from_slice(&8000u32.to_le_bytes()); // nSamplesPerSec
        fmt.extend_from_slice(&8000u32.to_le_bytes()); // nAvgBytesPerSec
        fmt.extend_from_slice(&1u16.to_le_bytes()); // nBlockAlign
        fmt.extend_from_slice(&8u16.to_le_bytes()); // wBitsPerSample

        // fact: 4-byte dwSampleLength.
        let fact = 1000u32.to_le_bytes().to_vec();

        // LIST INFO { INAM "Hi" }
        let mut info = Vec::new();
        info.extend_from_slice(b"INFO");
        encode_chunk(&mut info, b"INAM", b"Hi").unwrap();

        // LIST adtl { labl: dwCuePointID(0) + "lbl\0" }
        let mut labl = Vec::new();
        labl.extend_from_slice(&0u32.to_le_bytes());
        labl.extend_from_slice(b"lbl\0");
        let mut adtl = Vec::new();
        adtl.extend_from_slice(b"adtl");
        encode_chunk(&mut adtl, b"labl", &labl).unwrap();

        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"fmt ", &fmt).unwrap();
        encode_chunk(&mut body, b"fact", &fact).unwrap();
        encode_chunk(&mut body, b"data", &[0u8; 16]).unwrap();
        encode_chunk(&mut body, b"LIST", &info).unwrap();
        encode_chunk(&mut body, b"LIST", &adtl).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        RiffTree::parse(&bytes).unwrap()
    }

    #[test]
    fn decodes_the_common_wave_chunks() {
        let tree = sample_tree();
        let wav = WaveFile::from_tree(&tree).unwrap();
        assert!(wav.has_format());
        let fmt = wav.format.as_ref().unwrap();
        assert_eq!(fmt.channels, 1);
        assert_eq!(fmt.sample_rate, 8000);
        assert_eq!(wav.fact.as_ref().unwrap().sample_length, 1000);
        let info = wav.info.as_ref().unwrap();
        assert_eq!(info.len(), 1);
        let adtl = wav.adtl.as_ref().unwrap();
        assert_eq!(adtl.entries().len(), 1);
        // Chunks not present are None.
        assert!(wav.cue.is_none());
        assert!(wav.bext.is_none());
    }

    #[test]
    fn rejects_non_wave_form() {
        // Build a RIFF/AVI tree.
        let mut body = Vec::new();
        body.extend_from_slice(b"AVI ");
        encode_chunk(&mut body, b"junk", &[0u8; 4]).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        let err = WaveFile::from_tree(&tree).unwrap_err();
        assert!(format!("{err}").contains("not 'WAVE'"));
    }

    #[test]
    fn empty_wave_decodes_to_all_none() {
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        let wav = WaveFile::from_tree(&tree).unwrap();
        assert!(!wav.has_format());
        assert_eq!(wav, WaveFile::default());
    }

    #[test]
    fn malformed_fmt_propagates_error() {
        // fmt with a 3-byte body — too short for WAVEFORMAT.
        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        encode_chunk(&mut body, b"fmt ", &[1, 0, 1]).unwrap();
        let mut bytes = Vec::new();
        encode_chunk(&mut bytes, b"RIFF", &body).unwrap();
        let tree = RiffTree::parse(&bytes).unwrap();
        assert!(WaveFile::from_tree(&tree).is_err());
    }
}
