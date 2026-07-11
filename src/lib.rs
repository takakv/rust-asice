//! ASiC-E container format.

use thiserror::Error;

mod container;
mod manifest;

pub use container::{Container, DataFile, OpenOptions, SignatureFile};

/// ASiC-E container media type.
pub const MIMETYPE: &str = "application/vnd.etsi.asic-e+zip";

/// OpenDocument manifest namespace (META-INF/manifest.xml).
pub(crate) const MANIFEST_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";

/// Errors produced while reading or writing containers.
#[derive(Error, Debug)]
pub enum LibError {
    /// Structural problem.
    #[error("container: {0}")]
    Container(String),

    /// Underlying zip archive error.
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// I/O error while reading or writing.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Malformed XML entry.
    #[error("xml: {0}")]
    Xml(String),
}

pub type Result<T> = std::result::Result<T, LibError>;
