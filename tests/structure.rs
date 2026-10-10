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

#[test]
fn warns_on_reserved_meta_inf_names() {
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
        // Allowed by EN 319 162-1, 4.4.3.2.
        ("META-INF/container.xml", b"<container/>".as_slice()),
        ("META-INF/metadata.xml", b"<metadata/>".as_slice()),
        (
            "META-INF/ASiCEvidenceRecordManifest1.xml",
            b"<ASiCManifest/>".as_slice(),
        ),
        // Contain a reserved word.
        ("META-INF/timestamp.tst", b"tst".as_slice()),
        (
            "META-INF/Signatures1.xml",
            b"<asic:XAdESSignatures/>".as_slice(),
        ),
    ] {
        zip.start_file(name, stored).unwrap();
        zip.write_all(content).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();

    let container = Container::from_bytes(&bytes).unwrap();
    assert_eq!(container.meta_inf_files().len(), 5);
    let warned: Vec<_> = container
        .warnings()
        .iter()
        .filter_map(|w| w.split(':').next())
        .collect();
    assert_eq!(
        warned,
        ["META-INF/timestamp.tst", "META-INF/Signatures1.xml"]
    );
}

fn uncompressed_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in files {
        zip.start_file(*name, stored).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

const MANIFEST_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";

fn container_with_manifest(mimetype: bool, entries: &[(&str, &str)]) -> asic_e::Result<Container> {
    let entries: String = entries
        .iter()
        .map(|(path, media_type)| {
            format!(r#"<manifest:file-entry manifest:full-path="{path}" manifest:media-type="{media_type}"/>"#)
        })
        .collect();
    let manifest = format!(
        r#"<manifest:manifest xmlns:manifest="{MANIFEST_NS}" manifest:version="1.2">{entries}</manifest:manifest>"#
    );

    let mut files = vec![
        ("test.txt", b"lorem ipsum".as_slice()),
        ("META-INF/manifest.xml", manifest.as_bytes()),
    ];
    if mimetype {
        files.insert(0, ("mimetype", asic_e::MIMETYPE.as_bytes()));
    }
    Container::from_bytes(&uncompressed_zip(&files))
}

#[test]
fn checks_manifest_against_odf_rules() {
    let warnings = |mimetype: bool, entries: &[(&str, &str)]| {
        container_with_manifest(mimetype, entries)
            .unwrap()
            .warnings()
            .to_vec()
    };
    let root = ("/", asic_e::MIMETYPE);
    let txt = ("test.txt", "text/plain");

    assert!(warnings(true, &[root, txt]).is_empty());
    assert_eq!(
        warnings(
            true,
            &[
                root,
                txt,
                ("mimetype", "text/plain"),
                ("META-INF/manifest.xml", "text/xml")
            ]
        ),
        [
            "manifest must not list 'mimetype'",
            "manifest must not list 'META-INF/manifest.xml'"
        ]
    );
    assert_eq!(
        warnings(true, &[root, txt, ("missing.txt", "text/plain")]),
        ["manifest lists 'missing.txt', which is not in the container"]
    );
    assert_eq!(
        warnings(true, &[txt]),
        ["manifest has no '/' entry, although the container has a mimetype"]
    );
    assert_eq!(
        warnings(false, &[root, txt]),
        [
            "container has no mimetype entry",
            "manifest has a '/' entry, although the container has no mimetype"
        ]
    );
    assert_eq!(
        warnings(true, &[("/", "application/zip"), txt]),
        ["manifest gives '/' the media type 'application/zip', which differs from the mimetype"]
    );

    let err = container_with_manifest(true, &[root, txt, txt]).unwrap_err();
    assert!(
        err.to_string()
            .contains("manifest lists 'test.txt' more than once"),
        "{err}"
    );
}

fn minimal_container() -> Vec<u8> {
    uncompressed_zip(&[
        ("mimetype", asic_e::MIMETYPE.as_bytes()),
        ("test.txt", b"lorem ipsum".as_slice()),
        ("META-INF/manifest.xml", EXTRAS_MANIFEST.as_bytes()),
    ])
}

#[test]
fn enforces_size_limit() {
    let bytes = minimal_container();
    let total = (asic_e::MIMETYPE.len() + "lorem ipsum".len() + EXTRAS_MANIFEST.len()) as u64;
    let opts = |max_total_size| asic_e::OpenOptions {
        max_total_size,
        ..Default::default()
    };
    assert!(Container::from_bytes_with(&bytes, &opts(total)).is_ok());

    let err = Container::from_bytes_with(&bytes, &opts(total - 1)).unwrap_err();
    let expected = format!("more than the {} byte limit", total - 1);
    assert!(err.to_string().contains(&expected), "{err}");
}
