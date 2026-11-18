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

/// A Grid, StackPanel, Canvas or Path made with `new()` is never freed in Explorer's XAML: every one of them was still alive long
/// after it was let go, whatever was done with it or wasn't (2026-11-16, Windows.UI.Xaml 6.2.26100.9818, counted with weak
/// references; a Border, a TextBlock, an Ellipse or a brush made the same way goes as it should). Made by XAML's own parser, the
/// same element goes, children added from code and all. So those four are made from markup, here: a notification card was two
/// dozen KB of Explorer's memory for good, each one, and a rebuilt meter strip or a remote some more. The markup is ours and
/// fixed: nothing from outside goes into it.
fn parsed<T: Interface>(tag: &str) -> R<T> {
    let markup = format!("<{tag} xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'/>");
    pms_xaml::XamlReader::Load(&windows_core::HSTRING::from(markup)).and_then(|o| o.cast()).map_err(|e| format!("making a {tag}: {}", e.message()))
}

pub(crate) fn grid() -> R<pms_xaml::Grid> {
    parsed("Grid")
}

pub(crate) fn stack_panel() -> R<pms_xaml::StackPanel> {
    parsed("StackPanel")
}

pub(crate) fn canvas() -> R<Canvas> {
    parsed("Canvas")
}

pub(crate) fn path() -> R<pms_xaml::Path> {
    parsed("Path")
}

/// The font family by `name`, made once and kept: one made for each element that uses it stays in Explorer's memory for good
/// (2026-11-16: a notification card's close glyph, a FontFamily and about 50 small blocks with it, for each card). UI thread.
pub(crate) fn font(name: &str) -> R<pms_xaml::FontFamily> {
    thread_local! {
        static FONTS: RefCell<Vec<(String, pms_xaml::FontFamily)>> = const { RefCell::new(Vec::new()) };
    }
    if let Some(f) = FONTS.with_borrow(|fonts| fonts.iter().find(|(n, _)| n == name).map(|(_, f)| f.clone())) {
        return Ok(f);
    }
    let f = pms_xaml::FontFamily::CreateInstanceWithName(&windows_core::HSTRING::from(name)).map_err(|e| format!("the font {name}: {}", e.message()))?;
    FONTS.with_borrow_mut(|fonts| fonts.push((name.to_string(), f.clone())));
    Ok(f)
}

/// What the pooled visuals are tagged with (their `Comment`), for `unbuild` to know them.
const POOLED: &str = "pms-pooled";
/// The most visuals of each kind kept for reuse.
const MOST_SPARE: usize = 512;

thread_local! {
    /// Visuals hung on elements that have gone. Explorer never gives back the memory of a composition visual that's been on
    /// screen, given back, closed or not (2026-11-16: a meter strip's rebuild left a dozen sprites, with their shadows and
    /// gradients, every time): so one, once made, is used again.
    static SPARE_SPRITES: RefCell<Vec<windows::UI::Composition::SpriteVisual>> = const { RefCell::new(Vec::new()) };
    static SPARE_CONTAINERS: RefCell<Vec<windows::UI::Composition::ContainerVisual>> = const { RefCell::new(Vec::new()) };
}

/// A sprite to hang on an element (SetElementChildVisual): one given back (`unbuild`), if there's one of `c`'s, else a new one.
/// It's as a new one is: no brush, shadow or clip, no size, nothing animating.
pub(crate) fn sprite(c: &windows::UI::Composition::Compositor) -> R<windows::UI::Composition::SpriteVisual> {
    while let Some(s) = SPARE_SPRITES.with_borrow_mut(Vec::pop) {
        // One of another compositor (the taskbar's been rebuilt on another, say) is simply let go.
        if s.Compositor().is_ok_and(|sc| sc == *c) {
            return Ok(s);
        }
    }
    let s = c.CreateSpriteVisual().map_err(|e| e.message())?;
    s.SetComment(&windows_core::HSTRING::from(POOLED)).map_err(|e| e.message())?;
    Ok(s)
}

/// The same, for a container (a layer of sprites of its own).
pub(crate) fn container(c: &windows::UI::Composition::Compositor) -> R<windows::UI::Composition::ContainerVisual> {
    while let Some(v) = SPARE_CONTAINERS.with_borrow_mut(Vec::pop) {
        if v.Compositor().is_ok_and(|vc| vc == *c) {
            return Ok(v);
        }
    }
    let v = c.CreateContainerVisual().map_err(|e| e.message())?;
    v.SetComment(&windows_core::HSTRING::from(POOLED)).map_err(|e| e.message())?;
    Ok(v)
}

