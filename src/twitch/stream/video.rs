use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ffmpeg_next::{self as ffmpeg, frame};

use super::video_decoder::Transcoder;
use super::YuvFrame;

/// Self-contained video pipeline: packet input → decode → paced frame output.
///
/// Spawns two internal threads:
/// - **Decode thread**: receives packets, decodes raw YUV frames, pushes to ring buffer.
/// - **Output thread**: pulls frames from ring buffer, paces them at `1/fps`, invokes callback.
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
        _target_resolution: Option<m3u8_rs::Resolution>,
        frame_rate: i32,
        _video_time_base: f64,
        mut new_frame_callback: impl FnMut(YuvFrame) + Send + 'static,
        _get_audio_pts_sec: impl Fn() -> f64 + Send + 'static,
    ) -> Result<Self, ffmpeg::Error> {
        let transcoder = Transcoder::new(input_stream)?;

        // Packet ring: generous capacity so the decoder never starves due to overwrites.
        let packet_cap = NonZeroUsize::new((frame_rate * 10).max(120) as usize).unwrap();
        let (packet_tx, packet_rx) = ring_channel::ring_channel(packet_cap);

        // Frame ring: ~7 seconds of raw frames. Overwrite old frames if output lags.
        let frame_cap = NonZeroUsize::new((frame_rate * 7).max(1) as usize).unwrap();
        let (frame_tx, frame_rx) = ring_channel::ring_channel(frame_cap);

        let cancel = Arc::new(AtomicBool::new(false));

        let cancel_decode = cancel.clone();
        let cancel_output = cancel.clone();

        let _decode_handle = std::thread::spawn(move || {
            let mut transcoder = transcoder;

            let process_frame = |raw_frame: frame::Video| -> Option<YuvFrame> {
                if raw_frame.format() != ffmpeg::format::Pixel::YUV420P {
                    // Only YUV420P is supported for zero-copy GPU rendering.
                    return None;
                }

                let w = raw_frame.width();
                let h = raw_frame.height();
                let half_w = w / 2;
                let half_h = h / 2;

                let mut y = Vec::with_capacity((w * h) as usize);
                let y_stride = raw_frame.stride(0);
                let y_data = raw_frame.data(0);
                for line in 0..h as usize {
                    let start = line * y_stride;
                    y.extend_from_slice(&y_data[start..start + w as usize]);
                }

                let mut u = Vec::with_capacity((half_w * half_h) as usize);
                let u_stride = raw_frame.stride(1);
                let u_data = raw_frame.data(1);
                for line in 0..half_h as usize {
                    let start = line * u_stride;
                    u.extend_from_slice(&u_data[start..start + half_w as usize]);
                }

                let mut v = Vec::with_capacity((half_w * half_h) as usize);
                let v_stride = raw_frame.stride(2);
                let v_data = raw_frame.data(2);
                for line in 0..half_h as usize {
                    let start = line * v_stride;
                    v.extend_from_slice(&v_data[start..start + half_w as usize]);
                }

                Some(YuvFrame {
                    width: w,
                    height: h,
                    y,
                    u,
                    v,
                })
            };

            let mut first_frame_logged = false;
            loop {
                if cancel_decode.load(Ordering::Relaxed) {
                    break;
                }

                let packet: ffmpeg::Packet = match packet_rx.recv() {
                    Ok(p) => p,
                    Err(_) => break,
                };

                if let Err(_) = transcoder.send_packet_to_decoder(&packet) {
                    continue;
                }

                while let Ok(raw_frame) = transcoder.receive_decoded_frames() {
                    let frame_pts = raw_frame.pts().unwrap_or(0);
                    if !first_frame_logged {
                        first_frame_logged = true;
                    }
                    if let Some(yuv) = process_frame(raw_frame) {
                        let _ = frame_tx.send((yuv, frame_pts));
                    }
                }
            }

            // Drain remaining frames from decoder.
            let _ = transcoder.send_eof();
            while let Ok(raw_frame) = transcoder.receive_decoded_frames() {
                let pts = raw_frame.pts().unwrap_or(0);
                if let Some(yuv) = process_frame(raw_frame) {
                    let _ = frame_tx.send((yuv, pts));
                }
            }
        });

        let _output_handle = std::thread::spawn(move || {
            let frame_duration = Duration::from_secs_f64(1.0 / frame_rate as f64);
            let mut next_frame_time = Instant::now();

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

                let (yuv, _pts) = frame;
                let now = Instant::now();

                if now < next_frame_time {
                    std::thread::sleep(next_frame_time - now);
                }
                new_frame_callback(yuv);
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
