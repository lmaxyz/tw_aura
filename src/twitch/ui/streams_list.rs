use std::sync::{Arc, RwLock};

use egui::{RichText, Ui, Widget};
use futures::TryStreamExt;
use twitch_api::helix::HelixClient;
use twitch_api::helix::search::Channel;
use twitch_api::helix::streams::Stream;
use twitch_api::twitch_oauth2::{AccessToken, UserToken};

pub fn streams_list_ui(ui: &mut Ui, streams: Arc<RwLock<Vec<Stream>>>) -> Option<Stream> {
    let mut selected_stream = None;
    for stream in streams.read().unwrap().iter() {
        let response = ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(&stream.user_login)
                .sense(egui::Sense::click()),
            |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    let aspect = 16.0 / 9.0;
                    let image_width = ui.available_width() / 3.;
                    let image_url = stream
                        .thumbnail_url
                        .replace("{width}", &(image_width as i32).to_string())
                        .replace("{height}", &((image_width / aspect) as i32).to_string());
                    egui::Image::new(image_url).fit_to_original_size(1.).ui(ui);
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                        egui::Label::new(RichText::new(stream.user_name.as_str()).strong())
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                        egui::Label::new(&stream.title)
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                        egui::Label::new(&stream.game_name)
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                        egui::Label::new(format!("Сейчас смотрят: {}", stream.viewer_count))
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                    });
                })
            },
        );

        if response.response.clicked() {
            selected_stream = Some(stream.clone())
        }
    }
    selected_stream
}

pub fn channels_list_ui(ui: &mut Ui, channels: Arc<RwLock<Vec<Channel>>>) -> Option<Channel> {
    let mut selected_channel = None;
    for channel in channels.read().unwrap().iter() {
        let response = ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(&channel.broadcaster_login)
                .sense(egui::Sense::click()),
            |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    let image_width = ui.available_width() / 5.;
                    let image_url = channel
                        .thumbnail_url
                        .replace("{width}", &(image_width as i32).to_string())
                        .replace("{height}", &(image_width as i32).to_string());
                    egui::Image::new(image_url).fit_to_original_size(1.).ui(ui);
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                        egui::Label::new(RichText::new(channel.display_name.as_str()).strong())
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                        egui::Label::new(&channel.title)
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                        egui::Label::new(&channel.game_name)
                            .truncate()
                            .selectable(false)
                            .ui(ui);
                        let status = if channel.is_live {
                            "🔴 Live"
                        } else {
                            "Offline"
                        };
                        egui::Label::new(status).truncate().selectable(false).ui(ui);
                    });
                })
            },
        );

        if response.response.clicked() {
            selected_channel = Some(channel.clone())
        }
    }
    selected_channel
}

pub fn get_streams(access_token: &str) -> Vec<Stream> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let client: HelixClient<reqwest::Client> = HelixClient::default();
            let user_token = UserToken::from_existing(
                &client,
                AccessToken::new(access_token.into()),
                None,
                None,
            )
            .await
            .unwrap();
            client
                .get_followed_streams(&user_token)
                .try_collect()
                .await
                .unwrap()
        })
}

pub fn search_channels(access_token: &str, query: &str, live_only: bool) -> Vec<Channel> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let client: HelixClient<reqwest::Client> = HelixClient::default();
            let user_token = UserToken::from_existing(
                &client,
                AccessToken::new(access_token.into()),
                None,
                None,
            )
            .await
            .unwrap();
            client
                .search_channels(query, live_only, &user_token)
                .try_collect()
                .await
                .unwrap()
        })
}
