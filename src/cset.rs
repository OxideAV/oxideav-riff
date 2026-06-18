//! Typed lookups for the RIFF **country / language / dialect** code
//! tables, plus the `CSET` character-set chunk decoder.
//!
//! The 1991 RIFF spec defines a `CSET` (character set) chunk that carries
//! the file-wide code page, country, language, and dialect codes, and it
//! registers fixed numeric tables for each:
//!
//! ```text
//! <CSET chunk> -> CSET( <wCodePage:WORD>
//!                       <wCountryCode:WORD>
//!                       <wLanguageCode:WORD>
//!                       <wDialect:WORD> )
//! ```
//!
//! The same `wCountry` / `wLanguage` / `wDialect` numeric codes appear in
//! the `LIST adtl` `ltxt` (labelled-text) record, where they scope a
//! text segment to a particular locale; the `ltxt` decoder records them
//! as raw `u16` values, and this module supplies the typed name lookups
//! ([`country_name`] / [`language_name`]) those raw codes resolve to.
//!
//! All four `CSET` fields default to the "assume US English" baseline
//! when the chunk is absent or a field is zero, per the spec's per-field
//! defaulting rules: code page → ISO 8859/1, country → `001` (USA),
//! language/dialect → `9` / `1` (US English).
//!
//! ## Clean-room sources
//!
//! - `docs/container/riff/metadata/microsoft-riffmci.pdf` §2 —
//!   "CSET (Character Set) Chunk", "Country Codes", and "Language and
//!   Dialect Codes" (the chunk grammar, the per-field defaulting rules,
//!   and the two complete numeric-code tables transcribed below).

use crate::error::{Error, Result};

/// FourCC of the character-set chunk.
pub const FOURCC_CSET: [u8; 4] = *b"CSET";

/// Size in bytes of the `CSET` chunk body: four 16-bit fields.
pub const CSET_LEN: usize = 8;

/// Default country code (`001`, USA) assumed when `CSET` is absent or the
/// `wCountryCode` field is zero.
pub const DEFAULT_COUNTRY: u16 = 1;

/// Default language code (`9`, English) assumed when `CSET` is absent or
/// the `wLanguageCode` field is zero.
pub const DEFAULT_LANGUAGE: u16 = 9;

/// Default dialect code (`1`) assumed when `CSET` is absent or the
/// `wDialect` field is zero. Paired with [`DEFAULT_LANGUAGE`] this is
/// "US English".
pub const DEFAULT_DIALECT: u16 = 1;

/// Resolve a `wCountryCode` to its registered country name, or `None` for
/// an unregistered code.
///
/// Code `0` ("None — ignore this field") returns `None`, matching the
/// spec's "ignore" semantics; callers that want the defaulted "USA"
/// behaviour use [`country_name_defaulted`].
pub fn country_name(code: u16) -> Option<&'static str> {
    Some(match code {
        1 => "USA",
        2 => "Canada",
        3 => "Latin America",
        30 => "Greece",
        31 => "Netherlands",
        32 => "Belgium",
        33 => "France",
        34 => "Spain",
        39 => "Italy",
        41 => "Switzerland",
        43 => "Austria",
        44 => "United Kingdom",
        45 => "Denmark",
        46 => "Sweden",
        47 => "Norway",
        49 => "West Germany",
        52 => "Mexico",
        55 => "Brazil",
        61 => "Australia",
        64 => "New Zealand",
        81 => "Japan",
        82 => "Korea",
        86 => "People's Republic of China",
        88 => "Taiwan",
        90 => "Turkey",
        351 => "Portugal",
        352 => "Luxembourg",
        354 => "Iceland",
        358 => "Finland",
        _ => return None,
    })
}

/// Resolve a `wCountryCode` with the spec's "assume USA" default applied:
/// code `0` resolves to `"USA"` (country `001`), other codes resolve as
/// [`country_name`].
pub fn country_name_defaulted(code: u16) -> Option<&'static str> {
    if code == 0 {
        country_name(DEFAULT_COUNTRY)
    } else {
        country_name(code)
    }
}

