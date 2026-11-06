use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
const ID: &str = "com.localdictation.shutdown-test";
// macOS CI's temporary directory is commonly reached through /var ->
// /private/var. Model a canonical installed bundle without weakening the
// production refusal of symlinked installation locations.
struct Scratch {
    _directory: tempfile::TempDir,
    canonical: PathBuf,
}
impl Scratch {
    fn path(&self) -> &Path {
        &self.canonical
    }
}
fn scratch() -> Scratch {
    let directory = tempfile::tempdir().unwrap();
    let canonical = fs::canonicalize(directory.path()).unwrap();
    Scratch {
        _directory: directory,
        canonical,
    }
}
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
    let temp = scratch();
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
        let temp = scratch();
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
    let temp = scratch();
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
    let temp = scratch();
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
    let temp = scratch();
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
    let temp = scratch();
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

struct Server {
    address: std::net::SocketAddr,
    routes: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
    count: Arc<std::sync::atomic::AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let routes = Arc::new(Mutex::new(
            std::collections::HashMap::<String, Vec<u8>>::new(),
        ));
        let copy = routes.clone();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = count.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = [0; 8192];
                    let n = socket.read(&mut bytes).unwrap_or(0);
                    let request = String::from_utf8_lossy(&bytes[..n]);
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    calls.fetch_add(1, Ordering::AcqRel);
                    let body = copy.lock().unwrap().get(path).cloned();
                    let (code, body) = body
                        .map(|b| ("200 OK", b))
                        .unwrap_or_else(|| ("503 Unavailable", vec![]));
                    let header = format!(
                        "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes());
                    let _ = socket.write_all(&body);
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        Self {
            address,
            routes,
            count,
            stop,
            thread: Some(thread),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    fn set(&self, path: &str, bytes: Vec<u8>) {
        self.routes.lock().unwrap().insert(path.into(), bytes);
    }
    fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn network_owner(
    temp: &Path,
    server: &Server,
    forced: bool,
) -> (Updater, CompletionService, PathBuf) {
    let root = temp.join("data");
    let target = temp.join("Murmur.app");
    make_bundle(&target, "1.0.0");
    let new = temp.join("New.app");
    make_bundle(&new, "1.2.0");
    let bytes = archive(&new);
    let (key, sig) = signed(&bytes);
    let mut manifest = json!({"version":"1.2.0","notes":"Synthetic upgrade","platforms":{platform():{"url":server.url("/archive"),"signature":sig}}});
    if forced {
        manifest["min_version"] = json!("1.1.0");
    }
    server.set("/feed", manifest.to_string().into_bytes());
    server.set("/archive", bytes);
    let store = CompletionService::start(root.clone(), false).unwrap();
    let mut updater = Updater::production(
        root,
        target.join("Contents/MacOS/ui"),
        ID.into(),
        "1.0.0".into(),
        store.clone(),
        |_| {},
        |_| {},
    );
    updater.feed = server.url("/feed");
    updater.key = key;
    updater.local_fixture = true;
    (updater, store, target)
}
#[test]
fn real_http_owner_checks_downloads_retries_installs_and_confirms_notes_on_new_launch() {
    let temp = scratch();
    let server = Server::new();
    let (updater, store, target) = network_owner(temp.path(), &server, false);
    updater.act(Action::Configure { automatic: false }).unwrap();
    assert_eq!(server.count(), 0);
    updater.act(Action::Check { background: false }).unwrap();
    assert!(matches!(
        updater.snapshot().update_status,
        Status::Available {
            is_forced: false,
            ..
        }
    ));
    updater.act(Action::Dismiss).unwrap();
    assert!(!updater.snapshot().is_update_dialog_open);
    updater.act(Action::Show).unwrap();
    assert!(updater.snapshot().is_update_dialog_open);
    // Failed signature leaves the old app intact, then a corrected download retries.
    let original = server
        .routes
        .lock()
        .unwrap()
        .get("/archive")
        .unwrap()
        .clone();
    server.set("/archive", b"tampered".to_vec());
    assert!(updater.act(Action::Download).is_err());
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.0.0"
    );
    server.set("/archive", original);
    updater.act(Action::Download).unwrap();
    let calls = server.count();
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Download).unwrap();
    assert_eq!(server.count(), calls);
    assert!(matches!(
        updater.snapshot().update_status,
        Status::Ready { .. }
    ));
    updater.act(Action::Dismiss).unwrap();
    assert!(!updater.snapshot().is_update_dialog_open);
    // Fail an actual exchange and retry from retained verified bytes, without HTTP.
    let blocked = target.parent().unwrap().join("placeholder");
    fs::rename(&target, &blocked).unwrap();
    assert!(updater.act(Action::Install).is_err());
    fs::rename(&blocked, &target).unwrap();
    assert!(updater.act(Action::Install).unwrap());
    assert_eq!(server.count(), calls);
    assert!(!updater.act(Action::Install).unwrap());
    let next = Updater::production(
        temp.path().join("data"),
        target.join("Contents/MacOS/ui"),
        ID.into(),
        "1.2.0".into(),
        store.clone(),
        |_| {},
        |_| {},
    );
    next.act(Action::Configure { automatic: false }).unwrap();
    assert_eq!(
        next.snapshot().completed_update.unwrap().notes,
        "Synthetic upgrade"
    );
    assert!(!temp.path().join("data/updater-install.json").exists());
    next.act(Action::DismissCompleted).unwrap();
    next.act(Action::Configure { automatic: false }).unwrap();
    assert!(next.snapshot().completed_update.is_none());
}
#[test]
fn rust_schedule_gates_setup_six_hours_ready_and_forced_dismissal() {
    let temp = scratch();
    let server = Server::new();
    let (updater, store, _) = network_owner(temp.path(), &server, true);
    updater.act(Action::Configure { automatic: false }).unwrap();
    updater.act(Action::Check { background: true }).unwrap();
    assert_eq!(server.count(), 0);
    updater.act(Action::Configure { automatic: true }).unwrap();
    assert_eq!(server.count(), 1);
    updater.act(Action::Check { background: true }).unwrap();
    assert_eq!(server.count(), 1);
    updater.act(Action::Dismiss).unwrap();
    assert!(updater.snapshot().is_update_dialog_open);
    store
        .lifecycle(Operation::Save {
            key: "updater-last-check".into(),
            value: json!(now() - INTERVAL - 1),
        })
        .unwrap();
    updater.act(Action::Check { background: true }).unwrap();
    assert_eq!(server.count(), 2);
    updater.act(Action::Download).unwrap();
    updater.act(Action::Dismiss).unwrap();
    assert!(updater.snapshot().is_update_dialog_open);
    let count = server.count();
    updater.act(Action::Check { background: false }).unwrap();
    assert_eq!(server.count(), count);
}
#[test]
fn freshness_failure_keeps_prior_signed_offer_and_shutdown_fences_continuations() {
    let temp = scratch();
    let server = Server::new();
    let (updater, _, target) = network_owner(temp.path(), &server, false);
    updater.act(Action::Check { background: false }).unwrap();
    updater
        .engine
        .lock()
        .unwrap()
        .offer
        .as_mut()
        .unwrap()
        .discovered = 0;
    server.routes.lock().unwrap().remove("/feed");
    updater.act(Action::Download).unwrap();
    assert!(matches!(
        updater.snapshot().update_status,
        Status::Ready { .. }
    ));
    updater.stop();
    assert!(updater.drained());
    assert!(updater.act(Action::Install).is_err());
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.0.0"
    );
}
#[test]
fn uncertain_install_adopts_new_bundle_and_pre_exchange_failure_retries_without_swapback() {
    let temp = scratch();
    let server = Server::new();
    let (updater, _, target) = network_owner(temp.path(), &server, false);
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Download).unwrap();
    let offer = updater.engine.lock().unwrap().offer.clone().unwrap();
    let bytes = updater.engine.lock().unwrap().bytes.clone().unwrap();
    // Complete the real exchange but lose the process-local acknowledgement.
    install::install(
        &updater.root,
        &target,
        &offer,
        &bytes,
        &updater.key,
        ID,
        || true,
    )
    .unwrap();
    assert!(updater.act(Action::Install).unwrap());
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.2.0"
    );
}
#[test]
fn canary_dry_run_preserves_pending_stages_and_actual_flow_marks_only_new_version_passed() {
    for dry in [true, false] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, store, target) = network_owner(temp.path(), &server, false);
        let path = temp.path().join("canary/result.json");
        updater.canary = Some(path.clone());
        updater.dry_run = dry;
        assert_eq!(
            updater.act(Action::Configure { automatic: true }).unwrap(),
            !dry
        );
        let result = updater.read_canary().unwrap();
        assert_eq!(result["checkedVersion"], "1.0.0");
        assert_eq!(result["offeredVersion"], "1.2.0");
        if dry {
            assert_eq!(result["status"], "dry-run");
            assert_eq!(result["stages"]["download"], "pending");
            assert_eq!(server.count(), 1);
        } else {
            assert_eq!(result["status"], "pending");
            assert_eq!(result["stages"]["install"], "passed");
            assert_eq!(result["stages"]["relaunch"], "pending");
            let mut next = Updater::production(
                temp.path().join("data"),
                target.join("Contents/MacOS/ui"),
                ID.into(),
                "1.2.0".into(),
                store,
                |_| {},
                |_| {},
            );
            next.canary = Some(path);
            assert!(!next.act(Action::Configure { automatic: true }).unwrap());
            let result = next.read_canary().unwrap();
            assert_eq!(result["status"], "passed");
            for stage in [
                "discover",
                "policy",
                "download",
                "signatureVerify",
                "install",
                "relaunch",
            ] {
                assert_eq!(result["stages"][stage], "passed");
            }
        }
    }
}

