/// Capability obtained from the actual host webview, never a guessed directory.
#[derive(Clone)]
pub struct StoreIdentity {
    store: Retained<WKWebsiteDataStore>,
}
impl StoreIdentity {
    /// # Safety
    /// `view` must be a live WKWebView on this main thread (Tauri with_webview.inner()).
    /// Refuse custom/incognito stores: the legacy app uses the default store.
    pub unsafe fn from_app_webview(
        view: *mut std::ffi::c_void,
        mtm: MainThreadMarker,
    ) -> Result<Self, &'static str> {
        let view = unsafe { (view as *mut WKWebView).as_ref() }.ok_or("missing_webview")?;
        let store = unsafe { view.configuration().websiteDataStore() };
        let expected = unsafe { WKWebsiteDataStore::defaultDataStore(mtm) };
        if !std::ptr::eq(&*store, &*expected) {
            return Err("unproven_store_identity");
        }
        Ok(Self { store })
    }
    /// Synthetic roots only. Never obtains the default store.
    pub fn synthetic(store: &WKWebsiteDataStore) -> Self {
        assert!(!unsafe { store.isPersistent() });
        Self {
            store: unsafe { Retained::retain(store as *const _ as *mut _).unwrap() },
        }
    }
}
use murmur_core::completion::{
    lifecycle::{Operation, TRANSCRIPT_KEYS, UPDATER_V3_KEYS, V2_KEYS},
    Command, CompletionService, Destination,
};
use objc2::{
    define_class, msg_send, rc::Retained, runtime::ProtocolObject, AnyThread, DefinedClass,
    MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{
    NSData, NSError, NSObject, NSObjectProtocol, NSRect, NSString, NSURLRequest, NSURLResponse,
    NSURL,
};
use objc2_web_kit::*;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Ivars {
    messages: RefCell<Option<mpsc::Sender<String>>>,
    page: RefCell<String>,
    failed: Rc<Cell<bool>>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Handler;
    unsafe impl NSObjectProtocol for Handler {}
    unsafe impl WKURLSchemeHandler for Handler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        unsafe fn start(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // A fixed local page, without a network surface or arbitrary resources.
            let request = unsafe { task.request() };
            let Some(url) = request.URL() else {
                return;
            };
            if url.absoluteString().as_deref() != Some(&NSString::from_str("tauri://localhost/")) {
                return;
            }
            let response = {
                NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
                    NSURLResponse::alloc(),
                    &url,
                    Some(&NSString::from_str("text/html")),
                    -1,
                    Some(&NSString::from_str("utf-8")),
                )
            };
            let data = NSData::with_bytes(self.ivars().page.borrow().as_bytes());
            unsafe {
                task.didReceiveResponse(&response);
                task.didReceiveData(&data);
                task.didFinish();
            }
        }
        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn stop(&self, _view: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }
    unsafe impl WKNavigationDelegate for Handler {
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        unsafe fn provisional_failed(
            &self,
            _view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            _error: &NSError,
        ) {
            self.ivars().failed.set(true);
        }
        #[unsafe(method(webView:didFailNavigation:withError:))]
        unsafe fn navigation_failed(
            &self,
            _view: &WKWebView,
            _navigation: Option<&WKNavigation>,
            _error: &NSError,
        ) {
            self.ivars().failed.set(true);
        }
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        unsafe fn process_failed(&self, _view: &WKWebView) {
            self.ivars().failed.set(true);
        }
    }
    unsafe impl WKScriptMessageHandler for Handler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        unsafe fn message(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            let body = unsafe { message.body() };
            if let Some(s) = body.downcast_ref::<NSString>() {
                if let Some(send) = self.ivars().messages.borrow().as_ref() {
                    let _ = send.send(s.to_string());
                }
            }
        }
    }
);
/// An invisible, standalone importer. The owning native host retains it and
/// calls poll until a terminal status. Both Done and PreservedForRetry unblock
/// presentation of Rust-owned data; failures retry on a later launch.
/// Drop it before store retirement. React hosts use their main startup instead.
#[derive(Clone, Copy)]
pub enum SyntheticFailure {
    Navigation,
    ProcessTermination,
    Javascript,
    Deadline,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RetirementFailure {
    Purge,
    Verification,
    DirectoryRemoval,
    ParentSync,
}
type WorkResult = Result<(Vec<String>, bool), ()>;
pub struct Importer {
    view: Retained<WKWebView>,
    handler: Retained<Handler>,
    receive: mpsc::Receiver<String>,
    service: CompletionService,
    pending: Option<mpsc::Receiver<WorkResult>>,
    removing: bool,
    done: bool,
    committed: bool,
    failed: bool,
    synthetic_failure: Option<SyntheticFailure>,
    deadline: Instant,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ImportStatus {
    Pending,
    Done,
    PreservedForRetry,
}
impl Importer {
    /// `store` is selected by the host. Production's native host passes the
    /// default store; tests always pass a custom nonpersistent store. This
    /// constructor itself never opens a default store or production directory.
    /// Native startup gates creation before touching WebKit. A failed importer
    /// is terminal for this launch; the host presents Rust data and retries later.
    pub fn start(
        mtm: MainThreadMarker,
        identity: impl FnOnce() -> Result<StoreIdentity, &'static str>,
        service: CompletionService,
    ) -> Result<Option<Self>, &'static str> {
        if service
            .lifecycle(Operation::Status)
            .map_err(|_| "source_storage_failed")?
            .retired
        {
            return Ok(None);
        }
        let identity = identity()?;
        Ok(Some(Self::new(mtm, &identity, service)))
    }
    /// Production adapter receives the actual Tauri/wry WKWebView pointer.
    /// # Safety
    /// Same pointer/main-thread preconditions as StoreIdentity::from_app_webview.
    pub unsafe fn start_from_app_webview(
        mtm: MainThreadMarker,
        view: *mut std::ffi::c_void,
        service: CompletionService,
    ) -> Result<Option<Self>, &'static str> {
        Self::start(
            mtm,
            || unsafe { StoreIdentity::from_app_webview(view, mtm) },
            service,
        )
    }
    fn new(mtm: MainThreadMarker, identity: &StoreIdentity, service: CompletionService) -> Self {
        Self::with_seed(mtm, &identity.store, service, None)
    }
    /// Synthetic fixture support; seeds only a caller-supplied custom store.
    /// Refuses persistent stores, so fixture code cannot touch production data.
    pub fn synthetic(
        mtm: MainThreadMarker,
        store: &WKWebsiteDataStore,
        service: CompletionService,
        seed: BTreeMap<String, String>,
    ) -> Self {
        assert!(!unsafe { store.isPersistent() });
        Self::with_seed(mtm, store, service, Some(seed))
    }
    fn with_seed(
        mtm: MainThreadMarker,
        store: &WKWebsiteDataStore,
        service: CompletionService,
        seed: Option<BTreeMap<String, String>>,
    ) -> Self {
        let (send, receive) = mpsc::channel();
        let keys: Vec<_> = TRANSCRIPT_KEYS
            .iter()
            .chain(V2_KEYS)
            .chain(UPDATER_V3_KEYS)
            .copied()
            .collect();
        let seed = seed
            .map(|seed| {
                format!(
                    "for (const [k,v] of Object.entries({})) localStorage.setItem(k,v);",
                    serde_json::to_string(&seed).unwrap()
                )
            })
            .unwrap_or_default();
        let page = format!(
            r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'"><script>
        const post = x => window.webkit.messageHandlers.migration.postMessage(JSON.stringify(x));
        try {{ {seed} post(Object.fromEntries({keys}.map(k=>[k,localStorage.getItem(k)]))); }} catch (_) {{ post({{error:'storage_unreadable'}}); }}
        function removeImported(keys) {{ try {{ for(const k of keys) localStorage.removeItem(k); if(keys.some(k=>localStorage.getItem(k)!==null)) throw 0; post({{removed:keys}}); }} catch(_) {{ post({{error:'removal_failed'}}); }} }}
        </script>"#,
            keys = serde_json::to_string(&keys).unwrap()
        );
        let handler: Retained<Handler> = unsafe {
            msg_send![
                super(Handler::alloc(mtm).set_ivars(Ivars {
                    messages: RefCell::new(Some(send)),
                    page: RefCell::new(page),
                    failed: Rc::new(Cell::new(false))
                })),
                init
            ]
        };
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        unsafe {
            configuration.setWebsiteDataStore(store);
            configuration.setURLSchemeHandler_forURLScheme(
                Some(ProtocolObject::from_ref(&*handler)),
                &NSString::from_str("tauri"),
            );
            configuration
                .userContentController()
                .addScriptMessageHandler_name(
                    ProtocolObject::from_ref(&*handler),
                    &NSString::from_str("migration"),
                );
        }
        let view = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm),
                NSRect::ZERO,
                &configuration,
            )
        };
        let url = NSURL::URLWithString(&NSString::from_str("tauri://localhost/")).unwrap();
        unsafe {
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*handler)));
            assert!(
                std::ptr::eq(&*view.configuration().websiteDataStore(), store),
                "importer store identity mismatch"
            );
            view.loadRequest(&NSURLRequest::requestWithURL(&url));
        }
        Self {
            view,
            handler,
            receive,
            service,
            pending: None,
            removing: false,
            done: false,
            committed: false,
            failed: false,
            synthetic_failure: None,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    /// Callback/deadline fault seams, restricted to caller-owned synthetic stores.
    pub fn fail_after_import(&mut self, failure: SyntheticFailure) {
        assert!(!unsafe { self.view.configuration().websiteDataStore().isPersistent() });
        self.synthetic_failure = Some(failure);
    }
    /// Inspect the four exact keys independently of Rust receipts (synthetic only).
    pub fn inspect_transcript_keys(&self) -> mpsc::Receiver<bool> {
        assert!(!unsafe { self.view.configuration().websiteDataStore().isPersistent() });
        let (send, receive) = mpsc::channel();
        let js = format!(
            "JSON.stringify({}.map(k=>localStorage.getItem(k)))",
            serde_json::to_string(TRANSCRIPT_KEYS).unwrap()
        );
        let done = block2::RcBlock::new(
            move |value: *mut objc2::runtime::AnyObject, error: *mut NSError| {
                let absent = error.is_null()
                    && unsafe { value.as_ref() }
                        .and_then(|v| v.downcast_ref::<NSString>())
                        .is_some_and(|s| s.to_string() == "[null,null,null,null]");
                let _ = send.send(absent);
            },
        );
        unsafe {
            self.view
                .evaluateJavaScript_completionHandler(&NSString::from_str(&js), Some(&done));
        }
        receive
    }
    /// Freeze admission without fabricating import/removal acknowledgements.
    /// In-flight native work retains its service handle and may settle off main.
    pub fn preserve_for_shutdown(&mut self) {
        self.failed = true;
    }
    /// The main thread only handles bounded messages and JavaScript transport;
    /// all durable I/O stays on a worker and the completion actor.
    pub fn poll(&mut self) -> ImportStatus {
        if self.done {
            return ImportStatus::Done;
        }
        if self.failed || self.handler.ivars().failed.get() || Instant::now() >= self.deadline {
            self.failed = true;
            return ImportStatus::PreservedForRetry;
        }
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok(Ok((keys, committed))) => {
                    self.pending = None;
                    if self.removing {
                        self.done = committed;
                        self.failed = !committed;
                        return if committed {
                            ImportStatus::Done
                        } else {
                            ImportStatus::PreservedForRetry
                        };
                    }
                    self.committed = committed;
                    let mut js =
                        format!("removeImported({})", serde_json::to_string(&keys).unwrap());
                    if let Some(failure) = self.synthetic_failure.take() {
                        match failure {
                            SyntheticFailure::Javascript => {
                                js = "throw new Error('synthetic')".into()
                            }
                            SyntheticFailure::Navigation => {
                                let error = NSError::new(1, &NSString::from_str("synthetic"));
                                unsafe {
                                    let _: () = msg_send![&*self.handler, webView: &*self.view, didFailNavigation: Option::<&WKNavigation>::None, withError: &*error];
                                }
                                return self.poll();
                            }
                            SyntheticFailure::ProcessTermination => {
                                unsafe {
                                    let _: () = msg_send![&*self.handler, webViewWebContentProcessDidTerminate: &*self.view];
                                }
                                return self.poll();
                            }
                            SyntheticFailure::Deadline => {
                                self.deadline = Instant::now();
                                return self.poll();
                            }
                        }
                    }
                    let failed = self.handler.ivars().failed.clone();
                    let completion = block2::RcBlock::new(
                        move |_value: *mut objc2::runtime::AnyObject, error: *mut NSError| {
                            if !error.is_null() {
                                failed.set(true);
                            }
                        },
                    );
                    unsafe {
                        self.view.evaluateJavaScript_completionHandler(
                            &NSString::from_str(&js),
                            Some(&completion),
                        );
                    }
                    self.removing = true;
                }
                Ok(Err(())) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.failed = true;
                    return ImportStatus::PreservedForRetry;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_none() {
            if let Ok(raw) = self.receive.try_recv() {
                if raw.len() > 32 * 1024 * 1024 {
                    self.failed = true;
                    return ImportStatus::PreservedForRetry;
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                    self.failed = true;
                    return ImportStatus::PreservedForRetry;
                };

                let service = self.service.clone();
                let removing = self.removing;
                let committed = self.committed;
                let (send, receive) = mpsc::channel();
                self.pending = Some(receive);
                std::thread::spawn(move || {
                    let result = (|| {
                        if removing {
                            let keys = value["removed"].as_array().ok_or(())?;
                            let mut destinations = vec![];
                            for (d, required) in [
                                (Destination::History, &TRANSCRIPT_KEYS[0..1]),
                                (Destination::Statistics, &TRANSCRIPT_KEYS[1..2]),
                                (Destination::Reports, &TRANSCRIPT_KEYS[2..4]),
                            ] {
                                if required
                                    .iter()
                                    .all(|k| keys.iter().any(|v| v.as_str() == Some(k)))
                                {
                                    destinations.push(d);
                                }
                            }
                            service
                                .execute(Command::KeysRemoved(destinations))
                                .map_err(|_| ())?;
                            Ok((vec![], committed))
                        } else {
                            let source = if value.get("error").is_none() {
                                serde_json::from_value(value).ok()
                            } else {
                                None
                            };
                            // A failed partial import may still expose durable
                            // destination receipts on the next launch. No broad purge.
                            let imported = source.is_some_and(|source| {
                                service.lifecycle(Operation::Import(source)).is_ok()
                            });
                            let mut keys = service
                                .execute(Command::Snapshot)
                                .map_err(|_| ())?
                                .remove_keys;
                            if service.lifecycle(Operation::Status).is_ok_and(|r| r.v3) {
                                keys.extend(UPDATER_V3_KEYS.iter().map(|k| (*k).to_owned()));
                            }
                            Ok((keys, imported))
                        }
                    })();
                    let _ = send.send(result);
                });
            }
        }
        ImportStatus::Pending
    }
}
impl Drop for Importer {
    fn drop(&mut self) {
        unsafe {
            self.view.setNavigationDelegate(None);
            self.view.stopLoading();
            self.view
                .configuration()
                .userContentController()
                .removeScriptMessageHandlerForName(&NSString::from_str("migration"));
        }
        self.handler.ivars().messages.borrow_mut().take();
    }
}

