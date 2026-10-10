use asic_e::Container;

fn container_with_extras() -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in [
        ("mimetype", asic_e::MIMETYPE.as_bytes()),
        ("test.txt", b"lorem ipsum".as_slice()),
        ("META-INF/manifest.xml", EXTRAS_MANIFEST.as_bytes()),
        (
            "META-INF/signatures0.xml",
            b"<asic:XAdESSignatures/>".as_slice(),
        ),
        ("META-INF/data.bin", b"lorem ipsum".as_slice()),
    ] {
        zip.start_file(name, stored).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

const EXTRAS_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
  <manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.etsi.asic-e+zip"/>
  <manifest:file-entry manifest:full-path="test.txt" manifest:media-type="text/plain"/>
</manifest:manifest>"#;

fn read_zip_entry(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
    Some(buf)
}

#[test]
fn round_trip_keeps_meta_inf_entries_and_manifest() {
    let mut container = Container::from_bytes(&container_with_extras()).unwrap();
    container.add_signature_xml("<asic:XAdESSignatures/>".into());
    let saved = container.to_bytes().unwrap();
    assert_eq!(
        read_zip_entry(&saved, "META-INF/data.bin").as_deref(),
        Some(b"lorem ipsum".as_slice())
    );
    assert_eq!(
        read_zip_entry(&saved, "META-INF/manifest.xml").as_deref(),
        Some(EXTRAS_MANIFEST.as_bytes())
    );

    container
        .add_file("new.txt", "text/plain", b"new".to_vec())
        .unwrap();
    let saved = container.to_bytes().unwrap();
    let manifest =
        String::from_utf8(read_zip_entry(&saved, "META-INF/manifest.xml").unwrap()).unwrap();
    assert!(manifest.contains(r#"manifest:full-path="new.txt""#));
    assert!(manifest.contains(r#"manifest:full-path="test.txt""#));
}
