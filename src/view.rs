//! Canvas viewport: maps image pixels to screen points.
//!
//! Annotations, hit testing and export always work in image space; zooming and
//! panning only change this transform, never the document.

use egui::{Pos2, Rect, Vec2};

/// Empty margin kept around the image when fitting it to the canvas.
pub const FIT_PAD: f32 = 24.0;
/// Lowest zoom the controls step down to (25%), unless fitting needs less.
pub const MIN_ZOOM: f32 = 0.25;
/// Highest zoom (800%).
pub const MAX_ZOOM: f32 = 8.0;
/// Zoom levels offered by the zoom buttons and menu.
pub const ZOOM_STEPS: [f32; 11] = [0.25, 0.33, 0.5, 0.67, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];

/// Zoom is expressed in physical pixels: at 1.0 one image pixel covers one
/// screen pixel, so a Retina screenshot shows at its real size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Follow the window: recompute the fitted zoom and recentre every frame.
    pub fit: bool,
    /// Physical screen pixels per image pixel (1.0 = 100%).
    pub zoom: f32,
    /// Image-space point shown at the centre of the canvas.
    pub center: Pos2,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { fit: true, zoom: 1.0, center: Pos2::ZERO }
    }
}

/// The resolved transform for one frame: `screen = origin + image * scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Screen points per image pixel.
    pub scale: f32,
    pub origin: Vec2,
}

impl Transform {
    pub fn to_screen(self, p: Pos2) -> Pos2 {
        Pos2::new(self.origin.x + p.x * self.scale, self.origin.y + p.y * self.scale)
    }

    pub fn to_image(self, p: Pos2) -> Pos2 {
        Pos2::new((p.x - self.origin.x) / self.scale, (p.y - self.origin.y) / self.scale)
    }
}

/// Zoom that fits `img` inside `canvas` with padding, never enlarging the image
/// beyond 100%.
pub fn fit_zoom(canvas: Rect, img: Vec2, ppp: f32) -> f32 {
    let avail_w = (canvas.width() - 2.0 * FIT_PAD).max(1.0);
    let avail_h = (canvas.height() - 2.0 * FIT_PAD).max(1.0);
    let points_per_px = (avail_w / img.x).min(avail_h / img.y);
    (points_per_px * ppp).clamp(1e-3, 1.0)
}

/// Lowest zoom allowed for this image: 25%, or less when that is what fitting
/// a very large image needs.
pub fn min_zoom(canvas: Rect, img: Vec2, ppp: f32) -> f32 {
    MIN_ZOOM.min(fit_zoom(canvas, img, ppp))
}

impl Viewport {
    /// Resolve the transform for this frame. In fit mode this also updates the
    /// stored zoom/centre so a later manual zoom or pan starts from what is on
    /// screen.
    pub fn resolve(&mut self, canvas: Rect, img: Vec2, ppp: f32) -> Transform {
        if self.fit {
            self.zoom = fit_zoom(canvas, img, ppp);
            self.center = (img / 2.0).to_pos2();
        }
        self.transform(canvas, img, ppp)
    }

    /// The zoom on screen for this canvas: the fitted zoom in fit mode.
    pub fn effective_zoom(&self, canvas: Rect, img: Vec2, ppp: f32) -> f32 {
        if self.fit {
            fit_zoom(canvas, img, ppp)
        } else {
            self.zoom
        }
    }

    /// The transform for this canvas, without updating the stored state.
    pub fn transform(&self, canvas: Rect, img: Vec2, ppp: f32) -> Transform {
        let center = if self.fit { (img / 2.0).to_pos2() } else { self.center };
        let scale = self.effective_zoom(canvas, img, ppp) / ppp;
        let origin = canvas.center().to_vec2() - center.to_vec2() * scale;
        // Snap to the physical pixel grid so 100% stays crisp.
        let origin = (origin * ppp).round() / ppp;
        Transform { scale, origin }
    }

    pub fn fit(&mut self) {
        self.fit = true;
    }

    /// Set the zoom, keeping the image point under `anchor` (a screen position
    /// inside `canvas`) fixed. Pass the canvas centre to zoom around the middle.
    pub fn zoom_to(&mut self, zoom: f32, anchor: Pos2, canvas: Rect, img: Vec2, ppp: f32) {
        let current = self.resolve(canvas, img, ppp);
        let fixed = current.to_image(anchor);
        let zoom = zoom.clamp(min_zoom(canvas, img, ppp), MAX_ZOOM);
        let scale = zoom / ppp;
        self.fit = false;
        self.zoom = zoom;
        self.center = fixed - (anchor - canvas.center()) / scale;
        self.clamp_center(img);
    }

    /// Next preset step above (or below) the current zoom.
    pub fn step(&self, up: bool, canvas: Rect, img: Vec2, ppp: f32) -> f32 {
        let z = self.effective_zoom(canvas, img, ppp);
        let next = if up {
            ZOOM_STEPS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(MAX_ZOOM)
        } else {
            ZOOM_STEPS.iter().rev().copied().find(|s| *s < z * 0.999).unwrap_or(0.0)
        };
        next.clamp(min_zoom(canvas, img, ppp), MAX_ZOOM)
    }

    /// Pan by a screen-space delta (points).
    pub fn pan(&mut self, delta: Vec2, canvas: Rect, img: Vec2, ppp: f32) {
        let current = self.resolve(canvas, img, ppp);
        self.fit = false;
        self.center -= delta / current.scale;
        self.clamp_center(img);
    }

