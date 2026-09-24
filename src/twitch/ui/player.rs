use std::sync::{Arc, Mutex};

use egui::{Color32, Rect, Vec2, pos2};
use egui_rotate::Rotation;

use crate::twitch::stream::YuvFrame;
use crate::twitch::stream::player::StreamPlayer;
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
    pub fn new(ctx: &egui::Context, player: StreamPlayer) -> Self {
        let mut view = Self {
            player,
            yuv_renderer: YuvRenderer::new(),
            pending_frame: Arc::new(Mutex::new(None)),
            is_playing: true,
            show_settings: false,
            overlay_visible: false,
        };
        view.resume(ctx);
        view
    }

    /// Callback для потока вывода видео: складывает кадр в `pending_frame`
    /// и запрашивает перерисовку UI.
    fn frame_callback(&self, ctx: &egui::Context) -> impl FnMut(YuvFrame) + Send + 'static {
        let pending = self.pending_frame.clone();
        let ctx = ctx.clone();
        move |yuv| {
            *pending.lock().unwrap() = Some(yuv);
            ctx.request_repaint();
        }
    }

    /// (Пере)запускает воспроизведение после паузы или смены качества.
    fn resume(&mut self, ctx: &egui::Context) {
        let cb = self.frame_callback(ctx);
        self.player.play(cb);
        self.is_playing = true;
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
        let is_landscape = matches!(rotation, Some(Rotation::CW90) | Some(Rotation::CW270));

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
            ui.painter().add(self.yuv_renderer.paint_callback(rect));

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

                    let back_size = egui::Vec2::new(120.0, 60.0);
                    let play_size = egui::Vec2::new(120.0, 60.0);
                    let gear_size = egui::Vec2::new(60.0, 60.0);
                    let side_padding = 12.0;

                    // Точное позиционирование: «Назад» слева, play строго по
                    // центру оверлея, качество справа. Колонки и ручные
                    // спейсеры дают смещение — не используем ни то, ни другое.
                    let center_y = overlay_rect.center().y;
                    let back_rect = Rect::from_center_size(
                        pos2(
                            overlay_rect.min.x + side_padding + back_size.x / 2.0,
                            center_y,
                        ),
                        back_size,
                    );
                    let play_rect =
                        Rect::from_center_size(pos2(overlay_rect.center().x, center_y), play_size);
                    let gear_rect = Rect::from_center_size(
                        pos2(
                            overlay_rect.max.x - side_padding - gear_size.x / 2.0,
                            center_y,
                        ),
                        gear_size,
                    );

                    if ui
                        .put(
                            back_rect,
                            egui::Button::new(egui::RichText::new("Назад").heading()),
                        )
                        .clicked()
                    {
                        back_clicked = true;
                    }

                    let label = if self.is_playing { "⏸" } else { "▶" };
                    if ui
                        .put(
                            play_rect,
                            egui::Button::new(egui::RichText::new(label).heading()),
                        )
                        .clicked()
                    {
                        if self.is_playing {
                            self.player.stop();
                            self.is_playing = false;
                        } else {
                            self.resume(ui.ctx());
                        }
                    }

                    let quality_btn_response = ui.put(
                        gear_rect,
                        egui::Button::new(egui::RichText::new("⚙").heading()),
                    );
                    egui::Popup::menu(&quality_btn_response).show(|ui| {
                        let streams = self.player.settings.available_streams().clone();
                        for stream in streams {
                            let label = stream.video.as_deref().unwrap_or("auto");
                            if ui
                                .radio(self.player.settings.selected_stream == stream, label)
                                .clicked()
                            {
                                self.player.set_stream_variant(&stream);
                                if let Some(mut config) = crate::config::Config::load() {
                                    config.last_quality = stream.video.clone();
                                    let _ = config.save();
                                }
                                self.pending_frame.lock().unwrap().take();
                                self.resume(ui.ctx());
                                self.show_settings = false;
                            };
                        }
                    })
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
