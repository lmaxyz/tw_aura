use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

pub mod stream;

use stream::player::StreamPlayer;


#[repr(C)]
pub struct PlayerContext {
    pub player: StreamPlayer
}

#[unsafe(no_mangle)]
pub extern "C" fn new_player_ctx() -> *mut PlayerContext {
    let player = StreamPlayer::new();
    let my_struct = Box::new(PlayerContext { player });
    Box::into_raw(my_struct)
}

#[unsafe(no_mangle)]
pub extern "C" fn play_stream(player_ctx: *mut PlayerContext, stream_uri: *const c_char) -> bool {
    unsafe {
        if let Ok(stream_uri) = CStr::from_ptr(stream_uri).to_str() {
            (*player_ctx).player.play(stream_uri);
            true
        } else {
            false
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn stop_stream(player_ctx: *mut PlayerContext) {

}

#[unsafe(no_mangle)]
pub extern "C" fn next_frame(player_ctx: *mut PlayerContext, frame_buffer: *mut u8) -> bool {
    let ctx = unsafe { &mut *player_ctx };
        let frame_data = ctx.player.next_frame();

        if let Some(frame_data) = frame_data && frame_data.len() == ctx.player.frame_buffer_size {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    frame_data.as_ptr(),
                    frame_buffer,
                    ctx.player.frame_buffer_size
                );
            }
            true
        } else {
            false  // ошибка размера кадра
        }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_buffer_size(player_ctx: *mut PlayerContext) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn destroy_ctx(ctx: *mut PlayerContext) {
    unsafe { drop(Box::from_raw(ctx)) };
}
