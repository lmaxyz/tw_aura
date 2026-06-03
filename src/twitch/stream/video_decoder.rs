use ffmpeg::codec::context::Context as CodecContext;
use ffmpeg::decoder::video::Video as VideoDecoder;
use ffmpeg::frame;
use ffmpeg_next::{self as ffmpeg, Stream};

pub struct Transcoder {
    decoder: VideoDecoder,
}

impl Transcoder {
    pub fn new(input_stream: &Stream) -> Result<Self, ffmpeg::Error> {
        let decoder = CodecContext::from_parameters(input_stream.parameters())?
            .decoder()
            .video()?;

        Ok(Transcoder { decoder })
    }

    pub fn send_packet_to_decoder(&mut self, packet: &ffmpeg::Packet) -> Result<(), ffmpeg::Error> {
        self.decoder.send_packet(packet)
    }

    pub fn send_eof(&mut self) {
        let _ = self.decoder.send_eof();
    }

    /// Возвращает *сырой* декодированный кадр (YUV). Scaling живёт в decode thread.
    pub fn receive_decoded_frames(&mut self) -> Result<frame::Video, ffmpeg::Error> {
        let mut frame = frame::Video::empty();
        self.decoder.receive_frame(&mut frame)?;
        if frame.is_corrupt() {
            return Err(ffmpeg::Error::Other {
                errno: ffmpeg::util::error::EAGAIN,
            });
        }
        Ok(frame)
    }
}
