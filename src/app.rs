use eframe::{egui::{self, load::SizedTexture, Color32, ColorImage, TextureHandle, TextureOptions, Widget}};

use crate::twitch::ui::streams_list;
use crate::twitch::player::StreamPlayer;

pub struct MyApp {
    streamer_login: String,
    stream_texture: TextureHandle,
}

impl MyApp {
    pub fn new(ctx: &egui::Context) -> Self {
        let stream_texture = ctx.load_texture("live_stream", ColorImage::example(), TextureOptions::default());
        Self {
            streamer_login: "dkfogas1".to_owned(),
            stream_texture,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ctx.set_pixels_per_point(1.0);
            ui.heading("Twitch Client");

            ui.horizontal(|ui| {
                let name_label = ui.label("Streamer login");
                ui.text_edit_singleline(&mut self.streamer_login)
                    .labelled_by(name_label.id);
            });

            if ui.button("Start stream").clicked() {
                let player = StreamPlayer::new(self.streamer_login.clone());
                let playlist = player.playlist();

                for stream in playlist.variants.iter() {
                    println!("{:?}", stream)
                }

                let mut texture_handle = self.stream_texture.clone();
                let ctx = ctx.clone();
                let streamer_login = self.streamer_login.clone();
                std::thread::spawn(move || {
                    // start_streaming(&streamer_login, Box::new(move |frame_data| {
                    //     let image_data = ColorImage::from_rgb([852, 480], frame_data);
                    //     // let mut texture_manager = texture_manager.write();
                    //     // let mut delta = texture_manager.take_delta();
                    //     texture_handle.set(image_data, TextureOptions::default());
                    //     ctx.request_repaint();
                    //     // delta.set = vec![(texture_id, ImageDelta::full(image_data, TextureOptions::default()))];
                    //     // texture_manager.set(texture_id, ImageDelta::full(image_data, TextureOptions::default()));
                    // }))
                });
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
