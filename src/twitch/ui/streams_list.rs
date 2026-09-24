use std::sync::{Arc, RwLock};
use std::time::Duration;

use egui::{RichText, Ui, Widget};
use futures::TryStreamExt;
use twitch_api::helix::HelixClient;
use twitch_api::helix::search::Channel;
use twitch_api::helix::streams::Stream;
use twitch_api::helix::{ClientRequestError, HelixRequestGetError};
use twitch_api::twitch_oauth2::tokens::errors::ValidationError;
use twitch_api::twitch_oauth2::{AccessToken, UserToken};

#[derive(Debug)]
pub enum FetchError {
    /// Токен истёк или отозван — требуется повторная авторизация.
    Unauthorized,
    Other(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Unauthorized => {
                write!(f, "Токен доступа истёк или недействителен")
            }
            FetchError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FetchError {}

fn map_validation_error<RE: std::error::Error + Send + Sync + 'static>(
    e: ValidationError<RE>,
) -> FetchError {
    match e {
        ValidationError::NotAuthorized => FetchError::Unauthorized,
        other => FetchError::Other(other.to_string()),
    }
}

fn map_request_error(e: ClientRequestError<reqwest::Error>) -> FetchError {
    match e {
        ClientRequestError::HelixRequestGetError(HelixRequestGetError::Error {
            status, ..
        }) if status.as_u16() == 401 => FetchError::Unauthorized,
        other => FetchError::Other(other.to_string()),
    }
}

fn helix_client() -> HelixClient<'static, reqwest::Client> {
    let reqwest_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    HelixClient::with_client(reqwest_client)
}

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

pub fn get_streams(access_token: &str) -> Result<Vec<Stream>, FetchError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| FetchError::Other(format!("Failed to create tokio runtime: {e}")))?
        .block_on(async {
            let client = helix_client();
            let user_token = UserToken::from_existing(
                &client,
                AccessToken::new(access_token.into()),
                None,
                None,
            )
            .await
            .map_err(map_validation_error)?;
            client
                .get_followed_streams(&user_token)
                .try_collect()
                .await
                .map_err(map_request_error)
        })
}

pub fn search_channels(
    access_token: &str,
    query: &str,
    live_only: bool,
) -> Result<Vec<Channel>, FetchError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| FetchError::Other(format!("Failed to create tokio runtime: {e}")))?
        .block_on(async {
            let client = helix_client();
            let user_token = UserToken::from_existing(
                &client,
                AccessToken::new(access_token.into()),
                None,
                None,
            )
            .await
            .map_err(map_validation_error)?;
            client
                .search_channels(query, live_only, &user_token)
                .try_collect()
                .await
                .map_err(map_request_error)
        })
}
