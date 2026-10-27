use ab_glyph::FontRef;
use egui::{
    Align2, Color32, ColorImage, Context, FontId, Pos2, Rect, Sense, TextureHandle, TextureOptions,
    Vec2,
};
use image::RgbaImage;

use crate::annotation::{Annotation, Handle, Tool};
use crate::render;

struct Drag {
    tool: Tool,
    start: Pos2,
    current: Pos2,
    points: Vec<Pos2>,
}

/// An in-progress move (or, with `handle`, resize) of an existing annotation.
struct MoveDrag {
    index: usize,
    grab: Pos2,
    current: Pos2,
    original: Annotation,
    /// The corner being dragged and the selection box at drag start.
    handle: Option<(Handle, Rect)>,
}

impl MoveDrag {
    /// The annotation as it would be if the drag ended now.
    fn preview(&self) -> Annotation {
        let mut ann = self.original.clone();
        let delta = self.current - self.grab;
        match self.handle {
            Some((handle, bounds)) => ann.resize(bounds, handle, delta),
            None => ann.translate(delta),
        }
        ann
    }
}

/// Screen-space padding between an annotation's bounds and its selection box.
const SELECTION_PAD: f32 = 4.0;
/// Screen-space side length of a drawn resize handle.
const HANDLE_SIZE: f32 = 6.0;
/// Screen-space side length of a resize handle's hit area.
const HANDLE_HIT: f32 = 12.0;

fn handle_cursor(handle: Handle) -> egui::CursorIcon {
    match handle {
        Handle::TopLeft | Handle::BottomRight => egui::CursorIcon::ResizeNwSe,
        Handle::TopRight | Handle::BottomLeft => egui::CursorIcon::ResizeNeSw,
    }
}

struct TextState {
    pos: Pos2,
    buffer: String,
    focused: bool,
}

/// A reversible history entry. Storing deltas instead of full snapshots keeps
/// undo/redo memory proportional to the edit, not to the whole annotation list.
enum Edit {
    Add(Annotation),
    Remove { index: usize, ann: Annotation },
    Replace { index: usize, old: Annotation, new: Annotation },
    Clear(Vec<Annotation>),
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
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    clipboard: Option<arboard::Clipboard>,
    font: Option<FontRef<'static>>,
    status: String,
    dirty: bool,
    view_scale: f32,
    view_origin: Vec2,
}

