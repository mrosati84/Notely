use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::FontRef;
use egui::{
    Align2, Color32, ColorImage, Context, FontId, Key, KeyboardShortcut, Modifiers, Pos2, Rect,
    Sense, TextureHandle, TextureOptions, Ui, Vec2,
};
use image::RgbaImage;

use crate::annotation::{
    arrow_head, arrow_shaft_end, layout_text, Annotation, Handle, Tool, MIN_HIGHLIGHT_WIDTH,
};
use crate::icons::Icon;
use crate::render;
use crate::theme::{self, Palette};
use crate::view::{Transform, Viewport};
use crate::widgets::{self, Command, Look, Shape};

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

/// Every user command, whichever control or shortcut triggers it. Buttons,
/// menus and keys all go through [`NotelyApp::enabled`] and [`NotelyApp::run`]
/// so their availability can never diverge.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Open,
    Paste,
    Save,
    Copy,
    Undo,
    Redo,
    DeleteSelected,
    ClearAll,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomTo(f32),
    ShowShortcuts,
    Tool(Tool),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NoticeKind {
    Info,
    Success,
    Error,
}

/// A transient, non-blocking message shown in the status bar.
struct Notice {
    kind: NoticeKind,
    text: String,
    /// `None` until first shown, then the egui time it appeared.
    shown_at: Option<f64>,
}

/// An image waiting for the user to confirm it may replace unsaved work.
struct PendingImage {
    image: RgbaImage,
    name: String,
}

/// How much room the toolbar has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tier {
    /// Icons with captions, one row.
    Wide,
    /// Icon-only buttons, one row.
    Medium,
    /// Icon-only buttons; tool options move to a second row.
    Narrow,
}

impl Tier {
    fn for_width(width: f32) -> Tier {
        if width >= 1100.0 {
            Tier::Wide
        } else if width >= 800.0 {
            Tier::Medium
        } else {
            Tier::Narrow
        }
    }
}

pub struct NotelyApp {
    base: Option<RgbaImage>,
    composite: Option<RgbaImage>,
    texture: Option<TextureHandle>,
    checker: Option<TextureHandle>,
    annotations: Vec<Annotation>,
    tool: Tool,
    color: Color32,
    /// Line width of arrows, rectangles and ellipses.
    stroke_width: f32,
    highlight_width: f32,
    blur_strength: f32,
    font_size: f32,
    drag: Option<Drag>,
    moving: Option<MoveDrag>,
    selected: Option<usize>,
    text_state: Option<TextState>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    clipboard: Option<arboard::Clipboard>,
    font: Option<FontRef<'static>>,
    notice: Option<Notice>,
    /// The composite/texture must be re-rendered.
    dirty: bool,
    /// Undo depth at the last load or save, or `None` once that state can no
    /// longer be reached through undo/redo. The document has unsaved changes
    /// whenever this differs from `undo.len()`.
    saved_at: Option<usize>,
    /// File name of the current image, or a description of where it came from.
    doc_name: Option<String>,
    pending: Option<PendingImage>,
    show_shortcuts: bool,
    /// A widget held keyboard focus at the end of the last frame. egui drops
    /// focus on Escape before this frame's handlers run, so that press was
    /// meant for the widget.
    widget_had_focus: bool,
    /// Window title last sent to the platform.
    title: String,
    viewport: Viewport,
    /// Canvas area and scale factor from the last frame. Together with
    /// `viewport` they define the image-to-screen transform (see `view`).
    canvas_rect: Rect,
    pixels_per_point: f32,
    canvas_hovered: bool,
    panning: bool,
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
/// Cancels the current gesture, closes the shortcuts window or deselects.
const CANCEL_KEY: Key = Key::Escape;

/// File extensions the open dialog and drag and drop accept. These match the
/// decoders enabled in the `image` crate.
const SUPPORTED_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"];
const SUPPORTED_LABEL: &str = "PNG, JPEG, GIF, BMP, WebP, TIFF";

const PRESETS: [(Color32, &str); 8] = [
    (Color32::from_rgb(230, 40, 40), "Red"),
    (Color32::from_rgb(240, 140, 20), "Orange"),
    (Color32::from_rgb(245, 205, 30), "Yellow"),
    (Color32::from_rgb(45, 190, 90), "Green"),
    (Color32::from_rgb(40, 130, 240), "Blue"),
    (Color32::from_rgb(150, 80, 230), "Purple"),
    (Color32::from_rgb(20, 20, 20), "Black"),
    (Color32::from_rgb(255, 255, 255), "White"),
];

/// Toolbar order and grouping: selection, shapes and text, pixel effects.
const TOOL_GROUPS: [&[Tool]; 3] = [
    &[Tool::Select],
    &[Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Text],
    &[Tool::Highlight, Tool::Blur, Tool::Spot],
];

struct ToolInfo {
    icon: Icon,
    /// Short caption under the icon in the wide toolbar.
    caption: &'static str,
    /// Full name for tooltips and accessibility.
    name: &'static str,
    key: Option<Key>,
    /// What to do on the canvas, shown in the status bar.
    hint: &'static str,
}

fn tool_info(tool: Tool) -> ToolInfo {
    let (icon, caption, name, key, hint) = match tool {
        Tool::Select => (
            Icon::Select,
            "Select",
            "Select",
            Some(Key::S),
            "Click an annotation to select it, drag to move it, drag a corner to resize",
        ),
        Tool::Arrow => (Icon::Arrow, "Arrow", "Arrow", Some(Key::A), "Drag on the image to draw an arrow"),
        Tool::Rect => (Icon::Rect, "Rect", "Rectangle", Some(Key::R), "Drag on the image to draw a rectangle"),
        Tool::Ellipse => (Icon::Ellipse, "Ellipse", "Ellipse", None, "Drag on the image to draw an ellipse"),
        Tool::Text => (Icon::Text, "Text", "Text", Some(Key::T), "Click on the image to add text"),
        Tool::Highlight => (Icon::Highlight, "Highlight", "Highlight", None, "Drag to highlight freehand"),
        Tool::Blur => (Icon::Blur, "Blur", "Blur", None, "Drag over an area to blur it"),
        Tool::Spot => (
            Icon::Spot,
            "Spot",
            "Spotlight",
            None,
            "Drag outwards from a point to spotlight a circle and dim the rest",
        ),
    };
    ToolInfo { icon, caption, name, key, hint }
}

/// A tool's size slider. `tool_width` clamps to the same range, so a committed
/// annotation always uses a size the slider can show.
struct SizeSetting {
    label: &'static str,
    range: std::ops::RangeInclusive<f32>,
    tip: &'static str,
}

fn size_setting(tool: Tool) -> Option<SizeSetting> {
    let (label, range, tip) = match tool {
        Tool::Text => ("Size", 10.0..=120.0, "Text size in image pixels"),
        Tool::Blur => ("Strength", 2.0..=40.0, "Blur radius in image pixels"),
        Tool::Highlight => ("Width", MIN_HIGHLIGHT_WIDTH..=40.0, "Highlighter width in image pixels"),
        Tool::Arrow | Tool::Rect | Tool::Ellipse => ("Stroke", 1.0..=40.0, "Line width in image pixels"),
        Tool::Select | Tool::Spot => return None,
    };
    Some(SizeSetting { label, range, tip })
}

impl NotelyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        crate::fonts::install(&cc.egui_ctx);

        let checker = cc.egui_ctx.load_texture(
            "checker",
            make_checker(),
            TextureOptions::NEAREST_REPEAT,
        );

        #[cfg(target_os = "macos")]
        crate::macos_open::set_context(cc.egui_ctx.clone());
        #[cfg(target_os = "macos")]
        crate::macos_paste::install(cc.egui_ctx.clone());

