use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use egui::{Color32, Rect, pos2, vec2};
use ffmpeg_next as ffmpeg;
use twitch_api::helix::search::Channel;
use twitch_api::helix::streams::Stream;

use crate::config::Config;
use crate::twitch::stream::player::StreamPlayer;
use crate::twitch::ui::auth::AuthView;
use crate::twitch::ui::player::PlayerView;
use crate::twitch::ui::streams_list;
use crate::twitch::ui::streams_list::FetchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Main,
    Subs,
    Find,
}

/// Вкладки нижней навигационной панели, в порядке отображения.
const TABS: [(Tab, &str); 3] = [
    (Tab::Main, "Главная"),
    (Tab::Subs, "Подписки"),
    (Tab::Find, "Стримеры"),
];

/// Высота нижней навигационной панели — увеличена для сенсорных экранов.
const NAV_HEIGHT: f32 = 64.0;

/// Состояние фонового подключения к стриму (загрузка плейлиста).
struct PendingPlayer {
    login: String,
    rx: std::sync::mpsc::Receiver<Result<StreamPlayer, String>>,
}

/// Результат проверки слота ошибки фоновой загрузки.
enum FetchStatus {
    /// Ошибки нет.
    Ok,
    /// Токен истёк/отозван — уже сброшен в `handle_unauthorized`.
    Unauthorized,
    /// Прочая ошибка — текст для отображения.
    Failed(String),
}

pub struct MyApp {
    streamer_login: String,
    player_view: Option<PlayerView>,
    pending_player: Option<PendingPlayer>,
    player_error: Option<String>,
    streams: Arc<RwLock<Vec<Stream>>>,
    streams_error: Arc<RwLock<Option<FetchError>>>,
    streams_loading: Arc<AtomicBool>,
    access_token: Option<String>,
    auth_view: AuthView,
    last_streams_load: Option<Instant>,
    current_tab: Tab,
    search_query: String,
    search_results: Arc<RwLock<Vec<Channel>>>,
    search_error: Arc<RwLock<Option<FetchError>>>,
    last_search_load: Option<Instant>,
    live_only_search: bool,
}

impl MyApp {
    pub fn new(_ctx: &egui::Context) -> Self {
        if let Err(e) = ffmpeg::init() {
            log::error!("Failed to initialize FFmpeg: {e:?}");
            std::process::exit(1);
        }
        let config = Config::load();
        let access_token = config.and_then(|c| c.access_token);

        Self {
            streamer_login: String::new(),
            player_view: None,
            pending_player: None,
            player_error: None,
            streams: Arc::new(RwLock::new(Vec::new())),
            streams_error: Arc::new(RwLock::new(None)),
            streams_loading: Arc::new(AtomicBool::new(false)),
            access_token,
            auth_view: AuthView::default(),
            last_streams_load: None,
            current_tab: Tab::default(),
            search_query: String::new(),
            search_results: Arc::new(RwLock::new(Vec::new())),
            search_error: Arc::new(RwLock::new(None)),
            last_search_load: None,
            live_only_search: false,
        }
    }

