//! Our elements inside the taskbar's background Grid, always kept in this order:
//!
//!   BackgroundFill, [Backdrop host], [Alert host], [Stage canvas], [Grain host], BackgroundStroke, ...
//!
//! so meters and other content on the stage sit above the blur/tint/alert and below the grain, and
//! everything sits below the buttons (which live in a later sibling of the background control).
//! UI thread only.

use std::cell::RefCell;

use pms_xaml::{Border, Canvas, ElementCompositionPreview, Panel, Popup, UIElement};
use windows_core::Interface;

use crate::Ctx;
use crate::commands::{R, resolve_elem};
use crate::layout::element;
use crate::{TREE, lock};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Slot {
    Backdrop,
    Alert,
    Stage,
    Grain,
}

thread_local! {
    static PLACED: RefCell<Vec<(Slot, UIElement)>> = const { RefCell::new(Vec::new()) };
    static STAGE: RefCell<Option<Canvas>> = const { RefCell::new(None) };
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

/// The Grid holding BackgroundFill, and BackgroundFill itself.
fn background(ctx: &Ctx) -> R<(Panel, UIElement)> {
    let fill = resolve_elem("BackgroundFill")?;
    let parent = lock(&TREE).get(&fill).map(|n| n.parent).ok_or("BackgroundFill vanished")?;
    let panel: Panel = element(ctx, parent)?.cast().map_err(w)?;
    let fill: UIElement = element(ctx, fill)?.cast().map_err(w)?;
    Ok((panel, fill))
}

fn index_of(panel: &Panel, e: &UIElement) -> R<Option<u32>> {
    let mut i = 0;
    Ok(panel.Children().map_err(w)?.IndexOf(e, &mut i).map_err(w)?.then_some(i))
}

pub fn insert(ctx: &Ctx, slot: Slot, e: &UIElement) -> R<()> {
    let (panel, fill) = background(ctx)?;
    let mut at = index_of(&panel, &fill)?.ok_or("BackgroundFill not in its panel")? + 1;
    let placed = PLACED.with_borrow(|p| p.clone());
    for (s, other) in &placed {
        if *s < slot
            && let Some(i) = index_of(&panel, other)? {
                at = at.max(i + 1);
            }
    }
    panel.Children().map_err(w)?.InsertAt(at, e).map_err(w)?;
    PLACED.with_borrow_mut(|p| p.push((slot, e.clone())));
    Ok(())
}

/// Takes `e` apart, from the inside out, for when it's about to go. XAML's diagnostics, which is how we got in here, hold on to
/// every element they've been told about until they're told it's gone, and taking a parent out only tells them about the parent:
/// everything under it would stay in Explorer's memory for good, pictures and all. Measured, a picture in a Canvas: 23 KB without
/// this, 1 with it. The composition visuals hung on an element go too: one that diagnostics keeps would otherwise keep them, and
/// all they draw with, textures and all. UI thread only.
pub(crate) fn unbuild(e: &UIElement) {
    let _ = ElementCompositionPreview::SetElementChildVisual(e, None::<&windows::UI::Composition::Visual>);
    if let Ok(panel) = e.cast::<Panel>() {
        empty(&panel);
    } else if let Ok(border) = e.cast::<Border>()
        && let Ok(child) = border.Child()
    {
        unbuild(&child);
        let _ = border.SetChild(None);
    }
}

/// Takes everything out of `panel`, the same way.
pub(crate) fn empty(panel: &Panel) {
    let Ok(children) = panel.Children() else { return };
    for i in 0..children.Size().unwrap_or(0) {
        if let Ok(child) = children.GetAt(i) {
            unbuild(&child);
        }
    }
    let _ = children.Clear();
}

/// The same for what's in `popup`.
pub(crate) fn unbuild_popup(popup: &Popup) {
    if let Ok(child) = popup.Child() {
        unbuild(&child);
        let _ = popup.SetChild(None);
    }
}

pub fn remove(ctx: &Ctx, e: &UIElement) -> R<()> {
    PLACED.with_borrow_mut(|p| p.retain(|(_, o)| o != e));
    unbuild(e);
    let (panel, _) = background(ctx)?;
    if let Some(i) = index_of(&panel, e)? {
        panel.Children().map_err(w)?.RemoveAt(i).map_err(w)?;
    }
    Ok(())
}

/// A hit-test-transparent Canvas in the Stage slot, created on first use.
pub fn stage(ctx: &Ctx) -> R<Canvas> {
    if let Some(c) = STAGE.with_borrow(|s| s.clone()) {
        return Ok(c);
    }
    let canvas = Canvas::new().map_err(w)?;
    canvas.SetIsHitTestVisible(false).map_err(w)?;
    insert(ctx, Slot::Stage, &canvas.cast().map_err(w)?)?;
    STAGE.with_borrow_mut(|s| *s = Some(canvas.clone()));
    Ok(canvas)
}

pub fn drop_stage(ctx: &Ctx) -> R<()> {
    if let Some(c) = STAGE.with_borrow_mut(Option::take) {
        remove(ctx, &c.cast().map_err(w)?)?;
    }
    Ok(())
}

/// Drop every reference without touching the tree: for after a taskbar rebuild, when our
/// elements went down with the old tree and removing them would fail.
pub fn forget() {
    PLACED.with_borrow_mut(Vec::clear);
    STAGE.with_borrow_mut(|s| *s = None);
}