        Self { checker: Some(checker), ..Self::blank() }
    }

    /// The initial state, without any GPU resources.
    fn blank() -> Self {
        Self {
            base: None,
            composite: None,
            texture: None,
            checker: None,
            annotations: Vec::new(),
            tool: Tool::Select,
            color: PRESETS[0].0,
            stroke_width: 4.0,
            highlight_width: 6.0,
            blur_strength: 4.0,
            font_size: 28.0,
            drag: None,
            moving: None,
            selected: None,
            text_state: None,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: None,
            font: FontRef::try_from_slice(crate::fonts::INTER_REGULAR).ok(),
            notice: None,
            dirty: false,
            saved_at: Some(0),
            doc_name: None,
            pending: None,
            show_shortcuts: false,
            widget_had_focus: false,
            title: String::new(),
            viewport: Viewport::default(),
            canvas_rect: Rect::ZERO,
            pixels_per_point: 1.0,
            canvas_hovered: false,
            panning: false,
        }
    }

    fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Some(Notice { kind, text: text.into(), shown_at: None });
    }

    // -- actions -----------------------------------------------------------

    fn gesture_active(&self) -> bool {
        self.drag.is_some() || self.moving.is_some() || self.panning
    }

    fn enabled(&self, action: Action) -> bool {
        let has_image = self.base.is_some();
        match action {
            Action::Open | Action::Paste | Action::ShowShortcuts | Action::Tool(_) => true,
            Action::Save | Action::Copy => has_image,
            Action::Undo => !self.undo.is_empty() && !self.gesture_active(),
            Action::Redo => !self.redo.is_empty() && !self.gesture_active(),
            Action::DeleteSelected => {
                self.tool == Tool::Select
                    && !self.gesture_active()
                    && self.selected.is_some_and(|i| i < self.annotations.len())
            }
            Action::ClearAll => !self.annotations.is_empty(),
            Action::ZoomIn | Action::ZoomOut | Action::ZoomFit | Action::ZoomTo(_) => has_image,
        }
    }

    fn run(&mut self, action: Action) {
        match action {
            Action::Open => self.open_dialog(),
            Action::Paste => self.paste_image(),
            Action::Save => self.save_export(),
            Action::Copy => self.copy_to_clipboard(),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::DeleteSelected => self.delete_selected(),
            Action::ClearAll => self.clear_annotations(),
            Action::ZoomIn => self.zoom_step(true),
            Action::ZoomOut => self.zoom_step(false),
            Action::ZoomFit => self.viewport.fit(),
            Action::ZoomTo(z) => self.zoom_to(z),
            Action::ShowShortcuts => self.show_shortcuts = true,
            Action::Tool(tool) => self.set_tool(tool),
        }
    }

    /// Keys that trigger an action without Cmd/Ctrl. `handle_keys` binds all of
    /// them; labels show the first, and the shortcuts window lists them all.
    fn plain_keys(action: Action) -> &'static [Key] {
        match action {
            Action::DeleteSelected => &[Key::Delete, Key::Backspace],
            // `=` is `+` without Shift on many layouts.
            Action::ZoomIn => &[Key::Plus, Key::Equals],
            Action::ZoomOut => &[Key::Minus],
            Action::ZoomFit => &[Key::Num0],
            _ => &[],
        }
    }

    /// The platform-formatted shortcut of an action, if it has one.
    fn shortcut(ctx: &Context, action: Action) -> Option<String> {
        let cmd = |key| ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key));
        Some(match action {
            Action::Open => cmd(Key::O),
            Action::Paste => cmd(Key::V),
            Action::Save => cmd(Key::S),
            Action::Copy => cmd(Key::C),
            Action::Undo => cmd(Key::Z),
            Action::Redo => ctx.format_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Z,
            )),
            Action::DeleteSelected | Action::ZoomIn | Action::ZoomOut | Action::ZoomFit => {
                Self::plain_keys(action).first()?.symbol_or_name().to_owned()
            }
            Action::Tool(tool) => tool_info(tool).key?.name().to_owned(),
            Action::ClearAll | Action::ZoomTo(_) | Action::ShowShortcuts => return None,
        })
    }

    // -- image loading -----------------------------------------------------

    /// Accept a decoded image: load it straight away, or ask first when it
    /// would discard annotations that were never saved.
    fn offer_image(&mut self, img: RgbaImage, name: String) {
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            self.notify(NoticeKind::Error, "Image has zero width or height.");
            return;
        }
        let pixels = u64::from(w) * u64::from(h);
        if pixels > MAX_PIXELS {
            self.notify(
                NoticeKind::Error,
                format!(
                    "Image too large: {w} × {h} px (limit is {} megapixels).",
                    MAX_PIXELS / 1_000_000
                ),
            );
            return;
        }
        if self.has_unsaved_work() {
            self.pending = Some(PendingImage { image: img, name });
        } else {
            self.set_image(img, name);
        }
    }

    fn has_unsaved_work(&self) -> bool {
        self.saved_at != Some(self.undo.len()) && !self.annotations.is_empty()
    }

    /// The current annotations are now on disk.
    fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
    }

    fn set_image(&mut self, img: RgbaImage, name: String) {
        self.notify(NoticeKind::Info, format!("Opened {name}"));
        self.base = Some(img);
        self.doc_name = Some(name);
        self.annotations.clear();
        self.undo.clear();
        self.redo.clear();
        self.texture = None;
        self.composite = None;
        self.drag = None;
        self.moving = None;
        self.panning = false;
        self.selected = None;
        self.text_state = None;
        self.viewport = Viewport::default();
        self.dirty = true;
        self.mark_saved();
    }

    fn load_bytes(&mut self, bytes: &[u8], name: String) {
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map(decode_limited);
        match reader {
            Ok(Ok(img)) => self.offer_image(img, name),
            Ok(Err(err)) => self.notify(NoticeKind::Error, format!("Could not open {name}: {err}")),
            Err(err) => self.notify(NoticeKind::Error, format!("Could not open {name}: {err}")),
        }
    }

    fn load_path(&mut self, path: &std::path::Path) {
        let name = file_name(path);
        match image::ImageReader::open(path).map(decode_limited) {
            Ok(Ok(img)) => self.offer_image(img, name),
            Ok(Err(err)) => self.notify(NoticeKind::Error, format!("Could not open {name}: {err}")),
            Err(err) => self.notify(NoticeKind::Error, format!("Could not open {name}: {err}")),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &SUPPORTED_EXTENSIONS)
            .pick_file()
        {
            self.load_path(&path);
        }
    }

    fn paste_image(&mut self) {
        let result = match self.ensure_clipboard() {
            Some(c) => c.get_image(),
            None => {
                self.notify(NoticeKind::Error, "Clipboard is not available.");
                return;
            }
        };
        match result {
            Ok(data) => {
                match RgbaImage::from_raw(data.width as u32, data.height as u32, data.bytes.into_owned())
                {
                    Some(img) => self.offer_image(img, "Pasted image".to_owned()),
                    None => self.notify(NoticeKind::Error, "Clipboard image has an unexpected size."),
                }
            }
            Err(_) => self.notify(NoticeKind::Error, "No image found in the clipboard."),
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

    /// Record an edit that has already been applied to `annotations`.
    fn record(&mut self, edit: Edit) {
        // A saved state sitting in the redo stack is about to be discarded.
        let depth = self.undo.len();
        self.saved_at = self.saved_at.filter(|&s| s <= depth);
        self.undo.push(edit);
        self.trim_history();
        self.redo.clear();
        self.dirty = true;
    }

    fn push(&mut self, ann: Annotation) {
        self.annotations.push(ann.clone());
        self.record(Edit::Add(ann));
    }

    fn trim_history(&mut self) {
        while self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
            // Depths shift down; a save before the dropped edit is now unreachable.
            self.saved_at = self.saved_at.and_then(|s| s.checked_sub(1));
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

    /// Remove the selected annotation. Clearing everything stays behind the
    /// explicit "Clear all annotations" command.
    fn delete_selected(&mut self) {
        if !self.enabled(Action::DeleteSelected) {
            return;
        }
        if let Some(i) = self.selected.take() {
            let ann = self.annotations.remove(i);
            self.record(Edit::Remove { index: i, ann });
        }
    }

    /// Remove every annotation but keep the image. Undoable.
    fn clear_annotations(&mut self) {
        self.drag = None;
        self.moving = None;
        self.selected = None;
        if !self.annotations.is_empty() {
            let prev = std::mem::take(&mut self.annotations);
            self.record(Edit::Clear(prev));
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
        // Zoomed in, show crisp image pixels rather than a blurry upscale.
        let options = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
        match &mut self.texture {
            Some(tex) => tex.set(color, options),
            None => self.texture = Some(ctx.load_texture("composite", color, options)),
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
                self.notify(NoticeKind::Error, "Nothing to copy yet. Open or paste an image first.");
                return;
            }
        };
        let result = match self.ensure_clipboard() {
            Some(c) => c.set_image(data),
            None => Err(arboard::Error::ClipboardNotSupported),
        };
        match result {
            Ok(()) => {
                self.notify(NoticeKind::Success, "Annotated image copied to clipboard.");
            }
            Err(err) => self.notify(NoticeKind::Error, format!("Copy failed: {err}")),
        }
    }

    fn save_export(&mut self) {
        self.ensure_fresh();
        let Some(img) = &self.composite else {
            self.notify(NoticeKind::Error, "Nothing to save yet. Open or paste an image first.");
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
            Ok(()) => {
                self.mark_saved();
                self.notify(NoticeKind::Success, format!("Saved to {}", final_path.display()));
            }
            Err(err) => self.notify(NoticeKind::Error, format!("Save failed: {err}")),
        }
    }

    // -- coordinates -------------------------------------------------------

    /// The image-to-screen transform, derived from the current viewport so it
    /// can never lag behind a zoom or pan.
    fn view(&self) -> Transform {
        match self.img_size() {
            Some(img) => self.viewport.transform(self.canvas_rect, img, self.pixels_per_point),
            None => Transform { scale: 1.0, origin: Vec2::ZERO },
        }
    }

    fn screen_to_img(&self, p: Pos2) -> Pos2 {
        self.view().to_image(p)
    }

    fn img_to_screen(&self, p: Pos2) -> Pos2 {
        self.view().to_screen(p)
    }

    /// Clamp an image-space point to the loaded image's bounds.
    fn img_clamped(&self, p: Pos2) -> Pos2 {
        match &self.base {
            Some(base) => clamp_to_image(p, base.width(), base.height()),
            None => p,
        }
    }

    fn img_size(&self) -> Option<Vec2> {
        self.base.as_ref().map(|b| egui::vec2(b.width() as f32, b.height() as f32))
    }

    // -- zoom --------------------------------------------------------------

    fn zoom_step(&mut self, up: bool) {
        if let Some(img) = self.img_size() {
            let z = self.viewport.step(up, self.canvas_rect, img, self.pixels_per_point);
            self.zoom_to(z);
        }
    }

    /// Zoom around the canvas centre.
    fn zoom_to(&mut self, zoom: f32) {
        if let Some(img) = self.img_size() {
            let (rect, ppp) = (self.canvas_rect, self.pixels_per_point);
            self.viewport.zoom_to(zoom, rect.center(), rect, img, ppp);
        }
    }

    // -- input -------------------------------------------------------------

    fn handle_dropped(&mut self, ctx: &Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = dropped.last() {
            let name = file
                .path
                .as_deref()
                .map(file_name)
                .unwrap_or_else(|| if file.name.is_empty() { "Dropped file".to_owned() } else { file.name.clone() });
            if let Some(bytes) = &file.bytes {
                self.load_bytes(bytes, name);
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

    /// Escape: cancel the gesture in progress without committing it, else
    /// close the shortcuts window, else drop the selection. Committed work is
    /// never touched.
    fn escape(&mut self) {
        if self.drag.take().is_some() {
            return;
        }
        if let Some(m) = self.moving.take() {
            // Pixel effects were updated in place during the drag; restore.
            if m.index < self.annotations.len() {
                self.annotations[m.index] = m.original;
            }
            self.dirty = true;
            return;
        }
        if self.show_shortcuts {
            self.show_shortcuts = false;
            return;
        }
        self.selected = None;
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if self.text_state.is_some() || self.pending.is_some() {
            return;
        }
        // Escape that leaves a focused widget must not also cancel the gesture
        // or drop the selection.
        if !self.widget_had_focus && ctx.input(|i| i.key_pressed(CANCEL_KEY)) {
            self.escape();
        }
        // A focused widget (e.g. the colour picker's hex field) owns the
        // keyboard: don't let canvas shortcuts steal Backspace or Cmd+Z from it.
        // Saving stays global so Cmd/Ctrl+S always works.
        if ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(Key::S) && i.modifiers.command) {
                self.run(Action::Save);
            }
            return;
        }
        // Space over the canvas starts panning; make sure it does not also
        // press whichever toolbar button has keyboard focus.
        if self.canvas_hovered && ctx.input(|i| i.key_pressed(Key::Space)) {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
        // While a drag is in progress, ignore shortcuts that could mutate the
        // annotation list (delete/undo/redo/paste) or switch tools, which would
        // invalidate the index stored by the in-progress drag.
        if self.gesture_active() {
            return;
        }
        let tool = ctx.input(|i| {
            if !i.modifiers.is_none() {
                return None;
            }
            TOOL_GROUPS
                .iter()
                .flat_map(|g| g.iter().copied())
                .find(|t| tool_info(*t).key.is_some_and(|k| i.key_pressed(k)))
        });
        if let Some(tool) = tool {
            self.run(Action::Tool(tool));
        }
        let actions = ctx.input(|i| {
            let cmd = |key| i.key_pressed(key) && i.modifiers.command && !i.modifiers.shift;
            // `+` needs Shift on many layouts, and Cmd/Ctrl variants are the
            // platform convention; accept both, but not with Alt.
            let plain = |action| Self::plain_keys(action).iter().any(|k| i.key_pressed(*k));
            let zoom_key = |action| !i.modifiers.alt && plain(action);
            [
                (Action::Undo, cmd(Key::Z)),
                (
                    Action::Redo,
                    i.key_pressed(Key::Z) && i.modifiers.command && i.modifiers.shift,
                ),
                // egui-winit turns Cmd/Ctrl+C into Event::Copy and swallows the key event.
                (Action::Copy, i.events.iter().any(|e| matches!(e, egui::Event::Copy))),
                (Action::Save, i.key_pressed(Key::S) && i.modifiers.command),
                (Action::Open, cmd(Key::O)),
                // On macOS an NSEvent monitor handles ⌘V for images (see
                // macos_paste). egui-winit swallows the paste key-down and only
                // emits Event::Paste for text, so elsewhere trigger on the
                // key-up, which is still delivered. A focused text field
                // returned above, so this cannot hijack a paste into one.
                (
                    Action::Paste,
                    if cfg!(target_os = "macos") {
                        i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))
                            || (i.key_pressed(Key::V) && i.modifiers.command)
                    } else {
                        i.key_released(Key::V) && i.modifiers.command
                    },
                ),
                (Action::DeleteSelected, plain(Action::DeleteSelected)),
                (Action::ZoomIn, zoom_key(Action::ZoomIn)),
                (Action::ZoomOut, zoom_key(Action::ZoomOut)),
                (Action::ZoomFit, zoom_key(Action::ZoomFit)),
            ]
        });
        for (action, pressed) in actions {
            if pressed {
                self.run(action);
            }
        }
    }

    /// Zoom with pinch or Cmd/Ctrl+scroll, pan with scroll, Space+drag or the
    /// middle button. Returns whether the canvas is being navigated, in which
    /// case it must not draw or select.
    fn navigate(&mut self, ctx: &Context, response: &egui::Response, rect: Rect, img: Vec2) -> bool {
        let ppp = self.pixels_per_point;
        if response.contains_pointer() && self.pending.is_none() {
            let (zoom, scroll, pointer) =
                ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if zoom != 1.0 {
                let anchor = pointer.unwrap_or(rect.center());
                let current = self.viewport.effective_zoom(rect, img, ppp);
                self.viewport.zoom_to(current * zoom, anchor, rect, img, ppp);
            } else if !self.viewport.fit && scroll != Vec2::ZERO {
                self.viewport.pan(scroll, rect, img, ppp);
            }
        }
        let space = self.text_state.is_none()
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.key_down(Key::Space));
        if response.drag_started()
            && self.drag.is_none()
            && self.moving.is_none()
            && (space || response.dragged_by(egui::PointerButton::Middle))
        {
            self.panning = true;
        }
        if self.panning {
            if response.dragged() {
                self.viewport.pan(response.drag_delta(), rect, img, ppp);
            }
            if response.drag_stopped() || !response.dragged() {
                self.panning = false;
            }
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            return true;
        }
        if space && response.contains_pointer() && !self.gesture_active() {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
            return true;
        }
        false
    }

    fn handle_canvas_input(&mut self, response: &egui::Response, img_rect: Rect) {
        if self.text_state.is_some() {
            return;
        }
        if self.tool == Tool::Text {
            if response.clicked() {
                let origin = response.ctx.input(|i| i.pointer.press_origin());
                if let Some(p) = origin.or(response.interact_pointer_pos()) {
                    if img_rect.contains(p) {
                        let ip = self.screen_to_img(p);
                        self.text_state =
                            Some(TextState { pos: ip, buffer: String::new(), focused: false });
                    }
                }
            }
            return;
        }

        if self.tool == Tool::Select {
            let tol = 5.0 / self.view().scale;
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
            // The drag threshold may be crossed after the pointer has left the
            // image; decide from the press origin so a gesture that started on
            // the image still works, but a gesture that started in the padding
            // never creates an out-of-image annotation.
            let origin = response.ctx.input(|i| i.pointer.press_origin());
            if let Some(p) = origin.or(response.interact_pointer_pos()) {
                if img_rect.contains(p) {
                    let start = self.img_clamped(self.screen_to_img(p));
                    let current = response
                        .interact_pointer_pos()
                        .map(|q| self.img_clamped(self.screen_to_img(q)))
                        .unwrap_or(start);
                    self.drag =
                        Some(Drag { tool: self.tool, start, current, points: vec![start] });
                }
            }
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let ip = self.img_clamped(self.screen_to_img(p));
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

    /// The line width a new annotation of `tool` gets.
    /// The size setting a tool reads and its slider edits.
    fn size_mut(&mut self, tool: Tool) -> &mut f32 {
        match tool {
            Tool::Text => &mut self.font_size,
            Tool::Highlight => &mut self.highlight_width,
            Tool::Blur => &mut self.blur_strength,
            _ => &mut self.stroke_width,
        }
    }

    fn tool_width(&self, tool: Tool) -> f32 {
        let value = match tool {
            Tool::Text => self.font_size,
            Tool::Highlight => self.highlight_width,
            Tool::Blur => self.blur_strength,
            _ => self.stroke_width,
        };
        match size_setting(tool) {
            Some(s) => value.clamp(*s.range.start(), *s.range.end()),
            None => value,
        }
    }

    fn commit_drag(&mut self) {
        let Some(d) = self.drag.take() else {
            return;
        };
        // tiny-skia colors use straight (unpremultiplied) alpha.
        let color = self.color.to_srgba_unmultiplied();
        let width = self.tool_width(d.tool);
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
                width,
            }),
            Tool::Blur => {
                let rect = Rect::from_two_pos(d.start, d.current);
                (rect.width() >= 4.0 && rect.height() >= 4.0)
                    .then_some(Annotation::Blur { rect, sigma: width })
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
    /// actually changed, so a mere press (or long-press without movement) leaves
    /// the redo stack intact.
    fn end_move(&mut self) {
        let Some(m) = self.moving.take() else {
            return;
        };
        if m.index < self.annotations.len() {
            let ann = m.preview();
            // Require both pointer movement and a real geometry change: a
            // one-axis text/spot drag used to be a no-op that still pushed a
            // history entry and cleared redo.
            let delta = m.current - m.grab;
            if delta.length() > 0.5 / self.view().scale && ann != m.original {
                self.annotations[m.index] = ann.clone();
                self.record(Edit::Replace { index: m.index, old: m.original, new: ann });
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

    /// Show a command button and queue its action when clicked.
    fn command(&self, ui: &mut Ui, out: &mut Option<Action>, action: Action, cmd: Command<'_>) {
        let cmd = cmd.enabled(self.enabled(action)).shortcut(Self::shortcut(ui.ctx(), action));
        if cmd.show(ui).clicked() {
            *out = Some(action);
        }
    }

    fn top_bar(&mut self, ctx: &Context) {
        let p = Palette::of(ctx);
        let tier = Tier::for_width(ctx.screen_rect().width());
        let frame = egui::Frame::new()
            .fill(p.chrome)
            .inner_margin(egui::Margin::symmetric(12, if tier == Tier::Wide { 8 } else { 7 }));
        let mut action = None;
        egui::TopBottomPanel::top("toolbar").frame(frame).show(ctx, |ui| {
            let has_options = self.tool_has_options();
            let mut second_row = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let shape = if tier == Tier::Wide { Shape::Stacked } else { Shape::Icon(theme::BUTTON) };

                // File actions.
                self.command(ui, &mut action, Action::Open, Command::new(Icon::Open, "Open").shape(shape));
                self.command(ui, &mut action, Action::Paste, Command::new(Icon::Paste, "Paste").shape(shape));
                ui.add_space(4.0);
                let save = Command::new(Icon::Save, "Save").look(Look::Primary).detail("Export as PNG or JPEG");
                let save = if tier == Tier::Wide { save.shape(Shape::Inline(34.0)) } else { save };
                self.command(ui, &mut action, Action::Save, save);
                widgets::divider(ui, 30.0);

                // Annotation tools.
                for (i, group) in TOOL_GROUPS.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(if tier == Tier::Wide { 6.0 } else { 4.0 });
                    }
                    for &tool in group.iter() {
                        let info = tool_info(tool);
                        // Icon-only buttons are named in full for tooltips.
                        let label = if tier == Tier::Wide { info.caption } else { info.name };
                        let cmd = Command::new(info.icon, label)
                            .shape(shape)
                            .toggle(self.tool == tool)
                            .detail(info.hint);
                        self.command(ui, &mut action, Action::Tool(tool), cmd);
                    }
                }

                // Options stay on this row only when they fit next to the
                // overflow menu; otherwise they move to a second row.
                const OPTIONS_WIDTH: f32 = 400.0;
                if has_options && tier != Tier::Narrow && ui.available_width() >= OPTIONS_WIDTH {
                    widgets::divider(ui, 30.0);
                    self.tool_options(ui);
                } else if has_options {
                    second_row = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.overflow_menu(ui, &mut action);
                });
            });
            if second_row {
                ui.add_space(4.0);
                ui.horizontal(|ui| self.tool_options(ui));
            }
        });
        if let Some(action) = action {
            self.run(action);
        }
    }

    fn tool_has_options(&self) -> bool {
        !matches!(self.tool, Tool::Select | Tool::Spot)
    }

    /// Contextual settings for the active tool. Tools without settings show
    /// nothing rather than a disabled panel.
    fn tool_options(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui.ctx());
        ui.spacing_mut().item_spacing.x = 8.0;
        let uses_color = !matches!(self.tool, Tool::Blur | Tool::Spot | Tool::Select);
        if uses_color {
            ui.label(egui::RichText::new("Color").color(p.text_muted));
            self.color_picker(ui);
            ui.add_space(4.0);
        }
        let Some(SizeSetting { label, range, tip }) = size_setting(self.tool) else {
            return;
        };
        let value = self.size_mut(self.tool);
        let label_resp = ui.label(egui::RichText::new(label).color(p.text_muted)).on_hover_text(tip);
        ui.spacing_mut().slider_width = 110.0;
        // The slider and its number box share one value, clamped to the range.
        ui.add(
            egui::Slider::new(value, range)
                .step_by(1.0)
                .fixed_decimals(0)
                .trailing_fill(true),
        )
        .labelled_by(label_resp.id)
        .on_hover_text(tip);
        if matches!(self.tool, Tool::Arrow | Tool::Rect | Tool::Ellipse | Tool::Highlight) {
            self.stroke_preview(ui);
        }
    }

    /// A short sample line in the current color and width.
    fn stroke_preview(&self, ui: &mut Ui) {
        let p = Palette::of(ui.ctx());
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(34.0, 24.0), Sense::hover());
        ui.painter().rect(rect, 5.0, p.group, egui::Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
        let w = self.tool_width(self.tool).min(16.0);
        let color = if self.tool == Tool::Highlight {
            let [r, g, b, _] = self.color.to_srgba_unmultiplied();
            Color32::from_rgba_unmultiplied(r, g, b, 90)
        } else {
            self.color
        };
        let y = rect.center().y;
        ui.painter().line_segment(
            [egui::pos2(rect.left() + 8.0, y), egui::pos2(rect.right() - 8.0, y)],
            egui::Stroke::new(w, color),
        );
        resp.on_hover_text("Preview of the current color and width");
    }

    fn color_picker(&mut self, ui: &mut Ui) {
        let id = ui.make_persistent_id("color-popup");
        let open = ui.memory(|m| m.is_popup_open(id));
        let resp = widgets::color_button(ui, self.color, open);
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        egui::popup::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(4.0 * 30.0);
            egui::Grid::new("presets").spacing([4.0, 4.0]).show(ui, |ui| {
                for (i, (preset, name)) in PRESETS.iter().enumerate() {
                    if widgets::swatch(ui, *preset, self.color == *preset, name).clicked() {
                        self.color = *preset;
                        ui.memory_mut(|m| m.close_popup());
                    }
                    if i % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Custom color").id_salt("custom-color").show(ui, |ui| {
                egui::color_picker::color_picker_color32(ui, &mut self.color, egui::color_picker::Alpha::OnlyBlend);
            });
        });
    }

    fn overflow_menu(&mut self, ui: &mut Ui, out: &mut Option<Action>) {
        let id = ui.make_persistent_id("overflow-menu");
        let resp = Command::new(Icon::More, "More commands").show(ui);
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        let entries: [Option<(Action, &str)>; 13] = [
            Some((Action::Open, "Open Image…")),
            Some((Action::Paste, "Paste Image")),
            Some((Action::Save, "Save…")),
            Some((Action::Copy, "Copy Image")),
            None,
            Some((Action::DeleteSelected, "Delete Selected")),
            Some((Action::ClearAll, "Clear All Annotations")),
            None,
            Some((Action::ZoomIn, "Zoom In")),
            Some((Action::ZoomOut, "Zoom Out")),
            Some((Action::ZoomFit, "Fit to Window")),
            None,
            Some((Action::ShowShortcuts, "Keyboard Shortcuts")),
        ];
        egui::popup::popup_below_widget(ui, id, &resp, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_width(250.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for entry in entries {
                match entry {
                    None => widgets::menu_separator(ui),
                    Some((action, label)) => {
                        let shortcut = Self::shortcut(ui.ctx(), action);
                        let resp = ui
                            .add_enabled_ui(self.enabled(action), |ui| {
                                widgets::menu_item(ui, label, shortcut.as_deref(), false)
                            })
                            .inner;
                        let resp = if action == Action::ClearAll {
                            resp.on_hover_text("Removes every annotation and keeps the image. Undoable.")
                        } else {
                            resp
                        };
                        if resp.clicked() {
                            *out = Some(action);
                        }
                    }
                }
            }
        });
    }

    fn bottom_bar(&mut self, ctx: &Context) {
        let p = Palette::of(ctx);
        let frame = egui::Frame::new().fill(p.chrome).inner_margin(egui::Margin::symmetric(10, 5));
        let mut action = None;
        egui::TopBottomPanel::bottom("status").frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let small = Shape::Icon(28.0);

                // Canvas navigation.
                self.command(ui, &mut action, Action::ZoomOut, Command::new(Icon::Minus, "Zoom out").shape(small));
                self.zoom_menu(ui, &mut action);
                self.command(ui, &mut action, Action::ZoomIn, Command::new(Icon::Plus, "Zoom in").shape(small));
                // Highlighted while fitting, but not a toggle: pressing it again
                // does not leave fit mode.
                let fitting = self.base.is_some() && self.viewport.fit;
                let fit = Command::new(Icon::Fit, "Fit to window")
                    .shape(small)
                    .look(if fitting { Look::Active } else { Look::Plain });
                self.command(ui, &mut action, Action::ZoomFit, fit);
                if let Some(img) = &self.base {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("{} × {} px", img.width(), img.height()))
                            .color(p.text_muted),
                    );
                }
                widgets::divider(ui, 20.0);

                // Status: a recent message, else what the active tool does.
                const ACTIONS_WIDTH: f32 = 260.0;
                let status_width = (ui.available_width() - ACTIONS_WIDTH).max(0.0);
                let (text, color) = self.status_line(ctx, &p);
                ui.allocate_ui_with_layout(
                    egui::vec2(status_width, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(text).color(color)).truncate());
                    },
                );

                // Editing actions.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.command(
                        ui,
                        &mut action,
                        Action::Copy,
                        Command::new(Icon::Copy, "Copy").shape(Shape::Inline(28.0)).detail("Copy the annotated image to the clipboard"),
                    );
                    widgets::divider(ui, 20.0);
                    self.command(
                        ui,
                        &mut action,
                        Action::ClearAll,
                        Command::new(Icon::Eraser, "Clear all annotations")
                            .shape(small)
                            .detail("Removes every annotation and keeps the image. Undoable."),
                    );
                    self.command(
                        ui,
                        &mut action,
                        Action::DeleteSelected,
                        Command::new(Icon::Trash, "Delete selected annotation").shape(small),
                    );
                    widgets::divider(ui, 20.0);
                    self.command(ui, &mut action, Action::Redo, Command::new(Icon::Redo, "Redo").shape(small));
                    self.command(ui, &mut action, Action::Undo, Command::new(Icon::Undo, "Undo").shape(small));
                });
            });
        });
        if let Some(action) = action {
            self.run(action);
        }
    }

    /// The text and color of the status message for this frame.
    fn status_line(&mut self, ctx: &Context, p: &Palette) -> (String, Color32) {
        let now = ctx.input(|i| i.time);
        if let Some(n) = &mut self.notice {
            let shown_at = *n.shown_at.get_or_insert(now);
            let lifetime = if n.kind == NoticeKind::Error { 8.0 } else { 4.0 };
            let left = shown_at + lifetime - now;
            if left > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
                let color = match n.kind {
                    NoticeKind::Info => p.text,
                    NoticeKind::Success => p.success,
                    NoticeKind::Error => p.error,
                };
                return (n.text.clone(), color);
            }
            self.notice = None;
        }
        if self.base.is_none() {
            return ("Open, paste or drop an image to start annotating".to_owned(), p.text_muted);
        }
        let mut hint = tool_info(self.tool).hint.to_owned();
        if !self.viewport.fit {
            hint.push_str("  ·  Hold Space and drag to pan");
        }
        (hint, p.text_muted)
    }

    fn zoom_menu(&mut self, ui: &mut Ui, out: &mut Option<Action>) {
        let id = ui.make_persistent_id("zoom-menu");
        let open = ui.memory(|m| m.is_popup_open(id));
        let enabled = self.enabled(Action::ZoomFit);
        // Computed from the viewport rather than read from `viewport.zoom`,
        // which in fit mode is only updated once the canvas is drawn.
        let zoom = self.img_size().map(|img| {
            self.viewport.effective_zoom(self.canvas_rect, img, self.pixels_per_point)
        });
        let text = match zoom {
            Some(z) if enabled && self.canvas_rect.area() > 0.0 => format!("{:.0}%", z * 100.0),
            _ => "—".to_owned(),
        };
        let resp = ui
            .add_enabled_ui(enabled, |ui| widgets::text_dropdown(ui, &text, 74.0, open))
            .inner;
        let resp = resp.on_hover_text(format!(
            "Zoom level\nPinch or {}+scroll to zoom, scroll or hold Space and drag to pan",
            if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }
        ));
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        egui::popup::popup_above_or_below_widget(
            ui,
            id,
            &resp,
            egui::AboveOrBelow::Above,
            egui::PopupCloseBehavior::CloseOnClick,
            |ui| {
                ui.set_width(150.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                for z in [8.0, 4.0, 2.0, 1.0, 0.5, 0.25] {
                    let current = !self.viewport.fit && (self.viewport.zoom - z).abs() < 1e-3;
                    if widgets::menu_item(ui, &format!("{:.0}%", z * 100.0), None, current).clicked() {
                        *out = Some(Action::ZoomTo(z));
                    }
                }
                widgets::menu_separator(ui);
                if widgets::menu_item(ui, "Fit to Window", None, self.viewport.fit).clicked() {
                    *out = Some(Action::ZoomFit);
                }
            },
        );
    }

    fn canvas(&mut self, ctx: &Context) {
        let frame = egui::Frame::NONE.fill(Palette::of(ctx).canvas);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            self.canvas_rect = rect;
            self.pixels_per_point = ctx.pixels_per_point();
            self.canvas_hovered = response.contains_pointer();
            let painter = ui.painter_at(rect);

            let dims = self.base.as_ref().map(|b| b.dimensions());
            let tex_id = self.texture.as_ref().map(|t| t.id());
            let (Some((bw, bh)), Some(tex_id)) = (dims, tex_id) else {
                self.empty_state(ui, rect);
                return;
            };

            // Geometry is derived from the full-resolution image, not the (possibly
            // downscaled) display texture, so pointer coordinates stay in image space.
            let img = egui::vec2(bw as f32, bh as f32);
            let navigating = self.navigate(ctx, &response, rect, img);
            let transform = self.viewport.resolve(rect, img, self.pixels_per_point);
            let disp = img * transform.scale;
            let img_rect = Rect::from_min_size(transform.to_screen(Pos2::ZERO), disp);

            // Handle input first so a rebuild triggered here (drag start, edit)
            // reaches the texture before it is painted this same frame.
            if !navigating {
                self.handle_canvas_input(&response, img_rect);
            }
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

            if !navigating {
                self.set_canvas_cursor(ctx, &response, img_rect);
            }
            self.drop_overlay(ctx, &painter, rect);
        });
    }

    fn set_canvas_cursor(&self, ctx: &Context, response: &egui::Response, img_rect: Rect) {
        let cursor = match self.tool {
            Tool::Select => {
                if let Some(m) = &self.moving {
                    Some(match m.handle {
                        Some((h, _)) => handle_cursor(h),
                        None => egui::CursorIcon::Grabbing,
                    })
                } else if let Some(h) = response.hover_pos().and_then(|p| self.handle_at(p)) {
                    Some(handle_cursor(h))
                } else if response.hover_pos().is_some_and(|p| {
                    self.hit_test(self.screen_to_img(p), 5.0 / self.view().scale).is_some()
                }) {
                    Some(egui::CursorIcon::Grab)
                } else {
                    None
                }
            }
            Tool::Text if response.hover_pos().is_some_and(|p| img_rect.contains(p)) => {
                Some(egui::CursorIcon::Text)
            }
            _ if self.drag.is_some() || response.hover_pos().is_some_and(|p| img_rect.contains(p)) => {
                Some(egui::CursorIcon::Crosshair)
            }
            _ => None,
        };
        if let Some(cursor) = cursor {
            ctx.set_cursor_icon(cursor);
        }
    }

    /// Whether files being dragged over the window look like images we can
    /// open. `None` when nothing is being dragged.
    fn hovered_files_valid(ctx: &Context) -> Option<bool> {
        let files = ctx.input(|i| i.raw.hovered_files.clone());
        let file = files.last()?;
        Some(match &file.path {
            Some(path) => is_supported_path(path),
            // Without a path (e.g. some platforms only give a MIME type), let
            // the decoder decide on drop.
            None => file.mime.is_empty() || file.mime.starts_with("image/"),
        })
    }

    fn drop_overlay(&self, ctx: &Context, painter: &egui::Painter, rect: Rect) {
        let Some(valid) = Self::hovered_files_valid(ctx) else {
            return;
        };
        let dark = theme::DARK;
        let (color, text) = if valid {
            (dark.focus, "Drop to open this image")
        } else {
            (dark.error, "This file type can't be opened")
        };
        let inner = rect.shrink(10.0);
        painter.rect_filled(inner, 10.0, Color32::from_black_alpha(150));
        painter.rect_stroke(inner, 10.0, egui::Stroke::new(2.0_f32, color), egui::StrokeKind::Inside);
        painter.text(inner.center(), Align2::CENTER_CENTER, text, FontId::proportional(18.0), color);
    }

    /// First-run view: a drop zone with the two ways to start.
    fn empty_state(&mut self, ui: &mut Ui, rect: Rect) {
        let dark = theme::DARK;
        let valid = Self::hovered_files_valid(ui.ctx());
        let zone = Rect::from_center_size(
            rect.center(),
            egui::vec2((rect.width() - 48.0).min(460.0), (rect.height() - 48.0).min(290.0)),
        );
        let painter = ui.painter_at(rect);
        let (border, fill) = match valid {
            Some(true) => (dark.focus, Color32::from_rgba_unmultiplied(37, 99, 235, 40)),
            Some(false) => (dark.error, Color32::from_rgba_unmultiplied(255, 120, 110, 25)),
            None => (Color32::from_gray(92), Color32::from_white_alpha(4)),
        };
        painter.rect_filled(zone, 12.0, fill);
        let corners = [zone.left_top(), zone.right_top(), zone.right_bottom(), zone.left_bottom(), zone.left_top()];
        painter.extend(egui::Shape::dashed_line(&corners, egui::Stroke::new(1.5_f32, border), 7.0, 5.0));

        let c = zone.center();
        crate::icons::paint(
            &painter,
            Rect::from_center_size(c - egui::vec2(0.0, 82.0), Vec2::splat(44.0)),
            Icon::Image,
            if valid.is_some() { border } else { dark.text_muted },
        );
        let (title, sub) = match valid {
            Some(true) => ("Drop to open this image", ""),
            Some(false) => ("This file type can't be opened", ""),
            None => ("Drag & drop an image here", "or use the buttons below"),
        };
        painter.text(c - egui::vec2(0.0, 38.0), Align2::CENTER_CENTER, title, FontId::proportional(17.0), dark.text);
        painter.text(c - egui::vec2(0.0, 14.0), Align2::CENTER_CENTER, sub, FontId::proportional(13.0), dark.text_muted);
        painter.text(
            c + egui::vec2(0.0, 92.0),
            Align2::CENTER_CENTER,
            format!("Supports: {SUPPORTED_LABEL}"),
            FontId::proportional(12.0),
            dark.text_muted,
        );

        let mut action = None;
        let row = Rect::from_center_size(c + egui::vec2(0.0, 32.0), egui::vec2(zone.width(), 40.0));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let open = Command::new(Icon::Open, "Open Image…").shape(Shape::Inline(38.0)).look(Look::Primary).on_canvas();
                let paste = Command::new(Icon::Paste, "Paste").shape(Shape::Inline(38.0)).look(Look::Secondary).on_canvas();
                // Centre the two buttons: measure, then pad.
                let width = 150.0 + 96.0 + ui.spacing().item_spacing.x * 2.0;
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                self.command(ui, &mut action, Action::Open, open);
                ui.add_space(4.0);
                self.command(ui, &mut action, Action::Paste, paste);
            },
        );
        if let Some(action) = action {
            self.run(action);
        }
    }

    fn draw_preview(&self, painter: &egui::Painter) {
        let Some(d) = &self.drag else {
            return;
        };
        let a = self.img_to_screen(d.start);
        let b = self.img_to_screen(d.current);
        let width = self.tool_width(d.tool);
        let stroke = egui::Stroke::new((width * self.view().scale).max(1.0), self.color);

        match d.tool {
            Tool::Arrow => {
                self.paint_arrow(painter, d.start, d.current, width, self.color);
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
                let w = (width * self.view().scale).max(4.0);
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
        let b = self.img_to_screen(arrow_shaft_end(from, to, width));
        let w = (width * self.view().scale).max(1.0);
        painter.line_segment([a, b], egui::Stroke::new(w, color));
        // The export strokes with round caps; mirror that at both ends.
        painter.circle_filled(a, w / 2.0, color);
        painter.circle_filled(b, w / 2.0, color);
        if let Some(head) = arrow_head(from, to, width) {
            painter.add(egui::Shape::convex_polygon(
                head.map(|p| self.img_to_screen(p)).to_vec(),
                color,
                egui::Stroke::NONE,
            ));
        }
    }

    fn paint_annotation(&self, painter: &egui::Painter, ann: &Annotation) {
        let s = self.view().scale;
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
                    let w = ((*width).max(MIN_HIGHLIGHT_WIDTH) * s).max(4.0);
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
                self.paint_text(painter, *pos, text, *size, rgba(color));
            }
        }
    }

    /// Paint text with the export's metrics and glyph positions, so a dragged
    /// or resized label keeps the size it will have once committed.
    fn paint_text(&self, painter: &egui::Painter, pos: Pos2, text: &str, size: f32, color: Color32) {
        use ab_glyph::Font;
        let view = self.view();
        let s = view.scale;
        let Some(font) = &self.font else {
            painter.text(self.img_to_screen(pos), Align2::LEFT_TOP, text, FontId::proportional((size * s).max(1.0)), color);
            return;
        };
        // The export scales glyphs so ascent - descent equals `size` pixels,
        // while an egui font size is the em size: convert between the two.
        let em = font.units_per_em().unwrap_or(1.0) / font.height_unscaled();
        let font_id = FontId::proportional((size * em * s).max(1.0));
        let layout = layout_text(font, pos, text, size);
        // Each distinct character is laid out once, under a single fonts lock.
        let placed = painter.fonts(|fonts| {
            let mut cache: HashMap<char, Arc<egui::Galley>> = HashMap::new();
            layout
                .glyphs
                .iter()
                .filter(|g| !g.ch.is_whitespace())
                .map(|g| {
                    let galley = cache
                        .entry(g.ch)
                        .or_insert_with(|| fonts.layout_no_wrap(g.ch.to_string(), font_id.clone(), color))
                        .clone();
                    (view.to_screen(Pos2::new(g.x, g.top)), galley)
                })
                .collect::<Vec<_>>()
        });
        for (at, galley) in placed {
            painter.galley(at, galley, color);
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
        self.img_rect_to_screen(ann.selection_bounds(self.font.as_ref())).expand(SELECTION_PAD)
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
                if entry.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    commit = true;
                }
                if ui.input(|i| i.key_pressed(CANCEL_KEY)) {
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
                    size: self.tool_width(Tool::Text),
                });
            }
        } else if !cancel {
            self.text_state = Some(st);
        }
    }

    /// Ask before an incoming image replaces annotations that were never saved.
    fn replace_dialog(&mut self, ctx: &Context) {
        let Some(pending) = &self.pending else {
            return;
        };
        let p = Palette::of(ctx);
        let count = self.annotations.len();
        let current = self.doc_name.clone().unwrap_or_else(|| "the current image".to_owned());
        let incoming = pending.name.clone();
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("replace-image")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(egui::RichText::new("Replace the current image?").size(16.0).strong());
            ui.add_space(4.0);
            ui.label(format!(
                "{count} annotation{} on “{current}” {} not been saved. Opening “{incoming}” discards {}.",
                if count == 1 { "" } else { "s" },
                if count == 1 { "has" } else { "have" },
                if count == 1 { "it" } else { "them" },
            ));
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let replace = egui::Button::new(egui::RichText::new("Replace Image").color(p.on_accent)).fill(p.accent);
                if ui.add(replace).clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                if let Some(PendingImage { image, name }) = self.pending.take() {
                    self.set_image(image, name);
                }
            }
            Some(false) => self.pending = None,
            None => {}
        }
    }

    fn shortcuts_window(&mut self, ctx: &Context) {
        if !self.show_shortcuts {
            return;
        }
        let rows: Vec<(String, &str)> = {
            let s = |a| Self::shortcut(ctx, a).unwrap_or_default();
            let all_keys = |a| {
                Self::plain_keys(a).iter().map(|k| k.symbol_or_name()).collect::<Vec<_>>().join(" / ")
            };
            let mut rows = vec![
                (s(Action::Open), "Open image"),
                (s(Action::Paste), "Paste image from clipboard"),
                (s(Action::Save), "Save as PNG or JPEG"),
                (s(Action::Copy), "Copy annotated image"),
                (s(Action::Undo), "Undo"),
                (s(Action::Redo), "Redo"),
                (all_keys(Action::DeleteSelected), "Delete selected annotation"),
                (CANCEL_KEY.symbol_or_name().to_owned(), "Cancel drawing, deselect"),
            ];
            for tool in Tool::ALL {
                let info = tool_info(tool);
                if let Some(k) = info.key {
                    rows.push((k.name().to_owned(), info.name));
                }
            }
            rows.extend([
                (format!("{} / {}", s(Action::ZoomIn), s(Action::ZoomOut)), "Zoom in / out"),
                (s(Action::ZoomFit), "Fit to window"),
                ("Space + drag".to_owned(), "Pan"),
            ]);
            rows
        };
        let mut open = true;
        egui::Window::new("Keyboard Shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (keys, what) in rows {
                        ui.label(egui::RichText::new(keys).strong());
                        ui.label(what);
                        ui.end_row();
                    }
                });
            });
        self.show_shortcuts &= open;
    }

    /// Mirror the document name and edited state in the native title bar.
    fn update_title(&mut self, ctx: &Context) {
        let title = match &self.doc_name {
            Some(name) if self.has_unsaved_work() => format!("{name} — Edited"),
            Some(name) => name.clone(),
            None => "Notely".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
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
            if crate::macos_paste::take_requested() && self.pending.is_none() {
                self.paste_image();
            }
        }

        self.handle_dropped(ctx);
        self.frame(ctx);
    }
}

