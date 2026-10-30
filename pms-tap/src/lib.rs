//! PimpMyShell TAP ("Tool/Test Attach Point"): a COM object that explorer.exe instantiates
//! when we call `InitializeXamlDiagnosticsEx` on it. XAML hands us an `IXamlDiagnostics`
//! site, from which we subscribe to the visual tree of every XAML island in the process.
//!
//! We mirror the tree (dumped to `%LOCALAPPDATA%\PimpMyShell\tree.txt` on change) and serve
//! commands from the `pms` CLI over a named pipe (see `commands`).

#![allow(non_snake_case)]

mod alert;
mod backdrop;
mod commands;
mod dwm;
mod glow;
mod layout;
mod meters;
mod pace;
pub mod ipc;
pub mod logfile;
mod props;
mod rings;
mod ripple;
mod server;
mod stage;
mod toast;
mod tooltip;
pub mod vendors;
pub mod xamlom;

use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, Once, OnceLock, mpsc};
use std::time::Duration;

use windows::UI::Core::{CoreDispatcher, CoreDispatcherPriority, DispatchedHandler};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, S_FALSE, S_OK};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl};
use windows_core::{BOOL, GUID, HRESULT, IUnknown, IUnknownImpl, Interface, PCWSTR, Ref, implement};

use xamlom::*;

pub const CLSID_PMS_TAP: GUID = GUID::from_u128(0x6755c7be_20d5_4200_a265_be2b78bc0c45);

// ---------------------------------------------------------------------------------------------
// Logging

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("PimpMyShell")
}

fn log(msg: impl AsRef<str>) {
    logfile::append(logfile::Log::Tap, &format!("[pid {}] {}", std::process::id(), msg.as_ref()));
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic here would abort explorer; never let poisoning escalate.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------
// Mirrored visual tree

pub(crate) struct Node {
    pub ty: String,
    pub name: String,
    pub parent: InstanceHandle,
    pub index: u32,
}

pub(crate) static TREE: LazyLock<Mutex<HashMap<InstanceHandle, Node>>> = LazyLock::new(Default::default);
static DIRTY: AtomicBool = AtomicBool::new(false);

fn children_map(tree: &HashMap<InstanceHandle, Node>) -> HashMap<InstanceHandle, Vec<(u32, InstanceHandle)>> {
    let mut children: HashMap<InstanceHandle, Vec<(u32, InstanceHandle)>> = HashMap::new();
    for (&h, n) in tree.iter() {
        children.entry(n.parent).or_default().push((n.index, h));
    }
    for v in children.values_mut() {
        v.sort();
    }
    children
}

/// Every element reachable from a root, depth-first in child order.
pub(crate) fn dfs_order(tree: &HashMap<InstanceHandle, Node>) -> Vec<InstanceHandle> {
    let children = children_map(tree);
    let mut out = Vec::with_capacity(tree.len());
    let mut stack: Vec<InstanceHandle> = children.get(&0).into_iter().flatten().rev().map(|&(_, h)| h).collect();
    while let Some(h) = stack.pop() {
        out.push(h);
        stack.extend(children.get(&h).into_iter().flatten().rev().map(|&(_, c)| c));
    }
    out
}

pub(crate) fn render_tree(root: Option<InstanceHandle>) -> String {
    let tree = lock(&TREE);
    let children = children_map(&tree);
    let orphans = tree.values().filter(|n| n.parent != 0 && !tree.contains_key(&n.parent)).count();

    fn walk(
        out: &mut String,
        tree: &HashMap<InstanceHandle, Node>,
        children: &HashMap<InstanceHandle, Vec<(u32, InstanceHandle)>>,
        h: InstanceHandle,
        depth: usize,
    ) {
        let Some(n) = tree.get(&h) else { return };
        let _ = write!(out, "{:indent$}{}", "", n.ty, indent = depth * 2);
        if !n.name.is_empty() {
            let _ = write!(out, " #{}", n.name);
        }
        let _ = writeln!(out, "  [{:#x}]", h);
        for &(_, c) in children.get(&h).map(Vec::as_slice).unwrap_or_default() {
            walk(out, tree, children, c, depth + 1);
        }
    }

    let mut out = String::new();
    match root {
        Some(h) => walk(&mut out, &tree, &children, h, 0),
        None => {
            let _ = writeln!(out, "{} elements, {} orphans\n", tree.len(), orphans);
            for &(_, root) in children.get(&0).map(Vec::as_slice).unwrap_or_default() {
                walk(&mut out, &tree, &children, root, 0);
                out.push('\n');
            }
        }
    }
    out
}

fn start_tree_writer() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(|| {
            while !DETACHED.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(500));
                if DIRTY.swap(false, Ordering::AcqRel) {
                    let _ = std::fs::write(data_dir().join("tree.txt"), render_tree(None));
                }
            }
        });
    });
}

