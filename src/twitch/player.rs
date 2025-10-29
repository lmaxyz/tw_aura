use libloading::{Library, Symbol};
use std::ffi::{CString, CStr, c_void, c_char};

use m3u8_rs::{MasterPlaylist, VariantStream};

use super::twitch_legacy;


type NewPlayerCtxFn = unsafe extern "C" fn() -> *mut c_void;
type PlayStreamFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;


pub struct StreamPlayer {
    streamer_login: String,
    master_playlist: MasterPlaylist,
    current_stream: Option<VariantStream>,

    context: PlayerContext,
}

impl StreamPlayer {
    pub fn new(streamer_login: String) -> Self {
        let master_playlist = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
            .block_on(async {
                twitch_legacy::get_streamer_playlist(&streamer_login).await.unwrap()
            });
        let context = PlayerContext::new();
        StreamPlayer {
            streamer_login,
            master_playlist,
            current_stream: None,
            context
        }
    }

    pub fn playlist(&self) -> MasterPlaylist {
        self.master_playlist.clone()
    }

    pub fn play_stream(&mut self, stream: VariantStream) {
        self.current_stream = Some(stream)
    }
}

struct PlayerContext {
    lib: Library,
    context: *mut c_void
}

impl PlayerContext {
    fn new() -> Self {
        unsafe {
            let lib = Library::new("./target/release/libtwitch_stream_lib.so").expect("Can't load library");

            let new_player_ctx: Symbol<NewPlayerCtxFn> = lib.get(b"new_player_ctx")
                .expect("Function not found");

            let context = new_player_ctx();

            if !context.is_null() {
                println!("Str len: {}", context as i32);
            }

            PlayerContext {
                lib,
                context
            }
        }
    }

    fn play_stream(&self, stream_uri: &str, resolution: (u32, u32)) -> bool {
        unsafe {
            let play: Symbol<PlayStreamFn> = self.lib.get(b"play_stream")
                .expect("Function not found");

            play(self.context, CString::new(stream_uri).unwrap().into_raw())
        }
    }
}
