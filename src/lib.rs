use thiserror::Error;

mod container;
mod manifest;

pub use container::{Container, DataFile, SignatureFile};

/// ASiC-E container media type.
pub const MIMETYPE: &str = "application/vnd.etsi.asic-e+zip";

/// OpenDocument manifest namespace (META-INF/manifest.xml).
pub const MANIFEST_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";

/// Errors produced while reading or writing containers.
#[derive(Error, Debug)]
pub enum LibError {
    #[error("container: {0}")]
    Container(String),

    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("xml: {0}")]
    Xml(String),
}

pub type Result<T> = std::result::Result<T, LibError>;
