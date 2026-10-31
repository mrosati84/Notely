use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use egui::{vec2, Pos2, Rect, Vec2};

/// The annotation tools available in the toolbar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Arrow,
    Ellipse,
    Rect,
    Highlight,
    Blur,
    Spot,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Ellipse,
        Tool::Rect,
        Tool::Highlight,
        Tool::Blur,
        Tool::Spot,
        Tool::Text,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Arrow => "Arrow",
            Tool::Ellipse => "Ellipse",
            Tool::Rect => "Rect",
            Tool::Highlight => "Highlight",
            Tool::Blur => "Blur",
            Tool::Spot => "Spot",
            Tool::Text => "Text",
        }
    }
}

/// A corner resize handle of the selection box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Handle {
    pub const ALL: [Handle; 4] =
        [Handle::TopLeft, Handle::TopRight, Handle::BottomLeft, Handle::BottomRight];

    /// Position of this handle on `r`.
    pub fn pos(self, r: Rect) -> Pos2 {
        match self {
            Handle::TopLeft => r.left_top(),
            Handle::TopRight => r.right_top(),
            Handle::BottomLeft => r.left_bottom(),
            Handle::BottomRight => r.right_bottom(),
        }
    }

    /// The diagonally opposite handle, which stays fixed while resizing.
    pub fn opposite(self) -> Handle {
        match self {
            Handle::TopLeft => Handle::BottomRight,
            Handle::TopRight => Handle::BottomLeft,
            Handle::BottomLeft => Handle::TopRight,
            Handle::BottomRight => Handle::TopLeft,
        }
    }
}

/// Smallest extent (image pixels) a resize may shrink a selection box to.
const MIN_RESIZE_EXTENT: f32 = 2.0;

/// A committed annotation. All coordinates are in image pixel space.
#[derive(Clone, PartialEq)]
pub enum Annotation {
    Arrow {
        from: Pos2,
        to: Pos2,
        color: [u8; 4],
        width: f32,
    },
    Rect {
        rect: Rect,
        color: [u8; 4],
        width: f32,
    },
    Ellipse {
        rect: Rect,
        color: [u8; 4],
        width: f32,
    },
    Highlight {
        points: Vec<Pos2>,
        color: [u8; 4],
        width: f32,
    },
    Blur {
        rect: Rect,
        sigma: f32,
    },
    Spot {
        center: Pos2,
        radius: f32,
    },
    Text {
        pos: Pos2,
        text: String,
        color: [u8; 4],
        size: f32,
    },
}

impl Annotation {
    /// Move the annotation by `delta` (image pixels).
    pub fn translate(&mut self, delta: Vec2) {
        match self {
            Annotation::Arrow { from, to, .. } => {
                *from += delta;
                *to += delta;
            }
            Annotation::Rect { rect, .. } | Annotation::Ellipse { rect, .. } => {
                *rect = rect.translate(delta);
            }
            Annotation::Highlight { points, .. } => {
                for p in points.iter_mut() {
                    *p += delta;
                }
            }
            Annotation::Blur { rect, .. } => {
                *rect = rect.translate(delta);
            }
            Annotation::Spot { center, .. } => {
                *center += delta;
            }
            Annotation::Text { pos, .. } => {
                *pos += delta;
            }
        }
    }

    /// Resize the annotation by dragging `handle` of its selection box `bounds`
    /// by `delta` (image pixels), keeping the opposite corner fixed. Dragging
    /// past the opposite corner mirrors the shape. Spotlights and text scale
    /// uniformly so they keep their proportions.
    pub fn resize(&mut self, bounds: Rect, handle: Handle, delta: Vec2) {
        // Scale the geometry itself, not the stroke padding included in `bounds`.
        let bounds = bounds.shrink(self.bounds_pad());
        let anchor = handle.opposite().pos(bounds);
        let old = handle.pos(bounds) - anchor;
        let new = old + delta;
        let clamp = |n: f32, o: f32| {
            if n.abs() >= MIN_RESIZE_EXTENT {
                n
            } else {
                MIN_RESIZE_EXTENT.copysign(if n != 0.0 { n } else { o })
            }
        };
        let new = vec2(clamp(new.x, old.x), clamp(new.y, old.y));
        let ratio = |n: f32, o: f32| if o.abs() > f32::EPSILON { n / o } else { 1.0 };
        let scale = vec2(ratio(new.x, old.x), ratio(new.y, old.y));
        let map = |p: Pos2| anchor + (p - anchor) * scale;
        // Uniform factor (with per-axis mirroring) for shapes that keep their
        // aspect ratio; returns the resulting selection frame.
        let uniform_frame = |u: f32| {
            let corner = anchor + old * vec2(u.copysign(scale.x), u.copysign(scale.y));
            Rect::from_two_pos(anchor, corner)
        };
        // Follow the axis that changed most, not the larger absolute scale.
        // The unchanged axis is always 1.0, so a plain `max` would prevent a
        // text/spot from being shrunk by dragging a single axis inwards.
        let changed_x = (scale.x.abs() - 1.0).abs();
        let changed_y = (scale.y.abs() - 1.0).abs();
        let uniform =
            if changed_x >= changed_y { scale.x.abs() } else { scale.y.abs() };

        match self {
            Annotation::Arrow { from, to, .. } => {
                *from = map(*from);
                *to = map(*to);
            }
            Annotation::Rect { rect, .. }
            | Annotation::Ellipse { rect, .. }
            | Annotation::Blur { rect, .. } => {
                *rect = Rect::from_two_pos(map(rect.min), map(rect.max));
            }
            Annotation::Highlight { points, .. } => {
                for p in points.iter_mut() {
                    *p = map(*p);
                }
            }
            Annotation::Spot { center, radius } => {
                let u = uniform.max(2.0 / radius.max(f32::EPSILON));
                let frame = uniform_frame(u);
                *center = frame.center();
                *radius *= u;
            }
            Annotation::Text { pos, size, .. } => {
                let u = uniform.max(4.0 / size.max(f32::EPSILON));
                *pos = uniform_frame(u).min;
                *size *= u;
            }
        }
    }

