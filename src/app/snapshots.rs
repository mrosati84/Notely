//! Headless UI snapshots, for reviewing layout changes without a display.
//!
//! The egui output is tessellated and rasterized on the CPU, so this needs no
//! GPU or screen-recording permission. Run with
//! `NOTELY_SNAPSHOTS=<dir> cargo test --release snapshots -- --ignored`.

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive};
use egui::{Context, Event, PointerButton, Pos2, RawInput, TextureId, Theme, Vec2};

use super::*;

const PPP: f32 = 2.0;

struct Harness {
    ctx: Context,
    app: NotelyApp,
    size: Vec2,
    textures: HashMap<TextureId, image::RgbaImage>,
    time: f64,
    last: Option<Vec<ClippedPrimitive>>,
}

impl Harness {
    fn new(width: f32, height: f32, theme: Theme) -> Self {
        let ctx = Context::default();
        theme::install(&ctx);
        ctx.set_theme(theme);
        crate::fonts::install(&ctx);
        let mut app = NotelyApp::blank();
        app.checker = Some(ctx.load_texture("checker", make_checker(), TextureOptions::NEAREST_REPEAT));
        Self { ctx, app, size: Vec2::new(width, height), textures: HashMap::new(), time: 0.0, last: None }
    }

    fn step(&mut self, events: Vec<Event>) {
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            max_texture_side: Some(16384),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        input.viewports.entry(input.viewport_id).or_default().native_pixels_per_point = Some(PPP);
        self.time += 0.1;
        let out = self.ctx.run(input, |ctx| self.app.frame(ctx));
        for (id, delta) in out.textures_delta.set {
            self.apply(id, delta);
        }
        self.last = Some(self.ctx.tessellate(out.shapes, out.pixels_per_point));
    }