    /// Keep the canvas centre over the image so it can never be panned away.
    fn clamp_center(&mut self, img: Vec2) {
        self.center = Pos2::new(self.center.x.clamp(0.0, img.x), self.center.y.clamp(0.0, img.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 80.0), Vec2::new(1000.0, 600.0))
    }

    #[test]
    fn fit_never_upscales() {
        let small = Vec2::new(200.0, 100.0);
        assert_eq!(fit_zoom(canvas(), small, 1.0), 1.0);
        assert_eq!(fit_zoom(canvas(), small, 2.0), 1.0);
        // Large images shrink to the padded canvas.
        let big = Vec2::new(4000.0, 1000.0);
        let z = fit_zoom(canvas(), big, 1.0);
        assert!((z - (1000.0 - 2.0 * FIT_PAD) / 4000.0).abs() < 1e-6);
        // On a 2x screen the same fit is twice as many physical pixels.
        assert!((fit_zoom(canvas(), big, 2.0) - 2.0 * z).abs() < 1e-6);
    }

    #[test]
    fn transform_matches_resolve_without_mutating() {
        let big = Vec2::new(4000.0, 1000.0);
        let v = Viewport::default();
        let t = v.transform(canvas(), big, 1.0);
        assert_eq!(v, Viewport::default(), "transform leaves the viewport alone");
        assert_eq!(v.effective_zoom(canvas(), big, 1.0), fit_zoom(canvas(), big, 1.0));
        let mut resolved = v;
        assert_eq!(resolved.resolve(canvas(), big, 1.0), t);
    }

    #[test]
    fn fit_centres_the_image() {
        let img = Vec2::new(400.0, 300.0);
        let mut v = Viewport::default();
        let t = v.resolve(canvas(), img, 1.0);
        assert_eq!(t.scale, 1.0);
        let centre = t.to_screen(Pos2::new(200.0, 150.0));
        assert!((centre - canvas().center()).length() < 0.51);
    }

    #[test]
    fn hundred_percent_maps_image_pixels_to_physical_pixels() {
        let img = Vec2::new(4000.0, 3000.0);
        let mut v = Viewport::default();
        v.zoom_to(1.0, canvas().center(), canvas(), img, 2.0);
        let t = v.resolve(canvas(), img, 2.0);
        assert_eq!(t.scale, 0.5);
    }

    #[test]
    fn screen_image_round_trip() {
        let img = Vec2::new(1920.0, 1080.0);
        let mut v = Viewport::default();
        v.zoom_to(2.5, Pos2::new(300.0, 200.0), canvas(), img, 2.0);
        let t = v.resolve(canvas(), img, 2.0);
        for p in [Pos2::new(0.0, 0.0), Pos2::new(123.4, 567.8), Pos2::new(1920.0, 1080.0)] {
            assert!((t.to_image(t.to_screen(p)) - p).length() < 1e-3);
        }
    }

    #[test]
    fn zoom_keeps_the_anchor_point_fixed() {
        let img = Vec2::new(1920.0, 1080.0);
        let mut v = Viewport::default();
        let anchor = Pos2::new(420.0, 300.0);
        let before = v.resolve(canvas(), img, 1.0).to_image(anchor);
        v.zoom_to(2.0, anchor, canvas(), img, 1.0);
        let after = v.resolve(canvas(), img, 1.0).to_image(anchor);
        // Pixel snapping of the origin may shift by under one screen point.
        assert!((before - after).length() < 1.0, "{before:?} vs {after:?}");
        assert!(!v.fit);
    }

    #[test]
    fn zoom_is_clamped() {
        let img = Vec2::new(400.0, 300.0);
        let mut v = Viewport::default();
        v.zoom_to(100.0, canvas().center(), canvas(), img, 1.0);
        assert_eq!(v.zoom, MAX_ZOOM);
        v.zoom_to(0.01, canvas().center(), canvas(), img, 1.0);
        assert_eq!(v.zoom, MIN_ZOOM);
        // A huge image may go below 25% so that it can still be fitted.
        let huge = Vec2::new(20_000.0, 10_000.0);
        v.zoom_to(0.01, canvas().center(), canvas(), huge, 1.0);
        assert!((v.zoom - fit_zoom(canvas(), huge, 1.0)).abs() < 1e-6);
    }

    #[test]
    fn steps_follow_presets() {
        let img = Vec2::new(400.0, 300.0);
        let v = Viewport { fit: false, zoom: 1.0, center: Pos2::ZERO };
        assert_eq!(v.step(true, canvas(), img, 1.0), 1.5);
        assert_eq!(v.step(false, canvas(), img, 1.0), 0.67);
        let v = Viewport { fit: false, zoom: 0.8, center: Pos2::ZERO };
        assert_eq!(v.step(true, canvas(), img, 1.0), 1.0);
        let v = Viewport { fit: false, zoom: MIN_ZOOM, center: Pos2::ZERO };
        assert_eq!(v.step(false, canvas(), img, 1.0), MIN_ZOOM);
    }

    #[test]
    fn pan_cannot_lose_the_image() {
        let img = Vec2::new(400.0, 300.0);
        let mut v = Viewport::default();
        v.pan(Vec2::new(5000.0, -5000.0), canvas(), img, 1.0);
        assert_eq!(v.center, Pos2::new(0.0, 300.0));
        assert!(!v.fit);
    }
}