    /// Запускает фоновую загрузку плейлиста и подключение к стриму.
    /// Результат придёт в `pending_player` и будет обработан в UI-цикле.
    fn start_player(&mut self, streamer_login: &str) {
        let login = streamer_login.trim().to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_login = login.clone();
        std::thread::spawn(move || {
            let preferred = Config::load().and_then(|c| c.last_quality);
            let result = StreamPlayer::new(&thread_login, preferred).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.pending_player = Some(PendingPlayer { login, rx });
    }

    /// Проверяет результат фонового подключения к стриму.
    fn poll_pending_player(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_player else {
            return;
        };
        match pending.rx.try_recv() {
            Ok(Ok(player)) => {
                self.pending_player = None;
                self.player_view = Some(PlayerView::new(ctx, player));
            }
            Ok(Err(e)) => {
                self.pending_player = None;
                self.player_error = Some(e);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending_player = None;
                self.player_error = Some("Внутренняя ошибка при подключении к стриму".to_string());
            }
        }
    }

    /// Сбрасывает истёкший/отозванный токен: чистит конфиг и показывает
    /// экран авторизации с пояснением.
    fn handle_unauthorized(&mut self) {
        self.access_token = None;
        self.streams.write().unwrap().clear();
        self.search_results.write().unwrap().clear();
        let mut config = Config::load().unwrap_or_default();
        config.access_token = None;
        if let Err(e) = config.save() {
            log::error!("Failed to save config: {e}");
        }
        self.auth_view.notice =
            Some("Срок действия токена истёк. Пожалуйста, авторизуйтесь заново.".to_string());
    }

    /// Проверяет слот ошибки фоновой загрузки. При Unauthorized сбрасывает
    /// токен через `handle_unauthorized` и очищает слот; прочие ошибки
    /// остаются в слоте (очищаются при повторной загрузке).
    fn check_fetch_error(&mut self, error_slot: &Arc<RwLock<Option<FetchError>>>) -> FetchStatus {
        let is_unauthorized =
            matches!(&*error_slot.read().unwrap(), Some(FetchError::Unauthorized));
        if is_unauthorized {
            *error_slot.write().unwrap() = None;
            self.handle_unauthorized();
            return FetchStatus::Unauthorized;
        }
        match &*error_slot.read().unwrap() {
            Some(e) => FetchStatus::Failed(e.to_string()),
            None => FetchStatus::Ok,
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

        central_panel.frame(main_frame).show(ui, |ui| {
            if let Some(player_view) = self.player_view.as_mut() {
                let response = player_view.ui(ui, is_landscape, rotation);
                if response.back_clicked {
                    self.player_view = None;
                }
                return;
            }

            self.poll_pending_player(ui.ctx());
            if self.player_view.is_some() {
                // Плеер только что создан — отрисуем его на следующем кадре.
                return;
            }

            if self.pending_player.is_some() {
                let mut cancelled = false;
                if let Some(pending) = &self.pending_player {
                    centered_status_screen(ui, |ui| {
                        ui.heading(format!("Подключение к {}…", pending.login));
                        ui.add_space(8.0);
                        ui.add(egui::Spinner::new());
                        ui.add_space(16.0);
                        if ui.button("Отмена").clicked() {
                            cancelled = true;
                        }
                    });
                }
                if cancelled {
                    self.pending_player = None;
                }
                return;
            }

            if self.player_error.is_some() {
                let mut dismissed = false;
                if let Some(error) = &self.player_error {
                    centered_status_screen(ui, |ui| {
                        ui.heading("Не удалось открыть стрим");
                        ui.add_space(8.0);
                        ui.colored_label(Color32::LIGHT_RED, error);
                        ui.add_space(16.0);
                        if ui.button("Назад").clicked() {
                            dismissed = true;
                        }
                    });
                }
                if dismissed {
                    self.player_error = None;
                }
                return;
            }

            // Контент вкладки занимает всё пространство над нижней навигацией.
            let full_rect = ui.max_rect();
            let content_rect = Rect::from_min_max(
                full_rect.min,
                pos2(full_rect.max.x, full_rect.max.y - NAV_HEIGHT),
            );
            let nav_rect = Rect::from_min_max(
                pos2(full_rect.min.x, full_rect.max.y - NAV_HEIGHT),
                full_rect.max,
            );

            ui.scope_builder(
                egui::UiBuilder::new().max_rect(content_rect),
                |ui| match self.current_tab {
                    Tab::Main => self.show_main_tab(ui),
                    Tab::Subs => self.show_subs_tab(ui),
                    Tab::Find => self.show_find_tab(ui),
                },
            );

            // Нижняя навигационная панель: растянута на всю ширину, кнопки
            // делят её на равные части без зазоров между ними.
            ui.painter()
                .rect_filled(nav_rect, 0.0, ui.visuals().extreme_bg_color);
            ui.painter().hline(
                nav_rect.min.x..=nav_rect.max.x,
                nav_rect.min.y,
                ui.visuals().widgets.noninteractive.bg_stroke,
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(nav_rect), |ui| {
                // Без отступов между колонками — кнопки занимают всю ширину.
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ui.columns(TABS.len(), |columns| {
                    for (column, (tab, label)) in columns.iter_mut().zip(TABS) {
                        let selected = self.current_tab == tab;
                        let text_color = if selected {
                            column.visuals().strong_text_color()
                        } else {
                            column.visuals().weak_text_color()
                        };
                        let button = egui::Button::new(
                            egui::RichText::new(label).heading().color(text_color),
                        )
                        .frame(selected);
                        let width = column.available_width();
                        if column.add_sized(vec2(width, NAV_HEIGHT), button).clicked() {
                            self.current_tab = tab;
                        }
                    }
                });
            });
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
                let login = self.streamer_login.trim().to_owned();
                self.start_player(&login);
            }
        });
    }

    fn show_subs_tab(&mut self, ui: &mut egui::Ui) {
        if !self.ensure_auth(ui) {
            return;
        }

        let error_slot = self.streams_error.clone();
        match self.check_fetch_error(&error_slot) {
            FetchStatus::Unauthorized => return,
            FetchStatus::Failed(error_text) => {
                ui.colored_label(Color32::LIGHT_RED, error_text);
                if ui.button("Повторить").clicked() {
                    *error_slot.write().unwrap() = None;
                    self.last_streams_load = None;
                }
            }
            FetchStatus::Ok => {}
        }

        if self.streams_loading.load(Ordering::Relaxed) {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new());
                ui.add_space(8.0);
                ui.label("Загрузка...");
            });
            // Спиннер анимированный и сам запрашивает перерисовку каждый
            // кадр, поэтому список появится сразу после завершения загрузки.
            return;
        }