    fn apply(&mut self, id: TextureId, delta: ImageDelta) {
        let (w, h) = (delta.image.width() as u32, delta.image.height() as u32);
        let pixels: Vec<egui::Color32> = match &delta.image {
            egui::ImageData::Color(img) => img.pixels.clone(),
            egui::ImageData::Font(font) => font.srgba_pixels(None).collect(),
        };
        let patch = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba(pixels[(y * w + x) as usize].to_array())
        });
        match delta.pos {
            None => {
                self.textures.insert(id, patch);
            }
            Some([x, y]) => {
                if let Some(tex) = self.textures.get_mut(&id) {
                    image::imageops::replace(tex, &patch, x as i64, y as i64);
                }
            }
        }
    }

    fn click(&mut self, at: Pos2) {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.step(vec![Event::PointerMoved(at)]);
        self.step(vec![button(true)]);
        self.step(vec![button(false)]);
        self.settle();
    }

    fn hover(&mut self, at: Pos2) {
        self.step(vec![Event::PointerMoved(at)]);
        self.settle();
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.step(vec![]);
        }
    }

    fn save(&self, dir: &std::path::Path, name: &str) {
        let w = (self.size.x * PPP) as u32;
        let h = (self.size.y * PPP) as u32;
        let mut target = image::RgbaImage::new(w, h);
        for prim in self.last.as_ref().unwrap() {
            raster(&mut target, prim, &self.textures);
        }
        target.save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// Fill every triangle of a mesh with premultiplied-alpha blending.
fn raster(target: &mut image::RgbaImage, prim: &ClippedPrimitive, textures: &HashMap<TextureId, image::RgbaImage>) {
    let Primitive::Mesh(mesh) = &prim.primitive else {
        return;
    };
    let Some(tex) = textures.get(&mesh.texture_id) else {
        return;
    };
    let clip = prim.clip_rect;
    let (tw, th) = (tex.width() as f32, tex.height() as f32);
    let (w, h) = (target.width() as f32, target.height() as f32);
    for tri in mesh.indices.as_chunks::<3>().0 {
        let v = [0, 1, 2].map(|i| mesh.vertices[tri[i] as usize]);
        let p = v.map(|v| v.pos * PPP);
        let min_x = p.iter().map(|q| q.x).fold(f32::MAX, f32::min).max(clip.min.x * PPP).max(0.0);
        let max_x = p.iter().map(|q| q.x).fold(f32::MIN, f32::max).min(clip.max.x * PPP).min(w);
        let min_y = p.iter().map(|q| q.y).fold(f32::MAX, f32::min).max(clip.min.y * PPP).max(0.0);
        let max_y = p.iter().map(|q| q.y).fold(f32::MIN, f32::max).min(clip.max.y * PPP).min(h);
        let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
        if area.abs() < 1e-6 {
            continue;
        }
        for y in (min_y.floor() as u32)..(max_y.ceil() as u32) {
            for x in (min_x.floor() as u32)..(max_x.ceil() as u32) {
                let c = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                let edge = |a: Pos2, b: Pos2| (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
                let w0 = edge(p[1], p[2]) / area;
                let w1 = edge(p[2], p[0]) / area;
                let w2 = edge(p[0], p[1]) / area;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let uv = v[0].uv.to_vec2() * w0 + v[1].uv.to_vec2() * w1 + v[2].uv.to_vec2() * w2;
                let tx = ((uv.x * tw) as u32).min(tex.width() - 1);
                let ty = ((uv.y * th) as u32).min(tex.height() - 1);
                let t = tex.get_pixel(tx, ty).0;
                let col = |i: usize| {
                    v[0].color.to_array()[i] as f32 * w0
                        + v[1].color.to_array()[i] as f32 * w1
                        + v[2].color.to_array()[i] as f32 * w2
                };
                let src: [f32; 4] = [0, 1, 2, 3].map(|i| col(i) * t[i] as f32 / 255.0);
                let dst = target.get_pixel_mut(x, y);
                let a = src[3] / 255.0;
                for (d, s) in dst.0.iter_mut().zip(src) {
                    *d = (s + *d as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

fn sample_image() -> image::RgbaImage {
    // A soft gradient "landscape" so annotations have something to sit on.
    image::RgbaImage::from_fn(1600, 1000, |x, y| {
        let t = y as f32 / 1000.0;
        let hill = ((x as f32 / 160.0).sin() * 60.0 + 620.0) as u32;
        if y > hill {
            image::Rgba([(70.0 + 40.0 * t) as u8, (110.0 - 20.0 * t) as u8, 60, 255])
        } else {
            image::Rgba([(240.0 - 60.0 * t) as u8, (200.0 - 40.0 * t) as u8, (150.0 + 60.0 * t) as u8, 255])
        }
    })
}

fn annotate(app: &mut NotelyApp) {
    app.push(Annotation::Rect {
        rect: Rect::from_min_max(Pos2::new(560.0, 420.0), Pos2::new(860.0, 620.0)),
        color: [230, 40, 40, 255],
        width: 8.0,
    });
    app.push(Annotation::Arrow {
        from: Pos2::new(380.0, 260.0),
        to: Pos2::new(550.0, 410.0),
        color: [230, 40, 40, 255],
        width: 8.0,
    });
    app.push(Annotation::Ellipse {
        rect: Rect::from_min_max(Pos2::new(1050.0, 380.0), Pos2::new(1300.0, 620.0)),
        color: [245, 205, 30, 255],
        width: 8.0,
    });
    app.push(Annotation::Text {
        pos: Pos2::new(250.0, 170.0),
        text: "Beautiful view!".to_owned(),
        color: [230, 40, 40, 255],
        size: 64.0,
    });
    app.push(Annotation::Blur { rect: Rect::from_min_max(Pos2::new(100.0, 700.0), Pos2::new(400.0, 900.0)), sigma: 12.0 });
}

#[test]
#[ignore]
fn snapshots() {
    let Some(dir) = std::env::var_os("NOTELY_SNAPSHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();

    for (theme, suffix) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        let mut h = Harness::new(1280.0, 860.0, theme);
        h.settle();
        h.save(&dir, &format!("empty-1280-{suffix}"));

        h.app.offer_image(sample_image(), "tuscany.png".to_owned());
        annotate(&mut h.app);
        h.app.tool = Tool::Arrow;
        h.settle();
        h.save(&dir, &format!("loaded-1280-{suffix}"));

        h.app.tool = Tool::Select;
        h.app.selected = Some(0);
        h.settle();
        h.save(&dir, &format!("selected-1280-{suffix}"));
    }

    for width in [1440.0, 1100.0, 800.0, 760.0] {
        let mut h = Harness::new(width, 760.0, Theme::Dark);
        h.app.offer_image(sample_image(), "tuscany.png".to_owned());
        annotate(&mut h.app);
        h.app.tool = Tool::Rect;
        h.settle();
        h.save(&dir, &format!("loaded-{width}"));
    }

    // Empty history and tools without options.
    let mut h = Harness::new(1280.0, 860.0, Theme::Dark);
    h.app.offer_image(sample_image(), "tuscany.png".to_owned());
    h.app.tool = Tool::Spot;
    h.settle();
    h.save(&dir, "disabled-history");

    // Popups, opened by clicking where the snapshot shows their trigger.
    let mut h = Harness::new(1280.0, 860.0, Theme::Dark);
    h.app.offer_image(sample_image(), "tuscany.png".to_owned());
    annotate(&mut h.app);
    h.app.tool = Tool::Arrow;
    h.settle();
    let pos = |name: &str| std::env::var(name).ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some(Pos2::new(x.parse().ok()?, y.parse().ok()?))
    });
    if let Some(at) = pos("NOTELY_CLICK_MORE") {
        h.click(at);
        h.save(&dir, "menu-more");
        if let Some(item) = pos("NOTELY_HOVER_MENU") {
            h.hover(item);
            h.save(&dir, "menu-more-hover");
        }
        h.click(at);
    }
    if let Some(at) = pos("NOTELY_CLICK_COLOR") {
        h.click(at);
        h.save(&dir, "menu-color");
        h.click(at);
    }
    if let Some(at) = pos("NOTELY_CLICK_ZOOM") {
        h.click(at);
        h.save(&dir, "menu-zoom");
        h.click(at);
    }
    if let Some(at) = pos("NOTELY_HOVER") {
        h.hover(at);
        h.step(vec![]);
        h.time += 1.0;
        h.settle();
        h.save(&dir, "tooltip");
    }

    // Replace confirmation.
    h.app.offer_image(sample_image(), "second.png".to_owned());
    h.settle();
    h.save(&dir, "replace-dialog");
}
