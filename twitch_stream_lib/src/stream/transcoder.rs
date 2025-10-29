use std::time::{Duration, Instant};

use ffmpeg_next::format::context::Output;
use ffmpeg_next::{self as ffmpeg, Stream};
use ffmpeg::codec::context::Context as CodecContext;
use ffmpeg::decoder::video::Video as VideoDecoder;
use ffmpeg::encoder::Video as VideoEncoder;
use ffmpeg::{format, frame, picture, Rational, Packet};
use ffmpeg::software::scaling::{context::Context as ScalerContext, flag::Flags};

use image::ImageBuffer;
use super::player::FrameQueue;


pub struct Transcoder {
    out_stream_index: usize,
    decoder: VideoDecoder,
    encoder: VideoEncoder,
    scaler: ScalerContext,
    stream_scaler:ScalerContext,
    frame_count: usize,
    input_time_base: Rational,
    sync: VideoSync,
}

impl Transcoder {
    pub fn new(input_stream: &Stream, out_ctx: &mut Output, out_stream_index: usize, resolution: (u64, u64)) -> Result<Self, ffmpeg::Error> {
        let global_header = out_ctx.format().flags().contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        let decoder = CodecContext::from_parameters(input_stream.parameters())?
            .decoder()
            .video()?;

        // decoder.set_frame_rate(Some(input_stream.rate()));
        // decoder.set_time_base(input_stream.time_base());

        println!("\nFrame rate: {:?}", decoder.frame_rate());
        println!("time base: {:?}", decoder.time_base());
        println!("packet time base: {:?}\n", decoder.packet_time_base());

        println!("IStream rate: {:?}", input_stream.rate());

        let scaler = ScalerContext::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            decoder.format(),
            resolution.0 as _,
            resolution.1 as _,
            Flags::BILINEAR)?;

