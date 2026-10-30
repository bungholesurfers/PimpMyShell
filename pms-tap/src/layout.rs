//! Where the buttons are, and where they aren't.
//!
//! "Occupants" are the direct children of the taskbar's button repeater and of the system tray
//! grid. On every XAML `LayoutUpdated` we measure them (in DIPs, relative to the island root,
//! i.e. the taskbar window), derive the free horizontal intervals, and publish a snapshot when
//! anything moved. Consumers on the UI thread (the backdrop's `gaps`) are notified directly.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{LazyLock, Mutex};

use pms_xaml::{AutomationProperties, FrameworkElement, UIElement, Visibility};
use windows::Foundation::{EventHandler, Rect};
use windows_core::{IInspectable, Interface};

use crate::commands::{R, resolve_elem};
use crate::xamlom::InstanceHandle;
use crate::{CTX, Ctx, TREE, dfs_order, lock, log};

/// Containers whose direct children occupy taskbar space.
const OCCUPANT_PARENTS: &[&str] = &["TaskbarFrameRepeater", "SystemTrayFrameGrid"];
/// Free intervals narrower than this aren't worth reporting.
const MIN_GAP: f32 = 4.0;

#[derive(Clone, PartialEq)]
pub struct Item {
    pub handle: InstanceHandle,
    pub label: String,
    pub name: String,
    pub rect: Rect,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub version: u64,
    /// The island root's size: the whole taskbar, in DIPs.
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    /// Free [start, end) intervals along x.
    pub free: Vec<(f32, f32)>,
}

pub static SNAPSHOT: LazyLock<Mutex<Snapshot>> = LazyLock::new(Default::default);

struct Tracker {
    anchor: FrameworkElement,
    token: i64,
    elements: HashMap<InstanceHandle, FrameworkElement>,
}

thread_local! {
    static TRACKER: RefCell<Option<Tracker>> = const { RefCell::new(None) };
}

pub(crate) fn element(ctx: &Ctx, h: InstanceHandle) -> R<FrameworkElement> {
    let mut insp: Option<IInspectable> = None;
    unsafe { ctx.diag.GetIInspectableFromHandle(h, &mut insp) }.ok().map_err(|e| e.to_string())?;
    insp.ok_or("null element")?.cast().map_err(|e| e.to_string())
}

/// Start tracking (idempotent). Must run on the UI thread.
pub fn ensure_tracking(ctx: &Ctx) -> R<()> {
    if TRACKER.with_borrow(Option::is_some) {
        return Ok(());
    }
    // LayoutUpdated fires for any layout pass in the tree, so one subscription is enough.
    let anchor = element(ctx, resolve_elem("TaskbarFrame")?)?;
    let handler = EventHandler::<IInspectable>::new(|_, _| {
        // A panic unwinding into XAML would abort explorer.
        let r = std::panic::catch_unwind(|| CTX.get().map(recompute));
        match r {
            Ok(Some(Err(e))) => log(format!("layout recompute: {e}")),
            Err(_) => log("layout recompute panicked"),
            _ => {}
        }
        Ok(())
    });
    let token = anchor.LayoutUpdated(&handler).map_err(|e| e.to_string())?;
    TRACKER.with_borrow_mut(|t| *t = Some(Tracker { anchor, token, elements: HashMap::new() }));
    recompute(ctx)
}

pub fn stop_tracking() {
    if let Some(t) = TRACKER.with_borrow_mut(Option::take) {
        let _ = t.anchor.RemoveLayoutUpdated(t.token);
    }
}

fn occupants() -> Vec<(InstanceHandle, String)> {
    let tree = lock(&TREE);
    let parents: Vec<InstanceHandle> =
        tree.iter().filter(|(_, n)| OCCUPANT_PARENTS.contains(&n.name.as_str())).map(|(&h, _)| h).collect();
    dfs_order(&tree)
        .into_iter()
        .filter(|h| parents.contains(&tree[h].parent))
        .map(|h| {
            let n = &tree[&h];
            let label = if n.name.is_empty() { n.ty.rsplit('.').next().unwrap_or(&n.ty).to_string() } else { n.name.clone() };
            (h, label)
        })
        .collect()
}

/// Bounds of `e` relative to the island root (the taskbar window), in DIPs.
pub fn bounds(e: &UIElement, w: f64, h: f64) -> R<Rect> {
    let t = e.TransformToVisual(None::<&UIElement>).map_err(|e| e.to_string())?;
    t.TransformBounds(Rect { X: 0.0, Y: 0.0, Width: w as f32, Height: h as f32 }).map_err(|e| e.to_string())
}

