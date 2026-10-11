#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod annotation;
mod app;
#[cfg(target_os = "macos")]
mod macos_open;
mod render;

fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../icons/icon.png")).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData { rgba: rgba.into_raw(), width, height })
}

fn main() -> eframe::Result {
    #[cfg(target_os = "macos")]
    macos_open::install();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 860.0])
        .with_min_inner_size([760.0, 560.0])
        .with_title("Notely");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Notely",
        options,
        Box::new(|cc| Ok(Box::new(app::NotelyApp::new(cc)))),
    )
}