    /// Padding `bounds` adds around stroked point geometry.
    fn bounds_pad(&self) -> f32 {
        match self {
            Annotation::Arrow { width, .. } | Annotation::Highlight { width, .. } => {
                width * 0.5 + 1.0
            }
            _ => 0.0,
        }
    }

    /// Approximate bounding box in image space, used for the selection outline.
    pub fn bounds(&self, font: Option<&FontRef<'static>>) -> Rect {
        match self {
            Annotation::Arrow { from, to, .. } => {
                Rect::from_two_pos(*from, *to).expand(self.bounds_pad())
            }
            Annotation::Rect { rect, .. } | Annotation::Ellipse { rect, .. } => *rect,
            Annotation::Highlight { points, .. } => {
                points_bounds(points).expand(self.bounds_pad())
            }
            Annotation::Blur { rect, .. } => *rect,
            Annotation::Spot { center, radius } => {
                Rect::from_center_size(*center, vec2(radius * 2.0, radius * 2.0))
            }
            Annotation::Text { pos, text, size, .. } => match font {
                Some(font) => text_bounds(font, *pos, text, *size),
                None => Rect::from_min_size(*pos, vec2(size * 0.5 * text.chars().count() as f32, size * 1.2)),
            },
        }
    }

    /// Whether `p` (image space) is on/near the annotation, within `tol` pixels.
    pub fn hit(&self, p: Pos2, tol: f32, font: Option<&FontRef<'static>>) -> bool {
        match self {
            Annotation::Arrow { from, to, width, .. } => {
                dist_to_segment(p, *from, *to) <= tol + width * 0.5 + 2.0
            }
            Annotation::Rect { rect, .. } => rect.expand(tol).contains(p),
            Annotation::Ellipse { rect, .. } => {
                let c = rect.center();
                let rx = rect.width() * 0.5 + tol;
                let ry = rect.height() * 0.5 + tol;
                if rx <= 0.0 || ry <= 0.0 {
                    false
                } else {
                    let dx = (p.x - c.x) / rx;
                    let dy = (p.y - c.y) / ry;
                    dx * dx + dy * dy <= 1.0
                }
            }
            Annotation::Highlight { points, width, .. } => {
                let r = tol + width * 0.5 + 2.0;
                points.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= r)
                    || points.iter().any(|q| (*q - p).length() <= r)
            }
            Annotation::Blur { rect, .. } => rect.expand(tol).contains(p),
            Annotation::Spot { center, radius } => (*center - p).length() <= radius + tol,
            Annotation::Text { .. } => self.bounds(font).expand(tol).contains(p),
        }
    }
}

