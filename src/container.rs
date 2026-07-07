use std::io::{Cursor, Read, Seek, Write};

use crate::{manifest, LibError, Result, MIMETYPE};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// A payload file stored in the container.
#[derive(Debug, Clone)]
pub struct DataFile {
    pub name: String,
    pub mime_type: String,
    pub content: Vec<u8>,
}

/// A signature document (`META-INF/*signatures*.xml`), carried as opaque XML.
#[derive(Debug, Clone)]
pub struct SignatureFile {
    pub name: String,
    pub xml: String,
}

/// An ASiC-E container held in memory.
#[derive(Debug, Default)]
pub struct Container {
    data_files: Vec<DataFile>,
    signatures: Vec<SignatureFile>,
}

impl Container {
    /// Data files in the container.
    pub fn data_files(&self) -> &[DataFile] {
        &self.data_files
    }

    /// Signature documents in the container.
    pub fn signatures(&self) -> &[SignatureFile] {
        &self.signatures
    }

    fn open<R: Read + Seek>(reader: R) -> Result<Self> {
        let mut zip = ZipArchive::new(reader)?;
        let mut container = Container::default();

        let mut names: Vec<String> = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            names.push(zip.by_index(i)?.name().to_owned());
        }
        // EN 319 162-1 v1.1.1, A.1. If present, the mimetype shall
        // - be the first file in the ASiC container
        // - not be compressed
        // BDOC mandates the presence of the mimetype file.
        // TODO: parametrise the requirement for wider ASiC-E compatibility.
        match names.first() {
            Some(n) if n == "mimetype" => {
                let entry = zip.by_index(0)?;
                if entry.compression() != CompressionMethod::Stored {
                    return Err(LibError::Container("'mimetype' is compressed".into()));
                }
            }
            _ => {
                return if names.iter().any(|n| n == "mimetype") {
                    Err(LibError::Container(
                        "'mimetype' is not the first zip entry".into(),
                    ))
                } else {
                    Err(LibError::Container("missing mimetype entry".into()))
                };
            }
        }
        let mimetype = read_entry(&mut zip, "mimetype")?;
        let mimetype = String::from_utf8_lossy(&mimetype);
        if mimetype.trim() != MIMETYPE {
            return Err(LibError::Container(format!(
                "unexpected container mime type: {}",
                mimetype.trim()
            )));
        }

        // BDOC requires the presence of the 'manifest.xml'.
        // See '4.4.3.2 5) b)' of EN 319 162-1 v1.1.1 for manifest.xml specifics.
        // TODO: parametrise the requirement for wider ASiC-E compatibility.
        let manifest_types = match read_entry(&mut zip, "META-INF/manifest.xml") {
            Ok(bytes) => manifest::parse(&bytes)?,
            Err(_) => {
                return Err(LibError::Container(
                    "container has no META-INF/manifest.xml".into(),
                ));
            }
        };

        for name in &names {
            if name == "mimetype" || name.ends_with('/') {
                continue;
            }
            let bytes = read_entry(&mut zip, name)?;
            if let Some(rest) = name.strip_prefix("META-INF/") {
                if is_signature_entry(rest) {
                    container.signatures.push(SignatureFile {
                        name: name.clone(),
                        xml: String::from_utf8(bytes)
                            .map_err(|_| LibError::Xml(format!("{name} is not valid UTF-8")))?,
                    });
                }
                // '4.4.3.2 Note 5' of EN 319 162-1 v1.1.1:
                // Other file objects in META-INF/ need not be parsed and interpreted for the
                // purpose of the ASiC container validation, provided that they do not contain the
                // string "signature" or "timestamp" or "manifest" or "container.xml".
                // TODO: implement this check, and the processing of the other allowed files.
                continue;
            }
            let mime_type = manifest_types
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, m)| m.clone())
                .ok_or_else(|| {
                    LibError::Container(format!("missing MIME type in manifest for {name}"))
                })?;
            container.data_files.push(DataFile {
                name: name.clone(),
                mime_type,
                content: bytes,
            });
        }

        if container.data_files.is_empty() {
            return Err(LibError::Container("container has no data files".into()));
        }
        Ok(container)
    }

    /// Open a container from a byte buffer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::open(Cursor::new(bytes))
    }

    /// Open a container file from disk.
    pub fn open_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open(std::fs::File::open(path)?)
    }

    fn write_to<W: Write + Seek>(&self, writer: W) -> Result<()> {
        if self.data_files.is_empty() {
            return Err(LibError::Container("container has no data files".into()));
        }
        let mut zip = ZipWriter::new(writer);

        zip.start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )?;
        zip.write_all(MIMETYPE.as_bytes())?;

        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for file in &self.data_files {
            zip.start_file(&file.name, deflated)?;
            zip.write_all(&file.content)?;
        }

        zip.start_file("META-INF/manifest.xml", deflated)?;
        zip.write_all(&manifest::build(&self.data_files))?;

        for sig in &self.signatures {
            zip.start_file(&sig.name, deflated)?;
            zip.write_all(sig.xml.as_bytes())?;
        }

        zip.finish()?;
        Ok(())
    }

    /// Serialize the container to a byte buffer.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        self.write_to(&mut buf)?;
        Ok(buf.into_inner())
    }

    /// Write the container to a file on disk.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        self.write_to(std::fs::File::create(path)?)
    }
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>> {
    let mut entry = zip
        .by_name(name)
        .map_err(|_| LibError::Container(format!("missing zip entry: {name}")))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn is_signature_entry(meta_inf_rest: &str) -> bool {
    // See '4.4.3.2 2)' of EN 319 162-1 v1.1.1
    meta_inf_rest.contains("signatures") && meta_inf_rest.ends_with(".xml")
}
