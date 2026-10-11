//! Generates `icons/icon.png` (1024x1024), the source for the macOS `.icns`
//! and the runtime window icon. Run with: `cargo run --release --example make_icon`

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, SpreadMode, Stroke, Transform,
};

const SIZE: u32 = 1024;

fn main() {
    std::fs::create_dir_all("icons").expect("create icons dir");
    let img = render_icon();
    img.save("icons/icon.png").expect("save icons/icon.png");
    println!("wrote icons/icon.png ({}x{})", img.width(), img.height());
}

fn render_icon() -> image::RgbaImage {
    let mut pixmap = Pixmap::new(SIZE, SIZE).expect("pixmap");

    // Rounded-square background with a diagonal gradient.
    let bg = round_rect_path(64.0, 64.0, 896.0, 896.0, 210.0);
    let gradient = LinearGradient::new(
        tiny_skia::Point::from_xy(64.0, 64.0),
        tiny_skia::Point::from_xy(960.0, 960.0),
        vec![
            GradientStop::new(0.0, Color::from_rgba8(59, 107, 255, 255)),
            GradientStop::new(1.0, Color::from_rgba8(138, 43, 226, 255)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .expect("gradient");
    let mut bg_paint = Paint::default();
    bg_paint.shader = gradient;
    bg_paint.anti_alias = true;
    pixmap.fill_path(&bg, &bg_paint, FillRule::Winding, Transform::identity(), None);

    // A soft white highlight swipe behind the arrow.
    let mut swipe = PathBuilder::new();
    swipe.move_to(300.0, 636.0);
    swipe.line_to(760.0, 420.0);
    if let Some(path) = swipe.finish() {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 70));
        paint.anti_alias = true;
        pixmap.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: 120.0,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    // The white annotation arrow.
    let from = (300.0, 720.0);
    let to = (740.0, 300.0);
    let mut arrow = PathBuilder::new();
    arrow.move_to(from.0, from.1);
    arrow.line_to(to.0, to.1);
    if let Some(path) = arrow.finish() {
        let mut paint = Paint::default();
        paint.set_color(Color::WHITE);
        paint.anti_alias = true;
        pixmap.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: 72.0,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    // Arrow head.
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = (dx / len, dy / len);
    let head = 240.0;
    let base = (to.0 - ux * head, to.1 - uy * head);
    let half = 120.0;
    let (px, py) = (-uy, ux);
    if let Some(path) = {
        let mut pb = PathBuilder::new();
        pb.move_to(to.0, to.1);
        pb.line_to(base.0 + px * half, base.1 + py * half);
        pb.line_to(base.0 - px * half, base.1 - py * half);
        pb.close();
        pb.finish()
    } {
        let mut paint = Paint::default();
        paint.set_color(Color::WHITE);
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    pixmap_to_rgba(&pixmap)
}

fn round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let k = 0.5522847498 * r;
    let (x0, y0, x1, y1) = (x, y, x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + r, y0);
    pb.line_to(x1 - r, y0);
    pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    pb.line_to(x0 + r, y1);
    pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
    pb.line_to(x0, y0 + r);
    pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
    pb.close();
    pb.finish().expect("valid round rect")
}

fn pixmap_to_rgba(pm: &Pixmap) -> image::RgbaImage {
    let mut data = pm.data().to_vec();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
    image::RgbaImage::from_raw(pm.width(), pm.height(), data).expect("valid size")
}