// ---------------------------------------------------------------------------------------------
// Live context, shared with the command server

/// COM pointers from windows-rs `#[interface]` aren't `Send`; these are free-threaded in practice.
struct SendPtr<T>(T);
unsafe impl<T> Send for SendPtr<T> {}
impl<T> SendPtr<T> {
    /// Method call so closures capture the whole wrapper, not its (non-Send) fields.
    fn get(self) -> T {
        self.0
    }
}

pub(crate) struct Ctx {
    pub service: IVisualTreeService3,
    #[allow(dead_code)]
    pub diag: IXamlDiagnostics,
    callback: IVisualTreeServiceCallback2,
    pub(crate) dispatcher: CoreDispatcher,
}
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}

pub(crate) static CTX: OnceLock<Ctx> = OnceLock::new();
pub(crate) static DETACHED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Set while a dispatched job runs, so nested run_on_ui calls (a `batch`) run inline instead
    /// of waiting on themselves.
    static ON_UI: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` on the XAML UI thread and wait for its result.
pub(crate) fn run_on_ui<R: Send + 'static>(f: impl FnOnce(&Ctx) -> R + Send + 'static) -> Result<R, String> {
    let ctx = CTX.get().ok_or("TAP not initialised yet")?;
    if ON_UI.get() {
        return std::panic::catch_unwind(AssertUnwindSafe(|| f(ctx))).map_err(|_| "panicked on UI thread".to_string());
    }
    let (tx, rx) = mpsc::channel::<Result<R, String>>();
    let job = Mutex::new(Some((f, tx)));
    let handler = DispatchedHandler::new(move || {
        if let (Some((f, tx)), Some(ctx)) = (lock(&job).take(), CTX.get()) {
            // A panic unwinding into XAML would take explorer down with it.
            ON_UI.set(true);
            let r = std::panic::catch_unwind(AssertUnwindSafe(|| f(ctx))).map_err(|_| "panicked on UI thread".to_string());
            ON_UI.set(false);
            let _ = tx.send(r);
        }
        Ok(())
    });
    ctx.dispatcher.RunAsync(CoreDispatcherPriority::Normal, &handler).map_err(|e| format!("RunAsync: {e}"))?;
    rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "UI thread didn't run the command within 5s".to_string())?
}

/// Identifies this loaded copy, so a client can tell a re-injected TAP from the one it configured.
pub(crate) static TAP_ID: LazyLock<String> = LazyLock::new(|| {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{}-{:x}", std::process::id(), t.as_millis())
});

/// Take down everything we put in the taskbar, best effort: after a taskbar rebuild the old
/// elements are already gone and individual steps fail, but all our references get dropped.
/// UI thread only.
pub(crate) fn reset(ctx: &Ctx) -> String {
    layout::stop_tracking();
    let steps = [
        ("meters", meters::command(ctx, &["clear".to_string()])),
        ("toast", toast::off()),
        ("tooltips", { tooltip::off(); Ok("off".into()) }),
        ("alert", alert::off(ctx)),
        ("ripple", ripple::off(ctx).map(|()| "ripple off".into())),
        ("buttonglow", glow::off(ctx)),
        ("backdrop", backdrop::disable(ctx)),
    ];
    stage::forget();
    steps.into_iter().map(|(what, r)| format!("{what}: {}", r.unwrap_or_else(|e| format!("error: {e}")))).collect::<Vec<_>>().join("; ")
}

pub(crate) fn detach() -> Result<String, String> {
    let ctx = CTX.get().ok_or("TAP not initialised yet")?;
    if DETACHED.load(Ordering::Acquire) {
        return Ok("already detached".into());
    }
    // Undo what lives on the UI thread while our tree mirror still resolves names.
    let teardown = run_on_ui(reset);
    DETACHED.store(true, Ordering::Release);
    log(format!("detach teardown: {teardown:?}"));
    let hr = unsafe { ctx.service.UnadviseVisualTreeChange(ctx.callback.as_raw()) };
    lock(&TREE).clear();
    log(format!("detached (Unadvise -> {hr:?})"));
    Ok(format!("detached (Unadvise -> {hr:?}); this TAP copy stays loaded but inert"))
}

// ---------------------------------------------------------------------------------------------
// The TAP object

#[implement(IObjectWithSite, IVisualTreeServiceCallback2)]
struct Tap {
    site: Mutex<Option<IUnknown>>,
}

impl IObjectWithSite_Impl for Tap_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> windows_core::Result<()> {
        let Some(site) = site.as_ref() else {
            log("SetSite(null)");
            *lock(&self.site) = None;
            return Ok(());
        };
        log("SetSite: got IXamlDiagnostics site");
        let service: IVisualTreeService3 = site.cast()?;
        let diag: IXamlDiagnostics = site.cast()?;
        *lock(&self.site) = Some(site.clone());
        start_tree_writer();

        // AdviseVisualTreeChange synchronously replays the whole existing tree through our
        // callback on the UI thread; calling it from SetSite (also on the UI thread) deadlocks.
        let callback: IVisualTreeServiceCallback2 = self.to_interface();
        let args = SendPtr((service, callback, diag));
        std::thread::spawn(move || {
            let (service, callback, diag) = args.get();
            let hr = unsafe { service.AdviseVisualTreeChange(callback.as_raw()) };
            log(format!("AdviseVisualTreeChange -> {hr:?}"));
            let dispatcher = (|| {
                let mut d = None;
                unsafe { diag.GetDispatcher(&mut d).ok()? };
                d.ok_or(windows_core::Error::from(E_FAIL))?.cast::<CoreDispatcher>()
            })();
            match dispatcher {
                Ok(dispatcher) => {
                    let _ = CTX.set(Ctx { service, diag, callback, dispatcher });
                    server::start();
                }
                Err(e) => log(format!("no CoreDispatcher: {e}")),
            }
        });
        Ok(())
    }

    fn GetSite(&self, riid: *const GUID, ppvsite: *mut *mut c_void) -> windows_core::Result<()> {
        match &*lock(&self.site) {
            Some(site) => unsafe { site.query(riid, ppvsite).ok() },
            None => {
                unsafe { *ppvsite = std::ptr::null_mut() };
                Err(E_FAIL.into())
            }
        }
    }
}

impl IVisualTreeServiceCallback_Impl for Tap_Impl {
    unsafe fn OnVisualTreeChange(
        &self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation_type: VisualMutationType,
    ) -> HRESULT {
        if DETACHED.load(Ordering::Acquire) {
            return S_OK;
        }
        let mut tree = lock(&TREE);
        if mutation_type == VisualMutationType::Add {
            tree.insert(
                element.Handle,
                Node {
                    ty: element.Type.to_string(),
                    name: element.Name.to_string(),
                    parent: relation.Parent,
                    index: relation.ChildIndex,
                },
            );
        } else {
            tree.remove(&element.Handle);
        }
        DIRTY.store(true, Ordering::Release);
        S_OK
    }
}

impl IVisualTreeServiceCallback2_Impl for Tap_Impl {
    unsafe fn OnElementStateChanged(&self, _: InstanceHandle, _: VisualElementState, _: PCWSTR) -> HRESULT {
        S_OK
    }
}

// ---------------------------------------------------------------------------------------------
// COM server plumbing

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        unsafe { *ppv = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let tap: IUnknown = Tap { site: Mutex::new(None) }.into();
        unsafe { tap.query(riid, ppv).ok() }
    }

    fn LockServer(&self, _: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    unsafe {
        *ppv = std::ptr::null_mut();
        if *rclsid != CLSID_PMS_TAP {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(riid, ppv)
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    // We leave threads running inside explorer; never let COM unload us.
    S_FALSE
}

/// The shell is in light mode: our "ink" (white text, tracks, separators) turns dark.
pub(crate) static LIGHT: AtomicBool = AtomicBool::new(false);

/// Pure white (any alpha) is ink: near-black in light mode. Other colors are left alone.
pub(crate) fn themed(c: windows::UI::Color) -> windows::UI::Color {
    if LIGHT.load(Ordering::Relaxed) && (c.R, c.G, c.B) == (0xFF, 0xFF, 0xFF) {
        windows::UI::Color { R: 0x1C, G: 0x1C, B: 0x1C, ..c }
    } else {
        c
    }
}
