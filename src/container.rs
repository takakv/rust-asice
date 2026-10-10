//! In-memory representation of an ASiC-E container.

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, Write};

use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{LibError, MIMETYPE, Result, manifest};

/// A data file stored in the container.
#[derive(Debug, Clone)]
pub struct DataFile {
    /// File name inside the archive.
    pub name: String,
    /// Media type recorded in the manifest.
    pub mime_type: String,
    /// Raw file content.
    pub content: Vec<u8>,
}

/// A signature document (`META-INF/*signatures*.xml`).
#[derive(Debug, Clone)]
pub struct SignatureFile {
    /// Entry name inside the archive, e.g. `META-INF/signatures0.xml`.
    pub name: String,
    /// The signature document as opaque XML.
    pub xml: String,
}

/// An uninterpreted `META-INF/` entry.
#[derive(Debug, Clone)]
pub struct MetaInfFile {
    /// Entry name inside the archive, e.g. `META-INF/metadata.xml`.
    pub name: String,
    /// Raw file content.
    pub content: Vec<u8>,
}

/// Policy for `Container::open_with`.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Require the `mimetype` entry to be present.
    ///
    /// Optional in EN 319 162-1 but additional profiles (e.g. BDOC) may require it.
    /// Defaults to `false`.
    pub require_mimetype: bool,
    /// Maximum total uncompressed size of the container's entries, in bytes.
    /// Defaults to [`OpenOptions::DEFAULT_MAX_TOTAL_SIZE`].
    pub max_total_size: u64,
}

impl OpenOptions {
    /// Default for [`OpenOptions::max_total_size`]: 512 MiB.
    pub const DEFAULT_MAX_TOTAL_SIZE: u64 = 512 * 1024 * 1024;
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            require_mimetype: false,
            max_total_size: Self::DEFAULT_MAX_TOTAL_SIZE,
        }
    }
}

/// An ASiC-E container held in memory.
#[derive(Debug, Default)]
pub struct Container {
    data_files: Vec<DataFile>,
    signatures: Vec<SignatureFile>,
    /// Potential structural anomalies found while opening a container.
    warnings: Vec<String>,
    /// Uninterpreted `META-INF/` entries.
    meta_inf_files: Vec<MetaInfFile>,
    original_manifest: Option<Vec<u8>>,
}

impl Container {
    /// Create an empty container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Data files in the container.
    pub fn data_files(&self) -> &[DataFile] {
        &self.data_files
    }

    /// Signature documents in the container.
    pub fn signatures(&self) -> &[SignatureFile] {
        &self.signatures
    }

    /// Uninterpreted `META-INF/` entries.
    pub fn meta_inf_files(&self) -> &[MetaInfFile] {
        &self.meta_inf_files
    }

