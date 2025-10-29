use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;
use m3u8_rs::{parse_master_playlist_res, MasterPlaylist};


#[derive(Debug, Deserialize)]
pub struct PlaybackAccessToken {
    pub value: String,
    pub signature: String,
    authorization: Authorization,
}

#[derive(Debug, Deserialize)]
struct Authorization {
    #[serde(rename = "isForbidden")]
    is_forbidden: bool,
    #[serde(rename = "forbiddenReasonCode")]
    forbidden_reason_code: String
}

#[derive(Debug, Deserialize)]
struct ResponseData {
    #[serde(rename = "streamPlaybackAccessToken")]
    stream_playback_access_token: Option<PlaybackAccessToken>,
}

#[derive(Debug, Deserialize)]
struct PATResponse {
    data: ResponseData,
}


async fn get_playback_access_token(streamer_login: &str) -> Result<PlaybackAccessToken, TwitchApiError> {
    let query = serde_json::json!({
        "operationName":"PlaybackAccessToken",
        "variables":{
            "isLive":true,
            "login":streamer_login,
            "isVod":false,
            "vodID":"",
            "playerType":"site",
            "platform":"web"
        },
        "extensions":{
            "persistedQuery":{
                "version":1,
                "sha256Hash":"ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9"
            }
        }
    });
    let response = Client::new().post("https://gql.twitch.tv/gql")
        .body(query.to_string())
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .header("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko")
        .send().await?;

    let response_json = response.json::<PATResponse>().await?;

    if let Some(stream_playback_access_token) = response_json.data.stream_playback_access_token {
        if stream_playback_access_token.authorization.is_forbidden {
            return Err(TwitchApiError::PlaybackTokenAuthError(stream_playback_access_token.authorization.forbidden_reason_code))
        }
        Ok(stream_playback_access_token)
    } else {
        Err(TwitchApiError::ApplicationError(format!("No playback token inside response: {:?}", response_json)))
    }
}

pub async fn get_streamer_playlist(streamer_login: &str) -> Result<MasterPlaylist, TwitchApiError> {
    let playback_access_token = get_playback_access_token(streamer_login).await?;
    let url = format!("https://usher.ttvnw.net/api/channel/hls/{}.m3u8?sig={}&token={}", streamer_login, playback_access_token.signature, playback_access_token.value);
    let response = Client::new().get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        .header("Client-ID", "kimne78kx3ncx6brgo4mv6wki5h1ko")
        .send().await?;

    if response.status().is_success() {
        let playlist_bytes = response.bytes().await?;
        if let Ok(playlist) = parse_master_playlist_res(&playlist_bytes) {
            return Ok(playlist)
        } else {
            return Err(TwitchApiError::PlaylistParseError)
        }
    }

    Err(TwitchApiError::StreamNotFound)
}


#[derive(Error, Debug)]
pub enum TwitchApiError {
    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error), // This variant will hold reqwest errors
    #[error("Failed to parse response: {0}")]
    ParseError(#[from] serde_json::Error), // Example for JSON parsing errors
    #[error("Custom application error: {0}")]
    ApplicationError(String),
    #[error("Stream not found")]
    StreamNotFound,
    #[error("Failed to parse master playlist")]
    PlaylistParseError,
    #[error("Playback Token auth error: {0}")]
    PlaybackTokenAuthError(String),
}
