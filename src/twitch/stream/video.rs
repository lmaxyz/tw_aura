use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ffmpeg_next::{
    self as ffmpeg, format, frame,
    software::scaling::{context::Context as ScalerContext, flag::Flags},
};

use super::video_decoder::Transcoder;

/// Self-contained video pipeline: packet input → decode → scale → paced frame output.
///
/// Spawns two internal threads:
/// - **Decode thread**: receives packets, decodes raw frames, scales to RGB24, pushes to ring buffer.
/// - **Output thread**: pulls scaled frames from ring buffer, paces them at `1/fps`, invokes callback.
///
/// The struct is recreated on each stream reconnect so that decoder state and pacing reset cleanly.
pub struct VideoStream {
    packet_tx: ring_channel::RingSender<ffmpeg::Packet>,
    cancel: Arc<AtomicBool>,
    _decode_handle: JoinHandle<()>,
    _output_handle: JoinHandle<()>,
}

impl VideoStream {
    pub fn new(
        input_stream: &ffmpeg::Stream,
        target_resolution: Option<m3u8_rs::Resolution>,
        frame_rate: i32,
        _video_time_base: f64,
        mut new_frame_callback: impl FnMut(Vec<u8>) + Send + 'static,
        _get_audio_pts_sec: impl Fn() -> f64 + Send + 'static,
    ) -> Result<Self, ffmpeg::Error> {
        let transcoder = Transcoder::new(input_stream)?;

        let (target_width, target_height) = match target_resolution {
            Some(res) => (res.width as u32, res.height as u32),
            None => {
                let decoder =
                    ffmpeg::codec::context::Context::from_parameters(input_stream.parameters())?
                        .decoder()
                        .video()?;
                (decoder.width(), decoder.height())
            }
        };

        // Packet ring: generous capacity so the decoder never starves due to overwrites.
        // We use ~10 seconds of packets to absorb decode bursts.
        let packet_cap = NonZeroUsize::new((frame_rate * 10).max(120) as usize).unwrap();
        let (packet_tx, packet_rx) = ring_channel::ring_channel(packet_cap);

        // Frame ring: ~7 seconds of scaled frames. Overwrite old frames if output lags.
        let frame_cap = NonZeroUsize::new((frame_rate * 7).max(1) as usize).unwrap();
        let (frame_tx, frame_rx) = ring_channel::ring_channel(frame_cap);

        let cancel = Arc::new(AtomicBool::new(false));

        let cancel_decode = cancel.clone();
        let cancel_output = cancel.clone();

        let _decode_handle = std::thread::spawn(move || {
            let mut transcoder = transcoder;
            let mut scaler: Option<ScalerContext> = None;
            let mut last_input_desc: Option<(format::Pixel, u32, u32)> = None;

            let mut process_frame = |raw_frame: frame::Video| -> Option<Vec<u8>> {
                let input_desc = (raw_frame.format(), raw_frame.width(), raw_frame.height());
                if last_input_desc != Some(input_desc) {
                    match ScalerContext::get(
                        raw_frame.format(),
                        raw_frame.width(),
                        raw_frame.height(),
                        format::Pixel::RGB24,
                        target_width,
                        target_height,
                        Flags::FAST_BILINEAR,
                    ) {
                        Ok(s) => {
                            println!(
                                "Scaler reinit: {:?} {}x{} -> RGB24 {}x{}",
                                raw_frame.format(),
                                raw_frame.width(),
                                raw_frame.height(),
                                target_width,
                                target_height,
                            );
                            scaler = Some(s);
                            last_input_desc = Some(input_desc);
                        }
                        Err(e) => {
                            println!("Scaler init failed: {:?}", e);
                            return None;
                        }
                    }
                }

                let mut scaled_frame = frame::Video::empty();
                scaled_frame.set_pts(raw_frame.pts());
                if let Some(ref mut s) = scaler {
                    if s.run(&raw_frame, &mut scaled_frame).is_err() {
                        println!("Scaling failed");
                        return None;
                    }
                }

                let frame_data = scaled_frame.data(0);
                let stride = scaled_frame.stride(0);
                let width_in_bytes = 3 * target_width as usize;
                let frame_target_len = target_height as usize * width_in_bytes;

                if frame_target_len > frame_data.len() {
                    println!(
                        "Skipped frame: {} vs {}",
                        frame_data.len(),
                        frame_target_len,
                    );
                    return None;
                }

                let mut pixels = Vec::with_capacity(frame_target_len);
                for line in 0..scaled_frame.height() as usize {
                    let begin = line * stride;
                    let end = begin + width_in_bytes;
                    pixels.extend_from_slice(&frame_data[begin..end]);
                }
                Some(pixels)
            };

            let mut packet_count = 0u64;
            let mut first_frame_logged = false;
            loop {
                if cancel_decode.load(Ordering::Relaxed) {
                    break;
                }

                let packet: ffmpeg::Packet = match packet_rx.recv() {
                    Ok(p) => p,
                    Err(_) => break,
                };
                packet_count += 1;

                if let Err(e) = transcoder.send_packet_to_decoder(&packet) {
                    println!("[VideoDecode] send_packet failed: {:?}", e);
                    continue;
                }

                while let Ok(raw_frame) = transcoder.receive_decoded_frames() {
                    let frame_pts = raw_frame.pts().unwrap_or(0);
                    if !first_frame_logged {
                        first_frame_logged = true;
                        println!(
                            "[VideoDecode] first frame decoded pts={} after {} packets",
                            frame_pts, packet_count
                        );
                    }
                    if let Some(pixels) = process_frame(raw_frame) {
                        let _ = frame_tx.send((pixels, frame_pts));
                    }
                }
            }

            // Drain remaining frames from decoder.
            let _ = transcoder.send_eof();
            while let Ok(raw_frame) = transcoder.receive_decoded_frames() {
                let pts = raw_frame.pts().unwrap_or(0);
                if let Some(pixels) = process_frame(raw_frame) {
                    let _ = frame_tx.send((pixels, pts));
                }
            }
        });

        let _output_handle = std::thread::spawn(move || {
            let frame_duration = Duration::from_secs_f64(1.0 / frame_rate as f64);
            let mut next_frame_time = Instant::now();
            let mut last_frame_instant = Instant::now();
            let mut frame_count = 0u64;

            loop {
                if cancel_output.load(Ordering::Relaxed) {
                    break;
                }

                let frame = match frame_rx.try_recv() {
                    Ok(f) => f,
                    Err(ring_channel::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(ring_channel::TryRecvError::Disconnected) => break,
                };

                let (pixels, _pts) = frame;
                let now = Instant::now();
                let since_last = now.duration_since(last_frame_instant).as_millis();
                last_frame_instant = now;
                frame_count += 1;
                if frame_count % 60 == 0 {
                    println!(
                        "[VideoOutput] frame #{} displayed, {} ms since last",
                        frame_count, since_last
                    );
                }

                if now < next_frame_time {
                    std::thread::sleep(next_frame_time - now);
                }
                new_frame_callback(pixels);
                next_frame_time += frame_duration;
            }
        });

        Ok(Self {
            packet_tx,
            cancel,
            _decode_handle,
            _output_handle,
        })
    }

    /// Non-blocking packet send. If the decoder is falling behind, old packets are overwritten.
    pub fn try_send_packet(&self, packet: ffmpeg::Packet) {
        match self.packet_tx.send(packet) {
            Ok(Some(_)) => {
                // An old packet was overwritten — decoder is lagging, expected during catch-up.
            }
            Ok(None) => {}
            Err(_) => {
                // All receivers dropped — VideoStream is shutting down.
            }
        }
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.stop();
    }
}