    /// Structural warnings collected when opening the container.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Add a data file.
    ///
    /// # Errors
    /// If the name is not relative, a duplicate, or reserved.
    pub fn add_file(
        &mut self,
        name: impl Into<String>,
        mime_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Result<()> {
        let name = name.into();
        validate_entry_name(&name)?;
        if self.data_files.iter().any(|f| f.name == name) {
            return Err(LibError::Container(format!("duplicate file name: {name}")));
        }
        self.data_files.push(DataFile {
            name,
            mime_type: mime_type.into(),
            content,
        });
        self.original_manifest = None;
        Ok(())
    }

    /// Append a signature document.
    ///
    /// The entry name is assigned automatically as `META-INF/signaturesN.xml`,
    /// using the first free index `N`.
    pub fn add_signature_xml(&mut self, xml: String) -> &SignatureFile {
        let mut n = self.signatures.len();
        let name = loop {
            let candidate = format!("META-INF/signatures{n}.xml");
            if !self.signatures.iter().any(|s| s.name == candidate) {
                break candidate;
            }
            n += 1;
        };
        self.signatures.push(SignatureFile { name, xml });
        self.signatures.last().expect("a signature should exist")
    }

    /// Open a container from any seekable reader, using default `OpenOptions`.
    pub fn open<R: Read + Seek>(reader: R) -> Result<Self> {
        Self::open_with(reader, &OpenOptions::default())
    }

    /// Open a container from any seekable reader under an explicit policy.
    pub fn open_with<R: Read + Seek>(reader: R, opts: &OpenOptions) -> Result<Self> {
        // TODO: check for duplicate entry names once https://github.com/zip-rs/zip2/issues/841
        // is addressed.
        let mut zip = ZipArchive::new(reader)?;
        let mut container = Container::default();
        let mut total_size = 0;

        let names = zip
            .file_names()
            .map(|name| Ok(name?.into()))
            .collect::<Result<Vec<String>>>()?;

        // EN 319 162-1 v1.1.1, A.1. If present, the mimetype shall
        // - be the first file in the ASiC container
        // - not be compressed
        // BDOC mandates the presence of the mimetype file.
        let mimetype_present = names.iter().any(|n| n == "mimetype");
        if let Some(n) = names.first() {
            if n == "mimetype" {
                let entry = zip.by_index(0)?;
                if entry.compression() != CompressionMethod::Stored {
                    container.warnings.push("'mimetype' is compressed".into());
                }
            } else if mimetype_present {
                container
                    .warnings
                    .push("'mimetype' is not the first zip entry".into());
            }
        }
        if mimetype_present {
            let mimetype = read_entry(&mut zip, "mimetype", opts, &mut total_size)?;
            let mimetype = String::from_utf8_lossy(&mimetype);
            if mimetype.trim() != MIMETYPE {
                return Err(LibError::Container(format!(
                    "unexpected container mime type: {}",
                    mimetype.trim()
                )));
            }
        } else if opts.require_mimetype {
            return Err(LibError::Container("missing mimetype entry".into()));
        } else {
            container
                .warnings
                .push("container has no mimetype entry".into());
        }

        // EN 319 162-1 V1.1.1 § 5.3.3 (Table 5)
        // 'META-INF/manifest.xml shall be present' for ASiC-E with XAdES signatures
        if !names.iter().any(|n| n == "META-INF/manifest.xml") {
            return Err(LibError::Container(
                "container has no META-INF/manifest.xml".into(),
            ));
        }
        let manifest_bytes = read_entry(&mut zip, "META-INF/manifest.xml", opts, &mut total_size)?;
        let manifest_entries = manifest::parse(&manifest_bytes)?;
        container.original_manifest = Some(manifest_bytes);

        let mut manifest_types: HashMap<String, String> = HashMap::new();
        for (path, media_type) in manifest_entries {
            // ODF 1.2 Part 3, 3.2: the manifest and the mimetype are not listed.
            if path == "META-INF/manifest.xml" || path == "mimetype" {
                container
                    .warnings
                    .push(format!("manifest must not list '{path}'"));
            }
            // ODF 1.2 Part 3, 4.8.4: only files and folders within the zip are listed.
            if !path.ends_with('/') && !names.contains(&path) {
                container.warnings.push(format!(
                    "manifest lists '{path}', which is not in the container"
                ));
            }
            // ODF 1.2 Part 3, 3.2: each file is listed only once.
            if manifest_types.contains_key(&path) {
                return Err(LibError::Container(format!(
                    "manifest lists '{path}' more than once"
                )));
            }
            manifest_types.insert(path, media_type);
        }

        // ODF 1.2 Part 3, 3.2 and 3.3: "/" is listed if and only if there is a mimetype,
        // and its media type matches the mimetype's content.
        match (manifest_types.get("/"), mimetype_present) {
            (None, true) => container
                .warnings
                .push("manifest has no '/' entry, although the container has a mimetype".into()),
            (Some(_), false) => container
                .warnings
                .push("manifest has a '/' entry, although the container has no mimetype".into()),
            (Some(media_type), true) if media_type != MIMETYPE => container.warnings.push(format!(
                "manifest gives '/' the media type '{media_type}', which differs from the mimetype"
            )),
            _ => {}
        }

        for name in &names {
            if name == "mimetype" || name == "META-INF/manifest.xml" || name.ends_with('/') {
                continue;
            }
            let bytes = read_entry(&mut zip, name, opts, &mut total_size)?;
            if let Some(rest) = name.strip_prefix("META-INF/") {
                if is_signature_entry(rest) {
                    container.signatures.push(SignatureFile {
                        name: name.clone(),
                        xml: String::from_utf8(bytes)
                            .map_err(|_| LibError::Xml(format!("{name} is not valid UTF-8")))?,
                    });
                } else {
                    if has_reserved_meta_inf_name(rest) {
                        container
                            .warnings
                            .push(format!("{name}: reserved word in META-INF file name"));
                    }
                    container.meta_inf_files.push(MetaInfFile {
                        name: name.clone(),
                        content: bytes,
                    });
                }
                continue;
            }
            let mime_type = manifest_types.remove(name).ok_or_else(|| {
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

    /// Open a container from a byte buffer under an explicit policy.
    pub fn from_bytes_with(bytes: &[u8], opts: &OpenOptions) -> Result<Self> {
        Self::open_with(Cursor::new(bytes), opts)
    }

    /// Open a container file from disk.
    pub fn open_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open(std::fs::File::open(path)?)
    }

    /// Open a container file from disk under an explicit policy.
    pub fn open_file_with(path: impl AsRef<std::path::Path>, opts: &OpenOptions) -> Result<Self> {
        Self::open_with(std::fs::File::open(path)?, opts)
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

        // A signature may cover the manifest (EN 319 162-1 v1.1.1, 4.4.3.2 Note 4).
        zip.start_file("META-INF/manifest.xml", deflated)?;
        match &self.original_manifest {
            Some(bytes) => zip.write_all(bytes)?,
            None => zip.write_all(&manifest::build(&self.data_files))?,
        }

        for sig in &self.signatures {
            zip.start_file(&sig.name, deflated)?;
            zip.write_all(sig.xml.as_bytes())?;
        }

        for file in &self.meta_inf_files {
            zip.start_file(&file.name, deflated)?;
            zip.write_all(&file.content)?;
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

fn read_entry<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    name: &str,
    opts: &OpenOptions,
    total_size: &mut u64,
) -> Result<Vec<u8>> {
    let mut entry = match zip.by_name(name) {
        Ok(entry) => entry,
        Err(ZipError::FileNotFound) => {
            return Err(LibError::Container(format!("missing zip entry: {name}")));
        }
        Err(e) => return Err(e.into()),
    };
    *total_size = total_size.saturating_add(entry.size());
    if *total_size > opts.max_total_size {
        return Err(LibError::Container(format!(
            "container entries declare more than the {} byte limit in total",
            opts.max_total_size
        )));
    }
    // `zip` fails the read if the entry decompresses to more than its declared size.
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn validate_entry_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(LibError::Container("file name must not be empty".into()));
    }
    if name == "mimetype" || name.starts_with("META-INF/") {
        return Err(LibError::Container(format!("reserved entry name: {name}")));
    }
    if name.starts_with('/') || name.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err(LibError::Container(format!(
            "file name must be a clean relative path: {name}"
        )));
    }
    Ok(())
}

fn is_signature_entry(meta_inf_rest: &str) -> bool {
    // See '4.4.3.2 2)' of EN 319 162-1 v1.1.1
    meta_inf_rest.contains("signatures") && meta_inf_rest.ends_with(".xml")
}

fn has_reserved_meta_inf_name(meta_inf_rest: &str) -> bool {
    // See '4.4.3.2 5) d) i)' of EN 319 162-1 v1.1.1
    let defined = meta_inf_rest == "container.xml"
        || meta_inf_rest == "metadata.xml"
        || (meta_inf_rest.starts_with("ASiCEvidenceRecordManifest")
            && meta_inf_rest.ends_with(".xml"));
    let lower = meta_inf_rest.to_ascii_lowercase();
    !defined
        && ["signature", "timestamp", "manifest", "container.xml"]
            .iter()
            .any(|word| lower.contains(word))
}