/// Largest image we are willing to hold in memory, in pixels. Several full
/// RGBA buffers coexist during rendering, so this bounds worst-case usage.
const MAX_PIXELS: u64 = 64_000_000;
/// Hard ceiling on a decoded image's width or height. Checked at header time by
/// the decoder; it only rejects pathologically long/fat images, while the pixel
/// budget below does the real bounding.
const MAX_DIMENSION: u32 = 65_536;
/// Bytes a decoder may allocate while expanding one image (covers 16-bit RGBA
/// at up to `MAX_PIXELS`).
const MAX_DECODE_BYTES: u64 = 640 * 1024 * 1024;
/// Cap on history entries to bound memory even for delta-based edits.
const MAX_HISTORY: usize = 256;

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
        crate::fonts::install(&cc.egui_ctx);

        let checker = cc.egui_ctx.load_texture(
            "checker",
            make_checker(),
            TextureOptions::NEAREST_REPEAT,
        );

        let font = FontRef::try_from_slice(crate::fonts::INTER_REGULAR).ok();

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
            clipboard: None,
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
        if w == 0 || h == 0 {
            self.status = "Image has zero width or height.".to_owned();
            return;
        }
        let pixels = u64::from(w) * u64::from(h);
        if pixels > MAX_PIXELS {
            self.status = format!(
                "Image too large: {w} x {h} px (limit is {} megapixels).",
                MAX_PIXELS / 1_000_000
            );
            return;
        }
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
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map(decode_limited);
        match reader {
            Ok(Ok(img)) => self.set_image(img),
            Ok(Err(err)) => self.status = format!("Could not load image: {err}"),
            Err(err) => self.status = format!("Could not load image: {err}"),
        }
    }

    fn load_path(&mut self, path: &std::path::Path) {
        match image::ImageReader::open(path).map(decode_limited) {
            Ok(Ok(img)) => self.set_image(img),
            Ok(Err(err)) => self.status = format!("Could not load image: {err}"),
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
        let result = match self.ensure_clipboard() {
            Some(c) => c.get_image(),
            None => {
                self.status = "Clipboard is not available.".to_owned();
                return;
            }
        };
        match result {
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

    /// A single clipboard instance kept for the life of the app. On X11/Wayland
    /// the process owns the selection, so dropping the handle after a copy makes
    /// the contents disappear; reusing it keeps them available.
    fn ensure_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    // -- history -----------------------------------------------------------

    fn push(&mut self, ann: Annotation) {
        self.undo.push(Edit::Add(ann.clone()));
        self.trim_history();
        self.redo.clear();
        self.annotations.push(ann);
        self.dirty = true;
    }

    fn trim_history(&mut self) {
        while self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
    }

    fn undo(&mut self) {
        let Some(edit) = self.undo.pop() else {
            return;
        };
        match &edit {
            Edit::Add(_) => {
                self.annotations.pop();
            }
            Edit::Remove { index, ann } => {
                self.annotations.insert((*index).min(self.annotations.len()), ann.clone());
            }
            Edit::Replace { index, old, .. } => {
                if *index < self.annotations.len() {
                    self.annotations[*index] = old.clone();
                }
            }
            Edit::Clear(prev) => {
                self.annotations = prev.clone();
            }
        }
        self.redo.push(edit);
        // Indices shift under undo/redo; drop the selection rather than let it
        // point at (and let Delete remove) a different annotation.
        self.selected = None;
        self.dirty = true;
    }

    fn redo(&mut self) {
        let Some(edit) = self.redo.pop() else {
            return;
        };
        match &edit {
            Edit::Add(ann) => {
                self.annotations.push(ann.clone());
            }
            Edit::Remove { index, .. } => {
                if *index < self.annotations.len() {
                    self.annotations.remove(*index);
                }
            }
            Edit::Replace { index, new, .. } => {
                if *index < self.annotations.len() {
                    self.annotations[*index] = new.clone();
                }
            }
            Edit::Clear(_) => {
                self.annotations.clear();
            }
        }
        self.undo.push(edit);
        self.selected = None;
        self.dirty = true;
    }

    fn clear_annotations(&mut self) {
        self.drag = None;
        self.moving = None;
        self.selected = None;
        if !self.annotations.is_empty() {
            self.undo.push(Edit::Clear(std::mem::take(&mut self.annotations)));
            self.trim_history();
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
        // Blur/Spot can't be represented by an egui overlay, so they stay in the
        // composite and are updated in place during a drag. Vector shapes are
        // skipped and drawn as an overlay.
        let skip = self
            .moving
            .as_ref()
            .filter(|m| !is_pixel_effect(&m.original))
            .map(|m| m.index);
        let out = render::render(base, &self.annotations, self.font.as_ref(), skip);
        // Keep the full-resolution composite for export, but upload a texture no
        // larger than the GPU supports so large screenshots still display.
        let max_side = ctx.input(|i| i.max_texture_side).max(1);
        let color = if out.width() as usize > max_side || out.height() as usize > max_side {
            let scale = (max_side as f32 / out.width().max(out.height()) as f32).min(1.0);
            let nw = ((out.width() as f32 * scale).round() as u32).max(1);
            let nh = ((out.height() as f32 * scale).round() as u32).max(1);
            let scaled =
                image::imageops::resize(&out, nw, nh, image::imageops::FilterType::Triangle);
            ColorImage::from_rgba_unmultiplied([nw as usize, nh as usize], scaled.as_raw())
        } else {
            ColorImage::from_rgba_unmultiplied(
                [out.width() as usize, out.height() as usize],
                out.as_raw(),
            )
        };
        match &mut self.texture {
            Some(tex) => tex.set(color, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("composite", color, TextureOptions::LINEAR)),
        }
        self.composite = Some(out);
        self.dirty = false;
    }

    fn ensure_fresh(&mut self) {
        // While a move is in progress the cached composite deliberately omits the
        // dragged vector annotation, so force a full render for export/copy.
        if self.dirty || self.composite.is_none() || self.moving.is_some() {
            if let Some(base) = &self.base {
                self.composite =
                    Some(render::render(base, &self.annotations, self.font.as_ref(), None));
            }
        }
    }

    fn copy_to_clipboard(&mut self) {
        self.ensure_fresh();
        let data = match &self.composite {
            Some(img) => arboard::ImageData {
                width: img.width() as usize,
                height: img.height() as usize,
                bytes: img.as_raw().clone().into(),
            },
            None => {
                self.status = "Nothing to copy yet.".to_owned();
                return;
            }
        };
        let result = match self.ensure_clipboard() {
            Some(c) => c.set_image(data),
            None => Err(arboard::Error::ClipboardNotSupported),
        };
        match result {
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
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let (final_path, result) = match ext.as_deref() {
            Some("jpg") | Some("jpeg") => (
                path.clone(),
                to_rgb_on_white(img).save_with_format(&path, image::ImageFormat::Jpeg),
            ),
            Some("png") => (path.clone(), img.save_with_format(&path, image::ImageFormat::Png)),
            // Unknown or missing extension: append `.png` (never replace the
            // existing name) and write an explicit PNG, rather than relying on
            // `save`, which fails when the format can't be inferred.
            _ => {
                let png = append_png(&path);
                (png.clone(), img.save_with_format(&png, image::ImageFormat::Png))
            }
        };
        match result {
            Ok(()) => self.status = format!("Saved to {}", final_path.display()),
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
        // Commit (or finish) any gesture in progress so a tool change can never
        // strand a `drag`/`moving` that references a stale annotation index.
        if self.drag.is_some() {
            self.commit_drag();
        }
        if self.moving.is_some() {
            self.end_move();
        }
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
        // A focused widget (e.g. the colour picker's hex field) owns the
        // keyboard: don't let canvas shortcuts steal Backspace or Cmd+Z from it.
        // Saving stays global so Cmd/Ctrl+S always works.
        if ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.command) {
                self.save_export();
            }
            return;
        }
        // While a drag is in progress, ignore shortcuts that could mutate the
        // annotation list (delete/undo/redo/paste) or switch tools, which would
        // invalidate the index stored by the in-progress drag.
        if self.drag.is_some() || self.moving.is_some() {
            return;
        }
        let tool = ctx.input(|i| {
            if !i.modifiers.is_none() {
                return None;
            }
            [
                (egui::Key::S, Tool::Select),
                (egui::Key::A, Tool::Arrow),
                (egui::Key::R, Tool::Rect),
                (egui::Key::T, Tool::Text),
            ]
            .into_iter()
            .find(|(key, _)| i.key_pressed(*key))
            .map(|(_, tool)| tool)
        });
        if let Some(tool) = tool {
            self.set_tool(tool);
        }
        let (undo, redo, copy, save, paste, delete) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Z) && i.modifiers.command && !i.modifiers.shift,
                i.key_pressed(egui::Key::Z) && i.modifiers.command && i.modifiers.shift,
                // egui-winit turns Cmd/Ctrl+C into Event::Copy and swallows the key event.
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
                i.key_pressed(egui::Key::S) && i.modifiers.command,
                // On macOS an NSEvent monitor handles ⌘V for images (see
                // macos_paste); elsewhere fall back to Event::Paste (text) or
                // Key::V. A focused text field returned above, so this cannot
                // hijack a paste into one.
                i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
                    || (i.key_pressed(egui::Key::V) && i.modifiers.command),
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
        if delete {
            // Only remove the selected annotation; clearing everything stays
            // behind the explicit "Clear" button.
            if let Some(i) = self.selected {
                if i < self.annotations.len() {
                    let ann = self.annotations.remove(i);
                    self.undo.push(Edit::Remove { index: i, ann });
                    self.trim_history();
                    self.redo.clear();
                    self.selected = None;
                    self.dirty = true;
                }
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
                // Hit-test where the button went down, not where the drag
                // threshold was crossed, so small handles are reliably grabbed.
                let origin = response.ctx.input(|i| i.pointer.press_origin());
                if let Some(p) = origin.or(response.interact_pointer_pos()) {
                    let ip = self.screen_to_img(p);
                    // Handles of the selected annotation take priority: they sit
                    // outside the shape and may overlap other annotations.
                    let resize = self.handle_at(p).and_then(|h| {
                        let idx = self.selected?;
                        let bounds = self.annotations.get(idx)?.bounds(self.font.as_ref());
                        Some((idx, h, bounds))
                    });
                    let target = match resize {
                        Some((idx, h, bounds)) => Some((idx, Some((h, bounds)))),
                        None => self.hit_test(ip, tol).map(|idx| (idx, None)),
                    };
                    if let Some((idx, handle)) = target {
                        self.selected = Some(idx);
                        let original = self.annotations[idx].clone();
                        self.moving =
                            Some(MoveDrag { index: idx, grab: ip, current: ip, original, handle });
                        // Re-render without the dragged vector annotation; it is
                        // drawn as an overlay until the drop. Pixel effects stay
                        // in the composite and update in place below.
                        self.dirty = true;
                    } else {
                        self.selected = None;
                    }
                }
            }
            if response.dragged() {
                if let Some(p) = response.interact_pointer_pos() {
                    let ip = self.screen_to_img(p);
                    if let Some(m) = self.moving.as_mut() {
                        m.current = ip;
                    }
                    // Live-update pixel effects (blur/spotlight) in place; vector
                    // shapes use the cheap overlay and only commit on drop.
                    if let Some(m) = &self.moving {
                        if is_pixel_effect(&m.original) {
                            let idx = m.index;
                            let ann = m.preview();
                            if idx < self.annotations.len() {
                                self.annotations[idx] = ann;
                            }
                            self.dirty = true;
                        }
                    }
                }
            }
            if response.drag_stopped() {
                self.end_move();
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
        // tiny-skia colors use straight (unpremultiplied) alpha.
        let color = self.color.to_srgba_unmultiplied();
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

    /// Commit an in-progress move. History is only touched when the annotation
    /// actually moved, so a mere press (or long-press without movement) leaves
    /// the redo stack intact.
    fn end_move(&mut self) {
        let Some(m) = self.moving.take() else {
            return;
        };
        if m.index < self.annotations.len() {
            // Decide from the final offset, so dragging away and back to the
            // start is treated as a no-op.
            let delta = m.current - m.grab;
            if delta.length() > 0.5 / self.view_scale {
                let ann = m.preview();
                self.undo.push(Edit::Replace {
                    index: m.index,
                    old: m.original.clone(),
                    new: ann.clone(),
                });
                self.trim_history();
                self.redo.clear();
                self.annotations[m.index] = ann;
            } else {
                // Pixel effects were updated in place during the drag; restore.
                self.annotations[m.index] = m.original.clone();
            }
        }
        // Put the annotation back into the composite (or restore it unchanged).
        self.dirty = true;
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
                    ui.label("S select  ·  A arrow  ·  R rect  ·  Cmd/Ctrl+Z undo  ·  Cmd/Ctrl+Shift+Z redo  ·  Cmd/Ctrl+C copy  ·  Cmd/Ctrl+S save  ·  Del remove");
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

            let dims = self.base.as_ref().map(|b| b.dimensions());
            let tex_id = self.texture.as_ref().map(|t| t.id());
            let (Some((bw, bh)), Some(tex_id)) = (dims, tex_id) else {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "Drag & drop an image, click \"Open image\", or use Paste to begin",
                    FontId::proportional(18.0),
                    Color32::from_gray(150),
                );
                return;
            };

            // Geometry is derived from the full-resolution image, not the (possibly
            // downscaled) display texture, so pointer coordinates stay in image space.
            let img = egui::vec2(bw as f32, bh as f32);
            let pad = 24.0;
            let scale = ((rect.width() - 2.0 * pad) / img.x)
                .min((rect.height() - 2.0 * pad) / img.y)
                .max(0.01);
            let disp = img * scale;
            let tl = rect.center() - disp / 2.0;
            let img_rect = Rect::from_min_size(tl, disp);
            self.view_scale = scale;
            self.view_origin = tl.to_vec2();

            // Handle input first so a rebuild triggered here (drag start, edit)
            // reaches the texture before it is painted this same frame.
            self.handle_canvas_input(&response);
            if self.dirty {
                self.rebuild(ctx);
            }

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

            self.draw_preview(&painter);
            self.draw_moving(&painter);
            self.draw_selection(&painter);

            if self.tool == Tool::Select {
                let cursor = if let Some(m) = &self.moving {
                    Some(match m.handle {
                        Some((h, _)) => handle_cursor(h),
                        None => egui::CursorIcon::Grabbing,
                    })
                } else if let Some(h) = response.hover_pos().and_then(|p| self.handle_at(p)) {
                    Some(handle_cursor(h))
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
                self.paint_arrow(painter, d.start, d.current, self.width, self.color);
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
                let [r, g, b, _] = self.color.to_srgba_unmultiplied();
                let hi = Color32::from_rgba_unmultiplied(r, g, b, 90);
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

    /// Draw the annotation being dragged as an egui overlay, so a move costs no
    /// per-frame re-render or texture upload.
    fn draw_moving(&self, painter: &egui::Painter) {
        let Some(m) = &self.moving else {
            return;
        };
        // Pixel effects are already shown live in the composite.
        if is_pixel_effect(&m.original) {
            return;
        }
        self.paint_annotation(painter, &m.preview());
    }

    /// Paint an arrow given in image space with the same geometry as the export.
    fn paint_arrow(&self, painter: &egui::Painter, from: Pos2, to: Pos2, width: f32, color: Color32) {
        let a = self.img_to_screen(from);
        let b = self.img_to_screen(render::arrow_shaft_end(from, to, width));
        let w = (width * self.view_scale).max(1.0);
        painter.line_segment([a, b], egui::Stroke::new(w, color));
        // The export strokes with round caps; mirror that at both ends.
        painter.circle_filled(a, w / 2.0, color);
        painter.circle_filled(b, w / 2.0, color);
        if let Some(head) = render::arrow_head(from, to, width) {
            painter.add(egui::Shape::convex_polygon(
                head.map(|p| self.img_to_screen(p)).to_vec(),
                color,
                egui::Stroke::NONE,
            ));
        }
    }

    fn paint_annotation(&self, painter: &egui::Painter, ann: &Annotation) {
        let s = self.view_scale;
        let rgba = |c: &[u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
        match ann {
            Annotation::Arrow { from, to, color, width } => {
                self.paint_arrow(painter, *from, *to, *width, rgba(color));
            }
            Annotation::Rect { rect, color, width } => {
                painter.rect_stroke(
                    self.img_rect_to_screen(*rect),
                    0.0,
                    egui::Stroke::new((*width * s).max(1.0), rgba(color)),
                    egui::StrokeKind::Middle,
                );
            }
            Annotation::Ellipse { rect, color, width } => {
                let r = self.img_rect_to_screen(*rect);
                painter.add(egui::Shape::ellipse_stroke(
                    r.center(),
                    r.size() / 2.0,
                    egui::Stroke::new((*width * s).max(1.0), rgba(color)),
                ));
            }
            Annotation::Highlight { points, color, width } => {
                let pts: Vec<Pos2> = points.iter().map(|p| self.img_to_screen(*p)).collect();
                if pts.len() >= 2 {
                    let hi = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 90);
                    let w = ((*width).max(6.0) * s).max(4.0);
                    painter.add(egui::Shape::line(pts, egui::Stroke::new(w, hi)));
                }
            }
            Annotation::Blur { rect, .. } => {
                let r = self.img_rect_to_screen(*rect);
                painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(120, 120, 120, 60));
                painter.rect_stroke(
                    r,
                    0.0,
                    egui::Stroke::new(2.0_f32, Color32::from_gray(220)),
                    egui::StrokeKind::Middle,
                );
                painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "Blur",
                    FontId::proportional(14.0),
                    Color32::from_gray(220),
                );
            }
            Annotation::Spot { center, radius } => {
                painter.circle_stroke(
                    self.img_to_screen(*center),
                    radius * s,
                    egui::Stroke::new(2.0_f32, Color32::from_gray(220)),
                );
            }
            Annotation::Text { pos, text, color, size } => {
                painter.text(
                    self.img_to_screen(*pos),
                    Align2::LEFT_TOP,
                    text,
                    FontId::proportional((*size * s).max(1.0)),
                    rgba(color),
                );
            }
        }
    }

    fn img_rect_to_screen(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.img_to_screen(r.min), self.img_to_screen(r.max))
    }

    fn draw_selection(&self, painter: &egui::Painter) {
        if self.tool != Tool::Select {
            return;
        }
        // While moving/resizing, outline the annotation at its dragged geometry.
        let ann = if let Some(m) = &self.moving {
            m.preview()
        } else {
            match self.selected.and_then(|i| self.annotations.get(i)) {
                Some(a) => a.clone(),
                None => return,
            }
        };
        let rect = self.selection_rect(&ann);
        let accent = Color32::from_rgb(70, 150, 255);
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.5_f32, accent), egui::StrokeKind::Outside);
        for h in Handle::ALL {
            painter.rect_filled(Rect::from_center_size(h.pos(rect), Vec2::splat(HANDLE_SIZE)), 1.0, accent);
        }
    }

    /// Screen-space selection box drawn around `ann`.
    fn selection_rect(&self, ann: &Annotation) -> Rect {
        self.img_rect_to_screen(ann.bounds(self.font.as_ref())).expand(SELECTION_PAD)
    }

    /// The resize handle of the selected annotation under screen point `p`.
    fn handle_at(&self, p: Pos2) -> Option<Handle> {
        if self.tool != Tool::Select {
            return None;
        }
        let rect = self.selection_rect(self.annotations.get(self.selected?)?);
        Handle::ALL
            .into_iter()
            .find(|h| Rect::from_center_size(h.pos(rect), Vec2::splat(HANDLE_HIT)).contains(p))
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
                    color: self.color.to_srgba_unmultiplied(),
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

/// Decode an image while enforcing decoder resource limits, returning RGBA8.
fn decode_limited<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<R>,
) -> Result<RgbaImage, image::ImageError> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let img = reader.decode()?;
    // Reject oversized images *before* expanding to RGBA, which can otherwise
    // allocate several times the decoded size (e.g. 8-bit grey -> 32-bit RGBA).
    let pixels = u64::from(img.width()) * u64::from(img.height());
    if pixels > MAX_PIXELS {
        return Err(image::ImageError::Limits(image::error::LimitError::from_kind(
            image::error::LimitErrorKind::DimensionError,
        )));
    }
    Ok(img.into_rgba8())
}

/// Blur and spotlight are whole-pixel effects that cannot be represented by an
/// egui vector overlay.
fn is_pixel_effect(ann: &Annotation) -> bool {
    matches!(ann, Annotation::Blur { .. } | Annotation::Spot { .. })
}

/// Append `.png` to a path's file name (rather than replacing its extension),
/// so normalising an unknown extension can never clobber a different file.
fn append_png(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".png");
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name),
        _ => std::path::PathBuf::from(name),
    }
}

/// Flatten an RGBA image onto a white background, for formats without alpha.
fn to_rgb_on_white(img: &RgbaImage) -> image::RgbImage {
    let (w, h) = img.dimensions();
    let mut out = image::RgbImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(img.pixels()) {
        let a = u32::from(src[3]);
        let inv = 255 - a;
        dst[0] = ((u32::from(src[0]) * a + 255 * inv + 127) / 255) as u8;
        dst[1] = ((u32::from(src[1]) * a + 255 * inv + 127) / 255) as u8;
        dst[2] = ((u32::from(src[2]) * a + 255 * inv + 127) / 255) as u8;
    }
    out
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
