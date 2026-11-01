//! Custom-painted controls: icon command buttons and color swatches. They
//! share the theme tokens and draw their own hover, pressed, active, focus and
//! disabled states.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType};

use crate::icons::{self, Icon};
use crate::theme::{self, Palette};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Subdued until hovered.
    Plain,
    /// The selected tool: filled accent with a contrasting ring.
    Active,
    /// The one primary command (Save).
    Primary,
    /// Filled neutral button, for use next to a primary one.
    Secondary,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Shape {
    /// Square icon-only button with the given side.
    Icon(f32),
    /// Icon above a small caption.
    Stacked,
    /// Icon followed by the label, with the given height.
    Inline(f32),
}

pub struct Command<'a> {
    pub icon: Icon,
    pub label: &'a str,
    /// Shortcut shown in the tooltip, when the command has one.
    pub shortcut: Option<String>,
    /// Extra tooltip line, e.g. why the command is unavailable.
    pub detail: Option<&'a str>,
    pub shape: Shape,
    pub look: Look,
    pub enabled: bool,
    /// Whether this is a toggle (a tool) that is currently on.
    pub selected: Option<bool>,
    /// Drawn over the dark canvas, so use the dark tokens in either theme.
    pub on_canvas: bool,
}

impl<'a> Command<'a> {
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            shortcut: None,
            detail: None,
            shape: Shape::Icon(theme::BUTTON),
            look: Look::Plain,
            enabled: true,
            selected: None,
            on_canvas: false,
        }
    }

    pub fn on_canvas(mut self) -> Self {
        self.on_canvas = true;
        self
    }

    pub fn shortcut(mut self, s: impl Into<Option<String>>) -> Self {
        self.shortcut = s.into();
        self
    }

    pub fn detail(mut self, d: &'a str) -> Self {
        self.detail = Some(d);
        self
    }

    pub fn shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }

    pub fn look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn toggle(mut self, on: bool) -> Self {
        self.selected = Some(on);
        if on {
            self.look = Look::Active;
        }
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        ui.scope(|ui| {
            if !self.enabled {
                ui.disable();
            }
            self.paint(ui)
        })
        .inner
    }

    fn paint(self, ui: &mut Ui) -> Response {
        let p = if self.on_canvas { theme::DARK } else { Palette::of(ui.ctx()) };
        let caption = egui::FontId::proportional(11.5);
        let body = egui::FontId::proportional(13.0);
        let galley = |font: egui::FontId| {
            ui.painter().layout_no_wrap(self.label.to_owned(), font, Color32::PLACEHOLDER)
        };
        let (size, label) = match self.shape {
            Shape::Icon(side) => (Vec2::splat(side), None),
            Shape::Stacked => {
                let g = galley(caption);
                (Vec2::new((g.size().x + 14.0).max(46.0), 48.0), Some(g))
            }
            Shape::Inline(h) => {
                let g = galley(body);
                (Vec2::new(theme::ICON + g.size().x + 26.0, h), Some(g))
            }
        };
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let label_text = self.label.to_owned();
        let enabled = self.enabled;
        match self.selected {
            Some(on) => resp.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, enabled, on, &label_text)
            }),
            None => resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label_text)),
        }

        if ui.is_rect_visible(rect) {
            let hovered = resp.hovered() && enabled;
            let pressed = resp.is_pointer_button_down_on() && enabled;
            let (bg, fg) = match self.look {
                Look::Active => (p.accent, p.on_accent),
                Look::Primary if !enabled => (p.group, p.text_disabled),
                Look::Primary => (if hovered { p.accent_hover } else { p.accent }, p.on_accent),
                Look::Secondary if !enabled => (p.group, p.text_disabled),
                Look::Secondary if pressed => (p.pressed, p.text),
                Look::Secondary if hovered => (p.hover, p.text),
                Look::Secondary => (p.group, p.text),
                Look::Plain if !enabled => (Color32::TRANSPARENT, p.text_disabled),
                Look::Plain if pressed => (p.pressed, p.text),
                Look::Plain if hovered => (p.hover, p.text),
                Look::Plain => (Color32::TRANSPARENT, p.text),
            };
            let radius = CornerRadius::same(theme::RADIUS);
            let painter = ui.painter();
            painter.rect_filled(rect, radius, bg);
            if self.look == Look::Secondary {
                painter.rect_stroke(rect, radius, Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
            }
            if self.look == Look::Active {
                // A shape cue in addition to color: an inner contrasting ring.
                painter.rect_stroke(
                    rect.shrink(1.5),
                    CornerRadius::same(theme::RADIUS - 1),
                    Stroke::new(1.0_f32, Color32::from_white_alpha(110)),
                    egui::StrokeKind::Inside,
                );
            }
            if pressed && matches!(self.look, Look::Active | Look::Primary) {
                painter.rect_filled(rect, radius, Color32::from_black_alpha(40));
            }
            let icon_rect = |center: egui::Pos2| egui::Rect::from_center_size(center, Vec2::splat(theme::ICON));
            match (self.shape, label) {
                (Shape::Icon(_), _) | (_, None) => {
                    icons::paint(painter, icon_rect(rect.center()), self.icon, fg);
                }
                (Shape::Stacked, Some(g)) => {
                    icons::paint(painter, icon_rect(rect.center_top() + Vec2::new(0.0, 17.0)), self.icon, fg);
                    let pos = rect.center_bottom() - Vec2::new(g.size().x / 2.0, g.size().y + 5.0);
                    painter.galley(pos, g, fg);
                }
                (Shape::Inline(_), Some(g)) => {
                    let x = rect.left() + 10.0;
                    icons::paint(painter, icon_rect(egui::pos2(x + theme::ICON / 2.0, rect.center().y)), self.icon, fg);
                    let pos = egui::pos2(x + theme::ICON + 6.0, rect.center().y - g.size().y / 2.0);
                    painter.galley(pos, g, fg);
                }
            }
            if resp.has_focus() {
                painter.rect_stroke(
                    rect.expand(1.0),
                    CornerRadius::same(theme::RADIUS + 1),
                    Stroke::new(2.0_f32, p.focus),
                    egui::StrokeKind::Outside,
                );
            }
        }

        let tip = tooltip(self.label, self.shortcut.as_deref(), self.detail);
        if enabled {
            resp.on_hover_text(tip)
        } else {
            resp.on_disabled_hover_text(tip)
        }
    }
}

