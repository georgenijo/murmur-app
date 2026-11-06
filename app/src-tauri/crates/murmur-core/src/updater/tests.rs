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
    signed_with(bytes, &pair)
}
fn signed_with(bytes: &[u8], pair: &Ed25519KeyPair) -> (String, String) {
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
    for name in install::EXECUTABLES {
        let path = path.join("Contents/MacOS").join(name);
        if *name != "ui" {
            fs::write(&path, "synthetic helper").unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
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
    tar.follow_symlinks(false);
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
    use std::{io::Write, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read_request(&mut socket).unwrap();
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
        let offer = offer(sig);
        e.bytes = Some(Download {
            identity: Download::identity(&offer),
            bytes,
        });
        e.offer = Some(offer);
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

fn read_request(socket: &mut impl std::io::Read) -> std::io::Result<String> {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        socket.read_exact(&mut byte)?;
        request.push(byte[0]);
        if request.len() > 8192 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "fixture headers too large",
            ));
        }
    }
    String::from_utf8(request).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

type ResponseGate = (
    String,
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);
struct Server {
    gate: Arc<Mutex<Option<ResponseGate>>>,
    feed_failures: Arc<std::sync::atomic::AtomicUsize>,
    omit_length: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    routes: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
    count: Arc<std::sync::atomic::AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        use std::{io::Write, net::TcpListener};
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
        let gate = Arc::new(Mutex::new(None::<ResponseGate>));
        let paused = gate.clone();
        let feed_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failures = feed_failures.clone();
        let omit_length = Arc::new(AtomicBool::new(false));
        let omit = omit_length.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let request = read_request(&mut socket)
                        .expect("fixture request must contain complete headers");
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .expect("fixture request target");
                    calls.fetch_add(1, Ordering::AcqRel);
                    let pause = {
                        let mut gate = paused.lock().unwrap();
                        if gate.as_ref().is_some_and(|g| g.0 == path) {
                            gate.take()
                        } else {
                            None
                        }
                    };
                    if let Some((_, arrived, resume)) = pause {
                        arrived.send(()).unwrap();
                        resume
                            .recv_timeout(Duration::from_secs(5))
                            .expect("fixture gate released");
                    }
                    let body = if path == "/feed"
                        && failures
                            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                            .is_ok()
                    {
                        None
                    } else {
                        copy.lock().unwrap().get(path).cloned()
                    };
                    let (code, body) = body
                        .map(|b| ("200 OK", b))
                        .unwrap_or_else(|| ("503 Unavailable", vec![]));
                    let length = if omit.load(Ordering::Acquire) {
                        String::new()
                    } else {
                        format!("Content-Length: {}\r\n", body.len())
                    };
                    let header = format!("HTTP/1.1 {code}\r\n{length}Connection: close\r\n\r\n");
                    let _ = socket.write_all(header.as_bytes());
                    let _ = socket.write_all(&body);
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        Self {
            gate,
            feed_failures,
            omit_length,
            address,
            routes,
            count,
            stop,
            thread: Some(thread),
        }
    }
    fn hold(&self, path: &str) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (arrived, waiting) = std::sync::mpsc::channel();
        let (resume, held) = std::sync::mpsc::channel();
        *self.gate.lock().unwrap() = Some((path.into(), arrived, held));
        (waiting, resume)
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
    let bytes = updater.engine.lock().unwrap().bytes.clone().unwrap().bytes;
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

fn pending(store: &CompletionService) -> Value {
    store
        .lifecycle(Operation::Status)
        .unwrap()
        .values
        .get("pending-update-release-notes")
        .cloned()
        .unwrap_or(Value::Null)
}
fn stale(store: &CompletionService) {
    store
        .lifecycle(Operation::Save {
            key: "updater-last-check".into(),
            value: json!(0),
        })
        .unwrap();
}
fn feed_value(server: &Server) -> Value {
    serde_json::from_slice(server.routes.lock().unwrap().get("/feed").unwrap()).unwrap()
}
fn publish(server: &Server, manifest: &Value) {
    server.set("/feed", serde_json::to_vec(manifest).unwrap());
}
fn assert_original(updater: &Updater, target: &Path) {
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.0.0"
    );
    let engine = updater.engine.lock().unwrap();
    assert!(!engine.installed);
    assert!(!engine.relaunch_requested);
    assert!(engine.bytes.is_none());
    assert!(!updater.root.join("updater-install.json").exists());
    assert!(fs::read_dir(target.parent().unwrap()).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".murmur-update-")));
}
#[test]
fn nonexecutable_missing_symlinked_helpers_or_invalid_main_are_refused_before_swap() {
    for name in install::EXECUTABLES {
        for failure in ["mode", "missing", "symlink"] {
            let temp = scratch();
            let server = Server::new();
            let (mut updater, _, target) = network_owner(temp.path(), &server, false);
            let file = temp.path().join("New.app/Contents/MacOS").join(name);
            match failure {
                "mode" => {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(file, fs::Permissions::from_mode(0o644)).unwrap();
                }
                "missing" => fs::remove_file(file).unwrap(),
                _ => {
                    fs::remove_file(&file).unwrap();
                    std::os::unix::fs::symlink("ui", file).unwrap();
                }
            }
            let bytes = archive(&temp.path().join("New.app"));
            let (key, signature) = signed(&bytes);
            updater.key = key;
            let mut manifest = feed_value(&server);
            manifest["platforms"][platform()]["signature"] = json!(signature);
            publish(&server, &manifest);
            server.set("/archive", bytes);
            updater.act(Action::Check { background: false }).unwrap();
            updater.act(Action::Download).unwrap();
            assert!(updater.act(Action::Install).is_err(), "{name}: {failure}");
            assert_original(&updater, &target);
        }
    }
    let temp = scratch();
    let server = Server::new();
    let (mut updater, _, target) = network_owner(temp.path(), &server, false);
    let path = temp.path().join("New.app/Contents/Info.plist");
    let mut plist = plist::Value::from_file(&path).unwrap();
    plist.as_dictionary_mut().unwrap().insert(
        "CFBundleExecutable".into(),
        plist::Value::String("other".into()),
    );
    plist.to_file_xml(path).unwrap();
    let bytes = archive(&temp.path().join("New.app"));
    let (key, signature) = signed(&bytes);
    updater.key = key;
    let mut manifest = feed_value(&server);
    manifest["platforms"][platform()]["signature"] = json!(signature);
    publish(&server, &manifest);
    server.set("/archive", bytes);
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Download).unwrap();
    assert!(updater.act(Action::Install).is_err());
    assert_original(&updater, &target);
}
#[test]
fn signed_bad_archive_can_be_corrected_at_same_version_without_restarting() {
    for failure in ["extract", "validate", "verify"] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, _, target) = network_owner(temp.path(), &server, false);
        let pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let good = archive(&temp.path().join("New.app"));
        let bad = if failure == "extract" {
            b"not gzip".to_vec()
        } else {
            make_bundle(&temp.path().join("Wrong.app"), "9.9.9");
            archive(&temp.path().join("Wrong.app"))
        };
        let (key, sig) = signed_with(&bad, &pair);
        updater.key = key;
        let mut manifest = feed_value(&server);
        manifest["platforms"][platform()]["signature"] = json!(sig);
        publish(&server, &manifest);
        server.set("/archive", bad);
        updater.act(Action::Check { background: false }).unwrap();
        updater.act(Action::Download).unwrap();
        if failure == "verify" {
            updater.engine.lock().unwrap().bytes.as_mut().unwrap().bytes[0] ^= 1;
        }
        assert!(updater.act(Action::Install).is_err());
        assert_original(&updater, &target);
        let (_, sig) = signed_with(&good, &pair);
        manifest["platforms"][platform()]["signature"] = json!(sig);
        manifest["platforms"][platform()]["url"] = json!(server.url("/corrected"));
        publish(&server, &manifest);
        server.set("/corrected", good);
        updater.act(Action::Check { background: false }).unwrap();
        updater.act(Action::Download).unwrap();
        assert!(updater.act(Action::Install).unwrap());
        assert_eq!(
            fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
            "synthetic 1.2.0"
        );
    }
}
#[test]
fn cache_identity_requires_version_url_and_signature() {
    for field in ["version", "url", "signature"] {
        let temp = scratch();
        let server = Server::new();
        let (updater, _, target) = network_owner(temp.path(), &server, false);
        updater.act(Action::Check { background: false }).unwrap();
        updater.act(Action::Download).unwrap();
        let mut e = updater.engine.lock().unwrap();
        let offer = e.offer.as_mut().unwrap();
        match field {
            "version" => offer.version = "1.3.0".into(),
            "url" => offer.url = server.url("/new"),
            _ => offer.signature.push('x'),
        }
        drop(e);
        assert!(updater.act(Action::Install).is_err());
        assert_original(&updater, &target);
    }
}
#[test]
fn configure_preserves_downloaded_notes_then_relaunch_shows_once_after_dismissal() {
    let temp = scratch();
    let server = Server::new();
    let (updater, store, target) = network_owner(temp.path(), &server, false);
    let mut manifest = feed_value(&server);
    manifest["release_history"] = json!([{"version":"1.1.0","notes":"Intermediate"}]);
    publish(&server, &manifest);
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Download).unwrap();
    let saved = pending(&store);
    assert!(saved["notes"].as_str().unwrap().contains("Intermediate"));
    for automatic in [false, true, false] {
        updater.act(Action::Configure { automatic }).unwrap();
        assert_eq!(pending(&store), saved);
        assert!(updater.snapshot().completed_update.is_none());
    }
    assert!(updater.act(Action::Install).unwrap());
    let next = || {
        Updater::production(
            temp.path().join("data"),
            target.join("Contents/MacOS/ui"),
            ID.into(),
            "1.2.0".into(),
            store.clone(),
            |_| {},
            |_| {},
        )
    };
    let replacement = next();
    replacement
        .act(Action::Configure { automatic: false })
        .unwrap();
    assert_eq!(
        replacement.snapshot().completed_update.unwrap().notes,
        saved["notes"]
    );
    replacement.act(Action::DismissCompleted).unwrap();
    assert!(pending(&store).is_null());
    let later = next();
    later.act(Action::Configure { automatic: false }).unwrap();
    assert!(later.snapshot().completed_update.is_none());
}
#[test]
fn pending_notes_preserve_future_clear_obsolete_and_are_superseded_on_new_download() {
    let temp = scratch();
    let server = Server::new();
    let (updater, store, _) = network_owner(temp.path(), &server, false);
    for version in ["0.9.0", "1.1.0", "1.0.0-beta.1", "1.0.0+other"] {
        store
            .lifecycle(Operation::Save {
                key: "pending-update-release-notes".into(),
                value: json!({"version":version,"notes":"Saved"}),
            })
            .unwrap();
        updater.act(Action::Configure { automatic: false }).unwrap();
        assert!(updater.snapshot().completed_update.is_none());
        // Future notes, including a distinct build, are never mistaken for installed notes.
        assert_eq!(
            pending(&store).is_null(),
            matches!(version, "0.9.0" | "1.0.0-beta.1")
        );
    }
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Download).unwrap();
    assert_eq!(pending(&store)["version"], "1.2.0");
}
#[test]
fn startup_timer_focus_visibility_and_wake_errors_are_quiet_but_manual_actions_open() {
    // All lifecycle sources map to Check(background:true); exercise each label
    // separately and pair with the frontend forwarding tests.
    for trigger in [
        "startup",
        "timer",
        "focus",
        "visibility",
        "wake",
        "manual",
        "download",
        "install",
        "canary",
    ] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, _, _) = network_owner(temp.path(), &server, false);
        updater.automatic.store(true, Ordering::Release);
        server.routes.lock().unwrap().remove("/feed");
        let action = match trigger {
            "startup" => Action::Configure { automatic: true },
            "manual" => Action::Check { background: false },
            "download" => Action::Download,
            "install" => Action::Install,
            "canary" => {
                updater.canary = Some(temp.path().join("canary.json"));
                Action::Configure { automatic: true }
            }
            _ => Action::Check { background: true },
        };
        assert!(updater.act(action).is_err(), "{trigger}");
        assert!(matches!(
            updater.snapshot().update_status,
            Status::Error { .. }
        ));
        assert_eq!(
            updater.snapshot().is_update_dialog_open,
            matches!(trigger, "manual" | "download" | "install" | "canary"),
            "{trigger}"
        );
    }
}
#[test]
fn request_fixture_waits_for_fragmented_headers_and_rejects_truncation() {
    use std::{
        io::{Read, Write},
        net::TcpStream,
    };
    let server = Server::new();
    server.set("/fragmented", b"synthetic response".to_vec());
    let mut socket = TcpStream::connect(server.address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket.write_all(b"GET /frag").unwrap();
    std::thread::sleep(Duration::from_millis(10));
    assert_eq!(server.count(), 0);
    socket
        .write_all(b"mented HTTP/1.1\r\nHost: fixture\r\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(10));
    assert_eq!(server.count(), 0);
    socket.write_all(b"\r\n").unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    assert!(response.ends_with("synthetic response"));
    assert_eq!(server.count(), 1);
    assert!(read_request(&mut &b"GET / HTTP/1.1\r\n"[..]).is_err());
    assert!(read_request(&mut &vec![b'a'; 8193][..]).is_err());
}

#[test]
fn stale_refresh_selects_newer_or_corrected_same_version_and_retains_no_newer() {
    for change in ["newer", "same-version", "withdrawn", "older", "recent"] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, store, _) = network_owner(temp.path(), &server, false);
        updater.act(Action::Check { background: false }).unwrap();
        if change != "recent" {
            updater
                .engine
                .lock()
                .unwrap()
                .offer
                .as_mut()
                .unwrap()
                .discovered = 0;
        }
        let mut manifest = feed_value(&server);
        let original_sig = manifest["platforms"][platform()]["signature"].clone();
        if matches!(change, "newer" | "same-version") {
            let target = if change == "newer" { "1.3.0" } else { "1.2.0" };
            make_bundle(&temp.path().join("Corrected.app"), target);
            let bytes = archive(&temp.path().join("Corrected.app"));
            let (key, sig) = signed(&bytes);
            updater.key = key;
            manifest["version"] = json!(target);
            manifest["min_version"] = json!("1.1.0");
            manifest["notes"] = json!("Corrected");
            manifest["release_history"] = json!([{"version":"1.1.0","notes":"Missed"}]);
            manifest["platforms"][platform()]["signature"] = json!(sig);
            manifest["platforms"][platform()]["url"] = json!(server.url("/corrected"));
            server.set("/corrected", bytes);
        } else if change == "withdrawn" {
            manifest["version"] = json!("1.0.0");
        } else {
            manifest["version"] = json!("1.1.0");
        }
        publish(&server, &manifest);
        updater.act(Action::Download).unwrap();
        let e = updater.engine.lock().unwrap();
        let offered = e.offer.as_ref().unwrap();
        assert_eq!(
            offered.version,
            if change == "newer" { "1.3.0" } else { "1.2.0" }
        );
        assert!(e.bytes.as_ref().unwrap().matches(offered));
        if matches!(change, "newer" | "same-version") {
            assert!(offered.forced);
            assert!(pending(&store)["notes"]
                .as_str()
                .unwrap()
                .contains("Missed"));
        } else {
            assert_eq!(offered.signature, original_sig);
        }
        assert_eq!(server.count(), if change == "recent" { 2 } else { 3 });
    }
}
#[test]
fn semver_policy_and_notes_cover_legacy_edge_matrix() {
    for (a, b, result) in [
        ("1.2.3", "1.2.3", Some(std::cmp::Ordering::Equal)),
        ("0.6.1", "0.6.2", Some(std::cmp::Ordering::Less)),
        ("0.6.3", "0.6.2", Some(std::cmp::Ordering::Greater)),
        ("1.0.0", "0.9.9", Some(std::cmp::Ordering::Greater)),
        (
            " v1.2.3-rc.1+build ",
            "1.2.3",
            Some(std::cmp::Ordering::Equal),
        ),
        ("bad", "1.0.0", None),
        ("1.0.0", "bad", None),
        ("bad", "bad", None),
    ] {
        assert_eq!(compare(a, b), result);
    }
    for invalid in ["", "abc", "1.2"] {
        assert!(version(invalid).is_none());
    }
    for valid in [
        "1.2.3",
        "0.6.2",
        "10.20.30",
        "1.2.3-beta.1",
        "1.2.3+build.123",
        " v0.6.2 ",
    ] {
        assert!(version(valid).is_some());
    }
    assert!(!forced("0.7.0", &json!("0.7.0")));
    assert!(forced("bad", &json!("bad")));
    let mut m = manifest();
    for minimum in [
        None,
        Some(json!("1.1.0")),
        Some(json!(23)),
        Some(Value::Null),
    ] {
        m.as_object_mut().unwrap().remove("min_version");
        if let Some(v) = &minimum {
            m["min_version"] = v.clone();
        }
        let offer = parse_feed(
            &serde_json::to_vec(&m).unwrap(),
            "1.0.0",
            "darwin-aarch64",
            0,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            offer.policy_verified,
            minimum.is_none() || minimum.as_ref().unwrap().is_string()
        );
        assert_eq!(offer.forced, minimum.as_ref().is_some_and(Value::is_string));
    }
    for (stored, current, matches) in [
        ("v1.2.0", "1.2.0", true),
        ("1.2.0-beta.1", "1.2.0", false),
        ("bad", "bad", false),
    ] {
        assert_eq!(
            pending_for(&json!({"version":stored,"notes":"Notes"}), current).is_some(),
            matches
        );
    }
    assert!(normalize_pending(json!("{broken")).is_err());
    assert!(pending_for(&Value::Null, "1.2.0").is_none());
    m["version"] = json!("0.59.11");
    m["notes"] = json!("Eleven");
    m["release_history"] = json!([{"version":"v0.59.9","notes":"Nine"},{"version":"0.59.10","notes":"Ten"},null,{"version":"bad","notes":"Invalid"},{"version":"0.59.10-beta","notes":"Preview"},{"version":"0.59.12","notes":"Future"}]);
    let notes = parse_feed(
        &serde_json::to_vec(&m).unwrap(),
        "v0.59.8",
        "darwin-aarch64",
        0,
    )
    .unwrap()
    .unwrap()
    .notes;
    assert_eq!(
        notes,
        "# v0.59.11\n\nEleven\n\n---\n\n# v0.59.10\n\nTen\n\n---\n\n# v0.59.9\n\nNine"
    );
    for history in [
        Value::Null,
        json!("bad"),
        json!([null, {}, {"version":"0.59.9","notes":42}]),
    ] {
        m["release_history"] = history;
        assert_eq!(
            parse_feed(
                &serde_json::to_vec(&m).unwrap(),
                "0.59.8",
                "darwin-aarch64",
                0
            )
            .unwrap()
            .unwrap()
            .notes,
            "Eleven"
        );
    }
    m["release_history"] = json!([{"version":"0.59.9","notes":"Nine"}]);
    assert_eq!(
        parse_feed(
            &serde_json::to_vec(&m).unwrap(),
            "0.59.8-beta",
            "darwin-aarch64",
            0
        )
        .unwrap()
        .unwrap()
        .notes,
        "Eleven"
    );
}
#[test]
fn background_policy_optional_forced_notification_once_and_withdrawal() {
    for forced in [false, true] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, store, _) = network_owner(temp.path(), &server, forced);
        let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let copy = notified.clone();
        updater.notify = Arc::new(move |_| {
            copy.fetch_add(1, Ordering::AcqRel);
        });
        updater.act(Action::Configure { automatic: false }).unwrap();
        assert_eq!(server.count(), 0);
        updater.act(Action::Configure { automatic: true }).unwrap();
        assert_eq!(updater.snapshot().is_update_dialog_open, forced);
        assert_eq!(notified.load(Ordering::Acquire), 1);
        updater.act(Action::Configure { automatic: true }).unwrap();
        assert_eq!(server.count(), 1);
        stale(&store);
        updater.act(Action::Check { background: true }).unwrap();
        assert_eq!(server.count(), 2);
        assert_eq!(notified.load(Ordering::Acquire), 1);
        updater.act(Action::Show).unwrap();
        let mut m = feed_value(&server);
        m["version"] = json!("1.0.0");
        publish(&server, &m);
        stale(&store);
        updater.act(Action::Check { background: true }).unwrap();
        assert_eq!(updater.snapshot().update_status, Status::Idle);
        assert!(!updater.snapshot().is_update_dialog_open);
        assert!(updater.engine.lock().unwrap().offer.is_none());
        assert!(updater.act(Action::Download).is_err());
    }
}
#[test]
fn in_flight_background_setup_suppresses_presentation_then_due_checks_resume() {
    for forced in [false, true] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, store, _) = network_owner(temp.path(), &server, forced);
        let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let copy = notified.clone();
        updater.notify = Arc::new(move |_| {
            copy.fetch_add(1, Ordering::AcqRel);
        });
        let updater = Arc::new(updater);
        let (arrived, resume) = server.hold("/feed");
        let copy = updater.clone();
        let background =
            std::thread::spawn(move || copy.act(Action::Configure { automatic: true }));
        arrived.recv_timeout(Duration::from_secs(2)).unwrap();
        // Configure stores readiness before waiting for the serialized owner.
        let copy = updater.clone();
        let setup = std::thread::spawn(move || copy.act(Action::Configure { automatic: false }));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while updater.automatic.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "setup readiness published before gate deadline"
            );
            std::thread::yield_now();
        }
        resume.send(()).unwrap();
        background.join().unwrap().unwrap();
        setup.join().unwrap().unwrap();
        assert!(!updater.snapshot().is_update_dialog_open);
        assert_eq!(notified.load(Ordering::Acquire), 0);
        updater.act(Action::Check { background: false }).unwrap();
        assert!(updater.snapshot().is_update_dialog_open);
        assert_eq!(notified.load(Ordering::Acquire), 0);
        updater.act(Action::Configure { automatic: true }).unwrap();
        assert_eq!(server.count(), 2); // no repeated startup check
        stale(&store);
        updater.act(Action::Check { background: true }).unwrap();
        assert_eq!(notified.load(Ordering::Acquire), 1);
        stale(&store);
        updater.act(Action::Check { background: true }).unwrap();
        assert_eq!(notified.load(Ordering::Acquire), 1);
    }
}
#[test]
fn concurrent_checks_downloads_and_installs_have_one_owner_and_preserve_dialog() {
    for owner in ["check", "background", "download"] {
        let temp = scratch();
        let server = Server::new();
        let (updater, _, _) = network_owner(temp.path(), &server, false);
        let updater = Arc::new(updater);
        if owner == "download" {
            updater.act(Action::Check { background: false }).unwrap();
        }
        updater.automatic.store(true, Ordering::Release);
        let (arrived, resume) = server.hold(if owner == "download" {
            "/archive"
        } else {
            "/feed"
        });
        let copy = updater.clone();
        let kind = owner.to_string();
        let first = std::thread::spawn(move || {
            copy.act(match kind.as_str() {
                "check" => Action::Check { background: false },
                "background" => Action::Check { background: true },
                _ => Action::Download,
            })
        });
        arrived.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!updater.drained());
        let calls = server.count();
        if owner == "download" {
            updater.act(Action::Check { background: false }).unwrap();
            assert!(matches!(
                updater.snapshot().update_status,
                Status::Downloading { .. }
            ));
        }
        updater.act(Action::Check { background: true }).unwrap();
        assert_eq!(server.count(), calls);
        let copy = updater.clone();
        let second = std::thread::spawn(move || copy.act(Action::Download));
        // A second download joins the first owner; no second transfer.
        resume.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        assert!(matches!(
            updater.snapshot().update_status,
            Status::Ready { .. }
        ));
        assert!(updater.snapshot().is_update_dialog_open);
        assert_eq!(server.count(), 2);
        updater.act(Action::Check { background: false }).unwrap();
        updater.act(Action::Download).unwrap();
        assert_eq!(server.count(), 2);
        let copy = updater.clone();
        let first = std::thread::spawn(move || copy.act(Action::Install));
        let copy = updater.clone();
        let second = std::thread::spawn(move || copy.act(Action::Install));
        assert_ne!(
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap()
        );
    }
}
#[test]
fn canary_policy_download_archive_and_completion_failures_preserve_exact_stage() {
    for failure in ["policy", "download", "archive", "relaunch"] {
        let temp = scratch();
        let server = Server::new();
        let (mut updater, _, target) = network_owner(temp.path(), &server, false);
        updater.canary = Some(temp.path().join("canary.json"));
        let mut m = feed_value(&server);
        match failure {
            "policy" => {
                m["min_version"] = json!(23);
                publish(&server, &m);
            }
            "download" => {
                server.routes.lock().unwrap().remove("/archive");
            }
            "archive" => {
                let bad = b"signed invalid archive".to_vec();
                let (key, sig) = signed(&bad);
                updater.key = key;
                m["platforms"][platform()]["signature"] = json!(sig);
                publish(&server, &m);
                server.set("/archive", bad);
            }
            _ => {}
        }
        let result = updater.act(Action::Configure { automatic: true });
        if failure == "relaunch" {
            assert!(result.unwrap());
            updater.canary_failed("Synthetic host relaunch refused");
        } else {
            assert!(result.is_err());
        }
        let result = updater.read_canary().unwrap();
        assert_eq!(result["status"], "failed");
        let stage = match failure {
            "archive" => "install",
            other => other,
        };
        assert_eq!(result["stages"][stage], "failed");
        if failure == "relaunch" {
            assert_eq!(result["stages"]["install"], "passed");
        } else {
            assert_eq!(
                fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
                "synthetic 1.0.0"
            );
            assert!(!updater.engine.lock().unwrap().relaunch_requested);
        }
    }
}