/// Resolve a (`wLanguage`, `wDialect`) pair to its registered language
/// name, or `None` for an unregistered pair.
///
/// The spec registers language/dialect as a *pair*: a single language
/// code can carry several dialects (e.g. language `12` French has
/// dialects 1 Belgian-less French, 2 Belgian, 3 Canadian, 4 Swiss). The
/// pair `(0, 0)` ("None — ignore these fields") returns `None`.
pub fn language_name(language: u16, dialect: u16) -> Option<&'static str> {
    Some(match (language, dialect) {
        (1, 1) => "Arabic",
        (2, 1) => "Bulgarian",
        (3, 1) => "Catalan",
        (4, 1) => "Traditional Chinese",
        (4, 2) => "Simplified Chinese",
        (5, 1) => "Czech",
        (6, 1) => "Danish",
        (7, 1) => "German",
        (7, 2) => "Swiss German",
        (8, 1) => "Greek",
        (9, 1) => "US English",
        (9, 2) => "UK English",
        (10, 1) => "Spanish",
        (10, 2) => "Spanish Mexican",
        (11, 1) => "Finnish",
        (12, 1) => "French",
        (12, 2) => "Belgian French",
        (12, 3) => "Canadian French",
        (12, 4) => "Swiss French",
        (13, 1) => "Hebrew",
        (14, 1) => "Hungarian",
        (15, 1) => "Icelandic",
        (16, 1) => "Italian",
        (16, 2) => "Swiss Italian",
        (17, 1) => "Japanese",
        (18, 1) => "Korean",
        (19, 1) => "Dutch",
        (19, 2) => "Belgian Dutch",
        (20, 1) => "Norwegian - Bokmal",
        (20, 2) => "Norwegian - Nynorsk",
        (21, 1) => "Polish",
        (22, 1) => "Brazilian Portuguese",
        (22, 2) => "Portuguese",
        (23, 1) => "Rhaeto-Romanic",
        (24, 1) => "Romanian",
        (25, 1) => "Russian",
        (26, 1) => "Serbo-Croatian (Latin)",
        (26, 2) => "Serbo-Croatian (Cyrillic)",
        (27, 1) => "Slovak",
        (28, 1) => "Albanian",
        (29, 1) => "Swedish",
        (30, 1) => "Thai",
        (31, 1) => "Turkish",
        (32, 1) => "Urdu",
        (33, 1) => "Bahasa",
        _ => return None,
    })
}

/// Resolve a (`wLanguage`, `wDialect`) pair with the spec's "assume US
/// English" default applied: the pair `(0, 0)` resolves to `"US English"`
/// (language `9`, dialect `1`), other pairs resolve as [`language_name`].
pub fn language_name_defaulted(language: u16, dialect: u16) -> Option<&'static str> {
    if language == 0 && dialect == 0 {
        language_name(DEFAULT_LANGUAGE, DEFAULT_DIALECT)
    } else {
        language_name(language, dialect)
    }
}

/// A decoded `CSET` (character set) chunk.
///
/// Carries the file-wide code page, country, language, and dialect
/// codes. All four are raw on-wire `u16` values; the typed name lookups
/// are exposed via the [`country`](CharacterSet::country) /
/// [`language`](CharacterSet::language) accessors which apply the spec's
/// zero-field defaulting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CharacterSet {
    /// `wCodePage` — the code page used for file elements. Zero means
    /// "assume standard ISO 8859/1".
    pub code_page: u16,
    /// `wCountryCode` — the country code (the §2 Country Codes table).
    pub country_code: u16,
    /// `wLanguageCode` — the language code (the §2 Language and Dialect
    /// Codes table).
    pub language_code: u16,
    /// `wDialect` — the dialect code, paired with `language_code`.
    pub dialect: u16,
}

