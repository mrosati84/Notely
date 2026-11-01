//! Colors and spacing shared by the custom-painted controls, for both the light
//! and dark system appearance.

use egui::{Color32, Context, CornerRadius, Stroke, Theme, Visuals};

/// Design tokens for one appearance.
#[derive(Clone, Copy)]
pub struct Palette {
    /// Toolbar and status bar background.
    pub chrome: Color32,
    /// Background of a group of related commands.
    pub group: Color32,
    /// Near-black surround behind the image (same in both appearances).
    pub canvas: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// The single accent: active tool and the primary Save command.
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    /// Keyboard focus ring.
    pub focus: Color32,
    pub success: Color32,
    pub error: Color32,
}

pub const DARK: Palette = Palette {
    chrome: Color32::from_rgb(38, 39, 43),
    group: Color32::from_rgb(48, 49, 54),
    canvas: Color32::from_rgb(22, 23, 26),
    border: Color32::from_rgb(64, 66, 72),
    text: Color32::from_rgb(232, 233, 236),
    text_muted: Color32::from_rgb(165, 167, 175),
    text_disabled: Color32::from_rgb(98, 100, 108),
    hover: Color32::from_rgb(62, 64, 70),
    pressed: Color32::from_rgb(74, 76, 84),
    accent: Color32::from_rgb(37, 99, 235),
    accent_hover: Color32::from_rgb(59, 115, 245),
    on_accent: Color32::WHITE,
    focus: Color32::from_rgb(120, 170, 255),
    success: Color32::from_rgb(74, 210, 130),
    error: Color32::from_rgb(255, 120, 110),
};

const LIGHT: Palette = Palette {
    chrome: Color32::from_rgb(246, 246, 248),
    group: Color32::from_rgb(236, 236, 240),
    canvas: Color32::from_rgb(30, 31, 34),
    border: Color32::from_rgb(214, 214, 220),
    text: Color32::from_rgb(28, 28, 32),
    text_muted: Color32::from_rgb(88, 89, 98),
    text_disabled: Color32::from_rgb(168, 169, 176),
    hover: Color32::from_rgb(222, 222, 228),
    pressed: Color32::from_rgb(208, 208, 216),
    accent: Color32::from_rgb(37, 99, 235),
    accent_hover: Color32::from_rgb(29, 84, 210),
    on_accent: Color32::WHITE,
    focus: Color32::from_rgb(37, 99, 235),
    success: Color32::from_rgb(20, 128, 60),
    error: Color32::from_rgb(196, 40, 32),
};

impl Palette {
    pub fn of(ctx: &Context) -> Palette {
        if ctx.style().visuals.dark_mode {
            DARK
        } else {
            LIGHT
        }
    }
}

/// Radius of command groups and buttons.
pub const RADIUS: u8 = 7;
/// Glyph size of toolbar icons.
pub const ICON: f32 = 18.0;
/// Side of an icon-only button's hit area.
pub const BUTTON: f32 = 34.0;

/// Follow the system appearance and apply the tokens to egui's own widgets
/// (sliders, menus, tooltips, dialogs).
pub fn install(ctx: &Context) {
    ctx.set_theme(egui::ThemePreference::System);
    ctx.set_visuals_of(Theme::Dark, visuals(Visuals::dark(), &DARK));
    ctx.set_visuals_of(Theme::Light, visuals(Visuals::light(), &LIGHT));
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.spacing.interact_size.y = 26.0;
            style.spacing.menu_margin = egui::Margin::same(6);
            style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
            style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
            style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
        });
    }
    // Cmd/Ctrl +, - and 0 zoom the canvas instead of the whole interface.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

fn visuals(mut v: Visuals, p: &Palette) -> Visuals {
    let radius = CornerRadius::same(RADIUS);
    v.panel_fill = p.chrome;
    v.window_fill = p.chrome;
    v.extreme_bg_color = if v.dark_mode { p.canvas } else { Color32::WHITE };
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0_f32, p.on_accent);
    v.hyperlink_color = p.accent;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    v.widgets.inactive.weak_bg_fill = p.group;
    v.widgets.inactive.bg_fill = p.group;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.focus);
    v.widgets.active.weak_bg_fill = p.pressed;
    v.widgets.active.bg_fill = p.pressed;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    v
}
