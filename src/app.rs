use std::sync::{Arc, RwLock};
use std::time::Instant;

use egui::Color32;
use ffmpeg_next as ffmpeg;
use twitch_api::helix::search::Channel;
use twitch_api::helix::streams::Stream;

use crate::config::Config;
use crate::twitch::ui::auth::AuthView;
use crate::twitch::ui::player::PlayerView;
use crate::twitch::ui::streams_list;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Main,
    Subs,
    Find,
}

pub struct MyApp {
    streamer_login: String,
    player_view: Option<PlayerView>,
    streams: Arc<RwLock<Vec<Stream>>>,
    access_token: Option<String>,
    auth_view: AuthView,
    last_streams_load: Option<Instant>,
    current_tab: Tab,
    search_query: String,
    search_results: Arc<RwLock<Vec<Channel>>>,
    last_search_load: Option<Instant>,
    live_only_search: bool,
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
            current_tab: Tab::default(),
            search_query: String::new(),
            search_results: Arc::new(RwLock::new(Vec::new())),
            last_search_load: None,
            live_only_search: false,
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, #[cfg(feature = "aurora")] frame: &mut aurora_egui::Frame) {
        let central_panel = egui::CentralPanel::default();

        #[cfg(feature = "aurora")]
        let is_landscape = frame.is_landscape();
        #[cfg(feature = "aurora")]
        let rotation = Some(frame.rotation());

        #[cfg(not(feature = "aurora"))]
        let is_landscape = false;
        #[cfg(not(feature = "aurora"))]
        let rotation = None;

        let main_frame = if self.player_view.is_some() {
            egui::Frame::new().fill(Color32::BLACK)
        } else {
            egui::Frame::central_panel(ui.style())
        };

        central_panel.frame(main_frame).show_inside(ui, |ui| {
            if let Some(player_view) = self.player_view.as_mut() {
                let response = player_view.ui(ui, is_landscape, rotation);
                if response.back_clicked {
                    self.player_view = None;
                }
                return;
            }

            // Tab buttons
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.current_tab == Tab::Main, "Главная")
                    .clicked()
                {
                    self.current_tab = Tab::Main;
                }
                if ui
                    .selectable_label(self.current_tab == Tab::Subs, "Подписки")
                    .clicked()
                {
                    self.current_tab = Tab::Subs;
                }
                if ui
                    .selectable_label(self.current_tab == Tab::Find, "Стримеры")
                    .clicked()
                {
                    self.current_tab = Tab::Find;
                }
            });
            ui.separator();

            match self.current_tab {
                Tab::Main => self.show_main_tab(ui),
                Tab::Subs => self.show_subs_tab(ui),
                Tab::Find => self.show_find_tab(ui),
            }
        });
    }

    fn show_main_tab(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 3.0);
            ui.heading("Twitch Client");
            ui.add_space(16.0);

            let name_label = ui.label("Логин стримера");
            ui.text_edit_singleline(&mut self.streamer_login)
                .labelled_by(name_label.id);

            if ui.button("Смотреть стрим").clicked() && !self.streamer_login.trim().is_empty()
            {
                let preferred = Config::load().and_then(|c| c.last_quality);
                self.player_view = Some(PlayerView::new(
                    ui.ctx(),
                    self.streamer_login.trim(),
                    preferred,
                ));
            }
        });
    }

    fn show_subs_tab(&mut self, ui: &mut egui::Ui) {
        if !self.ensure_auth(ui) {
            return;
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
    }

    fn show_find_tab(&mut self, ui: &mut egui::Ui) {
        if !self.ensure_auth(ui) {
            return;
        }

        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let label = ui.label("🔎");
                ui.text_edit_singleline(&mut self.search_query)
                    .labelled_by(label.id);
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.live_only_search, "Только онлайн");
                if ui.button("Поиск").clicked() && !self.search_query.trim().is_empty() {
                    self.last_search_load = Some(Instant::now());
                    let results = self.search_results.clone();
                    let token = self.access_token.clone().unwrap_or_default();
                    let query = self.search_query.trim().to_owned();
                    let live_only = self.live_only_search;
                    std::thread::spawn(move || {
                        let new_results = streams_list::search_channels(&token, &query, live_only);
                        let mut guard = results.write().unwrap();
                        *guard = new_results;
                    });
                }
            })
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(channel) = streams_list::channels_list_ui(ui, self.search_results.clone()) {
                let preferred = Config::load().and_then(|c| c.last_quality);
                self.player_view = Some(PlayerView::new(
                    ui.ctx(),
                    channel.broadcaster_login.as_str(),
                    preferred,
                ));
            }
        });
    }

    /// Returns `true` if the user is authenticated.
    fn ensure_auth(&mut self, ui: &mut egui::Ui) -> bool {
        if self.access_token.is_none() {
            let mut token = None;
            self.auth_view.ui(ui, &mut token);
            if let Some(t) = token {
                self.access_token = Some(t);
            }
            false
        } else {
            true
        }
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