/// "Label (shortcut)" plus an optional second line.
pub fn tooltip(label: &str, shortcut: Option<&str>, detail: Option<&str>) -> String {
    let mut s = match shortcut {
        Some(k) => format!("{label}  ({k})"),
        None => label.to_owned(),
    };
    if let Some(d) = detail {
        s.push('\n');
        s.push_str(d);
    }
    s
}

/// A round color swatch; selected swatches get a ring and a check mark.
pub fn swatch(ui: &mut Ui, color: Color32, selected: bool, name: &str) -> Response {
    let p = Palette::of(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, name));
    let painter = ui.painter();
    let c = rect.center();
    if selected || resp.hovered() {
        let ring = if selected { p.accent } else { p.border };
        painter.circle_stroke(c, 12.0, Stroke::new(2.0_f32, ring));
    }
    painter.circle_filled(c, 9.0, color);
    // Light swatches need an edge to stand out from a light background.
    painter.circle_stroke(c, 9.0, Stroke::new(1.0_f32, Color32::from_black_alpha(50)));
    if selected {
        let mark = if luminance(color) > 0.6 { Color32::BLACK } else { Color32::WHITE };
        icons::paint(painter, egui::Rect::from_center_size(c, Vec2::splat(13.0)), Icon::Check, mark);
    }
    if resp.has_focus() {
        painter.circle_stroke(c, 13.5, Stroke::new(2.0_f32, p.focus));
    }
    resp.on_hover_text(name)
}

