//! Native service capabilities. No Tauri runtime, WebKit, or erased managed state.
use super::ServiceAccess;
use murmur_core::context::AppContext;

use std::{
    marker::PhantomData,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
pub trait Runtime: Clone + Send + Sync + 'static {}
#[derive(Clone)]
pub struct DefaultRuntime;
impl Runtime for DefaultRuntime {}
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<String> for Error {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl From<&str> for Error {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}
impl From<Error> for String {
    fn from(value: Error) -> Self {
        value.0
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub struct StateRef<'a, T>(&'a T);
impl<'a, T> StateRef<'a, T> {
    pub fn inner(&self) -> &'a T {
        self.0
    }
}
impl<T> std::ops::Deref for StateRef<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}
#[derive(Clone)]
pub struct NativeConfig {
    pub identifier: String,
}
pub struct PackageInfo {
    pub version: String,
}
/// Isolated QA identity, the only one that may report an overridden version.
const QA_IDENTIFIER: &str = "com.localdictation.gpui-host-qa110";
/// Build-time version for a QA binary (`MURMUR_QA_PACKAGE_VERSION=0.62.2 cargo
/// build --features native-host-qa`). The updater compares the running version
/// with the bundle's plist, so an update fixture needs a binary whose reported
/// version equals its plist version. Compiled out of every other build.
#[cfg(feature = "native-host-qa")]
const QA_PACKAGE_VERSION: Option<&str> = option_env!("MURMUR_QA_PACKAGE_VERSION");
#[cfg(not(feature = "native-host-qa"))]
const QA_PACKAGE_VERSION: Option<&str> = None;
/// The version the updater treats as running. The override applies only to a
/// `native-host-qa` build running as the QA identity; every other identity, and
/// every build without the feature, reports the package version.
fn reported_version(identifier: &str, qa_override: Option<&str>) -> String {
    match qa_override {
        Some(version) if cfg!(feature = "native-host-qa") && identifier == QA_IDENTIFIER => {
            version.into()
        }
        _ => env!("CARGO_PKG_VERSION").into(),
    }
}
#[cfg(test)]
mod reported_version_tests {
    use super::{reported_version, QA_IDENTIFIER};
    #[test]
    fn production_and_other_identities_ignore_the_qa_override() {
        let package = env!("CARGO_PKG_VERSION");
        for identifier in ["com.localdictation", "com.localdictation.shutdown-test", ""] {
            assert_eq!(reported_version(identifier, Some("9.9.9")), package);
        }
        assert_eq!(reported_version(QA_IDENTIFIER, None), package);
    }
    #[cfg(feature = "native-host-qa")]
    #[test]
    fn the_qa_identity_reports_the_override() {
        assert_eq!(reported_version(QA_IDENTIFIER, Some("9.9.9")), "9.9.9");
    }
    #[cfg(not(feature = "native-host-qa"))]
    #[test]
    fn a_build_without_the_qa_feature_ignores_the_override_even_for_the_qa_identity() {
        assert_eq!(
            reported_version(QA_IDENTIFIER, Some("9.9.9")),
            env!("CARGO_PKG_VERSION")
        );
        assert!(super::QA_PACKAGE_VERSION.is_none());
    }
}
#[derive(Clone)]
pub struct NativePaths {
    pub home: PathBuf,
    pub app: PathBuf,
    pub resources: PathBuf,
    pub cache: PathBuf,
}
impl NativePaths {
    pub fn home_dir(&self) -> Result<PathBuf> {
        Ok(self.home.clone())
    }
    pub fn app_data_dir(&self) -> Result<PathBuf> {
        Ok(self.app.clone())
    }
    pub fn resource_dir(&self) -> Result<PathBuf> {
        Ok(self.resources.clone())
    }
    pub fn app_cache_dir(&self) -> Result<PathBuf> {
        Ok(self.cache.clone())
    }
}
pub trait NativePorts: Send + Sync + 'static {
    fn window(&self, label: &str) -> Option<Arc<dyn WindowPort>>;
    /// All attached displays, in physical top-left desktop coordinates.
    fn monitors(&self) -> Vec<Monitor>;
    /// System primary display, independent of the active/Main window screen.
    fn primary_monitor(&self) -> Option<Monitor>;
    fn send(&self, label: &str, event: &str, payload: &str) -> Result<()>;
    /// Off-main drain of native presentation-owned workers (for example playback).
    fn quiesce(&self, _deadline: std::time::Instant) -> bool {
        true
    }
    fn terminal(&self, action: murmur_core::shutdown::Action);
    fn notify_update(&self, version: &str);
    fn dock_context(&self) -> Option<(crate::dock_watch::PillDisplay, bool)>;
    fn dock_changed(&self);
    fn mode_status(&self, status: serde_json::Value);
    fn open_path(&self, path: PathBuf) -> Result<()>;
    fn open_url(&self, _url: String, _lease: crate::native_services::EffectLease) -> Result<()> {
        Err(Error("Native opener unavailable".into()))
    }
    fn tray_update(&self, version: Option<String>) -> Result<()>;
}
pub trait WindowPort: Send + Sync + 'static {
    fn label(&self) -> &str;
    fn snapshot(&self) -> WindowSnapshot;
    fn command(&self, command: WindowCommand) -> Result<()>;
    /// Caller must be on main; the port validates retained window identity.
    fn ns_window(&self) -> Result<*mut std::ffi::c_void>;
}
#[derive(Clone)]
pub struct WindowSnapshot {
    pub visible: bool,
    pub minimized: bool,
    pub fullscreen: bool,
    pub maximized: bool,
    /// Content geometry, distinct from the titlebar-inclusive frame.
    pub size: dpi::PhysicalSize<u32>,
    pub inner_position: dpi::PhysicalPosition<i32>,
    pub outer_size: dpi::PhysicalSize<u32>,
    pub position: dpi::PhysicalPosition<i32>,
    pub scale: f64,
    pub monitor: Option<Monitor>,
}
pub enum WindowCommand {
    Show,
    Hide,
    Focus,
    Destroy,
    Minimize,
    Unminimize,
    Fullscreen(bool),
    Focusable(bool),
    Maximize,
    Zoom(f64),
    Size(dpi::Size),
    Position(dpi::Position),
    IgnoreCursor(bool),
    Background(Option<window::Color>),
    Center,
}
#[derive(Clone)]
pub struct Monitor {
    pub work: WorkArea,
    pub name: Option<String>,
    pub size: dpi::PhysicalSize<u32>,
    pub position: dpi::PhysicalPosition<i32>,
    pub scale: f64,
}
#[derive(Clone)]
pub struct WorkArea {
    pub position: dpi::PhysicalPosition<i32>,
    pub size: dpi::PhysicalSize<u32>,
}
impl Monitor {
    pub fn work_area(&self) -> &WorkArea {
        &self.work
    }
    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }
    pub fn size(&self) -> &dpi::PhysicalSize<u32> {
        &self.size
    }
    pub fn position(&self) -> &dpi::PhysicalPosition<i32> {
        &self.position
    }
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }
}
#[derive(Clone)]
pub struct Window<R: Runtime = DefaultRuntime> {
    port: Arc<dyn WindowPort>,
    app: Context<R>,
}
impl<R: Runtime> Window<R> {
    pub fn label(&self) -> &str {
        self.port.label()
    }
    pub fn app_handle(&self) -> &Context<R> {
        &self.app
    }
    pub fn ns_window(&self) -> Result<*mut std::ffi::c_void> {
        self.port.ns_window()
    }
    pub fn is_visible(&self) -> Result<bool> {
        Ok(self.port.snapshot().visible)
    }
    pub fn is_minimized(&self) -> Result<bool> {
        Ok(self.port.snapshot().minimized)
    }
    pub fn is_maximized(&self) -> Result<bool> {
        Ok(self.port.snapshot().maximized)
    }
    pub fn maximize(&self) -> Result<()> {
        self.port.command(WindowCommand::Maximize)
    }
    pub fn set_focusable(&self, value: bool) -> Result<()> {
        self.port.command(WindowCommand::Focusable(value))
    }
    pub fn set_zoom(&self, value: f64) -> Result<()> {
        self.port.command(WindowCommand::Zoom(value))
    }
    pub fn is_fullscreen(&self) -> Result<bool> {
        Ok(self.port.snapshot().fullscreen)
    }
    pub fn inner_size(&self) -> Result<dpi::PhysicalSize<u32>> {
        Ok(self.port.snapshot().size)
    }
    pub fn outer_size(&self) -> Result<dpi::PhysicalSize<u32>> {
        Ok(self.port.snapshot().outer_size)
    }
    pub fn outer_position(&self) -> Result<dpi::PhysicalPosition<i32>> {
        Ok(self.port.snapshot().position)
    }
    pub fn inner_position(&self) -> Result<dpi::PhysicalPosition<i32>> {
        Ok(self.port.snapshot().inner_position)
    }
    pub fn scale_factor(&self) -> Result<f64> {
        Ok(self.port.snapshot().scale)
    }
    pub fn current_monitor(&self) -> Result<Option<Monitor>> {
        Ok(self.port.snapshot().monitor)
    }
    pub fn primary_monitor(&self) -> Result<Option<Monitor>> {
        self.app.primary_monitor()
    }
    pub fn available_monitors(&self) -> Result<Vec<Monitor>> {
        self.app.available_monitors()
    }
    pub fn show(&self) -> Result<()> {
        self.port.command(WindowCommand::Show)
    }
    pub fn hide(&self) -> Result<()> {
        self.port.command(WindowCommand::Hide)
    }
    pub fn set_focus(&self) -> Result<()> {
        self.port.command(WindowCommand::Focus)
    }
    pub fn destroy(&self) -> Result<()> {
        self.port.command(WindowCommand::Destroy)
    }
    pub fn minimize(&self) -> Result<()> {
        self.port.command(WindowCommand::Minimize)
    }
    pub fn unminimize(&self) -> Result<()> {
        self.port.command(WindowCommand::Unminimize)
    }
    pub fn set_fullscreen(&self, v: bool) -> Result<()> {
        self.port.command(WindowCommand::Fullscreen(v))
    }
    pub fn set_size(&self, v: impl Into<dpi::Size>) -> Result<()> {
        self.port.command(WindowCommand::Size(v.into()))
    }
    pub fn set_position(&self, v: impl Into<dpi::Position>) -> Result<()> {
        self.port.command(WindowCommand::Position(v.into()))
    }
    pub fn set_ignore_cursor_events(&self, v: bool) -> Result<()> {
        self.port.command(WindowCommand::IgnoreCursor(v))
    }
    pub fn center(&self) -> Result<()> {
        self.port.command(WindowCommand::Center)
    }
    pub fn set_background_color(&self, v: Option<window::Color>) -> Result<()> {
        self.port.command(WindowCommand::Background(v))
    }
    /// Web content evaluation is unavailable in a native host. Migration must use
    /// the explicit standalone importer, never a hidden presentation view.
    pub fn eval(&self, _script: &str) -> Result<()> {
        Err("Web content evaluation unavailable in native window".into())
    }
}
pub struct Context<R: Runtime = DefaultRuntime> {
    inner: Arc<Owners>,
    marker: PhantomData<fn() -> R>,
}
impl<R: Runtime> Clone for Context<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            marker: PhantomData,
        }
    }
}
struct Owners {
    services: crate::State,
    context: AppContext,
    control: OnceLock<crate::dictation_control::Control>,
    event_owner: OnceLock<crate::events::Events>,
    completion_owner: OnceLock<crate::completion::Completion>,
    completion_initialization: OnceLock<crate::completion::CompletionInitialization>,
    window_state_owner: OnceLock<crate::window_state::WindowState>,
    shutdown_owner: OnceLock<crate::shutdown::Shutdown>,
    updater_owner: OnceLock<Arc<murmur_core::updater::Updater>>,
    paths: NativePaths,
    config: NativeConfig,
    ports: Arc<dyn NativePorts>,
}
impl Context {
    pub fn new(
        context: AppContext,
        paths: NativePaths,
        identifier: String,
        ports: Arc<dyn NativePorts>,
    ) -> Self {
        crate::commands::tray::install(ports.clone());
        crate::native_services::install_ports(ports.clone());
        Self {
            inner: Arc::new(Owners {
                services: crate::State::default(),
                context,
                control: OnceLock::new(),
                event_owner: OnceLock::new(),
                completion_owner: OnceLock::new(),
                completion_initialization: OnceLock::new(),
                window_state_owner: OnceLock::new(),
                shutdown_owner: OnceLock::new(),
                updater_owner: OnceLock::new(),
                paths,
                config: NativeConfig { identifier },
                ports,
            }),
            marker: PhantomData,
        }
    }
}
impl<R: Runtime> Context<R> {
    pub fn path(&self) -> &NativePaths {
        &self.inner.paths
    }
    pub fn config(&self) -> &NativeConfig {
        &self.inner.config
    }
    pub fn package_info(&self) -> PackageInfo {
        PackageInfo {
            version: reported_version(&self.inner.config.identifier, QA_PACKAGE_VERSION),
        }
    }
    pub fn get_window(&self, label: &str) -> Option<Window<R>> {
        self.inner.ports.window(label).map(|port| Window {
            port,
            app: self.clone(),
        })
    }
    pub fn run_on_main_thread(&self, work: impl FnOnce() + Send + 'static) -> Result<()> {
        self.inner
            .context
            .main_thread
            .dispatch(Box::new(work))
            .map_err(|_| "Main-thread dispatch unavailable".into())
    }
    pub fn primary_monitor(&self) -> Result<Option<Monitor>> {
        Ok(self.inner.ports.primary_monitor())
    }
    pub fn available_monitors(&self) -> Result<Vec<Monitor>> {
        Ok(self.inner.ports.monitors())
    }
    pub fn dock_context(&self) -> Option<(crate::dock_watch::PillDisplay, bool)> {
        self.inner.ports.dock_context()
    }
    pub fn dock_changed(&self) {
        self.inner.ports.dock_changed()
    }
    pub fn notify_update(&self, version: &str) {
        self.inner.ports.notify_update(version)
    }
    pub fn open_path(&self, path: PathBuf) -> Result<()> {
        self.inner.ports.open_path(path)
    }
    pub fn quiesce_native(&self, deadline: std::time::Instant) -> bool {
        self.inner.ports.quiesce(deadline)
    }
    pub fn terminal(&self, action: murmur_core::shutdown::Action) {
        self.inner.ports.terminal(action)
    }
    pub fn send(&self, label: &str, event: &str, payload: &str) -> Result<()> {
        self.inner.ports.send(label, event, payload)
    }
}
pub mod window {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Color(pub u8, pub u8, pub u8, pub u8);
    impl std::str::FromStr for Color {
        type Err = String;
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let s = s.strip_prefix('#').ok_or("Invalid color")?;
            if s.len() != 6 && s.len() != 8 {
                return Err("Invalid color".into());
            }
            let c = u32::from_str_radix(s, 16).map_err(|_| "Invalid color")?;
            Ok(if s.len() == 6 {
                Self((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
            } else {
                Self((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8)
            })
        }
    }
}
pub mod runtime {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();
    /// The executable owns the runtime until coordinated termination completes.
    pub fn install(handle: tokio::runtime::Handle) -> Result<(), tokio::runtime::Handle> {
        RUNTIME.set(handle)
    }
    fn handle() -> &'static tokio::runtime::Handle {
        // Unit tests exercise shared async owners without launching an app.
        // Production still requires the executable-owned runtime explicitly.
        #[cfg(test)]
        {
            static TEST_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
            RUNTIME.get().unwrap_or_else(|| {
                TEST_RUNTIME
                    .get_or_init(|| tokio::runtime::Runtime::new().expect("test runtime"))
                    .handle()
            })
        }
        #[cfg(not(test))]
        RUNTIME.get().expect("host runtime must be installed")
    }
    pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        handle().spawn(future)
    }
    pub fn spawn_blocking<F, T>(work: F) -> tokio::task::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        handle().spawn_blocking(work)
    }
    pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
        handle().block_on(future)
    }
}

