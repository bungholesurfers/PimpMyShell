//! Sonar rings: an emitter sends rings out over and over, staggered, each growing from a dot while it fades. The
//! hot-mic alert has two (alert.rs), toasts one (ripple.rs).
//!
//! An emitter can be told to start over (a new center, a new reach) at any moment: the rings in flight then finish
//! the journey they were on, from where they are, while fresh ones set out - nothing stops or jumps mid-flight.
//! UI thread only.

use std::time::{Duration, Instant};

use windows::UI::Color;
use windows::UI::Composition::*;
use windows_core::{HSTRING, Interface};
use windows_numerics::Vector2;

use crate::commands::R;
use crate::pace::{self, Bezier};

fn w(e: windows_core::Error) -> String {
    e.message()
}

fn ms(n: i64) -> windows::Foundation::TimeSpan {
    windows::Foundation::TimeSpan { Duration: n * 10_000 }
}

pub struct Ring {
    pub geo: CompositionEllipseGeometry,
    brush: CompositionColorBrush,
    shape: CompositionSpriteShape,
    thickness: f32,
}

/// One round of rings: one ring's radius and color over `period` ms (they run it staggered, ring i `period * i / n`
/// behind the first).
pub struct Cycle {
    pub period: i64,
    pub grow: Vec<(f32, Vector2, Option<Bezier>)>,
    pub fade: Vec<(f32, Color, Option<Bezier>)>,
}

pub struct Emitter {
    list: CompositionShapeCollection,
    rings: Vec<Ring>,
    /// The cycle the rings are on, and since when.
    cycle: Option<(Cycle, Instant)>,
    /// Rings finishing their last journey, and when they're done.
    retiring: Vec<(CompositionSpriteShape, Instant)>,
}

fn ring(c: &Compositor, list: &CompositionShapeCollection, thickness: f32) -> R<Ring> {
    let geo = c.CreateEllipseGeometry().map_err(w)?;
    let brush = c.CreateColorBrushWithColor(Color { A: 0, R: 0xFF, G: 0xFF, B: 0xFF }).map_err(w)?;
    let shape = c.CreateSpriteShapeWithGeometry(&geo).map_err(w)?;
    shape.SetStrokeBrush(&brush).map_err(w)?;
    shape.SetStrokeThickness(thickness).map_err(w)?;
    list.Append(&shape).map_err(w)?;
    Ok(Ring { geo, brush, shape, thickness })
}

fn remove(list: &CompositionShapeCollection, shape: &CompositionSpriteShape) -> R<()> {
    let shape: CompositionShape = shape.cast().map_err(w)?;
    let mut at = 0u32;
    if list.IndexOf(&shape, &mut at).map_err(w)? {
        list.RemoveAt(at).map_err(w)?;
    }
    Ok(())
}

impl Emitter {
    /// Rings of these stroke thicknesses, drawn in `list`, idle (invisible) until started.
    pub fn new(c: &Compositor, list: &CompositionShapeCollection, thicknesses: &[f32]) -> R<Emitter> {
        let rings = thicknesses.iter().map(|&t| ring(c, list, t)).collect::<R<Vec<_>>>()?;
        Ok(Emitter { list: list.clone(), rings, cycle: None, retiring: Vec::new() })
    }

    /// The rings, for their centers to be set or animated.
    pub fn rings(&self) -> &[Ring] {
        &self.rings
    }

    pub fn centers(&self) -> R<Vec<CompositionObject>> {
        self.rings.iter().map(|r| r.geo.cast().map_err(w)).collect()
    }

    /// Send the rings out on `cycle`, staggered evenly over it, forever.
    pub fn start(&mut self, c: &Compositor, cycle: Cycle) -> R<()> {
        let n = self.rings.len() as i64;
        for (i, ring) in self.rings.iter().enumerate() {
            let delay = ms(cycle.period * i as i64 / n);
            let grow = c.CreateVector2KeyFrameAnimation().map_err(w)?;
            pace::vector2(c, &grow, &cycle.grow, cycle.period)?;
            grow.SetDuration(ms(cycle.period)).map_err(w)?;
            grow.SetDelayTime(delay).map_err(w)?;
            grow.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
            ring.geo.StartAnimation(&HSTRING::from("Radius"), &grow).map_err(w)?;
            let fade = c.CreateColorKeyFrameAnimation().map_err(w)?;
            pace::color(c, &fade, &cycle.fade, cycle.period)?;
            fade.SetDuration(ms(cycle.period)).map_err(w)?;
            fade.SetDelayTime(delay).map_err(w)?;
            fade.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
            ring.brush.StartAnimation(&HSTRING::from("Color"), &fade).map_err(w)?;
        }
        self.cycle = Some((cycle, Instant::now()));
        Ok(())
    }

    /// Let the rings in flight finish their journeys - outward from where each is now, fading as it would have,
    /// about the center they have (left as it is: a center following the mic label keeps following it) - and put
    /// fresh, idle rings in their place. Rings that hadn't set out yet just go.
    pub fn retire(&mut self, c: &Compositor) -> R<()> {
        let now = Instant::now();
        let list = &self.list;
        self.retiring.retain(|(shape, until)| now < *until || remove(list, shape).is_err());
        let fresh = self.rings.iter().map(|r| ring(c, list, r.thickness)).collect::<R<Vec<_>>>()?;
        let old = std::mem::replace(&mut self.rings, fresh);
        let cycle = self.cycle.take();
        let n = old.len() as i64;
        for (i, r) in old.into_iter().enumerate() {
            let Some((cy, started)) = &cycle else {
                remove(list, &r.shape)?;
                continue;
            };
            let t = now.duration_since(*started).as_millis() as i64 - cy.period * i as i64 / n;
            if t < 0 {
                remove(list, &r.shape)?;
                continue;
            }
            let p = (t % cy.period) as f32 / cy.period as f32;
            let rest = ((1.0 - p) * cy.period as f32) as i64 + 1;
            let grow = c.CreateVector2KeyFrameAnimation().map_err(w)?;
            pace::vector2(c, &grow, &pace::rest_of(&cy.grow, p), rest)?;
            grow.SetDuration(ms(rest)).map_err(w)?;
            r.geo.StartAnimation(&HSTRING::from("Radius"), &grow).map_err(w)?;
            let fade = c.CreateColorKeyFrameAnimation().map_err(w)?;
            pace::color(c, &fade, &pace::rest_of(&cy.fade, p), rest)?;
            fade.SetDuration(ms(rest)).map_err(w)?;
            r.brush.StartAnimation(&HSTRING::from("Color"), &fade).map_err(w)?;
            self.retiring.push((r.shape, now + Duration::from_millis(rest as u64 + 100)));
        }
        Ok(())
    }
}
