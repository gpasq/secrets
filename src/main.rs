// Don't open a console window alongside the app on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod aws;
mod connections;
mod highlight;
mod profiles;
mod samples;
mod theme;
mod util;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Secrets")
            .with_inner_size([1320.0, 880.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
                    .expect("bundled icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Secrets",
        options,
        Box::new(|cc| Ok(Box::new(app::SecretsApp::new(cc)))),
    )
}