impl CharacterSet {
    /// Decode a `CSET` chunk body (already pad-stripped by the walker).
    ///
    /// The body must be exactly [`CSET_LEN`] (8) bytes — four 16-bit
    /// little-endian fields. A short or over-long body is rejected.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() != CSET_LEN {
            return Err(Error::invalid("RIFF: CSET chunk is not 8 bytes"));
        }
        let w = |off: usize| u16::from_le_bytes([body[off], body[off + 1]]);
        Ok(CharacterSet {
            code_page: w(0),
            country_code: w(2),
            language_code: w(4),
            dialect: w(6),
        })
    }

    /// The registered country name, with the spec's "assume USA" default
    /// applied for a zero `wCountryCode`.
    pub fn country(&self) -> Option<&'static str> {
        country_name_defaulted(self.country_code)
    }

    /// The registered language name, with the spec's "assume US English"
    /// default applied for a zero (`wLanguageCode`, `wDialect`) pair.
    pub fn language(&self) -> Option<&'static str> {
        language_name_defaulted(self.language_code, self.dialect)
    }

    /// `true` when the chunk's code page is the implicit ISO 8859/1
    /// default (`wCodePage == 0`).
    pub fn is_default_code_page(&self) -> bool {
        self.code_page == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cset_len_is_eight() {
        assert_eq!(CSET_LEN, 8);
    }

    #[test]
    fn parse_cset_body() {
        // wCodePage = 1252, country = 044 (UK), language = 9 / dialect 2
        // (UK English).
        let mut body = Vec::new();
        body.extend_from_slice(&1252u16.to_le_bytes());
        body.extend_from_slice(&44u16.to_le_bytes());
        body.extend_from_slice(&9u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        let cs = CharacterSet::parse(&body).unwrap();
        assert_eq!(cs.code_page, 1252);
        assert_eq!(cs.country_code, 44);
        assert_eq!(cs.language_code, 9);
        assert_eq!(cs.dialect, 2);
        assert_eq!(cs.country(), Some("United Kingdom"));
        assert_eq!(cs.language(), Some("UK English"));
        assert!(!cs.is_default_code_page());
    }

    #[test]
    fn cset_zero_fields_default_to_us_english_usa() {
        let cs = CharacterSet::parse(&[0u8; 8]).unwrap();
        assert_eq!(cs.country(), Some("USA"));
        assert_eq!(cs.language(), Some("US English"));
        assert!(cs.is_default_code_page());
    }

    #[test]
    fn cset_wrong_length_rejected() {
        assert!(CharacterSet::parse(&[0u8; 7]).is_err());
        assert!(CharacterSet::parse(&[0u8; 9]).is_err());
    }

    #[test]
    fn country_table_spot_checks() {
        assert_eq!(country_name(1), Some("USA"));
        assert_eq!(country_name(33), Some("France"));
        assert_eq!(country_name(49), Some("West Germany"));
        assert_eq!(country_name(81), Some("Japan"));
        assert_eq!(country_name(86), Some("People's Republic of China"));
        assert_eq!(country_name(351), Some("Portugal"));
        assert_eq!(country_name(358), Some("Finland"));
        // Code 0 ("None — ignore") and an unregistered code both miss.
        assert_eq!(country_name(0), None);
        assert_eq!(country_name(999), None);
    }

    #[test]
    fn country_defaulting_maps_zero_to_usa() {
        assert_eq!(country_name_defaulted(0), Some("USA"));
        assert_eq!(country_name_defaulted(33), Some("France"));
        assert_eq!(country_name_defaulted(999), None);
    }

    #[test]
    fn language_pair_disambiguates_dialect() {
        // Same language code, different dialects → different names.
        assert_eq!(language_name(9, 1), Some("US English"));
        assert_eq!(language_name(9, 2), Some("UK English"));
        assert_eq!(language_name(12, 1), Some("French"));
        assert_eq!(language_name(12, 2), Some("Belgian French"));
        assert_eq!(language_name(12, 3), Some("Canadian French"));
        assert_eq!(language_name(12, 4), Some("Swiss French"));
        // Chinese: traditional vs simplified by dialect.
        assert_eq!(language_name(4, 1), Some("Traditional Chinese"));
        assert_eq!(language_name(4, 2), Some("Simplified Chinese"));
    }

    #[test]
    fn language_table_spot_checks() {
        assert_eq!(language_name(17, 1), Some("Japanese"));
        assert_eq!(language_name(25, 1), Some("Russian"));
        assert_eq!(language_name(33, 1), Some("Bahasa"));
        // (0, 0) ignore-pair and unregistered pairs miss.
        assert_eq!(language_name(0, 0), None);
        assert_eq!(language_name(9, 99), None);
        assert_eq!(language_name(999, 1), None);
    }

    #[test]
    fn language_defaulting_maps_zero_pair_to_us_english() {
        assert_eq!(language_name_defaulted(0, 0), Some("US English"));
        assert_eq!(language_name_defaulted(9, 2), Some("UK English"));
        // A non-zero language with an unregistered dialect is NOT
        // defaulted — only the all-zero "ignore" pair is.
        assert_eq!(language_name_defaulted(9, 99), None);
    }
}
