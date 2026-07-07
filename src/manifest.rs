use crate::container::DataFile;
use crate::{LibError, Result, MANIFEST_NS, MIMETYPE};

pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let doc = uppsala::parse_bytes(bytes).map_err(|e| LibError::Xml(format!("manifest: {e}")))?;
    let mut entries = Vec::new();
    for id in doc.descendants(doc.root()) {
        let Some(el) = doc.element(id) else { continue };
        if &*el.name.local_name != "file-entry"
            || el.name.namespace_uri.as_deref() != Some(MANIFEST_NS)
        {
            continue;
        }
        let full_path = el.get_attribute_ns(MANIFEST_NS, "full-path");
        let media_type = el.get_attribute_ns(MANIFEST_NS, "media-type");
        if let (Some(p), Some(m)) = (full_path, media_type)
            && p != "/"
        {
            entries.push((p.to_owned(), m.to_owned()));
        }
    }
    Ok(entries)
}

pub(crate) fn build(files: &[DataFile]) -> Vec<u8> {
    let mut w = uppsala::XmlWriter::new();
    w.write_declaration();
    w.start_element("manifest:manifest", &[("xmlns:manifest", MANIFEST_NS)]);
    w.empty_element(
        "manifest:file-entry",
        &[
            ("manifest:full-path", "/"),
            ("manifest:media-type", MIMETYPE),
        ],
    );
    for file in files {
        w.empty_element(
            "manifest:file-entry",
            &[
                ("manifest:full-path", &file.name),
                ("manifest:media-type", &file.mime_type),
            ],
        );
    }
    w.end_element("manifest:manifest");
    w.into_bytes()
}
