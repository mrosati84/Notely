use ab_glyph::FontRef;
use egui::{
    Align2, Color32, ColorImage, Context, FontId, Pos2, Rect, Sense, TextureHandle, TextureOptions,
    Vec2,
};
use image::RgbaImage;

use crate::annotation::{Annotation, Tool};
use crate::render;

struct Drag {
    tool: Tool,
    start: Pos2,
    current: Pos2,
    points: Vec<Pos2>,
}

struct MoveDrag {
    index: usize,
    grab: Pos2,
    original: Annotation,
    moved: bool,
}

struct TextState {
    pos: Pos2,
    buffer: String,
    focused: bool,
}

pub struct NotelyApp {
    base: Option<RgbaImage>,
    composite: Option<RgbaImage>,
    texture: Option<TextureHandle>,
    checker: Option<TextureHandle>,
    annotations: Vec<Annotation>,
    tool: Tool,
    color: Color32,
    width: f32,
    font_size: f32,
    drag: Option<Drag>,
    moving: Option<MoveDrag>,
    selected: Option<usize>,
    text_state: Option<TextState>,
    undo: Vec<Vec<Annotation>>,
    redo: Vec<Vec<Annotation>>,
    font: Option<FontRef<'static>>,
    status: String,
    dirty: bool,
    view_scale: f32,
    view_origin: Vec2,
}

const PRESETS: [Color32; 8] = [
    Color32::from_rgb(230, 40, 40),
    Color32::from_rgb(240, 140, 20),
    Color32::from_rgb(245, 205, 30),
    Color32::from_rgb(45, 190, 90),
    Color32::from_rgb(40, 130, 240),
    Color32::from_rgb(150, 80, 230),
    Color32::from_rgb(20, 20, 20),
    Color32::from_rgb(255, 255, 255),
];

