mod audio;
mod audio_decoder;
pub mod player;
pub mod video;
mod video_decoder;

/// Raw YUV420P frame data extracted directly from the decoder.
/// No software scaling or color conversion is applied.
#[derive(Clone)]
pub struct YuvFrame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}