#[test]
fn canary_reports_signature_failure_after_download_and_never_installs() {
    let temp = scratch();
    let server = Server::new();
    let (mut updater, _, target) = network_owner(temp.path(), &server, false);
    updater.canary = Some(temp.path().join("result.json"));
    server.set("/archive", b"tampered synthetic bytes".to_vec());
    assert!(updater.act(Action::Configure { automatic: true }).is_err());
    let result = updater.read_canary().unwrap();
    assert_eq!(result["status"], "failed");
    assert_eq!(result["stages"]["download"], "passed");
    assert_eq!(result["stages"]["signatureVerify"], "failed");
    assert_eq!(result["stages"]["install"], "pending");
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.0.0"
    );
}
#[test]
fn signed_archive_escaping_symlinks_and_foreign_identity_cannot_modify_outside_stage() {
    let temp = scratch();
    let target = temp.path().join("Murmur.app");
    make_bundle(&target, "1.0.0");
    let root = temp.path().join("data");
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_link_name("../../outside").unwrap();
    header.set_cksum();
    tar.append_data(&mut header, "Murmur.app/escape", std::io::empty())
        .unwrap();
    let bytes = tar.into_inner().unwrap().finish().unwrap();
    let (key, sig) = signed(&bytes);
    assert!(install::install(&root, &target, &offer(sig), &bytes, &key, ID, || true).is_err());
    assert!(!temp.path().join("outside").exists());
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.0.0"
    );
}
#[cfg(feature = "updater-test-support")]
#[test]
fn compiled_qa_seam_rejects_production_identity_and_nonloopback_endpoints() {
    let temp = scratch();
    let store = CompletionService::start(temp.path().into(), false).unwrap();
    let owner = |id: &str| {
        Updater::production(
            temp.path().into(),
            PathBuf::from("/synthetic/ui"),
            id.into(),
            "1.0.0".into(),
            store.clone(),
            |_| {},
            |_| {},
        )
    };
    assert!(owner("com.localdictation")
        .isolated_fixture("http://127.0.0.1/feed".into(), "test".into())
        .is_err());
    assert!(owner(ID)
        .isolated_fixture("https://example.test/feed".into(), "test".into())
        .is_err());
    assert!(owner(ID)
        .isolated_fixture("http://127.0.0.1/feed".into(), "test".into())
        .is_ok());
    assert_eq!(owner("com.localdictation").feed, FEED);
    assert_eq!(owner("com.localdictation").key, PUBLIC_KEY);
}