impl NotelyApp {
    /// One frame of the platform-independent UI.
    pub(crate) fn frame(&mut self, ctx: &Context) {
        self.handle_keys(ctx);
        if self.dirty {
            self.rebuild(ctx);
        }
        self.top_bar(ctx);
        self.bottom_bar(ctx);
        self.canvas(ctx);
        self.text_window(ctx);
        self.shortcuts_window(ctx);
        self.replace_dialog(ctx);
        self.update_title(ctx);
        self.widget_had_focus = ctx.wants_keyboard_input();

        // Changes made during this frame (e.g. a finished drag or a loaded image)
        // need one more frame for the updated texture to reach the screen.
        if self.dirty {
            ctx.request_repaint();
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn is_supported_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
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

/// Keep drawing coordinates inside the image instead of letting a gesture that
/// runs into the dark padding create negative or out-of-range annotations.
fn clamp_to_image(p: Pos2, width: u32, height: u32) -> Pos2 {
    Pos2::new(
        p.x.clamp(0.0, width as f32),
        p.y.clamp(0.0, height as f32),
    )
}

#[cfg(test)]
mod snapshots;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_drawing_positions_to_the_image() {
        assert_eq!(
            clamp_to_image(Pos2::new(-5.0, 150.0), 100, 120),
            Pos2::new(0.0, 120.0)
        );
        assert_eq!(
            clamp_to_image(Pos2::new(40.0, 60.0), 100, 120),
            Pos2::new(40.0, 60.0)
        );
    }

    fn image(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]))
    }

    fn rect_ann(x: f32) -> Annotation {
        Annotation::Rect {
            rect: Rect::from_min_size(Pos2::new(x, 10.0), Vec2::splat(20.0)),
            color: [255, 0, 0, 255],
            width: 4.0,
        }
    }

    fn app_with_image() -> NotelyApp {
        let mut app = NotelyApp::blank();
        app.offer_image(image(200, 100), "test.png".to_owned());
        app
    }

    #[test]
    fn dragged_text_matches_exported_size() {
        use ab_glyph::{Font, PxScale, ScaleFont};
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        // No image loaded, so the view is 1:1.
        let app = NotelyApp::blank();
        let size = 64.0;
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            app.paint_text(&painter, Pos2::ZERO, "H", size, Color32::WHITE);
        });
        let shapes = out.shapes;
        let glyph = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) => t.galley.rows.first()?.glyphs.first().copied(),
                _ => None,
            })
            .expect("text painted");
        let font = app.font.as_ref().unwrap();
        let scaled = font.as_scaled(PxScale::from(size));
        let outline = font.outline_glyph(scaled.scaled_glyph('H')).unwrap();
        let export_h = outline.px_bounds().height();
        let overlay_h = glyph.uv_rect.size.y;
        assert!((overlay_h - export_h).abs() <= 2.0, "overlay {overlay_h} vs export {export_h}");
    }

    #[test]
    fn actions_are_only_available_when_they_can_run() {
        let mut app = NotelyApp::blank();
        for a in [Action::Save, Action::Copy, Action::Undo, Action::Redo, Action::DeleteSelected, Action::ClearAll, Action::ZoomIn] {
            assert!(!app.enabled(a), "enabled without an image");
        }
        assert!(app.enabled(Action::Open) && app.enabled(Action::Paste));

        app.offer_image(image(200, 100), "test.png".to_owned());
        assert!(app.enabled(Action::Save) && app.enabled(Action::Copy) && app.enabled(Action::ZoomFit));
        assert!(!app.enabled(Action::Undo) && !app.enabled(Action::ClearAll));

        app.push(rect_ann(10.0));
        assert!(app.enabled(Action::Undo) && app.enabled(Action::ClearAll));
        assert!(!app.enabled(Action::DeleteSelected), "nothing selected");

        app.selected = Some(0);
        assert!(app.enabled(Action::DeleteSelected));
        // A gesture in progress blocks history and deletion.
        app.drag = Some(Drag { tool: Tool::Rect, start: Pos2::ZERO, current: Pos2::ZERO, points: vec![] });
        assert!(!app.enabled(Action::DeleteSelected) && !app.enabled(Action::Undo));
        app.drag = None;

        app.run(Action::Tool(Tool::Arrow));
        assert!(!app.enabled(Action::DeleteSelected), "selection is dropped by other tools");

        app.run(Action::Undo);
        assert!(app.enabled(Action::Redo) && !app.enabled(Action::Undo));
    }

    #[test]
    fn delete_selected_is_undoable() {
        let mut app = app_with_image();
        app.push(rect_ann(10.0));
        app.push(rect_ann(50.0));
        app.selected = Some(0);
        app.run(Action::DeleteSelected);
        assert_eq!(app.annotations, vec![rect_ann(50.0)]);
        assert_eq!(app.selected, None);
        app.run(Action::Undo);
        assert_eq!(app.annotations, vec![rect_ann(10.0), rect_ann(50.0)]);
    }

    #[test]
    fn clear_all_keeps_the_image_and_is_undoable() {
        let mut app = app_with_image();
        app.push(rect_ann(10.0));
        app.run(Action::ClearAll);
        assert!(app.annotations.is_empty());
        assert!(app.base.is_some());
        app.run(Action::Undo);
        assert_eq!(app.annotations.len(), 1);
    }

    #[test]
    fn replacing_unsaved_work_asks_first() {
        let mut app = app_with_image();
        // Nothing drawn yet: a new image replaces the old one straight away.
        app.offer_image(image(10, 10), "second.png".to_owned());
        assert!(app.pending.is_none());
        assert_eq!(app.base.as_ref().unwrap().dimensions(), (10, 10));

        app.push(rect_ann(1.0));
        assert!(app.has_unsaved_work());
        app.offer_image(image(30, 30), "third.png".to_owned());
        assert!(app.pending.is_some(), "must wait for confirmation");
        assert_eq!(app.base.as_ref().unwrap().dimensions(), (10, 10));
        assert_eq!(app.annotations.len(), 1, "work is untouched while asking");

        // An image that can't be used never prompts or clears work.
        app.pending = None;
        app.offer_image(image(0, 0), "empty.png".to_owned());
        assert!(app.pending.is_none());
        assert_eq!(app.annotations.len(), 1);
        assert_eq!(app.notice.as_ref().unwrap().kind, NoticeKind::Error);

        // After an export the work is safe, so no prompt.
        app.mark_saved();
        app.offer_image(image(30, 30), "third.png".to_owned());
        assert!(app.pending.is_none());
        assert!(app.annotations.is_empty());
        assert_eq!(app.doc_name.as_deref(), Some("third.png"));
    }

    #[test]
    fn unsaved_tracks_the_saved_history_position() {
        let mut app = app_with_image();
        app.push(rect_ann(1.0));
        app.mark_saved();
        assert!(!app.has_unsaved_work());

        app.push(rect_ann(2.0));
        assert!(app.has_unsaved_work());
        app.run(Action::Undo);
        assert!(!app.has_unsaved_work(), "undo back to the saved state is clean");
        app.run(Action::Undo);
        app.run(Action::Redo);
        assert!(!app.has_unsaved_work(), "redo back to the saved state is clean");

        // A new edit while the saved state is in the redo stack discards it.
        app.run(Action::Undo);
        app.push(rect_ann(3.0));
        app.run(Action::Undo);
        app.push(rect_ann(1.0));
        assert!(app.has_unsaved_work());
    }

    #[test]
    fn copying_does_not_count_as_saving() {
        let mut app = app_with_image();
        app.push(rect_ann(1.0));
        app.run(Action::Copy);
        assert!(app.has_unsaved_work());
    }

    #[test]
    fn saved_depth_follows_history_trimming() {
        let mut app = app_with_image();
        app.push(rect_ann(1.0));
        app.mark_saved();
        for i in 0..MAX_HISTORY {
            app.push(rect_ann(i as f32 + 2.0));
        }
        // The oldest edit was trimmed, but the saved state is still the bottom.
        for _ in 0..MAX_HISTORY {
            app.run(Action::Undo);
        }
        assert!(!app.has_unsaved_work());

        for _ in 0..MAX_HISTORY {
            app.run(Action::Redo);
        }
        app.push(rect_ann(99.0));
        for _ in 0..MAX_HISTORY {
            app.run(Action::Undo);
        }
        assert!(app.has_unsaved_work(), "the saved state fell off the history");
    }

    #[test]
    fn escape_cancels_gestures_without_touching_committed_work() {
        let mut app = app_with_image();
        app.push(rect_ann(10.0));
        app.tool = Tool::Arrow;
        app.drag = Some(Drag {
            tool: Tool::Arrow,
            start: Pos2::new(0.0, 0.0),
            current: Pos2::new(50.0, 50.0),
            points: vec![],
        });
        app.escape();
        assert!(app.drag.is_none());
        assert_eq!(app.annotations.len(), 1, "cancelled arrow is not committed");

        // A cancelled move restores the annotation.
        app.tool = Tool::Select;
        app.annotations[0] = rect_ann(99.0);
        app.moving = Some(MoveDrag {
            index: 0,
            grab: Pos2::ZERO,
            current: Pos2::new(89.0, 0.0),
            original: rect_ann(10.0),
            handle: None,
        });
        app.escape();
        assert!(app.moving.is_none());
        assert_eq!(app.annotations, vec![rect_ann(10.0)]);
        assert_eq!(app.undo.len(), 1);

        // Then Escape deselects.
        app.selected = Some(0);
        app.escape();
        assert_eq!(app.selected, None);
        assert_eq!(app.annotations.len(), 1);
    }

    #[test]
    fn each_tool_uses_its_own_width_setting() {
        let mut app = app_with_image();
        app.stroke_width = 3.0;
        app.highlight_width = 20.0;
        app.blur_strength = 12.0;
        let drag = |tool| Drag {
            tool,
            start: Pos2::new(10.0, 10.0),
            current: Pos2::new(60.0, 60.0),
            points: vec![Pos2::new(10.0, 10.0), Pos2::new(60.0, 60.0)],
        };
        for tool in [Tool::Arrow, Tool::Highlight, Tool::Blur] {
            app.drag = Some(drag(tool));
            app.commit_drag();
        }
        assert!(matches!(app.annotations[0], Annotation::Arrow { width, .. } if width == 3.0));
        assert!(matches!(app.annotations[1], Annotation::Highlight { width, .. } if width == 20.0));
        assert!(matches!(app.annotations[2], Annotation::Blur { sigma, .. } if sigma == 12.0));
    }

    #[test]
    fn tool_sizes_are_clamped_to_their_slider_range() {
        let mut app = app_with_image();
        for tool in Tool::ALL {
            let Some(setting) = size_setting(tool) else { continue };
            *app.size_mut(tool) = -1.0;
            assert_eq!(app.tool_width(tool), *setting.range.start());
            *app.size_mut(tool) = 1e6;
            assert_eq!(app.tool_width(tool), *setting.range.end());
        }
    }

    fn press(ctx: &Context, app: &mut NotelyApp, key: Key, modifiers: Modifiers) {
        let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let input = egui::RawInput { events: vec![event], modifiers, ..Default::default() };
        let _ = ctx.run(input, |ctx| app.handle_keys(ctx));
    }

    #[test]
    fn shortcuts_route_to_actions() {
        let ctx = Context::default();
        let mut app = app_with_image();
        press(&ctx, &mut app, Key::R, Modifiers::NONE);
        assert!(app.tool == Tool::Rect);
        press(&ctx, &mut app, Key::A, Modifiers::NONE);
        assert!(app.tool == Tool::Arrow);
        // Letters with a modifier are not tool shortcuts.
        press(&ctx, &mut app, Key::S, Modifiers::SHIFT);
        assert!(app.tool == Tool::Arrow);

        app.push(rect_ann(10.0));
        press(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
        assert!(app.annotations.is_empty());
        press(&ctx, &mut app, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(app.annotations.len(), 1);

        app.tool = Tool::Select;
        app.selected = Some(0);
        press(&ctx, &mut app, Key::Delete, Modifiers::NONE);
        assert!(app.annotations.is_empty());
    }

    #[test]
    fn shortcuts_leave_text_entry_alone() {
        let ctx = Context::default();
        let mut app = app_with_image();
        app.push(rect_ann(10.0));
        app.tool = Tool::Select;
        app.selected = Some(0);
        app.text_state = Some(TextState { pos: Pos2::ZERO, buffer: String::new(), focused: true });
        press(&ctx, &mut app, Key::R, Modifiers::NONE);
        press(&ctx, &mut app, Key::Backspace, Modifiers::NONE);
        press(&ctx, &mut app, Key::Z, Modifiers::COMMAND);
        assert!(app.tool == Tool::Select);
        assert_eq!(app.annotations.len(), 1);
    }

    #[test]
    fn escape_from_a_focused_widget_keeps_the_selection() {
        let ctx = Context::default();
        let mut app = app_with_image();
        app.push(rect_ann(10.0));
        app.selected = Some(0);
        app.widget_had_focus = true;
        press(&ctx, &mut app, Key::Escape, Modifiers::NONE);
        assert_eq!(app.selected, Some(0));

        app.widget_had_focus = false;
        press(&ctx, &mut app, Key::Escape, Modifiers::NONE);
        assert_eq!(app.selected, None);
    }

    #[test]
    fn zoom_shortcuts_change_only_the_view() {
        let ctx = Context::default();
        let mut app = app_with_image();
        app.canvas_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        app.push(rect_ann(10.0));
        let before = app.annotations.clone();
        press(&ctx, &mut app, Key::Plus, Modifiers::SHIFT);
        assert!(!app.viewport.fit);
        assert_eq!(app.viewport.zoom, 1.5);
        press(&ctx, &mut app, Key::Minus, Modifiers::NONE);
        assert_eq!(app.viewport.zoom, 1.0);
        press(&ctx, &mut app, Key::Num0, Modifiers::NONE);
        assert!(app.viewport.fit);
        assert_eq!(app.annotations, before);
    }

    #[test]
    fn toolbar_offers_every_tool_once() {
        let listed: Vec<Tool> = TOOL_GROUPS.iter().flat_map(|g| g.iter().copied()).collect();
        assert_eq!(listed.len(), Tool::ALL.len());
        for tool in Tool::ALL {
            assert_eq!(listed.iter().filter(|t| **t == tool).count(), 1);
        }
    }

    #[test]
    fn toolbar_tiers_follow_window_width() {
        assert_eq!(Tier::for_width(1440.0), Tier::Wide);
        assert_eq!(Tier::for_width(1100.0), Tier::Wide);
        assert_eq!(Tier::for_width(1099.0), Tier::Medium);
        assert_eq!(Tier::for_width(800.0), Tier::Medium);
        assert_eq!(Tier::for_width(760.0), Tier::Narrow);
    }

    #[test]
    fn drop_validation_uses_supported_extensions() {
        assert!(is_supported_path(std::path::Path::new("/a/shot.PNG")));
        assert!(is_supported_path(std::path::Path::new("photo.tif")));
        assert!(!is_supported_path(std::path::Path::new("notes.txt")));
        assert!(!is_supported_path(std::path::Path::new("noext")));
    }
}
