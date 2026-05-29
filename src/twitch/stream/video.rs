use ffmpeg_next::{self as ffmpeg, Stream};
use ffmpeg::codec::context::Context as CodecContext;
use ffmpeg::decoder::video::Video as VideoDecoder;
use ffmpeg::{format, frame};
use ffmpeg::software::scaling::{context::Context as ScalerContext, flag::Flags};


pub struct Transcoder {
    decoder: VideoDecoder,
    stream_scaler: ScalerContext,
}

impl Transcoder {
    pub fn new(input_stream: &Stream, resolution: (usize, usize)) -> Result<Self, ffmpeg::Error> {
        let decoder = CodecContext::from_parameters(input_stream.parameters())?
            .decoder()
            .video()?;

        let stream_scaler = ScalerContext::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            format::Pixel::RGB24,
            resolution.0 as _,
            resolution.1 as _,
            Flags::FAST_BILINEAR)?;

        Ok(Transcoder {
            decoder,
            stream_scaler,
        })
    }

    pub fn send_packet_to_decoder(&mut self, packet: &ffmpeg::Packet) -> Result<(), ffmpeg::Error> {
        self.decoder.send_packet(packet)
    }

    pub fn flush_decoder(&mut self) {
        self.decoder.flush();
    }

    pub fn send_eof_to_decoder(&mut self) {
        self.decoder.send_eof().unwrap();
    }

    pub fn receive_decoded_frames(&mut self) -> Result<frame::Video, ffmpeg::Error> {
        let mut frame = frame::Video::empty();
        self.decoder.receive_frame(&mut frame)?;
        if frame.is_corrupt() {
            return Err(ffmpeg::Error::InvalidData)
        }

        let mut scaled_frame = frame::Video::empty();
        scaled_frame.set_pts(frame.pts());
        if self.stream_scaler.run(&frame, &mut scaled_frame).is_err() {
            return Err(ffmpeg::Error::InvalidData)
        }
        frame = scaled_frame;
        Ok(frame)
    }
}
