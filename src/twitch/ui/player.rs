use std::sync::{Arc, Mutex};

use egui::{Color32, Rect, Vec2, pos2};
use egui_rotate::Rotation;

use crate::twitch::stream::player::StreamPlayer;
use crate::twitch::stream::YuvFrame;
use crate::twitch::ui::yuv_renderer::YuvRenderer;

pub struct PlayerView {
    player: StreamPlayer,
    yuv_renderer: YuvRenderer,
    pending_frame: Arc<Mutex<Option<YuvFrame>>>,
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
        let pending_frame = Arc::new(Mutex::new(None));
        let mut player = StreamPlayer::new(streamer_login, preferred_quality);

        let pending = pending_frame.clone();
        let ctx = ctx.clone();
        player.play(move |yuv| {
            *pending.lock().unwrap() = Some(yuv);
            ctx.request_repaint();
        });

        Self {
            player,
            yuv_renderer: YuvRenderer::new(),
            pending_frame,
            is_playing: true,
            show_settings: false,
            overlay_visible: false,
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        _is_landscape: bool,
        rotation: Option<Rotation>,
    ) -> PlayerResponse {
        self.yuv_renderer.set_rotation(rotation);
        self.update_frame();

        let available = ui.available_size();

        // Derive landscape from rotation directly — this matches what the renderer uses
        // for the viewport transform. `egui-rotate` always presents a portrait-shaped
        // logical screen, so we cannot tell from `available` alone.
        let is_landscape = matches!(
            rotation,
            Some(Rotation::CW90) | Some(Rotation::CW270)
        );

        // Always preserve 16:9 aspect ratio; never stretch to fill the screen.
        let aspect = 16.0 / 9.0;
        let image_size = if is_landscape {
            // Logical screen is landscape (wider than tall) because egui-rotate
            // presents a landscape logical canvas when the device is physically
            // rotated. The video is landscape 16:9, so constrain by height.
            let h = available.y;
            let w = h * aspect;
            Vec2::new(w.min(available.x), h)
        } else {
            // Logical screen is portrait. Video is landscape 16:9.
            // Constrain by width to get full-width / fit-height.
            let w = available.x;
            let h = w / aspect;
            Vec2::new(w, h.min(available.y))
        };

        let mut back_clicked = false;

        ui.vertical_centered(|ui| {
            // Center vertically so the physical viewport is centered regardless
            // of whether the actual rotation is CW90 or CW270.
            let y_padding = (available.y - image_size.y).max(0.0) / 2.0;
            ui.add_space(y_padding);

            let (rect, response) = ui.allocate_exact_size(image_size, egui::Sense::click());

            // Render video via custom GL YUV callback
            ui.painter()
                .add(self.yuv_renderer.paint_callback(rect));

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
                                egui::Button::new(egui::RichText::new("Взад").heading()),
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
                                self.player.play(move |yuv| {
                                    *pending.lock().unwrap() = Some(yuv);
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
                                    self.player.play(move |yuv| {
                                        *pending.lock().unwrap() = Some(yuv);
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

    fn update_frame(&mut self) {
        if let Some(frame) = self.pending_frame.lock().unwrap().take() {
            self.yuv_renderer.set_frame(frame);
        }
    }
}

impl Drop for PlayerView {
    fn drop(&mut self) {
        self.player.stop();
    }
}