        let stream_scaler = ScalerContext::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            format::Pixel::RGB24,
            resolution.0 as _,
            resolution.1 as _,
            Flags::BILINEAR)?;

        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::H264);
        let mut out_stream = out_ctx.add_stream(codec)?;

        let mut encoder = CodecContext::new_with_codec(codec.ok_or(ffmpeg::Error::InvalidData)?)
            .encoder()
            .video()?;

        encoder.set_height(resolution.1 as _);
        encoder.set_width(resolution.0 as _);
        encoder.set_aspect_ratio(decoder.aspect_ratio());
        encoder.set_format(decoder.format());
        encoder.set_frame_rate(Some(input_stream.rate()));
        encoder.set_time_base(input_stream.time_base());

        if global_header {
            encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }

        let opened_encoder = encoder
            .open()
            .expect("error opening x264 with supplied settings");

        out_stream.set_parameters(&opened_encoder);

        let sync = VideoSync::new(input_stream.time_base());

        Ok(Transcoder {
            out_stream_index,
            decoder,
            encoder: opened_encoder,
            frame_count: 0,
            input_time_base: input_stream.time_base(),
            scaler,
            stream_scaler,
            sync,
        })
    }

    pub fn send_packet_to_decoder(&mut self, packet: &ffmpeg::Packet) -> Result<(), ffmpeg::Error> {
        self.decoder.send_packet(packet)
    }

    pub fn send_eof_to_decoder(&mut self) {
        self.decoder.send_eof().unwrap();
    }

    pub fn receive_and_process_decoded_frames(
        &mut self,
        octx: &mut format::context::Output,
        ost_time_base: Rational,
        frames_queue: &mut FrameQueue,
    ) {
        let mut frame = frame::Video::empty();
        while self.decoder.receive_frame(&mut frame).is_ok() {
            self.frame_count += 1;
            if frame.is_corrupt() {
                continue;
            }

            self.send_frame_to_ui(frame.clone(), frames_queue);

            let timestamp = frame.timestamp();
            // if frame.width() != self.encoder.width() || frame.height() != self.encoder.height() {
            //     let mut rgb_frame = frame::Video::empty();
            //     self.scaler.run(&frame, &mut rgb_frame).unwrap();
            //     frame = rgb_frame;
            // }

            frame.set_pts(timestamp);
            frame.set_kind(picture::Type::None);

            // if self.encoder.send_frame(&frame).is_ok() {
            //     self.receive_and_process_encoded_packets(octx, ost_time_base);
            // }
        }
    }

    pub fn send_eof_to_encoder(&mut self) {
        self.encoder.send_eof().unwrap();
    }

    fn send_frame_to_ui(&mut self, mut frame: frame::Video, frames_queue: &mut FrameQueue,) {
        let mut scaled_frame = frame::Video::empty();
        scaled_frame.set_pts(frame.pts());
        if self.stream_scaler.run(&frame, &mut scaled_frame).is_err() {
            return;
        }
        frame = scaled_frame;

        let frame_data = frame.data(0);
        let stride = frame.stride(0);
        let size = [frame.width() as usize, frame.height() as usize];
        let width_in_bytes = 3 * size[0];
        let frame_target_len = size[1] * width_in_bytes;

        if frame_target_len > frame_data.len() {
            // Skip corrupted frames
            println!("Skipped frame: {} vs {}, size {:?}", frame_data.len(), frame_target_len, size);
            return;
        }

        if let Some(pts) = frame.pts() {
            match self.sync.sync_and_drop(pts) {
                SyncResult::Display => {
                    let mut pixels = Vec::with_capacity(frame_target_len);
                    for line in 0..frame.height() as usize {
                        let begin = line * stride;
                        let end = begin + width_in_bytes;
                        let data_line = &frame_data[begin..end];
                        pixels.extend(data_line)
                    }

                    let img: image::RgbImage = ImageBuffer::from_raw(size[0] as _, size[1] as _, pixels)
                            .ok_or_else(|| anyhow::anyhow!("Failed to create image from frame data")).unwrap();
                    let pixels = img.as_flat_samples();

                    frames_queue.put_new_frame(pixels.as_slice().to_vec());
                }
                SyncResult::Drop => {
                    // Пропускаем отстающий кадр
                    return;
                }
            }
        }
    }

    pub fn receive_and_process_encoded_packets(
        &mut self,
        octx: &mut format::context::Output,
        ost_time_base: Rational
    ) {
        let mut encoded = Packet::empty();
        while self.encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(self.out_stream_index);
            encoded.rescale_ts(self.input_time_base, ost_time_base);
            encoded.write_interleaved(octx).unwrap();
        }
    }
}

struct VideoSync {
    start_time: Instant,
    first_pts: Option<i64>,
    time_base: f64, // например 1/90000 для 90kHz
}

impl VideoSync {
    fn new(time_base: Rational) -> Self {
        Self {
            start_time: Instant::now(),
            first_pts: None,
            time_base: time_base.0 as f64 / time_base.1 as f64,
        }
    }

    // Для пропуска отстающих кадров
    fn sync_and_drop(&mut self, frame_pts: i64) -> SyncResult {
        if self.first_pts.is_none() {
            self.first_pts = Some(frame_pts);
            return SyncResult::Display;
        }

        let first_pts = self.first_pts.unwrap();
        let relative_pts = frame_pts - first_pts;
        let frame_time = relative_pts as f64 * self.time_base;
        let elapsed = self.start_time.elapsed().as_secs_f64();

        if frame_time <= elapsed {
            SyncResult::Display
        } else if elapsed - frame_time > 0.1 { // Кадр отстал больше чем на 1000ms
            SyncResult::Drop
        } else {
            // Ждем нужное время перед отрисовкой
            let sleep_time = frame_time - elapsed;
            std::thread::sleep(Duration::from_secs_f64(sleep_time));
            SyncResult::Display
        }
    }
}

enum SyncResult {
    Display,
    Drop,
}