fn points_bounds(points: &[Pos2]) -> Rect {
    let mut rect = Rect::NOTHING;
    for p in points {
        rect.extend_with(*p);
    }
    rect
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn text_bounds(font: &FontRef<'static>, pos: Pos2, text: &str, size: f32) -> Rect {
    let scaled = font.as_scaled(PxScale::from(size.max(1.0)));
    let line_height = (scaled.ascent() - scaled.descent() + scaled.line_gap()).max(size * 1.2);
    let mut width = 0.0_f32;
    let mut lines = 0;
    for line in text.split('\n') {
        let mut w = 0.0_f32;
        for ch in line.chars() {
            w += scaled.h_advance(font.glyph_id(ch));
        }
        width = width.max(w);
        lines += 1;
    }
    let height = line_height * lines.max(1) as f32;
    Rect::from_min_size(pos, vec2(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::FontRef;

    #[test]
    fn translate_and_hit() {
        let mut rect = Annotation::Rect {
            rect: Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(50.0, 50.0)),
            color: [255, 0, 0, 255],
            width: 4.0,
        };
        assert!(rect.hit(Pos2::new(30.0, 30.0), 2.0, None));
        assert!(!rect.hit(Pos2::new(200.0, 200.0), 2.0, None));
        rect.translate(egui::vec2(5.0, 7.0));
        assert!(rect.hit(Pos2::new(35.0, 37.0), 2.0, None));
        assert!(!rect.hit(Pos2::new(11.0, 11.0), 2.0, None));

        let arrow = Annotation::Arrow {
            from: Pos2::new(0.0, 0.0),
            to: Pos2::new(100.0, 0.0),
            color: [0, 0, 0, 255],
            width: 4.0,
        };
        assert!(arrow.hit(Pos2::new(50.0, 3.0), 2.0, None));
        assert!(!arrow.hit(Pos2::new(50.0, 40.0), 2.0, None));

        let ellipse = Annotation::Ellipse {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 50.0)),
            color: [0, 0, 0, 255],
            width: 4.0,
        };
        assert!(ellipse.hit(Pos2::new(50.0, 25.0), 1.0, None));
        assert!(!ellipse.hit(Pos2::new(98.0, 48.0), 1.0, None));

        let font = FontRef::try_from_slice(crate::fonts::INTER_REGULAR).ok();
        assert!(font.is_some());
        let text = Annotation::Text {
            pos: Pos2::new(0.0, 0.0),
            text: "hello".to_owned(),
            color: [0, 0, 0, 255],
            size: 40.0,
        };
        let b = text.bounds(font.as_ref());
        assert!(b.width() > 20.0 && b.height() > 0.0);
        assert!(text.hit(b.center(), 0.0, font.as_ref()));
        assert!(!text.hit(Pos2::new(1000.0, 1000.0), 0.0, font.as_ref()));
    }

    #[test]
    fn resize_keeps_opposite_corner() {
        let r = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(50.0, 30.0));
        let mut rect = Annotation::Rect { rect: r, color: [0, 0, 0, 255], width: 2.0 };
        rect.resize(r, Handle::BottomRight, egui::vec2(40.0, 20.0));
        let b = rect.bounds(None);
        assert_eq!(b.min, Pos2::new(10.0, 10.0));
        assert_eq!(b.max, Pos2::new(90.0, 50.0));

        // Dragging past the anchor mirrors instead of inverting the rect.
        let mut rect = Annotation::Rect { rect: r, color: [0, 0, 0, 255], width: 2.0 };
        rect.resize(r, Handle::TopLeft, egui::vec2(60.0, 0.0));
        let b = rect.bounds(None);
        assert_eq!(b.min, Pos2::new(50.0, 10.0));
        assert_eq!(b.max, Pos2::new(70.0, 30.0));

        let mut arrow = Annotation::Arrow {
            from: Pos2::new(0.0, 100.0),
            to: Pos2::new(100.0, 0.0),
            color: [0, 0, 0, 255],
            width: 4.0,
        };
        let b = arrow.bounds(None);
        arrow.resize(b, Handle::TopRight, egui::vec2(b.width(), 0.0));
        let Annotation::Arrow { from, to, .. } = arrow else { unreachable!() };
        assert!((from - Pos2::new(0.0, 100.0)).length() < 1e-3);
        assert!((to.x - 206.0).abs() < 1e-3 && to.y.abs() < 1e-3);

        let mut spot = Annotation::Spot { center: Pos2::new(50.0, 50.0), radius: 10.0 };
        let b = spot.bounds(None);
        spot.resize(b, Handle::BottomRight, egui::vec2(20.0, 0.0));
        let Annotation::Spot { center, radius } = spot else { unreachable!() };
        assert!((radius - 20.0).abs() < 1e-3);
        assert!((center - Pos2::new(60.0, 60.0)).length() < 1e-3);
    }

    #[test]
    fn uniform_shapes_shrink_on_one_axis() {
        // A strictly horizontal inward drag must still shrink a spotlight.
        let mut spot = Annotation::Spot { center: Pos2::new(50.0, 50.0), radius: 10.0 };
        let b = spot.bounds(None);
        spot.resize(b, Handle::BottomRight, egui::vec2(-10.0, 0.0));
        let Annotation::Spot { center, radius } = spot else { unreachable!() };
        assert!((radius - 5.0).abs() < 1e-3, "radius was {radius}");
        assert!((center - Pos2::new(45.0, 45.0)).length() < 1e-3, "center was {center:?}");

        // Same for text: pulling the right handle left shrinks it uniformly.
        let mut text = Annotation::Text {
            pos: Pos2::new(0.0, 0.0),
            text: "hello".to_owned(),
            color: [0, 0, 0, 255],
            size: 40.0,
        };
        let b = text.bounds(None);
        text.resize(b, Handle::TopRight, egui::vec2(-10.0, 0.0));
        let Annotation::Text { pos, size, .. } = text else { unreachable!() };
        assert!((size - 36.0).abs() < 1e-3, "size was {size}");
        assert!((pos.x - 0.0).abs() < 1e-3 && (pos.y - 4.8).abs() < 1e-3, "pos was {pos:?}");

        // A strictly vertical inward drag works too.
        let mut text = Annotation::Text {
            pos: Pos2::new(0.0, 0.0),
            text: "hello".to_owned(),
            color: [0, 0, 0, 255],
            size: 40.0,
        };
        let b = text.bounds(None);
        text.resize(b, Handle::BottomRight, egui::vec2(0.0, -4.8));
        let Annotation::Text { size, .. } = text else { unreachable!() };
        assert!((size - 36.0).abs() < 1e-3, "size was {size}");
    }
}
