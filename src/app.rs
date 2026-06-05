use std::sync::{Arc, RwLock};
use std::time::Instant;

use egui::Color32;
use ffmpeg_next as ffmpeg;
use twitch_api::helix::streams::Stream;

use crate::config::Config;
use crate::twitch::ui::auth::AuthView;
use crate::twitch::ui::player::PlayerView;
use crate::twitch::ui::streams_list;

pub struct MyApp {
    streamer_login: String,
    player_view: Option<PlayerView>,
    streams: Arc<RwLock<Vec<Stream>>>,
    access_token: Option<String>,
    auth_view: AuthView,
    last_streams_load: Option<Instant>,
}

impl MyApp {
    pub fn new(_ctx: &egui::Context) -> Self {
        ffmpeg::init().unwrap();
        let config = Config::load();
        let access_token = config.and_then(|c| c.access_token);

        Self {
            streamer_login: "".to_string(),
            player_view: None,
            streams: Arc::new(RwLock::new(Vec::new())),
            access_token,
            auth_view: AuthView::default(),
            last_streams_load: None,
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, #[cfg(feature = "aurora")] frame: &mut aurora_egui::Frame) {
        let central_panel = egui::CentralPanel::default();

        #[cfg(feature = "aurora")]
        let is_landscape = frame.is_landscape();

        #[cfg(not(feature = "aurora"))]
        let is_landscape = false;

        let main_frame = if self.player_view.is_some() {
            egui::Frame::new().fill(Color32::BLACK)
        } else {
            egui::Frame::central_panel(ui.style())
        };

        central_panel.frame(main_frame).show_inside(ui, |ui| {
            if self.access_token.is_none() {
                let mut token = None;
                self.auth_view.ui(ui, &mut token);
                if let Some(t) = token {
                    self.access_token = Some(t);
                }
                return;
            }

            if let Some(player_view) = self.player_view.as_mut() {
                let response = player_view.ui(ui, is_landscape);
                if response.back_clicked {
                    self.player_view = None;
                }
                return;
            }

            if !is_landscape {
                ui.heading("Twitch Client");

                ui.horizontal(|ui| {
                    let name_label = ui.label("Streamer login");
                    ui.text_edit_singleline(&mut self.streamer_login)
                        .labelled_by(name_label.id);
                });

                if ui.button("Play stream").clicked() {
                    let preferred = Config::load().and_then(|c| c.last_quality);
                    self.player_view =
                        Some(PlayerView::new(ui.ctx(), &self.streamer_login, preferred));
                }
            }

            // Auto-load followed streams if empty and cooldown passed
            {
                let is_empty = self.streams.read().unwrap().is_empty();
                let should_load = is_empty
                    && self
                        .last_streams_load
                        .map_or(true, |t| t.elapsed().as_secs() >= 30);
                if should_load {
                    self.last_streams_load = Some(Instant::now());
                    let streams = self.streams.clone();
                    let token = self.access_token.clone().unwrap_or_default();
                    std::thread::spawn(move || {
                        let new_streams = streams_list::get_streams(&token);
                        let mut guard = streams.write().unwrap();
                        *guard = new_streams;
                    });
                }
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(stream) = streams_list::streams_list_ui(ui, self.streams.clone()) {
                    let preferred = Config::load().and_then(|c| c.last_quality);
                    self.player_view = Some(PlayerView::new(
                        ui.ctx(),
                        stream.user_login.as_str(),
                        preferred,
                    ));
                }
            });
        });
    }
}

#[cfg(feature = "desktop")]
impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui(ui);
    }
}

#[cfg(feature = "aurora")]
impl aurora_egui::App for MyApp {
    fn update(&mut self, ui: &mut egui::Ui, frame: &mut aurora_egui::Frame) {
        self.ui(ui, frame);
    }
}