fn recompute(ctx: &Ctx) -> R<()> {
    let occupants = occupants();
    let measured = TRACKER.with(|cell| -> R<Option<(f32, f32, Vec<Item>)>> {
        // Busy means we re-entered from our own XAML calls; the outer pass will finish the job.
        let Ok(mut guard) = cell.try_borrow_mut() else { return Ok(None) };
        let Some(t) = guard.as_mut() else { return Ok(None) };
        t.elements.retain(|h, _| occupants.iter().any(|(o, _)| o == h));

        // The repeater stretches across the whole taskbar, so the TaskbarFrame is the extent.
        let (width, height) = (t.anchor.ActualWidth().unwrap_or(0.0) as f32, t.anchor.ActualHeight().unwrap_or(0.0) as f32);
        let mut items = Vec::new();
        for (h, label) in &occupants {
            let e = match t.elements.get(h) {
                Some(e) => e.clone(),
                None => match element(ctx, *h) {
                    Ok(e) => {
                        t.elements.insert(*h, e.clone());
                        e
                    }
                    Err(_) => continue,
                },
            };
            let ui: UIElement = e.cast().map_err(|e| e.to_string())?;
            if ui.Visibility().map_err(|e| e.to_string())? != Visibility::Visible || ui.Opacity().unwrap_or(1.0) == 0.0 {
                continue;
            }
            let (w, hgt) = (e.ActualWidth().unwrap_or(0.0), e.ActualHeight().unwrap_or(0.0));
            if w <= 0.0 || hgt <= 0.0 {
                continue;
            }
            let rect = bounds(&ui, w, hgt)?;
            // ItemsRepeater parks recycled elements far off-screen.
            if rect.X + rect.Width <= 0.0 || rect.X >= width {
                continue;
            }
            let name = AutomationProperties::GetName(&ui).map(|s| s.to_string()).unwrap_or_default();
            items.push(Item { handle: *h, label: label.clone(), name, rect });
        }
        items.sort_by(|a, b| a.rect.X.total_cmp(&b.rect.X));
        Ok(Some((width, height, items)))
    })?;
    let Some((width, height, items)) = measured else { return Ok(()) };

    // Publish only real changes. The lock must be released before calling out: listeners read
    // the snapshot themselves (re-locking here would deadlock the taskbar's UI thread).
    let changed = {
        let mut snap = lock(&SNAPSHOT);
        let same = snap.width == width && snap.height == height && snap.items == items;
        (!same).then(|| {
            snap.version += 1;
            snap.width = width;
            snap.height = height;
            snap.free = free_intervals(width, &items);
            snap.items = items;
            snap.clone()
        })
    };
    let Some(snapshot) = changed else {
        // Our own cells may have just been laid out (a rebuild): place what depends on them.
        crate::meters::after_layout();
        return Ok(());
    };
    crate::backdrop::on_layout(&snapshot);
    crate::meters::place(&snapshot);
    crate::toast::place(&snapshot);
    crate::glow::on_layout(ctx);
    Ok(())
}

fn free_intervals(width: f32, items: &[Item]) -> Vec<(f32, f32)> {
    let mut free = Vec::new();
    let mut cursor = 0.0f32;
    for it in items {
        let (start, end) = (it.rect.X.max(0.0), (it.rect.X + it.rect.Width).min(width));
        if start - cursor >= MIN_GAP {
            free.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if width - cursor >= MIN_GAP {
        free.push((cursor, width));
    }
    free
}

pub fn describe(s: &Snapshot) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "layout v{}  taskbar {}x{} DIPs", s.version, s.width, s.height);
    let _ = writeln!(out, "occupied:");
    for it in &s.items {
        let r = &it.rect;
        let _ = writeln!(
            out,
            "  {:>7.1} .. {:>7.1}  (w {:>5.1})  {:<24} {}",
            r.X,
            r.X + r.Width,
            r.Width,
            it.label,
            it.name
        );
    }
    let _ = writeln!(out, "free:");
    for (a, b) in &s.free {
        let _ = writeln!(out, "  {a:>7.1} .. {b:>7.1}  (w {:>5.1})", b - a);
    }
    out
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    match args.first().map(String::as_str) {
        None => {
            ensure_tracking(ctx)?;
            Ok(describe(&lock(&SNAPSHOT)))
        }
        Some("stop") => {
            stop_tracking();
            Ok("layout tracking stopped".into())
        }
        Some(other) => Err(format!("unknown layout subcommand `{other}` (none|stop; `pms layout watch` is client-side)")),
    }
}
