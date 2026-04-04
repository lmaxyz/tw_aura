use ffmpeg_next as ffmpeg;
use eframe::{egui::{self, load::SizedTexture, Color32, ColorImage, TextureHandle, TextureOptions, Widget}};

use crate::twitch::ui::streams_list;
use crate::twitch::stream::player::StreamPlayer;

pub struct MyApp {
    streamer_login: String,
    player: Option<StreamPlayer>,
    stream_texture: TextureHandle,
}

impl MyApp {
    pub fn new(ctx: &egui::Context) -> Self {
        ffmpeg::init().unwrap();
        let stream_texture = ctx.load_texture("live_stream", ColorImage::example(), TextureOptions::default());

        Self {
            streamer_login: "ilame".to_owned(),
            player: None,
            stream_texture,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let top_panel = egui::TopBottomPanel::top("top_bar")
            .exact_height(41.0) // Необходимо получать значение из dconf
            .show_separator_line(false);

        let central_panel = egui::CentralPanel::default();

        // if self.use_system_background {
        //     let frame = egui::Frame::default().fill(Color32::TRANSPARENT);
        //     top_panel = top_panel.frame(frame);

        //     let frame = egui::Frame::default().inner_margin(12.).fill(Color32::TRANSPARENT);
        //     central_panel = central_panel.frame(frame);
        // }

        top_panel.show(ctx, |_| {});
        central_panel.show(ctx, |ui| {
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

                    let selected_resolution = player.settings.resolution();

                    let mut texture_handle = self.stream_texture.clone();
                    let ctx = ctx.clone();
                    player.play(move |frame_data| {
                        let image_data = ColorImage::from_rgb([selected_resolution.width as _, selected_resolution.height as _], &frame_data);
                        texture_handle.set(image_data, TextureOptions::default());
                        ctx.request_repaint();
                    });
                }
                if ui.button("Play Audio").clicked() {
                    self.player.as_ref().map(|p| p.play_audio());
                }
                if let Some(player) = self.player.as_mut() {
                    ui.menu_button("Quality", |ui| {
                        let streams = player.settings.available_streams().clone();
                        for stream in streams {
                            if ui.radio(player.settings.selected_stream == stream, stream.video.as_ref().unwrap()).clicked() {
                                player.set_stream_variant(&stream);
                            };
                            // if ui.button(stream.video.as_ref().unwrap()).clicked() {
                            //     player.set_stream_variant(&stream);
                            // }
                        }
                    });
                }
            });

            let texture = SizedTexture::new(self.stream_texture.id(), [1280., 720.]);

            egui::Image::new(texture)
                .bg_fill(Color32::BLACK)
                // .rotate(90.0_f32.to_radians(), egui::Vec2::splat(0.5))
                .shrink_to_fit()
                .ui(ui);

            streams_list::streams_list_ui(ui);
        });
    }
}
