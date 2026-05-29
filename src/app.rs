use std::sync::{Arc, Mutex};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Widget, load::SizedTexture};
use ffmpeg_next as ffmpeg;

use crate::twitch::stream::player::StreamPlayer;
use crate::twitch::ui::streams_list;

pub struct MyApp {
    streamer_login: String,
    player: Option<StreamPlayer>,
    stream_texture: TextureHandle,
    /// Double-buffer для видео-кадров: рендер-поток пишет, UI-поток читает
    pending_frame: Arc<Mutex<Option<Vec<u8>>>>,
}

impl MyApp {
    pub fn new(ctx: &egui::Context) -> Self {
        ffmpeg::init().unwrap();
        let stream_texture = ctx.load_texture(
            "live_stream",
            ColorImage::example(),
            TextureOptions::default(),
        );

        Self {
            streamer_login: "stray228".to_owned(),
            player: None,
            stream_texture,
            pending_frame: Arc::new(Mutex::new(None)),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, #[cfg(feature = "aurora")] frame: &mut aurora_egui::Frame) {
        let central_panel = egui::CentralPanel::default();

        #[cfg(feature = "aurora")]
        let is_landscape = frame.is_landscape();

        #[cfg(not(feature = "aurora"))]
        let is_landscape = false;

        central_panel.show_inside(ui, |ui| {
            if !is_landscape {
                ui.heading("Twitch Client");

                ui.horizontal(|ui| {
                    let name_label = ui.label("Streamer login");
                    ui.text_edit_singleline(&mut self.streamer_login)
                        .labelled_by(name_label.id);
                });

                ui.horizontal(|ui| {
                    if ui.button("Play stream").clicked() {
                        if self.player.is_none() {
                            self.player = Some(StreamPlayer::new(&self.streamer_login, None));
                        }

                        let player = self.player.as_mut().unwrap();
                        let playlist = player.playlist();

                        for stream in playlist.variants.iter() {
                            println!("{:?}", stream)
                        }

                        let pending = self.pending_frame.clone();
                        let ctx = ui.ctx().clone();
                        player.play(move |frame_data| {
                            // Минимальная работа в рендер-потоке: просто кладём кадр и просим репейнт
                            *pending.lock().unwrap() = Some(frame_data);
                            ctx.request_repaint();
                        });
                    }
                    if let Some(player) = self.player.as_mut() {
                        ui.menu_button("Quality", |ui| {
                            let streams = player.settings.available_streams().clone();
                            for stream in streams {
                                if ui
                                    .radio(
                                        player.settings.selected_stream == stream,
                                        stream.video.as_ref().unwrap(),
                                    )
                                    .clicked()
                                {
                                    player.set_stream_variant(&stream);
                                    // Перезапускаем воспроизведение с новым качеством
                                    player.stop();
                                    // Сбрасываем старый кадр, чтобы не было mismatch разрешения
                                    *self.pending_frame.lock().unwrap() = None;
                                    let pending = self.pending_frame.clone();
                                    let ctx = ui.ctx().clone();
                                    player.play(move |frame_data| {
                                        *pending.lock().unwrap() = Some(frame_data);
                                        ctx.request_repaint();
                                    });
                                };
                            }
                        });
                    }
                });
            }

            // Обновляем текстуру из UI-потока (thread-safe, никаких блокировок рендера)
            if let Some(player) = self.player.as_ref() {
                if let Some(frame_data) = self.pending_frame.lock().unwrap().take() {
                    let res = player.settings.resolution();
                    let expected = (res.width as usize) * (res.height as usize) * 3;
                    if frame_data.len() == expected {
                        let image_data =
                            ColorImage::from_rgb([res.width as _, res.height as _], &frame_data);
                        self.stream_texture
                            .set(image_data, TextureOptions::default());
                    } else {
                        println!(
                            "Frame size mismatch: expected {} for {:?}, got {}",
                            expected,
                            res,
                            frame_data.len()
                        );
                    }
                }
            }

            let texture = SizedTexture::new(self.stream_texture.id(), [1280., 720.]);

            egui::Image::new(texture)
                .bg_fill(Color32::BLACK)
                .shrink_to_fit()
                .ui(ui);

            streams_list::streams_list_ui(ui);
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