#[test]
fn transient_feed_failure_retries_once_and_canary_marker_absence_is_inert() {
    let temp = scratch();
    let server = Server::new();
    let (updater, _, _) = network_owner(temp.path(), &server, false);
    server.feed_failures.store(1, Ordering::Release);
    updater.act(Action::Configure { automatic: true }).unwrap();
    assert_eq!(server.count(), 2);
    assert!(matches!(
        updater.snapshot().update_status,
        Status::Available { .. }
    ));
    assert!(updater.canary.is_none());
    assert!(updater.read_canary().is_none());
    assert!(updater.engine.lock().unwrap().bytes.is_none());
}
#[test]
fn preparing_progress_ready_and_translocated_recovery_follow_the_real_owner() {
    let temp = scratch();
    let server = Server::new();
    let (mut updater, _, target) = network_owner(temp.path(), &server, false);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copy = seen.clone();
    updater.changed = Arc::new(move |s| {
        copy.lock().unwrap().push(s);
    });
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Dismiss).unwrap();
    updater.act(Action::Download).unwrap();
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|s| matches!(s.update_status, Status::Preparing { .. }) && s.is_update_dialog_open));
    assert!(seen.lock().unwrap().iter().any(|s| matches!(
        s.update_status,
        Status::Downloading { progress: None, .. }
    ) && s.is_update_dialog_open));
    assert!(seen.lock().unwrap().iter().any(
        |s| matches!(s.update_status,Status::Downloading { progress: Some(p), .. } if p == 100.)
    ));
    assert!(!updater.engine.lock().unwrap().installed);
    assert_eq!(
        fs::read_to_string(target.join("Contents/MacOS/ui")).unwrap(),
        "synthetic 1.0.0"
    );
    updater.act(Action::Dismiss).unwrap();
    assert!(!updater.snapshot().is_update_dialog_open);
    let temp = scratch();
    let server = Server::new();
    let (mut updater, _, _) = network_owner(temp.path(), &server, false);
    updater.act(Action::Check { background: false }).unwrap();
    updater.executable =
        PathBuf::from("/private/AppTranslocation/fixture/Murmur.app/Contents/MacOS/ui");
    assert!(updater.act(Action::Download).is_err());
    assert_eq!(server.count(), 1);
    assert!(
        matches!(updater.snapshot().update_status,Status::Error { recovery: Some(r), .. } if r=="reinstall")
    );
}
#[test]
fn manual_check_joins_busy_owner_without_another_fetch() {
    let temp = scratch();
    let server = Server::new();
    let (updater, _, _) = network_owner(temp.path(), &server, false);
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Dismiss).unwrap();
    let updater = Arc::new(updater);
    let (locked, ready) = std::sync::mpsc::channel();
    let copy = updater.clone();
    let owner = std::thread::spawn(move || {
        let _engine = copy.engine.lock().unwrap();
        locked.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    updater.act(Action::Check { background: false }).unwrap();
    owner.join().unwrap();
    assert_eq!(server.count(), 1);
    assert!(updater.snapshot().is_update_dialog_open);
}

#[derive(Clone)]
struct DiagnosticCodes(Arc<Mutex<Vec<String>>>);
impl tracing::Subscriber for DiagnosticCodes {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        // Other test threads have no subscriber. Reevaluate this thread's
        // dispatcher rather than caching their disabled callsite globally.
        tracing::subscriber::Interest::sometimes()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::TRACE)
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Codes<'a>(&'a mut Vec<String>);
        impl tracing::field::Visit for Codes<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "event_code" {
                    self.0
                        .push(format!("{value:?}").trim_matches('"').to_owned());
                }
            }
        }
        event.record(&mut Codes(&mut self.0.lock().unwrap()));
    }
}
#[test]
fn stable_diagnostics_cover_current_retry_policy_download_ready_and_failures() {
    // Tracing caches callsite interest process-wide while the rest of the
    // suite runs without subscribers. Isolate this diagnostic contract test
    // from those concurrent callsites; this is a test binary, never the app.
    if std::env::var_os("MURMUR_UPDATER_DIAGNOSTIC_TEST").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "updater::tests::stable_diagnostics_cover_current_retry_policy_download_ready_and_failures", "--test-threads=1"])
            .env("MURMUR_UPDATER_DIAGNOSTIC_TEST", "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let codes = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(DiagnosticCodes(codes.clone()), || {
        let temp = scratch();
        let server = Server::new();
        let (updater, _, _) = network_owner(temp.path(), &server, false);
        let original = feed_value(&server);
        let mut m = original.clone();
        m["version"] = json!("1.0.0");
        publish(&server, &m);
        updater.act(Action::Check { background: false }).unwrap();
        let mut m = original;
        m["min_version"] = json!(23);
        publish(&server, &m);
        server.feed_failures.store(1, Ordering::Release);
        updater.act(Action::Check { background: false }).unwrap();
        updater.act(Action::Download).unwrap();
        updater.engine.lock().unwrap().bytes = None;
        server.routes.lock().unwrap().remove("/archive");
        assert!(updater.act(Action::Download).is_err());
        assert!(updater.act(Action::Install).is_err());
        server.routes.lock().unwrap().remove("/feed");
        assert!(updater.act(Action::Check { background: false }).is_err());
    });
    let codes = codes.lock().unwrap();
    for code in [
        "updater.check_current",
        "updater.check_retry",
        "updater.policy_unavailable",
        "updater.download_starting",
        "updater.download_ready",
        "updater.install_failed",
        "updater.restart_failed",
        "updater.check_failed",
    ] {
        assert!(codes.iter().any(|c| c == code), "missing {code}: {codes:?}");
    }
}

