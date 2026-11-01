//! A small set of line icons painted with egui shapes, so they stay sharp at
//! any scale and share one stroke weight. Coordinates use a 24-unit grid.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Open,
    Paste,
    Save,
    ChevronDown,
    Select,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Highlight,
    Blur,
    Spot,
    Undo,
    Redo,
    Copy,
    Trash,
    Eraser,
    Fit,
    Plus,
    Minus,
    More,
    Image,
    Check,
}

struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    unit: f32,
    stroke: Stroke,
}

impl Pen<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.unit
    }

    fn line(&self, pts: &[(f32, f32)]) {
        let pts = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::line(pts, self.stroke));
    }

    fn closed(&self, pts: &[(f32, f32)]) {
        let pts = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::closed_line(pts, self.stroke));
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        self.painter.rect_stroke(
            Rect::from_min_max(self.p(x0, y0), self.p(x1, y1)),
            r * self.unit,
            self.stroke,
            egui::StrokeKind::Middle,
        );
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.p(x, y), r * self.unit, self.stroke);
    }

    fn dot(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_filled(self.p(x, y), r * self.unit, self.stroke.color);
    }

    /// Polyline along a circular arc; angles in degrees, y pointing down.
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
        const STEPS: usize = 16;
        (0..=STEPS)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / STEPS as f32).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    }
}

/// Paint `icon` centred in `rect` (a square; its width sets the size).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let size = rect.width().min(rect.height());
    let unit = size / 24.0;
    let pen = Pen {
        painter,
        origin: rect.center() - Vec2::splat(size / 2.0),
        unit,
        stroke: Stroke::new((1.9 * unit).max(1.25), color),
    };
    match icon {
        Icon::Open => {
            pen.closed(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0)]);
        }
        Icon::Paste => {
            pen.rect(5.0, 4.5, 19.0, 21.0, 2.0);
            pen.rect(9.0, 2.5, 15.0, 6.5, 1.0);
        }
        Icon::Save => {
            pen.line(&[(12.0, 3.0), (12.0, 15.0)]);
            pen.line(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]);
            pen.line(&[(4.0, 16.0), (4.0, 20.5), (20.0, 20.5), (20.0, 16.0)]);
        }
        Icon::ChevronDown => pen.line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Icon::Select => {
            pen.closed(&[
                (6.0, 3.0),
                (6.0, 19.0),
                (10.0, 15.0),
                (13.0, 21.0),
                (15.5, 20.0),
                (12.5, 14.0),
                (18.0, 14.0),
            ]);
        }
        Icon::Arrow => {
            pen.line(&[(5.0, 19.0), (18.5, 5.5)]);
            pen.line(&[(9.0, 5.0), (19.0, 5.0), (19.0, 15.0)]);
        }
        Icon::Rect => pen.rect(3.5, 6.0, 20.5, 18.0, 1.5),
        Icon::Ellipse => {
            let pts = pen.arc(12.0, 12.0, 1.0, 0.0, 360.0);
            let pts: Vec<_> = pts.iter().map(|&(x, y)| (12.0 + (x - 12.0) * 9.0, 12.0 + (y - 12.0) * 7.0)).collect();
            pen.closed(&pts);
        }
        Icon::Text => {
            pen.line(&[(5.0, 7.0), (5.0, 4.5), (19.0, 4.5), (19.0, 7.0)]);
            pen.line(&[(12.0, 4.5), (12.0, 20.0)]);
            pen.line(&[(9.0, 20.0), (15.0, 20.0)]);
        }
        Icon::Highlight => {
            pen.closed(&[(14.0, 3.5), (20.5, 10.0), (11.0, 19.5), (4.5, 13.0)]);
            pen.line(&[(4.5, 13.0), (2.5, 19.5), (7.0, 19.0)]);
            pen.line(&[(13.0, 21.5), (21.5, 21.5)]);
        }
        Icon::Blur => {
            let mut pts = vec![(12.0, 2.5)];
            pts.extend(pen.arc(12.0, 15.0, 6.0, 210.0, -30.0));
            pen.closed(&pts);
        }
        Icon::Spot => {
            pen.rect(2.5, 4.5, 21.5, 19.5, 2.0);
            pen.circle(12.0, 12.0, 4.5);
            pen.dot(12.0, 12.0, 1.4);
        }
        Icon::Undo | Icon::Redo => {
            let flip = |pts: Vec<(f32, f32)>| -> Vec<(f32, f32)> {
                if icon == Icon::Redo {
                    pts.into_iter().map(|(x, y)| (24.0 - x, y)).collect()
                } else {
                    pts
                }
            };
            pen.line(&flip(vec![(9.0, 14.0), (4.0, 9.0), (9.0, 4.0)]));
            let mut path = vec![(4.0, 9.0)];
            path.extend(pen.arc(14.5, 14.5, 5.5, -90.0, 90.0));
            path.push((11.0, 20.0));
            pen.line(&flip(path));
        }
        Icon::Copy => {
            pen.rect(8.5, 8.5, 20.5, 20.5, 2.0);
            pen.line(&[(4.0, 15.5), (4.0, 5.5), (5.5, 4.0), (15.5, 4.0)]);
        }
        Icon::Trash => {
            pen.line(&[(3.5, 6.0), (20.5, 6.0)]);
            pen.line(&[(9.0, 6.0), (9.0, 3.5), (15.0, 3.5), (15.0, 6.0)]);
            pen.line(&[(5.5, 6.0), (6.5, 20.5), (17.5, 20.5), (18.5, 6.0)]);
            pen.line(&[(10.0, 10.5), (10.0, 16.5)]);
            pen.line(&[(14.0, 10.5), (14.0, 16.5)]);
        }
        Icon::Eraser => {
            pen.closed(&[(7.5, 20.5), (3.0, 16.0), (13.5, 5.5), (20.5, 12.5), (12.5, 20.5)]);
            pen.line(&[(8.0, 11.0), (15.0, 18.0)]);
            pen.line(&[(12.5, 20.5), (21.0, 20.5)]);
        }
        Icon::Fit => {
            pen.line(&[(3.5, 8.5), (3.5, 3.5), (8.5, 3.5)]);
            pen.line(&[(15.5, 3.5), (20.5, 3.5), (20.5, 8.5)]);
            pen.line(&[(20.5, 15.5), (20.5, 20.5), (15.5, 20.5)]);
            pen.line(&[(8.5, 20.5), (3.5, 20.5), (3.5, 15.5)]);
        }
        Icon::Plus => {
            pen.line(&[(5.0, 12.0), (19.0, 12.0)]);
            pen.line(&[(12.0, 5.0), (12.0, 19.0)]);
        }
        Icon::Minus => pen.line(&[(5.0, 12.0), (19.0, 12.0)]),
        Icon::More => {
            for x in [5.0, 12.0, 19.0] {
                pen.dot(x, 12.0, 1.7);
            }
        }
        Icon::Image => {
            pen.rect(3.0, 4.0, 21.0, 20.0, 2.0);
            pen.circle(8.5, 9.5, 1.8);
            pen.line(&[(3.5, 18.0), (9.5, 12.5), (13.5, 16.0), (16.0, 13.5), (20.5, 17.5)]);
        }
        Icon::Check => pen.line(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)]),
    }
}