/// Retirement has its own durable prepare/finish receipts. The host must drop
/// the importer and all React views before constructing this coordinator.
pub struct Retirement {
    store: Option<Retained<WKWebsiteDataStore>>,
    directory: std::path::PathBuf,
    service: CompletionService,
    result: mpsc::Receiver<bool>,
    phase: u8,
    synthetic_failure: Option<RetirementFailure>,
    deadline: Instant,
    failed: bool,
}
impl Retirement {
    /// # Safety
    /// No webview or React process may use `store` or `directory` for the entire
    /// operation. `directory` must belong exclusively to this bundle's store.
    /// Never pass production paths in a test. The v3 receipt is mandatory; 1.5
    /// cannot retire a store because #990 has not shipped that receipt yet.
    pub unsafe fn new(
        identity: StoreIdentity,
        directory: std::path::PathBuf,
        service: CompletionService,
    ) -> Self {
        let (send, result) = mpsc::channel();
        let worker = service.clone();
        std::thread::spawn(move || {
            let _ = send.send(worker.lifecycle(Operation::BeginRetirement).is_ok());
        });
        Self {
            store: Some(identity.store),
            directory,
            service,
            result,
            phase: 0,
            synthetic_failure: None,
            deadline: Instant::now() + Duration::from_secs(30),
            failed: false,
        }
    }
    pub fn inject_failure(&mut self, failure: RetirementFailure) {
        assert!(!unsafe { self.store.as_ref().unwrap().isPersistent() });
        self.synthetic_failure = Some(failure);
    }
    pub fn poll(&mut self, mtm: MainThreadMarker) -> ImportStatus {
        if self.phase == 3 {
            return ImportStatus::Done;
        }
        if self.failed || Instant::now() >= self.deadline {
            self.failed = true;
            return ImportStatus::PreservedForRetry;
        }
        match self.result.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return ImportStatus::Pending,
            Ok(true) => {}
            _ => {
                self.failed = true;
                return ImportStatus::PreservedForRetry;
            }
        }
        if self.phase == 4 {
            self.phase = 3;
            return ImportStatus::Done;
        }
        let (send, result) = mpsc::channel();
        self.result = result;
        match self.phase {
            0 => {
                if self.synthetic_failure == Some(RetirementFailure::Purge) {
                    self.failed = true;
                    return ImportStatus::PreservedForRetry;
                }
                let done = block2::RcBlock::new(move || {
                    let _ = send.send(true);
                });
                unsafe {
                    self.store
                        .as_ref()
                        .unwrap()
                        .removeDataOfTypes_modifiedSince_completionHandler(
                            &WKWebsiteDataStore::allWebsiteDataTypes(mtm),
                            &objc2_foundation::NSDate::distantPast(),
                            &done,
                        );
                }
            }
            1 => {
                let fail_verification =
                    self.synthetic_failure == Some(RetirementFailure::Verification);
                let done = block2::RcBlock::new(
                    move |records: std::ptr::NonNull<
                        objc2_foundation::NSArray<WKWebsiteDataRecord>,
                    >| {
                        let empty = unsafe { records.as_ref() }.count() == 0 && !fail_verification;
                        let _ = send.send(empty);
                    },
                );
                unsafe {
                    self.store
                        .as_ref()
                        .unwrap()
                        .fetchDataRecordsOfTypes_completionHandler(
                            &WKWebsiteDataStore::allWebsiteDataTypes(mtm),
                            &done,
                        );
                }
            }
            2 => {
                self.store.take();
                let directory = self.directory.clone();
                let failure = self.synthetic_failure;
                let service = self.service.clone();
                std::thread::spawn(move || {
                    let done = (|| {
                        match std::fs::symlink_metadata(&directory) {
                            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
                                if failure == Some(RetirementFailure::DirectoryRemoval) {
                                    if let Some(entry) =
                                        std::fs::read_dir(&directory).map_err(|_| ())?.next()
                                    {
                                        std::fs::remove_file(entry.map_err(|_| ())?.path())
                                            .map_err(|_| ())?;
                                    }
                                    return Err(());
                                }
                                std::fs::remove_dir_all(&directory).map_err(|_| ())?
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            _ => return Err(()),
                        }
                        if failure == Some(RetirementFailure::ParentSync) {
                            return Err(());
                        }
                        std::fs::File::open(directory.parent().ok_or(())?)
                            .and_then(|f| f.sync_all())
                            .map_err(|_| ())?;
                        service.lifecycle(Operation::Retired).map_err(|_| ())?;
                        Ok::<_, ()>(())
                    })();
                    let _ = send.send(done.is_ok());
                });
                // Phase 3 waits for the directory and completion marker.
            }
            _ => {}
        }
        self.phase += 1;
        if self.phase == 3 {
            // Retain a final waiting phase; never report Done before the worker.
            self.phase = 4;
        } else if self.phase == 5 {
            self.phase = 3;
            return ImportStatus::Done;
        }
        ImportStatus::Pending
    }
}
