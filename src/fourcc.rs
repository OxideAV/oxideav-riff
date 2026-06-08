//! FourCC helpers.
//!
//! RIFF identifies every chunk type with a 4-byte ASCII tag called a
//! *FourCC* (four-character-code). Per the 1991 spec
//! (`docs/container/riff/metadata/microsoft-riffmci.pdf` §1.3), a
//! FourCC is "four printable ASCII characters; if the meaningful
//! string is shorter than four characters, it is padded on the right
//! with ASCII space (0x20)". Examples seen in the wild:
//!
//! - `RIFF` — outer wrapper.
//! - `LIST` — nested grouping.
//! - `WAVE` — RIFF form type for WAV files.
//! - `AVI ` — RIFF form type for AVI files (trailing space pad).
//! - `WEBP` — RIFF form type for WebP images.
//! - `fmt ` — WAV format-descriptor chunk (trailing space pad).
//! - `bext` — Broadcast Wave metadata chunk.
//! - `data` — payload-bearing chunk in WAV / AVI.
//! - `JUNK` — pad chunk.
//!
//! Real-world readers must tolerate non-printable bytes too (corrupt
//! files, AMV-style RIFF-derivatives with vendor extensions, …) — the
//! helpers below decode such tags as a hex-escaped fallback rather
//! than panicking so a debug dump of a malformed file is still
//! human-readable.

/// Convert a literal 4-byte ASCII tag into the wire byte array.
///
/// The compile-time helper accepts string literals of exactly 4
/// bytes; callers that need a runtime conversion should construct
/// the byte array directly.
///
/// ```
/// use oxideav_riff::fourcc_bytes;
/// assert_eq!(fourcc_bytes(b"RIFF"), *b"RIFF");
/// assert_eq!(fourcc_bytes(b"fmt "), [b'f', b'm', b't', b' ']);
/// ```
pub const fn fourcc_bytes(s: &[u8; 4]) -> [u8; 4] {
    [s[0], s[1], s[2], s[3]]
}

/// Render a FourCC as a human-readable string.
///
/// Printable ASCII characters (0x20..=0x7E) are emitted verbatim;
/// any non-printable byte is rendered as `\xNN` so the result is
/// safe to drop into `panic!` / `log::warn!` / `Debug` impls without
/// breaking the surrounding text.
///
/// ```
/// use oxideav_riff::fourcc_to_string;
/// assert_eq!(fourcc_to_string(b"RIFF"), "RIFF");
/// assert_eq!(fourcc_to_string(b"fmt "), "fmt ");
/// assert_eq!(fourcc_to_string(&[0x01, b'A', b'B', 0xFF]), r"\x01AB\xff");
/// ```
pub fn fourcc_to_string(tag: &[u8; 4]) -> String {
    let mut out = String::with_capacity(4);
    for &b in tag {
        if (0x20..=0x7E).contains(&b) {
            out.push(b as char);
        } else {
            // Two-hex-digit escape, lower-case to match Rust's own
            // `{:x?}` debug rendering of unprintable byte slices.
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

/// Return `true` if every byte of the FourCC is a printable ASCII
/// character (0x20..=0x7E).
///
/// The chunk-walker uses this on outermost-`RIFF` tag candidates to
/// reject obvious garbage (e.g. a JPEG SOI marker mis-fed into a RIFF
/// parser) before it tries to seek by `ckSize` bytes.
///
/// ```
/// use oxideav_riff::is_printable_fourcc;
/// assert!(is_printable_fourcc(b"RIFF"));
/// assert!(is_printable_fourcc(b"fmt "));
/// assert!(!is_printable_fourcc(&[0xFF, 0xD8, 0xFF, 0xE0]));
/// ```
pub const fn is_printable_fourcc(tag: &[u8; 4]) -> bool {
    let mut i = 0;
    while i < 4 {
        let b = tag[i];
        if b < 0x20 || b > 0x7E {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_bytes_basic() {
        assert_eq!(fourcc_bytes(b"RIFF"), [b'R', b'I', b'F', b'F']);
        assert_eq!(fourcc_bytes(b"LIST"), [b'L', b'I', b'S', b'T']);
        assert_eq!(fourcc_bytes(b"AVI "), [b'A', b'V', b'I', b' ']);
    }

    #[test]
    fn fourcc_to_string_printable() {
        assert_eq!(fourcc_to_string(b"RIFF"), "RIFF");
        assert_eq!(fourcc_to_string(b"WAVE"), "WAVE");
        assert_eq!(fourcc_to_string(b"AVI "), "AVI ");
        assert_eq!(fourcc_to_string(b"fmt "), "fmt ");
        // Whole boundary range — every printable ASCII byte round-trips.
        for b in 0x20u8..=0x7E {
            let s = fourcc_to_string(&[b, b, b, b]);
            assert_eq!(s.chars().count(), 4);
        }
    }

    #[test]
    fn fourcc_to_string_escapes_unprintable() {
        assert_eq!(
            fourcc_to_string(&[0x00, 0x00, 0x00, 0x00]),
            r"\x00\x00\x00\x00"
        );
        assert_eq!(
            fourcc_to_string(&[0xFF, 0xFE, 0x00, 0x7F]),
            r"\xff\xfe\x00\x7f"
        );
        // Mixed printable + non-printable.
        assert_eq!(fourcc_to_string(&[b'a', 0x01, b'B', 0x80]), r"a\x01B\x80");
    }

    #[test]
    fn is_printable_fourcc_basic() {
        assert!(is_printable_fourcc(b"RIFF"));
        assert!(is_printable_fourcc(b"fmt "));
        assert!(is_printable_fourcc(b"    "));
        assert!(is_printable_fourcc(b"~~~~"));
        assert!(!is_printable_fourcc(&[0x1F, b'A', b'B', b'C']));
        assert!(!is_printable_fourcc(&[b'A', b'B', b'C', 0x7F]));
        assert!(!is_printable_fourcc(&[0xFF, 0xFE, 0xFD, 0xFC]));
        // The literal value at the boundary (0x20 space, 0x7E tilde) is allowed.
        assert!(is_printable_fourcc(&[0x20, 0x7E, 0x20, 0x7E]));
    }
}