impl<R: Runtime> ServiceAccess<R> for Context<R> {
    fn services(&self) -> StateRef<'_, crate::State> {
        StateRef(&self.inner.services)
    }
    fn try_services(&self) -> Option<StateRef<'_, crate::State>> {
        Some(self.services())
    }
    fn context(&self) -> StateRef<'_, murmur_core::context::AppContext> {
        StateRef(&self.inner.context)
    }
    fn control(&self) -> StateRef<'_, crate::dictation_control::Control> {
        StateRef(
            self.inner
                .control
                .get()
                .expect("Service owner not initialized"),
        )
    }
    fn try_control(&self) -> Option<StateRef<'_, crate::dictation_control::Control>> {
        self.inner.control.get().map(StateRef)
    }
    fn event_owner(&self) -> StateRef<'_, crate::events::Events> {
        StateRef(
            self.inner
                .event_owner
                .get()
                .expect("Service owner not initialized"),
        )
    }
    fn try_event_owner(&self) -> Option<StateRef<'_, crate::events::Events>> {
        self.inner.event_owner.get().map(StateRef)
    }
    fn completion_owner(&self) -> StateRef<'_, crate::completion::Completion> {
        StateRef(
            self.inner
                .completion_owner
                .get()
                .expect("Service owner not initialized"),
        )
    }
    fn try_completion_owner(&self) -> Option<StateRef<'_, crate::completion::Completion>> {
        self.inner.completion_owner.get().map(StateRef)
    }
    fn completion_initialization(
        &self,
    ) -> StateRef<'_, crate::completion::CompletionInitialization> {
        StateRef(
            self.inner
                .completion_initialization
                .get()
                .expect("Service owner not initialized"),
        )
    }
    fn try_completion_initialization(
        &self,
    ) -> Option<StateRef<'_, crate::completion::CompletionInitialization>> {
        self.inner.completion_initialization.get().map(StateRef)
    }
    fn window_state_owner(&self) -> StateRef<'_, crate::window_state::WindowState> {
        StateRef(
            self.inner
                .window_state_owner
                .get()
                .expect("Service owner not initialized"),
        )
    }
    fn try_window_state_owner(&self) -> Option<StateRef<'_, crate::window_state::WindowState>> {
        self.inner.window_state_owner.get().map(StateRef)
    }
    fn try_shutdown_owner(&self) -> Option<StateRef<'_, crate::shutdown::Shutdown>> {
        self.inner.shutdown_owner.get().map(StateRef)
    }
    fn try_updater_owner(
        &self,
    ) -> Option<StateRef<'_, std::sync::Arc<murmur_core::updater::Updater>>> {
        self.inner.updater_owner.get().map(StateRef)
    }
    fn install_control(&self, owner: crate::dictation_control::Control) -> bool {
        self.inner.control.set(owner).is_ok()
    }
    fn install_event_owner(&self, owner: crate::events::Events) -> bool {
        self.inner.event_owner.set(owner).is_ok()
    }
    fn install_completion_owner(&self, owner: crate::completion::Completion) -> bool {
        self.inner.completion_owner.set(owner).is_ok()
    }
    fn install_completion_initialization(
        &self,
        owner: crate::completion::CompletionInitialization,
    ) -> bool {
        self.inner.completion_initialization.set(owner).is_ok()
    }
    fn install_window_state_owner(&self, owner: crate::window_state::WindowState) -> bool {
        self.inner.window_state_owner.set(owner).is_ok()
    }
    fn install_shutdown_owner(&self, owner: crate::shutdown::Shutdown) -> bool {
        self.inner.shutdown_owner.set(owner).is_ok()
    }
    fn install_updater_owner(&self, owner: std::sync::Arc<murmur_core::updater::Updater>) -> bool {
        self.inner.updater_owner.set(owner).is_ok()
    }
}

pub type HostWindow = Window;
pub enum WindowEvent {
    Moved(dpi::PhysicalPosition<i32>),
    Resized(dpi::PhysicalSize<u32>),
    CloseRequested {},
    ScaleFactorChanged { scale_factor: f64 },
    Destroyed,
    Focused(bool),
}

pub struct BinaryResponse(pub Vec<u8>);
impl BinaryResponse {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}
