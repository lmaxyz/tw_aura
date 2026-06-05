use std::sync::{Arc, Mutex};

use egui::{
    Color32, ColorImage, Rect, TextureHandle, TextureOptions, Vec2, load::SizedTexture, pos2,
};

use crate::twitch::stream::player::StreamPlayer;

pub struct PlayerView {
    player: StreamPlayer,
    stream_texture: TextureHandle,
    pending_frame: Arc<Mutex<Option<Vec<u8>>>>,
    is_playing: bool,
    show_settings: bool,
    overlay_visible: bool,
}

pub struct PlayerResponse {
    pub back_clicked: bool,
}

impl PlayerView {
    pub fn new(
        ctx: &egui::Context,
        streamer_login: &str,
        preferred_quality: Option<String>,
    ) -> Self {
        let stream_texture = ctx.load_texture(
            "live_stream",
            ColorImage::example(),
            TextureOptions::default(),
        );
        let pending_frame = Arc::new(Mutex::new(None));
        let mut player = StreamPlayer::new(streamer_login, preferred_quality);

        let pending = pending_frame.clone();
        let ctx = ctx.clone();
        player.play(move |frame_data| {
            *pending.lock().unwrap() = Some(frame_data);
            ctx.request_repaint();
        });

        Self {
            player,
            stream_texture,
            pending_frame,
            is_playing: true,
            show_settings: false,
            overlay_visible: false,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, is_landscape: bool) -> PlayerResponse {
        self.update_texture(ui.ctx());

        let available = ui.available_size();
        let texture = SizedTexture::new(self.stream_texture.id(), [1280., 720.]);

        let image_size = if is_landscape {
            available
        } else {
            let aspect = 16.0 / 9.0;
            let w = available.x;
            let h = w / aspect;
            Vec2::new(w, h.min(available.y))
        };

        let image = egui::Image::new(texture)
            .bg_fill(Color32::BLACK)
            .fit_to_exact_size(image_size)
            .sense(egui::Sense::click());

        let mut back_clicked = false;
        ui.vertical_centered(|ui| {
            let response = ui.add(image);
            if response.clicked() {
                self.overlay_visible = !self.overlay_visible;
            }

            // Overlay controls
            if self.overlay_visible || self.show_settings {
                let overlay_height = if is_landscape { 72.0 } else { 80.0 };
                let overlay_rect = Rect::from_min_max(
                    pos2(response.rect.min.x, response.rect.max.y - overlay_height),
                    response.rect.max,
                );

                ui.scope_builder(egui::UiBuilder::new().max_rect(overlay_rect), |ui| {
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::from_black_alpha(200);
                    ui.visuals_mut().widgets.active.weak_bg_fill = Color32::from_black_alpha(230);
                    ui.visuals_mut().widgets.hovered.weak_bg_fill = Color32::from_black_alpha(220);

                    let total_width = ui.available_width();
                    let back_size = egui::Vec2::new(80.0, 48.0);
                    let play_size = egui::Vec2::new(80.0, 48.0);
                    let gear_size = egui::Vec2::new(48.0, 48.0);
                    let spacer = (total_width
                        - back_size.x
                        - play_size.x
                        - gear_size.x
                        - ui.spacing().item_spacing.x * 2.)
                        / 2.0;

                    ui.horizontal(|ui| {
                        if ui
                            .add_sized(
                                back_size,
                                egui::Button::new(egui::RichText::new("Back").heading()),
                            )
                            .clicked()
                        {
                            back_clicked = true;
                        }

                        ui.add_space(spacer.max(0.0));

                        let label = if self.is_playing { "⏸" } else { "▶" };
                        if ui
                            .add_sized(
                                play_size,
                                egui::Button::new(egui::RichText::new(label).heading()),
                            )
                            .clicked()
                        {
                            if self.is_playing {
                                self.player.stop();
                                self.is_playing = false;
                            } else {
                                let pending = self.pending_frame.clone();
                                let ctx = ui.ctx().clone();
                                self.player.play(move |frame_data| {
                                    *pending.lock().unwrap() = Some(frame_data);
                                    ctx.request_repaint();
                                });
                                self.is_playing = true;
                            }
                        }

                        ui.add_space(spacer.max(0.0));

                        let quality_btn_response = ui.add_sized(
                            gear_size,
                            egui::Button::new(egui::RichText::new("⚙").heading()),
                        );
                        egui::Popup::menu(&quality_btn_response).show(|ui| {
                            let streams = self.player.settings.available_streams().clone();
                            for stream in streams {
                                if ui
                                    .radio(
                                        self.player.settings.selected_stream == stream,
                                        stream.video.as_ref().unwrap(),
                                    )
                                    .clicked()
                                {
                                    self.player.set_stream_variant(&stream);
                                    if let Some(mut config) = crate::config::Config::load() {
                                        config.last_quality = stream.video.clone();
                                        let _ = config.save();
                                    }
                                    self.player.stop();
                                    self.pending_frame.lock().unwrap().take();
                                    let pending = self.pending_frame.clone();
                                    let ctx = ui.ctx().clone();
                                    self.player.play(move |frame_data| {
                                        *pending.lock().unwrap() = Some(frame_data);
                                        ctx.request_repaint();
                                    });
                                    self.is_playing = true;
                                    self.show_settings = false;
                                };
                            }
                        })
                    });
                });
            }
        });

        PlayerResponse { back_clicked }
    }

    fn update_texture(&mut self, _ctx: &egui::Context) {
        if let Some(frame_data) = self.pending_frame.lock().unwrap().take() {
            let res = self.player.resolution();
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
}

impl Drop for PlayerView {
    fn drop(&mut self) {
        self.player.stop();
    }
}
