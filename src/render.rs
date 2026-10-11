use ab_glyph::{Font, FontRef, OutlineCurve, PxScale, ScaleFont};
use egui::{Pos2, Rect};
use image::{imageops, RgbaImage};
use tiny_skia::{
    Color, FillRule, IntSize, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::annotation::Annotation;

/// Render the base image together with every annotation into a new RGBA image.
pub fn render(base: &RgbaImage, annotations: &[Annotation], font: Option<&FontRef<'static>>) -> RgbaImage {
    let mut buf = base.clone();

    // 1. Pixel-level effects first: blur and spotlight.
    for a in annotations {
        match a {
            Annotation::Blur { rect, sigma } => apply_blur(&mut buf, *rect, *sigma),
            Annotation::Spot { center, radius } => apply_spot(&mut buf, *center, *radius),
            _ => {}
        }
    }

    let mut pixmap = match image_to_pixmap(&buf) {
        Some(p) => p,
        None => return buf,
    };

    // 2. Highlighter strokes go underneath the crisp vector shapes.
    for a in annotations {
        if let Annotation::Highlight { points, color, width } = a {
            draw_highlight(&mut pixmap, points, *color, *width);
        }
    }

    // 3. Vector shapes and text.
    for a in annotations {
        match a {
            Annotation::Arrow { from, to, color, width } => {
                draw_arrow(&mut pixmap, *from, *to, *color, *width);
            }
            Annotation::Rect { rect, color, width } => draw_rect(&mut pixmap, *rect, *color, *width),
            Annotation::Ellipse { rect, color, width } => {
                draw_ellipse(&mut pixmap, *rect, *color, *width);
            }
            Annotation::Text { pos, text, color, size } => {
                if let Some(font) = font {
                    draw_text(&mut pixmap, font, *pos, text, *color, *size);
                }
            }
            _ => {}
        }
    }

    pixmap_to_image(&pixmap)
}

// ---------------------------------------------------------------------------
// Pixel-level effects
// ---------------------------------------------------------------------------

fn apply_blur(buf: &mut RgbaImage, rect: Rect, sigma: f32) {
    let (x, y, w, h) = clip_rect(buf, rect);
    if w == 0 || h == 0 {
        return;
    }
    let sub = imageops::crop_imm(buf, x, y, w, h).to_image();
    let blurred = imageops::blur(&sub, sigma.max(0.5));
    imageops::replace(buf, &blurred, x as i64, y as i64);
}

fn apply_spot(buf: &mut RgbaImage, center: Pos2, radius: f32) {
    let (w, h) = buf.dimensions();
    if radius <= 0.0 {
        return;
    }
    let feather = (radius * 0.35).max(1.0);
    let inner = radius;
    let outer = radius + feather;
    let dim = 0.30_f32;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 + 0.5 - center.x;
            let dy = y as f32 + 0.5 - center.y;
            let d = (dx * dx + dy * dy).sqrt();
            if d <= inner {
                continue;
            }
            let t = if d >= outer { 1.0 } else { (d - inner) / feather };
            let f = 1.0 - (1.0 - dim) * t;
            let px = buf.get_pixel_mut(x, y);
            px[0] = (px[0] as f32 * f) as u8;
            px[1] = (px[1] as f32 * f) as u8;
            px[2] = (px[2] as f32 * f) as u8;
        }
    }
}

fn clip_rect(buf: &RgbaImage, rect: Rect) -> (u32, u32, u32, u32) {
    let (iw, ih) = buf.dimensions();
    let x0 = rect.min.x.floor().clamp(0.0, iw as f32) as u32;
    let y0 = rect.min.y.floor().clamp(0.0, ih as f32) as u32;
    let x1 = rect.max.x.ceil().clamp(0.0, iw as f32) as u32;
    let y1 = rect.max.y.ceil().clamp(0.0, ih as f32) as u32;
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

// ---------------------------------------------------------------------------
// Vector drawing
// ---------------------------------------------------------------------------

fn paint_rgba(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(c[0], c[1], c[2], c[3]));
    p.anti_alias = true;
    p
}

