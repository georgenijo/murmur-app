use super::{archive, color, compile, fixtures, import, library, open_vsx};
use crate::section::Problem;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{cell::Cell, collections::BTreeMap};
fn zip(files: &[(&str, &str)]) -> Vec<u8> {
    zip_method(files, false)
}
fn zip_method(files: &[(&str, &str)], compress: bool) -> Vec<u8> {
    let mut bytes = vec![];
    let mut directory = vec![];
    for (name, text) in files {
        let packed = if compress {
            use std::io::Write;
            let mut enc =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(text.as_bytes()).unwrap();
            enc.finish().unwrap()
        } else {
            text.as_bytes().to_vec()
        };
        let method = if compress { 8u16 } else { 0u16 };
        let offset = bytes.len() as u32;
        let crc = archive::crc(text.as_bytes());
        let size = text.len() as u32;
        bytes.extend(0x04034b50u32.to_le_bytes());
        bytes.extend(20u16.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(method.to_le_bytes());
        bytes.extend([0u8; 4]);
        bytes.extend(crc.to_le_bytes());
        bytes.extend((packed.len() as u32).to_le_bytes());
        bytes.extend(size.to_le_bytes());
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(&packed);
        directory.extend(0x02014b50u32.to_le_bytes());
        directory.extend(20u16.to_le_bytes());
        directory.extend(20u16.to_le_bytes());
        directory.extend(0u16.to_le_bytes());
        directory.extend(method.to_le_bytes());
        directory.extend([0u8; 4]);
        directory.extend(crc.to_le_bytes());
        directory.extend((packed.len() as u32).to_le_bytes());
        directory.extend(size.to_le_bytes());
        directory.extend((name.len() as u16).to_le_bytes());
        directory.extend([0u8; 12]);
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    let offset = bytes.len() as u32;
    let size = directory.len() as u32;
    bytes.extend(directory);
    bytes.extend(0x06054b50u32.to_le_bytes());
    bytes.extend([0u8; 4]);
    bytes.extend((files.len() as u16).to_le_bytes());
    bytes.extend((files.len() as u16).to_le_bytes());
    bytes.extend(size.to_le_bytes());
    bytes.extend(offset.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes
}
struct Mock {
    responses: BTreeMap<String, Vec<u8>>,
    calls: Cell<usize>,
}
impl open_vsx::Fetch for Mock {
    fn get(&self, url: &str, max: usize, current: &dyn Fn() -> bool) -> Result<Vec<u8>, Problem> {
        if !current() {
            return Err(Problem::Conflict);
        }
        self.calls.set(self.calls.get() + 1);
        open_vsx::trusted(url)?;
        let data = self.responses.get(url).ok_or(Problem::Unavailable)?.clone();
        if data.len() > max {
            return Err(Problem::Invalid);
        }
        Ok(data)
    }
}
fn extension() -> open_vsx::Extension {
    open_vsx::Extension {
        id: "fixture.colors".into(),
        publisher: "fixture".into(),
        name: "colors".into(),
        label: "Synthetic colors".into(),
        version: "1.0.0".into(),
        license: "MIT".into(),
        manifest: "https://open-vsx.org/manifest".into(),
        checksum: "https://open-vsx.org/checksum".into(),
        archive: "https://open-vsx.org/archive".into(),
        source: None,
    }
}
fn mock(files: &[(&str, &str)]) -> Mock {
    let bytes = zip(files);
    let checksum: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Mock {
        responses: BTreeMap::from([
            ("https://open-vsx.org/archive".into(), bytes),
            (
                "https://open-vsx.org/checksum".into(),
                checksum.into_bytes(),
            ),
        ]),
        calls: Cell::new(0),
    }
}
const MANIFEST: &str = r#"{"publisher":"fixture","name":"colors","version":"1.0.0","license":"MIT","contributes":{"themes":[{"label":"Fixture dark","uiTheme":"vs-dark","path":"themes/dark.json"},{"label":"Fixture light","uiTheme":"vs","path":"themes/light.json"}]},"main":"DO_NOT_EXECUTE.js"}"#;
#[test]
fn archive_validation_and_crc() {
    let bytes = zip(&[("extension/theme.json", "{}")]);
    assert_eq!(
        archive::Archive::new(&bytes)
            .unwrap()
            .read("extension/theme.json", 2)
            .unwrap(),
        b"{}"
    );
    assert!(
        archive::Archive::new(&bytes)
            .unwrap()
            .read("extension/theme.json", 1)
            .is_err()
    );
    for name in [
        "../escape",
        "/absolute",
        "C:/absolute",
        "extension/../../escape",
        "extension\\escape",
    ] {
        assert!(archive::Archive::new(&zip(&[(name, "{}")])).is_err());
    }
    assert!(archive::Archive::new(&zip(&[("extension/a", "{}"), ("extension/a", "{}")])).is_err());
    let mut corrupted = bytes.clone();
    corrupted[30 + "extension/theme.json".len()] = 0;
    assert!(
        archive::Archive::new(&corrupted)
            .unwrap()
            .read("extension/theme.json", 10)
            .is_err()
    );
    let mut huge = bytes.clone();
    let start = huge.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    huge[start + 24..start + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(archive::Archive::new(&huge).is_err());
}
#[test]
fn open_vsx_declared_only_inheritance_pairing_and_stable_ids() {
    let m = mock(&[
        ("extension/package.json", MANIFEST),
        (
            "extension/themes/dark.json",
            r##"{//comment
"include":"base.json","colors":{"editor.foreground":"#eeeeee"},}"##,
        ),
        (
            "extension/themes/base.json",
            r##"{"colors":{"editor.background":"#151515","focusBorder":"#aa77ee"}}"##,
        ),
        (
            "extension/themes/light.json",
            r##"{"colors":{"editor.background":"#ffffff","editor.foreground":"#111111"}}"##,
        ),
        (
            "extension/DO_NOT_EXECUTE.js",
            "throw new Error('must never execute')",
        ),
    ]);
    let entries = open_vsx::download(&m, &extension(), &|| true).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["modes"], json!(["light", "dark"]));
    assert_eq!(entries[0]["source"]["extensionId"], "fixture.colors");
    assert_eq!(
        entries[0]["id"],
        open_vsx::download(&m, &extension(), &|| true).unwrap()[0]["id"]
    );
    assert_eq!(m.calls.get(), 4);
}
#[test]
fn open_vsx_refuses_bad_hash_identity_cycles_paths_and_stale_work() {
    let files = [
        ("extension/package.json", MANIFEST),
        (
            "extension/themes/dark.json",
            r##"{"include":"dark.json","colors":{"editor.background":"#151515"}}"##,
        ),
        ("extension/themes/light.json", "{}"),
    ];
    let mut m = mock(&files);
    assert!(open_vsx::download(&m, &extension(), &|| true).is_err());
    m.responses
        .insert("https://open-vsx.org/checksum".into(), vec![b'0'; 64]);
    assert!(open_vsx::download(&m, &extension(), &|| true).is_err());
    assert_eq!(
        open_vsx::download(&m, &extension(), &|| false).err(),
        Some(Problem::Conflict)
    );
    for url in [
        "http://open-vsx.org/a",
        "https://evil.invalid/a",
        "https://user@open-vsx.org/a",
        "https://open-vsx.org:8080/a",
    ] {
        assert!(open_vsx::trusted(url).is_err());
    }
    let mut e = extension();
    e.license = "Proprietary".into();
    assert!(open_vsx::download(&m, &e, &|| true).is_err());
}
#[test]
fn community_search_validates_manifest_before_listing() {
    let e = extension();
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    let detail = json!({"namespace":e.publisher,"name":e.name,"displayName":e.label,"version":e.version,"license":"MIT","files":{"manifest":e.manifest,"sha256":e.checksum,"download":e.archive}});
    let m=Mock{responses:BTreeMap::from([("https://open-vsx.org/api/-/search?query=fixture&category=Themes&size=16&sortBy=downloadCount".into(),serde_json::to_vec(&json!({"extensions":[{"namespace":"fixture","name":"colors"}]})).unwrap()),("https://open-vsx.org/api/fixture/colors".into(),serde_json::to_vec(&detail).unwrap()),(e.manifest.clone(),serde_json::to_vec(&manifest).unwrap())]),calls:Cell::new(0)};
    assert_eq!(
        open_vsx::search(&m, "fixture", "downloadCount", &|| true)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(m.calls.get(), 3);
    assert!(open_vsx::search(&m, "fixture", "bogus", &|| true).is_err());
}
#[test]
fn library_selection_keeps_other_half_and_removal_keeps_config() {
    let s = fixtures::snapshot();
    let doc = library::selection(&s.appearance, &s.library, "midnight", None).unwrap();
    assert_eq!(doc["mode"], "dark");
    assert_eq!(doc["selection"]["light"], "sonic");
    assert_eq!(doc["selection"]["dark"], "midnight");
    let removed = library::remove(&s.library, &["midnight".into()]).unwrap();
    assert_eq!(doc["theme"], doc["theme"].clone());
    assert!(library::selection(&doc, &removed, "midnight", None).is_err());
    assert!(library::remove(&s.library, &["sonic".into()]).is_err());
    assert_eq!(library::next_revision(2147483646), 1);
}
#[test]
fn update_refuses_concurrent_collection_changes() {
    let mut s = fixtures::snapshot();
    let mut e = fixtures::imported();
    e["id"] = json!("ovx-fixture");
    e["source"] =
        json!({"kind":"open-vsx","extensionId":"fixture.colors","version":"1","license":"MIT"});
    e["collection"] = json!({"id":"open-vsx:fixture.colors","label":"Colors"});
    s.library = library::install(&s.library, &[e.clone()], None).unwrap();
    let expected = vec![e.clone()];
    e["label"] = json!("Edited locally");
    let changed = library::install(
        &s.library,
        &[e.clone()],
        Some(("open-vsx:fixture.colors", &expected)),
    )
    .unwrap();
    assert_eq!(
        library::install(
            &changed,
            &expected,
            Some(("open-vsx:fixture.colors", &expected))
        )
        .err(),
        Some(Problem::Conflict)
    );
}
#[test]
fn color_math_and_mutable_contrast() {
    for dark in [false, true] {
        let sonic = compile::resolve(&json!({"version":1,"presetId":"sonic"}), dark);
        assert_eq!(
            compile::value(&sonic)["background"],
            if dark { "#0b0f11" } else { "#f7fafc" }
        );
        for theme in [
            json!({"version":1,"presetId":"custom","background":"#777777","foreground":"#777777","accent":"#777777"}),
            fixtures::midnight()["theme"].clone(),
            fixtures::imported()["theme"].clone(),
        ] {
            let t = compile::resolve(&theme, dark);
            assert!(!compile::failures(&t));
        }
    }
    assert_eq!(
        color::hex(import::color("#ff000080", [0.; 3]).unwrap()),
        "#800000"
    );
    assert!(import::color("url(javascript:bad)", [0.; 3]).is_none());
}
#[test]
fn jsonc_never_reinterprets_strings_as_comments() {
    let v = crate::json::jsonc(
        "{\"url\":\"https://example.invalid/a/*b*/\",/*x*/\"v\":1,}",
        1024,
    )
    .unwrap();
    assert_eq!(v["url"], "https://example.invalid/a/*b*/");
    assert!(crate::json::jsonc("{\"x\":1,\"x\":2}", 1024).is_err());
    assert!(crate::json::jsonc("{/* unterminated", 1024).is_err());
}
#[test]
fn rust_resolution_matches_retained_react_fixtures() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../fixtures/theme-resolution.json")).unwrap();
    assert!(
        cases.len() >= 14,
        "refresh neutral appearance parity fixtures"
    );
    for c in cases {
        assert_eq!(
            compile::value(&compile::resolve(&c["theme"], c["dark"].as_bool().unwrap())),
            c["tokens"],
            "{}",
            c["name"]
        );
    }
}

#[test]
fn deflate_stream_limits_and_archive_metadata_fail_closed() {
    let text = "{\"synthetic\":\"value\"}";
    let data = zip_method(&[("extension/theme.json", text)], true);
    assert_eq!(
        archive::Archive::new(&data)
            .unwrap()
            .read("extension/theme.json", 256)
            .unwrap(),
        text.as_bytes()
    );
    assert!(
        archive::Archive::new(&data)
            .unwrap()
            .read("extension/theme.json", 2)
            .is_err()
    );
    let mut symlink = data.clone();
    let central = symlink.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    symlink[central + 38..central + 42].copy_from_slice(&(0xa000u32 << 16).to_le_bytes());
    assert!(archive::Archive::new(&symlink).is_err());
    let mut expanded = data.clone();
    expanded[central + 24..central + 28].copy_from_slice(&(51u32 * 1024 * 1024).to_le_bytes());
    assert!(archive::Archive::new(&expanded).is_err());
    let names: Vec<_> = (0..2001)
        .map(|i| format!("extension/file-{i}.json"))
        .collect();
    let files: Vec<_> = names.iter().map(|s| (s.as_str(), "{}")).collect();
    assert!(archive::Archive::new(&zip(&files)).is_err());
}
#[test]
fn package_identity_malformed_json_and_include_traversal_are_rejected() {
    for manifest in [
        MANIFEST.replace("\"name\":\"colors\"", "\"name\":\"different\""),
        "{bad json".into(),
    ] {
        let m = mock(&[("extension/package.json", &manifest)]);
        assert!(open_vsx::download(&m, &extension(), &|| true).is_err());
    }
    let m = mock(&[
        ("extension/package.json", MANIFEST),
        (
            "extension/themes/dark.json",
            r##"{"include":"../../outside.json","colors":{"editor.background":"#111111"}}"##,
        ),
        ("extension/themes/light.json", "{}"),
    ]);
    assert!(open_vsx::download(&m, &extension(), &|| true).is_err());
}
