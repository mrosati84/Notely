use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const INTER_NAME: &str = "Inter";
pub const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Makes Inter the primary font for every egui font family, keeping the
/// default fonts as fallbacks (emoji, symbols, missing glyphs).
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        INTER_NAME.to_owned(),
        Arc::new(FontData::from_static(INTER_REGULAR)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, INTER_NAME.to_owned());
    }
    ctx.set_fonts(fonts);
}
