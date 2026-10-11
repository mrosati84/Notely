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

/// A committed annotation. All coordinates are in image pixel space.
#[derive(Clone)]
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

    /// Approximate bounding box in image space, used for the selection outline.
    pub fn bounds(&self, font: Option<&FontRef<'static>>) -> Rect {
        match self {
            Annotation::Arrow { from, to, width, .. } => {
                Rect::from_two_pos(*from, *to).expand(width * 0.5 + 1.0)
            }
            Annotation::Rect { rect, .. } | Annotation::Ellipse { rect, .. } => *rect,
            Annotation::Highlight { points, width, .. } => {
                points_bounds(points).expand(width * 0.5 + 1.0)
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

        let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).ok();
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
}
