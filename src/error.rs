//! Error / Result aliases.
//!
//! With the default-on `registry` feature, [`Error`] / [`Result`] are
//! re-exports of [`oxideav_core::Error`] / [`oxideav_core::Result`] so
//! the chunk-walker plugs directly into framework consumers without
//! conversion boilerplate.
//!
//! Without `registry`, the crate exposes a minimal in-tree [`Error`]
//! enum so it can be used as a standalone parsing library by callers
//! that don't want the framework dependency tree. The standalone enum
//! mirrors the framework's variant set at the API surface — `invalid`
//! for parser-detected wire violations and `Io` for transport
//! failures — so the same call sites work under either feature set.

#[cfg(feature = "registry")]
pub use oxideav_core::{Error, Result};

#[cfg(not(feature = "registry"))]
use core::fmt;

/// Standalone [`Error`] type used when the `registry` feature is off.
#[cfg(not(feature = "registry"))]
#[derive(Debug)]
pub enum Error {
    /// Wire-format violation: a chunk header was truncated, a size
    /// field overflowed the enclosing parent, an outer FourCC was
    /// non-printable, …
    Invalid(String),
    /// Underlying [`std::io::Error`] propagated up from the reader.
    Io(std::io::Error),
}

#[cfg(not(feature = "registry"))]
impl Error {
    /// Constructor mirroring `oxideav_core::Error::invalid` so call
    /// sites don't fork on feature flags.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }

    /// Constructor mirroring `oxideav_core::Error::other`.
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}

#[cfg(not(feature = "registry"))]
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(m) => write!(f, "RIFF: {m}"),
            Error::Io(e) => write!(f, "RIFF I/O: {e}"),
        }
    }
}

#[cfg(not(feature = "registry"))]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(not(feature = "registry"))]
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Standalone [`Result`] alias used when the `registry` feature is off.
#[cfg(not(feature = "registry"))]
pub type Result<T> = core::result::Result<T, Error>;