/// The current color shown as a dropdown trigger.
pub fn color_button(ui: &mut Ui, color: Color32, open: bool) -> Response {
    let p = Palette::of(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(50.0, 28.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, true, "Color"));
    let painter = ui.painter();
    let bg = if open || resp.is_pointer_button_down_on() {
        p.pressed
    } else if resp.hovered() {
        p.hover
    } else {
        p.group
    };
    painter.rect(rect, CornerRadius::same(6), bg, Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
    let sw = egui::Rect::from_min_size(rect.min + Vec2::new(5.0, 5.0), Vec2::new(20.0, 18.0));
    painter.rect_filled(sw, CornerRadius::same(4), color);
    painter.rect_stroke(sw, CornerRadius::same(4), Stroke::new(1.0_f32, Color32::from_black_alpha(60)), egui::StrokeKind::Inside);
    let chev = egui::Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y), Vec2::splat(13.0));
    icons::paint(painter, chev, Icon::ChevronDown, p.text_muted);
    if resp.has_focus() {
        painter.rect_stroke(rect.expand(1.0), CornerRadius::same(7), Stroke::new(2.0_f32, p.focus), egui::StrokeKind::Outside);
    }
    resp.on_hover_text("Color")
}

/// Relative luminance in 0..=1 (sRGB approximation).
fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

/// A compact text dropdown trigger (e.g. the zoom percentage).
pub fn text_dropdown(ui: &mut Ui, text: &str, width: f32, open: bool) -> Response {
    let p = Palette::of(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, ui.is_enabled(), text));
    let enabled = ui.is_enabled();
    let painter = ui.painter();
    let bg = if !enabled {
        Color32::TRANSPARENT
    } else if open || resp.is_pointer_button_down_on() {
        p.pressed
    } else if resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    let fg = if enabled { p.text } else { p.text_disabled };
    painter.rect_filled(rect, CornerRadius::same(6), bg);
    painter.text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.0),
        fg,
    );
    let chev = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), Vec2::splat(12.0));
    icons::paint(painter, chev, Icon::ChevronDown, if enabled { p.text_muted } else { p.text_disabled });
    if resp.has_focus() {
        painter.rect_stroke(rect.expand(1.0), CornerRadius::same(7), Stroke::new(2.0_f32, p.focus), egui::StrokeKind::Outside);
    }
    resp
}

/// A thin vertical rule between command groups.
pub fn divider(ui: &mut Ui, height: f32) {
    let p = Palette::of(ui.ctx());
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.border);
    ui.add_space(6.0);
}

/// One row of a popup menu: label on the left, shortcut (or a check mark for
/// the current choice) on the right, with a full-width hover highlight.
pub fn menu_item(ui: &mut Ui, label: &str, shortcut: Option<&str>, checked: bool) -> Response {
    let p = Palette::of(ui.ctx());
    let width = ui.available_width().max(ui.min_rect().width());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), checked, label));
    let enabled = ui.is_enabled();
    let painter = ui.painter();
    if enabled && (resp.hovered() || resp.has_focus()) {
        let bg = if resp.is_pointer_button_down_on() { p.accent_hover } else { p.accent };
        painter.rect_filled(rect, CornerRadius::same(5), bg);
    }
    let highlighted = enabled && (resp.hovered() || resp.has_focus());
    let (fg, muted) = if !enabled {
        (p.text_disabled, p.text_disabled)
    } else if highlighted {
        (p.on_accent, p.on_accent)
    } else {
        (p.text, p.text_muted)
    };
    let font = egui::FontId::proportional(13.0);
    painter.text(egui::pos2(rect.left() + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), fg);
    if checked {
        let r = egui::Rect::from_center_size(egui::pos2(rect.right() - 17.0, rect.center().y), Vec2::splat(14.0));
        icons::paint(painter, r, Icon::Check, fg);
    } else if let Some(s) = shortcut {
        painter.text(egui::pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, s, font, muted);
    }
    resp
}

/// A horizontal rule between groups of menu rows.
pub fn menu_separator(ui: &mut Ui) {
    let p = Palette::of(ui.ctx());
    let width = ui.available_width().max(ui.min_rect().width());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 9.0), Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().hline(rect.x_range().shrink(6.0), y, Stroke::new(1.0_f32, p.border));
}