        // Auto-load followed streams if empty and cooldown passed
        {
            let is_empty = self.streams.read().unwrap().is_empty();
            let should_load = is_empty
                && self
                    .last_streams_load
                    .is_none_or(|t| t.elapsed().as_secs() >= 30);
            if should_load {
                self.last_streams_load = Some(Instant::now());
                let streams = self.streams.clone();
                let error_slot = self.streams_error.clone();
                let loading = self.streams_loading.clone();
                let token = self.access_token.clone().unwrap_or_default();
                loading.store(true, Ordering::Relaxed);
                std::thread::spawn(move || {
                    match streams_list::get_streams(&token) {
                        Ok(new_streams) => {
                            *streams.write().unwrap() = new_streams;
                            *error_slot.write().unwrap() = None;
                        }
                        Err(e) => {
                            *error_slot.write().unwrap() = Some(e);
                        }
                    }
                    loading.store(false, Ordering::Relaxed);
                });
            }
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(stream) = streams_list::streams_list_ui(ui, self.streams.clone()) {
                self.start_player(stream.user_login.as_str());
            }
        });
    }

    fn show_find_tab(&mut self, ui: &mut egui::Ui) {
        if !self.ensure_auth(ui) {
            return;
        }

        let error_slot = self.search_error.clone();
        match self.check_fetch_error(&error_slot) {
            FetchStatus::Unauthorized => return,
            FetchStatus::Failed(error_text) => {
                ui.colored_label(Color32::LIGHT_RED, error_text);
            }
            FetchStatus::Ok => {}
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
                    let error_slot = self.search_error.clone();
                    let token = self.access_token.clone().unwrap_or_default();
                    let query = self.search_query.trim().to_owned();
                    let live_only = self.live_only_search;
                    std::thread::spawn(move || {
                        match streams_list::search_channels(&token, &query, live_only) {
                            Ok(new_results) => {
                                *results.write().unwrap() = new_results;
                                *error_slot.write().unwrap() = None;
                            }
                            Err(e) => {
                                *error_slot.write().unwrap() = Some(e);
                            }
                        }
                    });
                }
            })
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(channel) = streams_list::channels_list_ui(ui, self.search_results.clone()) {
                self.start_player(channel.broadcaster_login.as_str());
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

/// Рисует центрированный по вертикали экран состояния (подключение, ошибка).
fn centered_status_screen(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        add_contents(ui);
    });
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
