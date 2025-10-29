use std::sync::{Arc, Mutex};

use futures::TryStreamExt;
use twitch_api::{helix::HelixClient, helix::streams::Stream};
use twitch_api::twitch_oauth2::{AccessToken, UserToken};
use eframe::egui::Ui;


pub fn streams_list_ui(ui: &mut Ui) {
    let streams = Arc::new(Mutex::new(Vec::new()));

    if ui.button("Load streams").clicked() {
        let streams = streams.clone();
        std::thread::spawn(move || {
            let mut streams = streams.lock().unwrap();
            *streams = get_streams();
        });
    }
    for stream in streams.lock().unwrap().iter() {
        ui.label(format!("{} is streaming {}", stream.user_name, stream.title));
    }
}

fn get_streams() -> Vec<Stream> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
        .block_on(async {
            let client: HelixClient<reqwest::Client> = HelixClient::default();
            let user_token = UserToken::from_existing(
                &client, AccessToken::new("bz4uvqogz58we0icwqaahv48sbz8gr".into()),
                None,
                None
            ).await.unwrap();
            client.get_followed_streams(&user_token).try_collect().await.unwrap()
        })

}
