//! Hover tooltips for the meter strip.
//!
//! Meters live on the stage, below the taskbar's own layers, where the pointer never reaches
//! them. So hover zones (transparent rectangles) sit in the top layer, in the Grid that holds the
//! TaskbarFrame and the tray, laid over each meter and the space around it (meters.rs places
//! them; a GPU, disk or network meter gets one per GPU or device). A zone may be split into bars
//! (per CPU, per GPU engine): the tooltip says which one the pointer is over. The
//! tooltip is a windowed Popup, so it can stand above the taskbar; it follows the pointer and
//! closes as soon as the pointer leaves the zone. UI thread only.

use std::cell::RefCell;

use pms_xaml::{
    Border, Canvas, CornerRadius, FrameworkElement, PointerEventHandler, Rectangle, SolidColorBrush, TextBlock, Thickness, UIElement,
};
use pms_xaml::Popup;
use windows::Foundation::{Point, Rect, Size};
use windows::UI::Color;
use windows_core::{HSTRING, Interface};

use crate::commands::{R, resolve_elem};
use crate::layout::{bounds, element};
use crate::{CTX, Ctx, lock};

const BACKGROUND: Color = Color { A: 0xF0, R: 0x2B, G: 0x2B, B: 0x2B };
const BORDER: Color = Color { A: 0x40, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT: Color = Color { A: 0xFF, R: 0xFF, G: 0xFF, B: 0xFF };
const OFFSET: Point = Point { X: 14.0, Y: 10.0 };

/// A zone as meters.rs describes it: which meter, where (taskbar-window DIPs), the bars' [start, end) x ranges
/// relative to the zone's left edge and the meter's index of the first of them, and which of the meter's GPUs or
/// devices it covers (if it covers just one).
pub struct ZoneSpec {
    pub meter: usize,
    pub rect: Rect,
    pub bars: Vec<(f64, f64)>,
    pub bar0: usize,
    pub device: Option<usize>,
}

struct Zone {
    rect: Rectangle,
    meter: usize,
    left: f64,
    bars: Vec<(f64, f64)>,
    bar0: usize,
    device: Option<usize>,
}

struct Tips {
    overlay: Canvas,
    popup: Popup,
    text: TextBlock,
    zones: Vec<Zone>,
    /// (zone, bar) under the pointer, and the pointer position.
    hover: Option<(usize, Option<usize>, Point)>,
}

thread_local! {
    static TIPS: RefCell<Option<Tips>> = const { RefCell::new(None) };
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

fn ui<T: Interface>(e: &T) -> R<UIElement> {
    e.cast().map_err(w)
}

fn brush(c: Color) -> R<SolidColorBrush> {
    SolidColorBrush::CreateInstanceWithColor(c).map_err(w)
}

fn build(ctx: &Ctx) -> R<Tips> {
    let tray = resolve_elem("SystemTrayFrame")?;
    let parent_h = lock(&crate::TREE).get(&tray).map(|n| n.parent).ok_or("SystemTrayFrame has no parent")?;
    let parent: pms_xaml::Panel = element(ctx, parent_h)?.cast().map_err(w)?;
    // No background: only the zones in it catch the pointer.
    let overlay = Canvas::new().map_err(w)?;
    parent.Children().map_err(w)?.Append(&ui(&overlay)?).map_err(w)?;

    let text = TextBlock::new().map_err(w)?;
    text.SetFontSize(12.0).map_err(w)?;
    text.SetForeground(&brush(TEXT)?).map_err(w)?;
    let border = Border::new().map_err(w)?;
    border.SetBackground(&brush(BACKGROUND)?).map_err(w)?;
    border.SetBorderBrush(&brush(BORDER)?).map_err(w)?;
    border.SetBorderThickness(Thickness { Left: 1.0, Top: 1.0, Right: 1.0, Bottom: 1.0 }).map_err(w)?;
    border.SetCornerRadius(CornerRadius { TopLeft: 4.0, TopRight: 4.0, BottomRight: 4.0, BottomLeft: 4.0 }).map_err(w)?;
    border.SetPadding(Thickness { Left: 8.0, Top: 5.0, Right: 8.0, Bottom: 6.0 }).map_err(w)?;
    border.SetChild(&ui(&text)?).map_err(w)?;
    border.SetIsHitTestVisible(false).map_err(w)?;

    let popup = Popup::new().map_err(w)?;
    popup.SetChild(&ui(&border)?).map_err(w)?;
    // Windowed, so it may stand outside (above) the taskbar.
    popup.SetShouldConstrainToRootBounds(false).map_err(w)?;
    popup.SetIsHitTestVisible(false).map_err(w)?;
    popup.SetXamlRoot(&ui(&overlay)?.XamlRoot().map_err(w)?).map_err(w)?;
    Ok(Tips { overlay, popup, text, zones: Vec::new(), hover: None })
}

/// Replace the hover zones (called whenever the strip is positioned).
pub fn set_zones(specs: Vec<ZoneSpec>) {
    let Some(ctx) = CTX.get() else { return };
    let r = (|| -> R<()> {
        if TIPS.with_borrow(Option::is_none) {
            let t = build(ctx)?;
            TIPS.with_borrow_mut(|s| *s = Some(t));
        }
        TIPS.with_borrow_mut(|s| -> R<()> {
            let t = s.as_mut().ok_or("tooltips vanished")?;
            let origin = bounds(&ui(&t.overlay)?, 0.0, 0.0)?;
            let children = t.overlay.Children().map_err(w)?;
            // Grow the pool of rectangles as needed; spare ones are parked at zero size.
            while t.zones.len() < specs.len() {
                let i = t.zones.len();
                let rect = Rectangle::new().map_err(w)?;
                rect.SetFill(&brush(Color { A: 0, R: 0, G: 0, B: 0 })?).map_err(w)?;
                let el = ui(&rect)?;
                let moved = PointerEventHandler::new(move |_, args| {
                    if let Some(p) = args.as_ref().and_then(|a| a.GetCurrentPoint(None::<&UIElement>).ok()).and_then(|p| p.Position().ok()) {
                        hover(i, p);
                    }
                    Ok(())
                });
                el.PointerEntered(&moved).map_err(w)?;
                el.PointerMoved(&moved).map_err(w)?;
                el.PointerExited(&PointerEventHandler::new(move |_, _| {
                    leave(i);
                    Ok(())
                }))
                .map_err(w)?;
                children.Append(&el).map_err(w)?;
                t.zones.push(Zone { rect, meter: usize::MAX, left: 0.0, bars: Vec::new(), bar0: 0, device: None });
            }
            for (i, z) in t.zones.iter_mut().enumerate() {
                let el = ui(&z.rect)?;
                match specs.get(i) {
                    Some(s) => {
                        Canvas::SetLeft(&el, (s.rect.X - origin.X) as f64).map_err(w)?;
                        Canvas::SetTop(&el, (s.rect.Y - origin.Y) as f64).map_err(w)?;
                        z.rect.SetWidth(s.rect.Width as f64).map_err(w)?;
                        z.rect.SetHeight(s.rect.Height as f64).map_err(w)?;
                        (z.meter, z.left, z.bars, z.bar0, z.device) = (s.meter, s.rect.X as f64, s.bars.clone(), s.bar0, s.device);
                    }
                    None => {
                        z.rect.SetWidth(0.0).map_err(w)?;
                        z.meter = usize::MAX;
                    }
                }
            }
            Ok(())
        })
    })();
    if let Err(e) = r {
        crate::log(format!("tooltip zones: {e}"));
    }
}

/// Zones stop catching the pointer while the strip is out of sight (a toast covers it).
pub fn set_enabled(on: bool) {
    TIPS.with_borrow(|s| {
        if let Some(t) = s {
            let _ = t.overlay.SetIsHitTestVisible(on);
            if !on {
                let _ = t.popup.SetIsOpen(false);
            }
        }
    });
}

fn hover(zone: usize, p: Point) {
    let target = TIPS.with_borrow_mut(|s| {
        let t = s.as_mut()?;
        let z = t.zones.get(zone).filter(|z| z.meter != usize::MAX)?;
        let x = p.X as f64 - z.left;
        // The bar under the pointer, or the nearest one in the gaps between bars.
        let bar = (!z.bars.is_empty()).then(|| {
            z.bars
                .iter()
                .enumerate()
                .min_by(|a, b| dist(x, *a.1).total_cmp(&dist(x, *b.1)))
                .map(|(i, _)| i)
                .unwrap_or(0)
                + z.bar0
        });
        t.hover = Some((zone, bar, p));
        Some((z.meter, bar, z.device))
    });
    if let Some((meter, bar, device)) = target {
        crate::meters::highlight(Some((meter, bar)));
        show(crate::meters::tip_text(meter, bar, device));
    }
}

fn dist(x: f64, (a, b): (f64, f64)) -> f64 {
    if x < a { a - x } else if x >= b { x - b } else { 0.0 }
}

fn leave(zone: usize) {
    TIPS.with_borrow_mut(|s| {
        if let Some(t) = s.as_mut().filter(|t| t.hover.is_some_and(|h| h.0 == zone)) {
            t.hover = None;
            let _ = t.popup.SetIsOpen(false);
        }
    });
    crate::meters::highlight(None);
}

/// The meter under the pointer got new values: rewrite the tooltip in place.
pub fn refresh(meter: usize) {
    let target = TIPS.with_borrow(|s| {
        let t = s.as_ref()?;
        let (zone, bar, _) = t.hover?;
        let z = t.zones.get(zone)?;
        (z.meter == meter).then_some((bar, z.device))
    });
    if let Some((bar, device)) = target {
        show(crate::meters::tip_text(meter, bar, device));
    }
}

/// Open (or update) the tooltip just above the pointer, to its right (left, near the screen's right edge).
fn show(text: Option<String>) {
    TIPS.with_borrow(|s| {
        let Some(t) = s else { return };
        let r = (|| -> R<()> {
            let Some(text) = text else { return t.popup.SetIsOpen(false).map_err(w) };
            let Some((_, _, p)) = t.hover else { return Ok(()) };
            t.text.SetText(&HSTRING::from(text)).map_err(w)?;
            let child: FrameworkElement = t.popup.Child().map_err(w)?.cast().map_err(w)?;
            child.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
            let size = child.DesiredSize().map_err(w)?;
            // Right of the pointer, unless that would run off the screen's (the taskbar's) right edge: then left of it.
            let room = t.popup.XamlRoot().and_then(|r| r.Size()).map_or(f32::MAX, |s| s.Width);
            let x = if p.X + OFFSET.X + size.Width + 4.0 > room { (p.X - OFFSET.X - size.Width).max(0.0) } else { p.X + OFFSET.X };
            t.popup.SetHorizontalOffset(x as f64).map_err(w)?;
            t.popup.SetVerticalOffset((p.Y - OFFSET.Y - size.Height) as f64 - 18.0).map_err(w)?;
            t.popup.SetIsOpen(true).map_err(w)
        })();
        if let Err(e) = r {
            crate::log(format!("tooltip: {e}"));
        }
    });
}

/// For reset/detach.
pub(crate) fn off() {
    if let Some(t) = TIPS.with_borrow_mut(Option::take) {
        let _ = t.popup.SetIsOpen(false);
        if let Ok(parent) = t.overlay.Parent().and_then(|p| p.cast::<pms_xaml::Panel>())
            && let (Ok(children), Ok(el)) = (parent.Children(), ui(&t.overlay)) {
                let mut i = 0;
                if children.IndexOf(&el, &mut i).unwrap_or(false) {
                    let _ = children.RemoveAt(i);
                }
            }
    }
}