fn stroke_of(width: f32) -> Stroke {
    Stroke {
        width: width.max(0.5),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

fn ts_rect(r: Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x, r.max.y)
}

fn draw_rect(pm: &mut Pixmap, rect: Rect, color: [u8; 4], width: f32) {
    if let Some(r) = ts_rect(rect) {
        let path = PathBuilder::from_rect(r);
        pm.stroke_path(&path, &paint_rgba(color), &stroke_of(width), Transform::identity(), None);
    }
}

fn draw_ellipse(pm: &mut Pixmap, rect: Rect, color: [u8; 4], width: f32) {
    if let Some(r) = ts_rect(rect) {
        if let Some(path) = PathBuilder::from_oval(r) {
            pm.stroke_path(&path, &paint_rgba(color), &stroke_of(width), Transform::identity(), None);
        }
    }
}

fn draw_arrow(pm: &mut Pixmap, from: Pos2, to: Pos2, color: [u8; 4], width: f32) {
    if let Some(path) = {
        let mut pb = PathBuilder::new();
        pb.move_to(from.x, from.y);
        pb.line_to(to.x, to.y);
        pb.finish()
    } {
        pm.stroke_path(&path, &paint_rgba(color), &stroke_of(width), Transform::identity(), None);
    }

    let dir = to - from;
    let len = dir.length();
    if len < 1.0 {
        return;
    }
    let dir = dir / len;
    // `f32::clamp` panics when the minimum exceeds the maximum; short arrows
    // (len * 0.6 < 10) would invert the bounds. Compose max/min instead.
    let head = (width * 4.5).max(10.0).min(len * 0.6);
    let perp = egui::vec2(-dir.y, dir.x);
    let base = to - dir * head;
    let left = base + perp * (head * 0.45);
    let right = base - perp * (head * 0.45);

    if let Some(path) = {
        let mut pb = PathBuilder::new();
        pb.move_to(to.x, to.y);
        pb.line_to(left.x, left.y);
        pb.line_to(right.x, right.y);
        pb.close();
        pb.finish()
    } {
        pm.fill_path(&path, &paint_rgba(color), FillRule::Winding, Transform::identity(), None);
    }
}

fn draw_highlight(pm: &mut Pixmap, points: &[Pos2], color: [u8; 4], width: f32) {
    if points.len() < 2 {
        return;
    }
    let path = {
        let mut pb = PathBuilder::new();
        pb.move_to(points[0].x, points[0].y);
        for p in &points[1..] {
            pb.line_to(p.x, p.y);
        }
        pb.finish()
    };
    if let Some(path) = path {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(color[0], color[1], color[2], 90));
        paint.anti_alias = true;
        pm.stroke_path(&path, &paint, &stroke_of(width.max(6.0)), Transform::identity(), None);
    }
}

// ---------------------------------------------------------------------------
// Text (glyph outlines rasterised with tiny-skia)
// ---------------------------------------------------------------------------

