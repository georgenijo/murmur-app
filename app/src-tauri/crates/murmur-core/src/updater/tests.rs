use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
const ID: &str = "com.localdictation.shutdown-test";
fn signed(bytes: &[u8]) -> (String, String) {
    let rng = ring::rand::SystemRandom::new();
    let pair =
        Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&rng).unwrap().as_ref()).unwrap();
    let id = [42; 8];
    let sig = pair.sign(bytes);
    let key = [b"Ed".as_slice(), &id, pair.public_key().as_ref()].concat();
    let packet = [b"Ed".as_slice(), &id, sig.as_ref()].concat();
    let global = pair.sign(&[sig.as_ref(), b"fixture"].concat());
    (
        STANDARD.encode(format!(
            "untrusted comment: test key\n{}\n",
            STANDARD.encode(key)
        )),
        STANDARD.encode(format!(
            "untrusted comment: test signature\n{}\ntrusted comment: fixture\n{}\n",
            STANDARD.encode(packet),
            STANDARD.encode(global)
        )),
    )
}
fn manifest() -> Value {
    json!({"version":"1.2.0","notes":"Latest","platforms":{"darwin-aarch64":{"url":"https://example.test/Murmur.app.tar.gz","signature":"encoded"}}})
}
#[test]
fn minisign_accepts_legacy_wire_encoding_and_rejects_tampering_wrong_key_bad_text() {
    let (key, sig) = signed(b"synthetic bytes");
    assert!(verify(b"synthetic bytes", &sig, &key).is_ok());
    assert!(verify(b"Synthetic bytes", &sig, &key).is_err());
    let (other, _) = signed(b"synthetic bytes");
    assert!(verify(b"synthetic bytes", &sig, &other).is_err());
    for bad in ["", "not base64", "eA=="] {
        assert!(verify(b"synthetic bytes", bad, &key).is_err());
    }
}
#[test]
fn installed_client_prehashed_minisign_vector_is_compatible() {
    // Upstream minisign-verify's published prehashed test vector (MIT).
    let key = STANDARD.encode("untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3");
    let sig = STANDARD.encode("untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1556193335\tfile:test\ny/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==");
    verify(b"test", &sig, &key).unwrap();
    assert!(verify(b"Test", &sig, &key).is_err());
}
#[test]
fn same_feed_platform_lookup_and_edge_cases() {
    let parse = |v: &Value, p| parse_feed(&serde_json::to_vec(v).unwrap(), "1.0.0", p, 42);
    assert_eq!(
        parse(&manifest(), "darwin-aarch64")
            .unwrap()
            .unwrap()
            .version,
        "1.2.0"
    );
    assert!(parse(&manifest(), "darwin-x86_64").is_err());
    assert!(parse(&manifest(), "macos-aarch64").is_err());
    for v in [
        Value::Null,
        json!([]),
        json!({"version":"broken"}),
        json!({"version":"2.0.0","platforms":null}),
    ] {
        assert!(parse(&v, "darwin-aarch64").is_err());
    }
    let mut v = manifest();
    v["platforms"]["darwin-aarch64-app"] =
        json!({"url":"https://example.test/preferred","signature":"sig"});
    assert!(parse(&v, "darwin-aarch64")
        .unwrap()
        .unwrap()
        .url
        .ends_with("preferred"));
    assert!(parse_feed(b"{truncated", "1.0.0", "darwin-aarch64", 0).is_err());
    assert!(parse_feed(
        &serde_json::to_vec(&manifest()).unwrap(),
        "1.2.0",
        "wrong-platform",
        0
    )
    .unwrap()
    .is_none());
}
#[test]
fn minimum_policy_preserves_optional_malformed_metadata_and_forces_unparseable_strings() {
    for v in [Value::Null, json!(12), json!({})] {
        assert!(!forced("1.0.0", &v));
    }
    assert!(forced("1.0.0", &json!("1.0.1")));
    assert!(forced("broken", &json!("1.0.1")));
    assert!(forced("1.0.0", &json!("broken")));
    assert!(!forced(" v1.2.0-beta.1 ", &json!("1.2.0")));
    assert!(!forced("1.3.0", &json!("1.2.0")));
    let mut v = manifest();
    v["min_version"] = json!(12);
    assert!(
        !parse_feed(&v.to_string().into_bytes(), "1.0.0", "darwin-aarch64", 0)
            .unwrap()
            .unwrap()
            .policy_verified
    );
}
#[test]
fn notes_include_complete_stable_upgrade_range_and_pending_requires_exact_identity() {
    let mut v = manifest();
    v["release_history"] = json!([{"version":"1.1.0","notes":"Intermediate"},{"version":"1.2.0","notes":"stale"},{"version":"1.0.0","notes":"old"},{"version":"1.3.0","notes":"future"},{"version":"1.1.1-beta","notes":"beta"}]);
    let offer = parse_feed(&v.to_string().into_bytes(), "1.0.0", "darwin-aarch64", 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        offer.notes,
        "# v1.2.0\n\nLatest\n\n---\n\n# v1.1.0\n\nIntermediate"
    );
    let pending = json!({"version":"v1.2.0+one","notes":"Hello"});
    assert!(pending_for(&pending, "1.2.0+one").is_some());
    assert!(pending_for(&pending, "1.2.0+two").is_none());
    assert!(normalize_pending(json!({"version":"bad","notes":"hi"})).is_err());
}
#[test]
fn translocation_is_a_path_component_and_bundle_paths_fail_closed() {
    assert!(translocated(Path::new(
        "/private/var/folders/fixture/AppTranslocation/uuid/d/Murmur.app/Contents/MacOS/ui"
    )));
    assert!(!translocated(Path::new(
        "/Applications/MyAppTranslocation.app/Contents/MacOS/ui"
    )));
    assert!(install::bundle(Path::new("/synthetic/ui")).is_err());
}
fn make_bundle(path: &Path, version: &str) {
    fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
    fs::write(
        path.join("Contents/MacOS/ui"),
        format!("synthetic {version}"),
    )
    .unwrap();
    let plist = plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("CFBundleIdentifier", plist::Value::String(ID.into())),
        ("CFBundleExecutable", plist::Value::String("ui".into())),
        (
            "CFBundleShortVersionString",
            plist::Value::String(version.into()),
        ),
    ]));
    plist.to_file_xml(path.join("Contents/Info.plist")).unwrap();
}
fn archive(bundle: &Path) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    tar.append_dir_all("Murmur.app", bundle).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}
fn offer(signature: String) -> Offer {
    Offer {
        version: "1.2.0".into(),
        notes: "Synthetic notes".into(),
        forced: false,
        policy_verified: true,
        url: "https://example.test/update".into(),
        signature,
        discovered: now(),
    }
}
#[test]
fn install_keeps_launch_path_and_recovers_post_exchange_without_double_install() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let target = temp.path().join("Murmur.app");
    make_bundle(&target, "1.0.0");
    let new = temp.path().join("New.app");
    make_bundle(&new, "1.2.0");
    let bytes = archive(&new);
    let (key, signature) = signed(&bytes);
    let offer = offer(signature);
    install::install(&root, &target, &offer, &bytes, &key, ID, || true).unwrap();
    assert!(target.join("Contents/MacOS/ui").exists());
    assert!(install::recover(&root, &target, "1.0.0", ID).is_err());
    assert!(install::install(&root, &target, &offer, &bytes, &key, ID, || true).is_err());
    install::recover(&root, &target, "1.2.0", ID).unwrap();
    install::recover(&root, &target, "1.2.0", ID).unwrap();
    assert!(!root.join("updater-install.json").exists());
    assert!(!fs::read_dir(temp.path()).unwrap().any(|p| p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".murmur-update-")));
}
#[test]
fn interrupted_pre_exchange_and_uncertain_exchange_recover_from_durable_receipt() {
    for swapped in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let target = temp.path().join("Murmur.app");
        make_bundle(&target, "1.0.0");
        let id = uuid::Uuid::new_v4();
        let stage = temp.path().join(format!(".murmur-update-{id}"));
        make_bundle(&stage.join("payload.app"), "1.2.0");
        install::atomic_json(
            &root.join("updater-install.json"),
            &json!({"schema":1,"id":id,"bundle":target,"version":"1.2.0","identifier":ID}),
        )
        .unwrap();
        if swapped {
            install::simulate_exchange(&target, &stage.join("payload.app")).unwrap();
        }
        install::recover(&root, &target, if swapped { "1.2.0" } else { "1.0.0" }, ID).unwrap();
        assert!(target.join("Contents/MacOS/ui").exists());
        assert!(!stage.exists());
    }
}
#[test]
fn failed_verification_bad_archive_and_wrong_bundle_never_change_installed_app() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let target = temp.path().join("Murmur.app");
    make_bundle(&target, "1.0.0");
    for bytes in [b"not an archive".to_vec(), {
        let b = temp.path().join("Wrong.app");
        make_bundle(&b, "9.9.9");
        archive(&b)
    }] {
        let (key, signature) = signed(&bytes);
        let offer = offer(signature);
        assert!(install::install(&root, &target, &offer, &bytes, &key, ID, || true).is_err());
        assert_eq!(
            fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
            "synthetic 1.0.0"
        );
    }
}
#[test]
fn corrupt_future_or_foreign_install_receipts_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("Murmur.app");
    make_bundle(&target, "1.0.0");
    let root = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    for raw in ["{truncated".to_owned(), json!({"schema":2,"id":uuid::Uuid::new_v4(),"bundle":target,"version":"1.2.0","identifier":ID}).to_string()] {
        fs::write(root.join("updater-install.json"), &raw).unwrap();
        assert!(install::recover(&root,&target,"1.0.0",ID).is_err());
        assert_eq!(fs::read_to_string(root.join("updater-install.json")).unwrap(), raw);
    }
}
#[test]
fn interrupted_download_cannot_become_ready() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort")
            .unwrap();
    });
    let result = install::download(
        &reqwest::blocking::Client::new(),
        &format!("http://{address}/update"),
        |_| {},
    );
    assert!(result.is_err());
    thread.join().unwrap();
}
#[test]
fn rust_owner_retains_ready_bytes_and_serializes_install_retry_and_relaunch() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let target = temp.path().join("Murmur.app");
    make_bundle(&target, "1.0.0");
    let new = temp.path().join("New.app");
    make_bundle(&new, "1.2.0");
    let bytes = archive(&new);
    let (key, sig) = signed(&bytes);
    let store = CompletionService::start(root.clone(), false).unwrap();
    let mut updater = Updater::production(
        root,
        target.join("Contents/MacOS/ui"),
        ID.into(),
        "1.0.0".into(),
        store,
        |_| {},
        |_| {},
    );
    updater.key = key;
    {
        let mut e = updater.engine.lock().unwrap();
        e.offer = Some(offer(sig));
        e.bytes = Some(bytes);
    }
    assert!(updater.act(Action::Install).unwrap());
    assert!(!updater.act(Action::Install).unwrap());
    updater.act(Action::Check { background: false }).unwrap();
    assert_eq!(
        updater.snapshot().update_status,
        Status::Restarting {
            version: "1.2.0".into()
        }
    );
}
#[test]
fn rust_owner_requires_v3_before_network_and_generic_sources_cannot_seed_invalid_updater_values() {
    let temp = tempfile::tempdir().unwrap();
    let store = CompletionService::start(temp.path().into(), true).unwrap();
    let updater = Updater::production(
        temp.path().into(),
        PathBuf::from("/synthetic/ui"),
        ID.into(),
        "1.0.0".into(),
        store,
        |_| {},
        |_| {},
    );
    assert!(updater
        .act(Action::Check { background: false })
        .unwrap_err()
        .contains("migration"));
    assert!(crate::state::normalize("updater-last-check", json!(-1)).is_err());
}