/// Puts `v` back with the spares, as new, if it's one of ours (`sprite`, `container`).
fn give_back(v: windows::UI::Composition::Visual) {
    use windows::UI::Composition::{CompositionBrush, CompositionClip, CompositionShadow, ContainerVisual, SpriteVisual};
    use windows_numerics::{Vector2, Vector3};
    if !v.Comment().is_ok_and(|c| c == POOLED) {
        return;
    }
    for p in ["Opacity", "Offset", "Offset.X", "Offset.Y", "Size", "Scale", "Scale.X", "Scale.Y", "RelativeOffsetAdjustment", "RelativeSizeAdjustment", "RotationAngle"] {
        let _ = v.StopAnimation(&windows_core::HSTRING::from(p));
    }
    let _ = v.SetClip(None::<&CompositionClip>);
    let _ = v.SetOpacity(1.0);
    let _ = v.SetOffset(Vector3::default());
    let _ = v.SetSize(Vector2::default());
    let _ = v.SetScale(Vector3 { X: 1.0, Y: 1.0, Z: 1.0 });
    let _ = v.SetRotationAngle(0.0);
    let _ = v.SetCenterPoint(Vector3::default());
    let _ = v.SetAnchorPoint(Vector2::default());
    let _ = v.SetRelativeOffsetAdjustment(Vector3::default());
    let _ = v.SetRelativeSizeAdjustment(Vector2::default());
    let _ = v.SetIsVisible(true);
    // What's in it goes back too (one of ours, that is: one that isn't is only taken out).
    if let Ok(c) = v.cast::<ContainerVisual>()
        && let Ok(children) = c.Children()
    {
        let mut inside = Vec::new();
        if let Ok(it) = children.First() {
            while it.HasCurrent().unwrap_or(false) {
                if let Ok(child) = it.Current() {
                    inside.push(child);
                }
                if !it.MoveNext().unwrap_or(false) {
                    break;
                }
            }
        }
        let _ = children.RemoveAll();
        inside.into_iter().for_each(give_back);
    }
    if let Ok(s) = v.cast::<SpriteVisual>() {
        let _ = s.SetBrush(None::<&CompositionBrush>);
        let _ = s.SetShadow(None::<&CompositionShadow>);
        SPARE_SPRITES.with_borrow_mut(|spare| {
            if spare.len() < MOST_SPARE {
                spare.push(s);
            }
        });
    } else if let Ok(c) = v.cast::<ContainerVisual>() {
        SPARE_CONTAINERS.with_borrow_mut(|spare| {
            if spare.len() < MOST_SPARE {
                spare.push(c);
            }
        });
    }
}

/// Takes `v` (a `sprite` or `container`) out of the layer it's in and puts it back with the spares: for one that isn't hung
/// on an element, which `unbuild` would give back.
pub(crate) fn spare(v: &impl Interface) {
    let Ok(v) = v.cast::<windows::UI::Composition::Visual>() else { return };
    if let Ok(parent) = v.Parent() {
        let _ = parent.Children().and_then(|c| c.Remove(&v));
    }
    give_back(v);
}

/// Lets go of the spare visuals: this copy of the TAP is done.
pub(crate) fn drop_spares() {
    SPARE_SPRITES.with_borrow_mut(Vec::clear);
    SPARE_CONTAINERS.with_borrow_mut(Vec::clear);
}

/// Takes `e` apart, from the inside out, for when it's about to go. XAML's diagnostics, which is how we got in here, hold on to
/// every element they've been told about until they're told it's gone, and taking a parent out only tells them about the parent:
/// everything under it would stay in Explorer's memory for good, pictures and all. Measured, a picture in a Canvas: 23 KB without
/// this, 1 with it. The composition visuals hung on an element go too: one that diagnostics keeps would otherwise keep them, and
/// all they draw with, textures and all. UI thread only.
pub(crate) fn unbuild(e: &UIElement) {
    // What's hung on it goes back with the spares, if it's one of ours.
    let hung = ElementCompositionPreview::GetElementChildVisual(e).ok();
    let _ = ElementCompositionPreview::SetElementChildVisual(e, None::<&windows::UI::Composition::Visual>);
    if let Some(v) = hung {
        give_back(v);
    }
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
    let canvas = canvas()?;
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