fn draw_text(pm: &mut Pixmap, font: &FontRef<'static>, pos: Pos2, text: &str, color: [u8; 4], size: f32) {
    let scaled = font.as_scaled(PxScale::from(size.max(1.0)));
    let sf = scaled.scale_factor();
    let line_height = (scaled.ascent() - scaled.descent() + scaled.line_gap()).max(size * 1.2);
    let paint = paint_rgba(color);

    let mut baseline = pos.y + scaled.ascent();
    for line in text.split('\n') {
        let mut pen_x = pos.x;
        for ch in line.chars() {
            let gid = font.glyph_id(ch);
            if let Some(outline) = font.outline(gid) {
                if let Some(path) = outline_to_path(&outline, pen_x, baseline, sf.horizontal, sf.vertical)
                {
                    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
            pen_x += scaled.h_advance(gid);
        }
        baseline += line_height;
    }
}

enum Seg {
    Line(ab_glyph::Point),
    Quad(ab_glyph::Point, ab_glyph::Point),
    Cubic(ab_glyph::Point, ab_glyph::Point, ab_glyph::Point),
}

fn outline_to_path(
    outline: &ab_glyph::Outline,
    ox: f32,
    oy: f32,
    sx: f32,
    sy: f32,
) -> Option<tiny_skia::Path> {
    let map = |p: ab_glyph::Point| (ox + p.x * sx, oy - p.y * sy);
    let mut pb = PathBuilder::new();
    let mut prev: Option<ab_glyph::Point> = None;

    for curve in &outline.curves {
        let (p0, seg) = match *curve {
            OutlineCurve::Line(p0, p1) => (p0, Seg::Line(p1)),
            OutlineCurve::Quad(p0, p1, p2) => (p0, Seg::Quad(p1, p2)),
            OutlineCurve::Cubic(p0, p1, p2, p3) => (p0, Seg::Cubic(p1, p2, p3)),
        };

        let (x0, y0) = map(p0);
        let new_contour = match prev {
            None => true,
            Some(e) => (e.x - p0.x).abs() > 1e-4 || (e.y - p0.y).abs() > 1e-4,
        };
        if new_contour {
            pb.move_to(x0, y0);
        } else {
            pb.line_to(x0, y0);
        }

        match seg {
            Seg::Line(p1) => {
                let (x, y) = map(p1);
                pb.line_to(x, y);
                prev = Some(p1);
            }
            Seg::Quad(p1, p2) => {
                let (x1, y1) = map(p1);
                let (x, y) = map(p2);
                pb.quad_to(x1, y1, x, y);
                prev = Some(p2);
            }
            Seg::Cubic(p1, p2, p3) => {
                let (x1, y1) = map(p1);
                let (x2, y2) = map(p2);
                let (x, y) = map(p3);
                pb.cubic_to(x1, y1, x2, y2, x, y);
                prev = Some(p3);
            }
        }
    }
    pb.close();
    pb.finish()
}

// ---------------------------------------------------------------------------
// Conversions between `image::RgbaImage` (straight alpha) and `Pixmap`
// (premultiplied alpha).
// ---------------------------------------------------------------------------

fn image_to_pixmap(img: &RgbaImage) -> Option<Pixmap> {
    let (w, h) = img.dimensions();
    let mut data = img.as_raw().clone();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a != 255 {
            px[0] = (px[0] as u32 * a / 255) as u8;
            px[1] = (px[1] as u32 * a / 255) as u8;
            px[2] = (px[2] as u32 * a / 255) as u8;
        }
    }
    let size = IntSize::from_wh(w, h)?;
    Pixmap::from_vec(data, size)
}

fn pixmap_to_image(pm: &Pixmap) -> RgbaImage {
    let mut data = pm.data().to_vec();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
    RgbaImage::from_raw(pm.width(), pm.height(), data).expect("valid pixmap size")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::FontRef;
    use egui::{pos2, Rect};

    fn base() -> RgbaImage {
        let (w, h) = (520u32, 360u32);
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let r = (x * 255 / w) as u8;
                let g = (y * 255 / h) as u8;
                let b = ((x + y) % 256) as u8;
                img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
            }
        }
        img
    }

    #[test]
    fn renders_all_tools() {
        let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).ok();
        let anns = vec![
            Annotation::Rect { rect: Rect::from_min_max(pos2(40.0, 40.0), pos2(200.0, 150.0)), color: [255, 60, 60, 255], width: 6.0 },
            Annotation::Ellipse { rect: Rect::from_min_max(pos2(240.0, 40.0), pos2(400.0, 150.0)), color: [60, 220, 120, 255], width: 6.0 },
            Annotation::Arrow { from: pos2(60.0, 300.0), to: pos2(220.0, 200.0), color: [40, 120, 255, 255], width: 6.0 },
            Annotation::Highlight {
                points: (0..=40)
                    .map(|i| {
                        let x = 250.0 + i as f32 * 3.5;
                        let y = 300.0 + (i as f32 * 0.4).sin() * 12.0;
                        pos2(x, y)
                    })
                    .collect(),
                color: [255, 230, 40, 255],
                width: 18.0,
            },
            Annotation::Blur { rect: Rect::from_min_max(pos2(40.0, 180.0), pos2(200.0, 260.0)), sigma: 10.0 },
            Annotation::Spot { center: pos2(330.0, 280.0), radius: 55.0 },
            Annotation::Text { pos: pos2(60.0, 320.0), text: "Hello Notely".to_owned(), color: [255, 255, 255, 255], size: 30.0 },
        ];

        let out = render(&base(), &anns, font.as_ref());
        assert_eq!(out.dimensions(), (520, 360));
        out.save(std::env::temp_dir().join("notely_render_test.png")).unwrap();
    }

    #[test]
    fn renders_short_arrows_without_panicking() {
        for len in [1.0_f32, 2.0, 5.0, 10.0, 16.0, 16.7] {
            let anns = vec![Annotation::Arrow {
                from: pos2(50.0, 50.0),
                to: pos2(50.0 + len, 50.0),
                color: [0, 0, 0, 255],
                width: 4.0,
            }];
            let out = render(&base(), &anns, None);
            assert_eq!(out.dimensions(), (520, 360));
        }
    }
}
