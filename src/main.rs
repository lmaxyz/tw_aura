mod app;
mod twitch;

fn main() {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
    println!(
        "Cookie exists on '{}': {}",
        std::env::home_dir()
            .unwrap()
            .join(".pulse-cookie")
            .to_str()
            .unwrap(),
        std::env::home_dir().unwrap().join(".pulse-cookie").exists()
    );
    println!(
        "Cookie exists on '{}': {}",
        std::env::home_dir()
            .unwrap()
            .join(".config/pulse/cookie")
            .to_str()
            .unwrap(),
        std::env::home_dir()
            .unwrap()
            .join(".config/pulse/cookie")
            .exists()
    );
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
            // This gives us image support:
            // egui_extras::install_image_loaders(&cc.egui_ctx);
            cc.egui_ctx.set_pixels_per_point(2.0);
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
        Box::new(|cc| Box::new(app::MyApp::new(&cc.egui_ctx))),
    )
    .unwrap();
}
