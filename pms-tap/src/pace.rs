//! Frame pacing for PimpMyShell's long-running animations (the hot-mic alert's breathing and rings).
//!
//! The compositor runs keyframe animations itself, at the display's rate, and has no setting for a slower one. What
//! it can do is hold a value: with step easing between keyframes a value only changes when the next keyframe comes
//! round. So a paced animation is the same curve, sampled `FPS` times a second into held steps; between them nothing
//! on screen changes for it.
//!
//!   pace <fps>      0 = smooth (the display's own rate)
//!
//! It applies to animations started from then on.

use std::sync::atomic::{AtomicU32, Ordering};

use windows::UI::Color;
use windows::UI::Composition::{ColorKeyFrameAnimation, CompositionEasingFunction, Compositor, Vector2KeyFrameAnimation};
use windows_core::Interface;
use windows_numerics::Vector2;

use crate::commands::R;

static FPS: AtomicU32 = AtomicU32::new(0);

pub fn command(args: &[String]) -> R<String> {
    match args.first() {
        Some(n) => {
            let fps: u32 = n.parse().map_err(|_| format!("bad fps {n:?}"))?;
            FPS.store(fps.min(1000), Ordering::Relaxed);
            Ok(format!("pace {fps}"))
        }
        None => Ok(format!("pace {}", FPS.load(Ordering::Relaxed))),
    }
}

/// Whether animations are paced at all.
pub fn on() -> bool {
    FPS.load(Ordering::Relaxed) > 0
}

fn w(e: windows_core::Error) -> String {
    e.message()
}

/// A cubic Bezier easing, CSS-style: control points (x1, y1), (x2, y2).
#[derive(Clone, Copy)]
pub struct Bezier(pub f32, pub f32, pub f32, pub f32);

impl Bezier {
    /// The eased progress at time `t` (0..1).
    fn at(self, t: f32) -> f32 {
        let Bezier(x1, y1, x2, y2) = self;
        let curve = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
        // Solve x(s) = t by bisection (x is monotonic for x1, x2 in 0..1); plenty precise for a few dozen samples.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if curve(x1, x2, mid) < t { lo = mid } else { hi = mid }
        }
        curve(y1, y2, (lo + hi) / 2.0)
    }

    fn easing(self, c: &Compositor) -> R<CompositionEasingFunction> {
        c.CreateCubicBezierEasingFunction(Vector2 { X: self.0, Y: self.1 }, Vector2 { X: self.2, Y: self.3 }).map_err(w)?.cast().map_err(w)
    }
}

/// What keyframes can hold.
pub trait Key: Copy {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
}

impl Key for f32 {
    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
}

impl Key for Vector2 {
    fn lerp(a: Vector2, b: Vector2, t: f32) -> Vector2 {
        Vector2 { X: f32::lerp(a.X, b.X, t), Y: f32::lerp(a.Y, b.Y, t) }
    }
}

impl Key for Color {
    fn lerp(a: Color, b: Color, t: f32) -> Color {
        let c = |x: u8, y: u8| f32::lerp(x as f32, y as f32, t).round().clamp(0.0, 255.0) as u8;
        Color { A: c(a.A, b.A), R: c(a.R, b.R), G: c(a.G, b.G), B: c(a.B, b.B) }
    }
}

/// An animation's stops: (progress, value, easing into it; None = linear).
pub type Stops<T> = [(f32, T, Option<Bezier>)];

/// The rest of an animation from progress `p` on, as stops of its own over 0..1: where it is at `p`, then the stops
/// still ahead. (The segment it's in keeps its easing over what's left of it: near enough.)
pub fn rest_of<T: Key>(stops: &Stops<T>, p: f32) -> Vec<(f32, T, Option<Bezier>)> {
    let span = (1.0 - p).max(1e-3);
    std::iter::once((0.0, sample(stops, p), None)).chain(stops.iter().filter(|s| s.0 > p).map(|&(at, v, e)| ((at - p) / span, v, e))).collect()
}

fn sample<T: Key>(stops: &Stops<T>, p: f32) -> T {
    let mut prev = stops[0];
    for &(at, v, ease) in &stops[1..] {
        if p <= at {
            let t = if at > prev.0 { (p - prev.0) / (at - prev.0) } else { 1.0 };
            return T::lerp(prev.1, v, ease.map_or(t, |e| e.at(t)));
        }
        prev = (at, v, ease);
    }
    prev.1
}

/// Fill a keyframe animation lasting `dur_ms` with `stops`: as they are (smooth), or sampled into held steps at the
/// paced rate. `insert(progress, value, easing)` is the animation's own InsertKeyFrame(WithEasingFunction).
fn fill<T: Key>(c: &Compositor, stops: &Stops<T>, dur_ms: i64, insert: impl Fn(f32, T, Option<&CompositionEasingFunction>) -> R<()>) -> R<()> {
    let fps = FPS.load(Ordering::Relaxed);
    if fps == 0 || stops.len() < 2 {
        for &(at, v, ease) in stops {
            match ease {
                Some(e) => insert(at, v, Some(&e.easing(c)?))?,
                None => insert(at, v, None)?,
            }
        }
        return Ok(());
    }
    let n = ((dur_ms as f32 * fps as f32 / 1000.0).ceil() as usize).clamp(2, 1000);
    let step: CompositionEasingFunction = c.CreateStepEasingFunctionWithStepCount(1).map_err(w)?.cast().map_err(w)?;
    insert(0.0, sample(stops, 0.0), None)?;
    for i in 1..=n {
        let p = i as f32 / n as f32;
        insert(p, sample(stops, p), Some(&step))?;
    }
    Ok(())
}

pub fn vector2(c: &Compositor, anim: &Vector2KeyFrameAnimation, stops: &Stops<Vector2>, dur_ms: i64) -> R<()> {
    fill(c, stops, dur_ms, |p, v, e| match e {
        Some(e) => anim.InsertKeyFrameWithEasingFunction(p, v, e).map_err(w),
        None => anim.InsertKeyFrame(p, v).map_err(w),
    })
}

pub fn color(c: &Compositor, anim: &ColorKeyFrameAnimation, stops: &Stops<Color>, dur_ms: i64) -> R<()> {
    fill(c, stops, dur_ms, |p, v, e| match e {
        Some(e) => anim.InsertKeyFrameWithEasingFunction(p, v, e).map_err(w),
        None => anim.InsertKeyFrame(p, v).map_err(w),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_ends_and_linear() {
        let lin = Bezier(0.0, 0.0, 1.0, 1.0);
        for t in [0.0, 0.25, 0.5, 0.9, 1.0] {
            assert!((lin.at(t) - t).abs() < 1e-3);
        }
        let ease = Bezier(0.3, 0.6, 0.5, 1.0);
        assert!(ease.at(0.0).abs() < 1e-3 && (ease.at(1.0) - 1.0).abs() < 1e-3 && ease.at(0.5) > 0.5);
    }

    #[test]
    fn samples_between_stops() {
        let stops = [(0.0, 0.0f32, None), (0.5, 1.0, None), (1.0, 0.0, None)];
        assert!((sample(&stops, 0.25) - 0.5).abs() < 1e-6);
        assert!((sample(&stops, 0.75) - 0.5).abs() < 1e-6);
        assert_eq!(sample(&stops, 1.0), 0.0);
    }
}