#[test]
fn canary_receipt_roundtrip_requires_matching_installed_version_to_confirm() {
    let temp = scratch();
    let server = Server::new();
    let (mut updater, _, _) = network_owner(temp.path(), &server, false);
    updater.canary = Some(temp.path().join("canary/result.json"));
    let receipt = json!({"schemaVersion":1,"status":"pending","checkedVersion":"0.9.0","offeredVersion":"1.2.0","forced":false,"dryRun":false,"stages":{"discover":"passed","policy":"passed","download":"passed","signatureVerify":"passed","install":"passed","relaunch":"pending"},"error":null});
    updater.write_canary(&receipt).unwrap();
    assert_eq!(updater.read_canary().unwrap(), receipt);
    server.routes.lock().unwrap().remove("/feed");
    assert!(updater.act(Action::Configure { automatic: true }).is_err());
    assert_eq!(updater.read_canary().unwrap()["status"], "failed");
    assert_ne!(
        updater.read_canary().unwrap()["stages"]["relaunch"],
        "passed"
    );
}

#[test]
fn download_without_content_length_remains_indeterminate_until_ready() {
    let temp = scratch();
    let server = Server::new();
    server.omit_length.store(true, Ordering::Release);
    let (mut updater, _, _) = network_owner(temp.path(), &server, false);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let copy = seen.clone();
    updater.changed = Arc::new(move |s| {
        copy.lock().unwrap().push(s.update_status);
    });
    updater.act(Action::Check { background: false }).unwrap();
    updater.act(Action::Download).unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen
        .iter()
        .any(|s| matches!(s, Status::Downloading { progress: None, .. })));
    assert!(!seen.iter().any(|s| matches!(
        s,
        Status::Downloading {
            progress: Some(_),
            ..
        }
    )));
    assert!(matches!(
        updater.snapshot().update_status,
        Status::Ready { .. }
    ));
}

#[test]
fn download_click_presents_dialog_while_waiting_for_background_discovery() {
    let temp = scratch();
    let server = Server::new();
    let (mut updater, _, _) = network_owner(temp.path(), &server, false);
    let (presented, waiting) = std::sync::mpsc::channel();
    updater.changed = Arc::new(move |s| {
        if s.is_update_dialog_open {
            let _ = presented.send(());
        }
    });
    updater.automatic.store(true, Ordering::Release);
    let updater = Arc::new(updater);
    let (arrived, resume) = server.hold("/feed");
    let copy = updater.clone();
    let background = std::thread::spawn(move || copy.act(Action::Check { background: true }));
    arrived.recv_timeout(Duration::from_secs(2)).unwrap();
    let copy = updater.clone();
    let download = std::thread::spawn(move || copy.act(Action::Download));
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(updater.snapshot().is_update_dialog_open);
    assert_eq!(updater.snapshot().update_status, Status::Idle);
    resume.send(()).unwrap();
    background.join().unwrap().unwrap();
    download.join().unwrap().unwrap();
    assert!(updater.snapshot().is_update_dialog_open);
    assert!(matches!(
        updater.snapshot().update_status,
        Status::Ready { .. }
    ));
    assert_eq!(server.count(), 2);
}
