mod app;
mod config;
mod twitch;

fn main() {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
    run_app();
}

#[cfg(feature = "desktop")]
fn run_app() {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default(),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "TwAura",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(app::MyApp::new(&cc.egui_ctx)))
        }),
    )
    .unwrap();
}

#[cfg(feature = "aurora")]
fn run_app() {
    let options = aurora_egui::NativeOptions {
        viewport: egui::ViewportBuilder::default(),
        ..Default::default()
    };
    aurora_egui::run_native(
        "TwAura",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::new(app::MyApp::new(&cc.egui_ctx))
        }),
    )
    .unwrap();
}