impl NotelyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let checker = cc.egui_ctx.load_texture(
            "checker",
            make_checker(),
            TextureOptions::NEAREST_REPEAT,
        );

        let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).ok();

        #[cfg(target_os = "macos")]
        crate::macos_open::set_context(cc.egui_ctx.clone());
        #[cfg(target_os = "macos")]
        crate::macos_paste::install(cc.egui_ctx.clone());

        Self {
            base: None,
            composite: None,
            texture: None,
            checker: Some(checker),
            annotations: Vec::new(),
            tool: Tool::Select,
            color: PRESETS[0],
            width: 4.0,
            font_size: 28.0,
            drag: None,
            moving: None,
            selected: None,
            text_state: None,
            undo: Vec::new(),
            redo: Vec::new(),
            font,
            status: "Drop, paste or open an image to start annotating.".to_owned(),
            dirty: false,
            view_scale: 1.0,
            view_origin: Vec2::ZERO,
        }
    }

    // -- image loading -----------------------------------------------------

    fn set_image(&mut self, img: RgbaImage) {
        let (w, h) = img.dimensions();
        self.status = format!("{w} x {h} px");
        self.base = Some(img);
        self.annotations.clear();
        self.undo.clear();
        self.redo.clear();
        self.texture = None;
        self.composite = None;
        self.drag = None;
        self.moving = None;
        self.selected = None;
        self.text_state = None;
        self.dirty = true;
    }

    fn load_bytes(&mut self, bytes: &[u8]) {
        match image::load_from_memory(bytes) {
            Ok(img) => self.set_image(img.to_rgba8()),
            Err(err) => self.status = format!("Could not load image: {err}"),
        }
    }

    fn load_path(&mut self, path: &std::path::Path) {
        match image::open(path) {
            Ok(img) => self.set_image(img.to_rgba8()),
            Err(err) => self.status = format!("Could not load image: {err}"),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tiff"])
            .pick_file()
        {
            self.load_path(&path);
        }
    }

    fn paste_image(&mut self) {
        match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
            Ok(data) => {
                match RgbaImage::from_raw(data.width as u32, data.height as u32, data.bytes.into_owned())
                {
                    Some(img) => self.set_image(img),
                    None => self.status = "Clipboard image has an unexpected size.".to_owned(),
                }
            }
            Err(_) => self.status = "No image found in the clipboard.".to_owned(),
        }
    }

    // -- history -----------------------------------------------------------

    fn push(&mut self, ann: Annotation) {
        self.undo.push(self.annotations.clone());
        self.redo.clear();
        self.annotations.push(ann);
        self.dirty = true;
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.annotations, prev));
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.annotations, next));
            self.dirty = true;
        }
    }

    fn clear_annotations(&mut self) {
        if !self.annotations.is_empty() {
            self.undo.push(std::mem::take(&mut self.annotations));
            self.redo.clear();
            self.dirty = true;
        }
    }

    // -- rendering ---------------------------------------------------------

    fn rebuild(&mut self, ctx: &Context) {
        let Some(base) = &self.base else {
            self.dirty = false;
            return;
        };
        let out = render::render(base, &self.annotations, self.font.as_ref());
        let size = [out.width() as usize, out.height() as usize];
        let color = ColorImage::from_rgba_unmultiplied(size, out.as_raw());
        match &mut self.texture {
            Some(tex) => tex.set(color, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("composite", color, TextureOptions::LINEAR)),
        }
        self.composite = Some(out);
        self.dirty = false;
    }

    fn ensure_fresh(&mut self) {
        if self.dirty || self.composite.is_none() {
            if let Some(base) = &self.base {
                self.composite = Some(render::render(base, &self.annotations, self.font.as_ref()));
            }
        }
    }

    fn copy_to_clipboard(&mut self) {
        self.ensure_fresh();
        let Some(img) = &self.composite else {
            self.status = "Nothing to copy yet.".to_owned();
            return;
        };
        let data = arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: img.as_raw().clone().into(),
        };
        match arboard::Clipboard::new().and_then(|mut c| c.set_image(data)) {
            Ok(()) => self.status = "Annotated image copied to clipboard.".to_owned(),
            Err(err) => self.status = format!("Copy failed: {err}"),
        }
    }

    fn save_export(&mut self) {
        self.ensure_fresh();
        let Some(img) = &self.composite else {
            self.status = "Nothing to save yet.".to_owned();
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("notely.png")
            .add_filter("PNG image", &["png"])
            .add_filter("JPEG image", &["jpg", "jpeg"])
            .save_file()
        else {
            return;
        };
        let result = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
            Some(ext) if ext == "jpg" || ext == "jpeg" => {
                image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().save(&path)
            }
            _ => img.save(&path),
        };
        match result {
            Ok(()) => self.status = format!("Saved to {}", path.display()),
            Err(err) => self.status = format!("Save failed: {err}"),
        }
    }

    // -- coordinates -------------------------------------------------------

    fn screen_to_img(&self, p: Pos2) -> Pos2 {
        Pos2::new(
            (p.x - self.view_origin.x) / self.view_scale,
            (p.y - self.view_origin.y) / self.view_scale,
        )
    }

    fn img_to_screen(&self, p: Pos2) -> Pos2 {
        Pos2::new(
            self.view_origin.x + p.x * self.view_scale,
            self.view_origin.y + p.y * self.view_scale,
        )
    }

    // -- input -------------------------------------------------------------

    fn handle_dropped(&mut self, ctx: &Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = dropped.last() {
            if let Some(bytes) = &file.bytes {
                self.load_bytes(bytes);
            } else if let Some(path) = &file.path {
                self.load_path(path);
            }
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        if tool != Tool::Select {
            self.selected = None;
        }
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if self.text_state.is_some() {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.selected = None;
        }
        // While a drag is in progress, ignore shortcuts that could mutate the
        // annotation list (delete/undo/redo/paste) or switch tools, which would
        // invalidate the index stored by the in-progress drag.
        if self.drag.is_some() || self.moving.is_some() {
            return;
        }
        let wants_kb = ctx.wants_keyboard_input();
        if !wants_kb {
            let tool = ctx.input(|i| {
                if !i.modifiers.is_none() {
                    return None;
                }
                [(egui::Key::S, Tool::Select), (egui::Key::A, Tool::Arrow), (egui::Key::R, Tool::Rect)]
                    .into_iter()
                    .find(|(key, _)| i.key_pressed(*key))
                    .map(|(_, tool)| tool)
            });
            if let Some(tool) = tool {
                self.set_tool(tool);
            }
        }
        let (undo, redo, copy, save, paste, clear) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Z) && i.modifiers.command && !i.modifiers.shift,
                i.key_pressed(egui::Key::Z) && i.modifiers.command && i.modifiers.shift,
                // egui-winit turns Cmd/Ctrl+C into Event::Copy and swallows the key event.
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.key_pressed(egui::Key::S) && i.modifiers.command,
                // On macOS an NSEvent monitor handles ⌘V for images (see
                // macos_paste); elsewhere fall back to Event::Paste (text) or
                // Key::V. Only paste into the canvas when a widget isn't editing
                // text, so this never hijacks a paste into a text field.
                !wants_kb
                    && (i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
                        || (i.key_pressed(egui::Key::V) && i.modifiers.command)),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if copy {
            self.copy_to_clipboard();
        }
        if save {
            self.save_export();
        }
        if paste {
            self.paste_image();
        }
        if clear {
            if let Some(i) = self.selected {
                if i < self.annotations.len() {
                    self.undo.push(self.annotations.clone());
                    self.redo.clear();
                    self.annotations.remove(i);
                    self.selected = None;
                    self.dirty = true;
                }
            } else if !self.annotations.is_empty() {
                self.clear_annotations();
            }
        }
    }

    fn handle_canvas_input(&mut self, response: &egui::Response) {
        if self.text_state.is_some() {
            return;
        }
        if self.tool == Tool::Text {
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let ip = self.screen_to_img(p);
                    self.text_state = Some(TextState { pos: ip, buffer: String::new(), focused: false });
                }
            }
            return;
        }

        if self.tool == Tool::Select {
            let tol = 5.0 / self.view_scale;
            if response.drag_started() {
                if let Some(p) = response.interact_pointer_pos() {
                    let ip = self.screen_to_img(p);
                    if let Some(idx) = self.hit_test(ip, tol) {
                        self.undo.push(self.annotations.clone());
                        self.redo.clear();
                        self.selected = Some(idx);
                        let original = self.annotations[idx].clone();
                        self.moving = Some(MoveDrag { index: idx, grab: ip, original, moved: false });
                    } else {
                        self.selected = None;
                    }
                }
            }
            if response.dragged() {
                if let Some(p) = response.interact_pointer_pos() {
                    let ip = self.screen_to_img(p);
                    if let Some(m) = self.moving.as_mut() {
                        let delta = ip - m.grab;
                        if delta.length() > 0.5 / self.view_scale {
                            m.moved = true;
                        }
                        let idx = m.index;
                        let mut ann = m.original.clone();
                        ann.translate(delta);
                        if let Some(slot) = self.annotations.get_mut(idx) {
                            *slot = ann;
                        }
                        self.dirty = true;
                    }
                }
            }
            if response.drag_stopped() {
                if let Some(m) = self.moving.take() {
                    if !m.moved {
                        self.undo.pop();
                    }
                }
            }
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let ip = self.screen_to_img(p);
                    self.selected = self.hit_test(ip, tol);
                }
            }
            return;
        }

        if response.drag_started() {
            if let Some(p) = response.interact_pointer_pos() {
                let ip = self.screen_to_img(p);
                self.drag = Some(Drag { tool: self.tool, start: ip, current: ip, points: vec![ip] });
            }
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let ip = self.screen_to_img(p);
                if let Some(d) = &mut self.drag {
                    d.current = ip;
                    if d.tool == Tool::Highlight {
                        let last = *d.points.last().unwrap_or(&ip);
                        if (ip - last).length() > 1.5 {
                            d.points.push(ip);
                        }
                    }
                }
            }
        }
        if response.drag_stopped() {
            self.commit_drag();
        }
    }

    fn commit_drag(&mut self) {
        let Some(d) = self.drag.take() else {
            return;
        };
        let color = self.color.to_array();
        let width = self.width;
        let ann = match d.tool {
            Tool::Arrow => {
                ((d.current - d.start).length() >= 2.0).then_some(Annotation::Arrow {
                    from: d.start,
                    to: d.current,
                    color,
                    width,
                })
            }
            Tool::Rect => {
                let rect = Rect::from_two_pos(d.start, d.current);
                (rect.width() >= 2.0 || rect.height() >= 2.0)
                    .then_some(Annotation::Rect { rect, color, width })
            }
            Tool::Ellipse => {
                let rect = Rect::from_two_pos(d.start, d.current);
                (rect.width() >= 2.0 || rect.height() >= 2.0)
                    .then_some(Annotation::Ellipse { rect, color, width })
            }
            Tool::Highlight => (d.points.len() >= 2).then(|| Annotation::Highlight {
                points: d.points,
                color,
                width: width.max(6.0),
            }),
            Tool::Blur => {
                let rect = Rect::from_two_pos(d.start, d.current);
                (rect.width() >= 4.0 && rect.height() >= 4.0)
                    .then(|| Annotation::Blur { rect, sigma: width.max(2.0) })
            }
            Tool::Spot => {
                let radius = (d.current - d.start).length();
                (radius >= 4.0).then_some(Annotation::Spot { center: d.start, radius })
            }
            Tool::Select | Tool::Text => None,
        };
        if let Some(ann) = ann {
            self.push(ann);
        }
    }

    fn hit_test(&self, p: Pos2, tol: f32) -> Option<usize> {
        self.annotations
            .iter()
            .enumerate()
            .rev()
            .find(|(_, a)| a.hit(p, tol, self.font.as_ref()))
            .map(|(i, _)| i)
    }

    // -- ui ----------------------------------------------------------------

    fn top_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open image").clicked() {
                    self.open_dialog();
                }
                if ui.button("Paste").clicked() {
                    self.paste_image();
                }
                ui.separator();

                for tool in Tool::ALL {
                    if ui.selectable_label(self.tool == tool, tool.label()).clicked() {
                        self.set_tool(tool);
                    }
                }
                ui.separator();

                ui.label("Color");
                for (i, preset) in PRESETS.iter().enumerate() {
                    let selected = self.color == *preset;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                    ui.painter().rect_filled(rect, 3.0, *preset);
                    if selected {
                        ui.painter().rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(2.0_f32, Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if resp.clicked() {
                        self.color = *preset;
                    }
                    let _ = i;
                }
                ui.color_edit_button_srgba(&mut self.color);
                ui.separator();

                match self.tool {
                    Tool::Text => {
                        ui.label("Size");
                        ui.add(egui::Slider::new(&mut self.font_size, 10.0..=120.0).show_value(false));
                    }
                    Tool::Blur => {
                        ui.label("Blur");
                        ui.add(egui::Slider::new(&mut self.width, 2.0..=40.0).show_value(false));
                    }
                    _ => {
                        ui.label("Width");
                        ui.add(egui::Slider::new(&mut self.width, 1.0..=40.0).show_value(false));
                    }
                }
                ui.separator();

                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                    self.redo();
                }
                ui.separator();

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Save").clicked() {
                        self.save_export();
                    }
                    if ui
                        .add_enabled(self.composite.is_some(), egui::Button::new("Copy to clipboard"))
                        .clicked()
                    {
                        self.copy_to_clipboard();
                    }
                    if ui.add_enabled(!self.annotations.is_empty(), egui::Button::new("Clear")).clicked()
                    {
                        self.clear_annotations();
                    }
                });
            });
            ui.add_space(4.0);
        });
    }

    fn bottom_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("S select  ·  A arrow  ·  R rect  ·  Cmd/Ctrl+Z undo  ·  Cmd/Ctrl+Shift+Z redo  ·  Cmd/Ctrl+C copy  ·  Cmd/Ctrl+S save  ·  Del clear");
                });
            });
        });
    }

    fn canvas(&mut self, ctx: &Context) {
        let frame = egui::Frame::NONE.fill(Color32::from_rgb(28, 29, 32));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            let Some((tex_id, img)) = self.texture.as_ref().map(|t| (t.id(), t.size_vec2())) else {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "Drag & drop an image, click \"Open image\", or use Paste to begin",
                    FontId::proportional(18.0),
                    Color32::from_gray(150),
                );
                return;
            };

            let pad = 24.0;
            let scale = ((rect.width() - 2.0 * pad) / img.x)
                .min((rect.height() - 2.0 * pad) / img.y)
                .max(0.01);
            let disp = img * scale;
            let tl = rect.center() - disp / 2.0;
            let img_rect = Rect::from_min_size(tl, disp);
            self.view_scale = scale;
            self.view_origin = tl.to_vec2();

            painter.rect_filled(img_rect.expand(1.0), 2.0, Color32::from_gray(70));

            if let Some(checker) = &self.checker {
                let mut mesh = egui::Mesh { texture_id: checker.id(), ..Default::default() };
                let uv = Rect::from_min_size(Pos2::ZERO, disp / 24.0);
                mesh.add_rect_with_uv(img_rect, uv, Color32::WHITE);
                painter.add(egui::Shape::mesh(mesh));
            }

            painter.image(
                tex_id,
                img_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );

            self.handle_canvas_input(&response);
            self.draw_preview(&painter);
            self.draw_selection(&painter);

            if self.tool == Tool::Select {
                let cursor = if self.moving.is_some() {
                    Some(egui::CursorIcon::Grabbing)
                } else if response.hover_pos().is_some_and(|p| {
                    self.hit_test(self.screen_to_img(p), 5.0 / self.view_scale).is_some()
                }) {
                    Some(egui::CursorIcon::Grab)
                } else {
                    None
                };
                if let Some(cursor) = cursor {
                    ctx.set_cursor_icon(cursor);
                }
            }
        });
    }

    fn draw_preview(&self, painter: &egui::Painter) {
        let Some(d) = &self.drag else {
            return;
        };
        let a = self.img_to_screen(d.start);
        let b = self.img_to_screen(d.current);
        let stroke = egui::Stroke::new((self.width * self.view_scale).max(1.0), self.color);

        match d.tool {
            Tool::Arrow => {
                painter.arrow(a, b - a, stroke);
            }
            Tool::Rect => {
                painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, stroke, egui::StrokeKind::Middle);
            }
            Tool::Ellipse => {
                let rect = Rect::from_two_pos(a, b);
                painter.add(egui::Shape::ellipse_stroke(rect.center(), rect.size() / 2.0, stroke));
            }
            Tool::Highlight => {
                let pts: Vec<Pos2> = d.points.iter().map(|p| self.img_to_screen(*p)).collect();
                let hi =
                    Color32::from_rgba_unmultiplied(self.color.r(), self.color.g(), self.color.b(), 90);
                let w = (self.width.max(6.0) * self.view_scale).max(4.0);
                painter.add(egui::Shape::line(pts, egui::Stroke::new(w, hi)));
            }
            Tool::Blur => {
                let rect = Rect::from_two_pos(a, b);
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0_f32, Color32::from_gray(220)),
                    egui::StrokeKind::Middle,
                );
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "Blur",
                    FontId::proportional(14.0),
                    Color32::from_gray(220),
                );
            }
            Tool::Spot => {
                painter.circle_stroke(
                    a,
                    (b - a).length(),
                    egui::Stroke::new(2.0_f32, Color32::from_gray(220)),
                );
            }
            Tool::Text | Tool::Select => {}
        }
    }

    fn draw_selection(&self, painter: &egui::Painter) {
        if self.tool != Tool::Select {
            return;
        }
        let Some(ann) = self.selected.and_then(|i| self.annotations.get(i)) else {
            return;
        };
        let b = ann.bounds(self.font.as_ref());
        let rect = Rect::from_min_max(self.img_to_screen(b.min), self.img_to_screen(b.max)).expand(4.0);
        let accent = Color32::from_rgb(70, 150, 255);
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.5_f32, accent), egui::StrokeKind::Outside);
        for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
            painter.rect_filled(Rect::from_center_size(corner, Vec2::splat(6.0)), 1.0, accent);
        }
    }

    fn text_window(&mut self, ctx: &Context) {
        let Some(mut st) = self.text_state.take() else {
            return;
        };
        let mut commit = false;
        let mut cancel = false;

        egui::Window::new("Add text")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let entry = ui.add(
                    egui::TextEdit::singleline(&mut st.buffer)
                        .desired_width(260.0)
                        .hint_text("Type your text and press Enter"),
                );
                if !st.focused {
                    entry.request_focus();
                    st.focused = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if entry.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });

        if commit {
            let text = st.buffer.trim().to_string();
            if !text.is_empty() {
                self.push(Annotation::Text {
                    pos: st.pos,
                    text,
                    color: self.color.to_array(),
                    size: self.font_size,
                });
            }
        } else if !cancel {
            self.text_state = Some(st);
        }
    }
}

impl eframe::App for NotelyApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        for path in crate::macos_open::take_pending() {
            self.load_path(&path);
        }
        #[cfg(target_os = "macos")]
        {
            crate::macos_paste::set_text_input_active(
                self.text_state.is_some() || ctx.wants_keyboard_input(),
            );
            if crate::macos_paste::take_requested() {
                self.paste_image();
            }
        }

        self.handle_dropped(ctx);
        self.handle_keys(ctx);
        if self.dirty {
            self.rebuild(ctx);
        }
        self.top_bar(ctx);
        self.bottom_bar(ctx);
        self.canvas(ctx);
        self.text_window(ctx);

        // Changes made during this frame (e.g. a finished drag or a loaded image)
        // need one more frame for the updated texture to reach the screen.
        if self.dirty {
            ctx.request_repaint();
        }
    }
}

fn make_checker() -> ColorImage {
    const SIZE: usize = 16;
    const CELL: usize = 8;
    let mut pixels = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let light = ((x / CELL) + (y / CELL)).is_multiple_of(2);
            pixels.push(if light {
                Color32::from_gray(180)
            } else {
                Color32::from_gray(140)
            });
        }
    }
    ColorImage { size: [SIZE, SIZE], pixels }
}
