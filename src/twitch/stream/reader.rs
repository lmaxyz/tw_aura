use std::sync::{Arc, Mutex};
use std::time::{Instant, Duration};
use std::collections::VecDeque;
use std::num::NonZeroUsize;

use ring_channel::*;

use ffmpeg_next as ffmpeg;
use ffmpeg::format::context::Input;
use ffmpeg::frame::{Video, Audio};

use crate::twitch::stream::video::Transcoder;

pub struct StreamReader {
    input_ctx: Input,
    video_transcoder: Transcoder,
}

impl StreamReader {
    pub fn new(input_ctx: Input, target_resolution: (usize, usize)) -> Result<Self, ffmpeg::Error> {
        let mut video_transcoder = None;

        for stream in input_ctx.streams() {
            match stream.parameters().medium() {
                ffmpeg_next::media::Type::Video => {
                    video_transcoder = Some(Transcoder::new(&stream, target_resolution)?);
                },
                _ => {}
            }
        }

        if video_transcoder.is_none() {
            return Err(ffmpeg::Error::StreamNotFound)
        }

        Ok(Self {
            input_ctx,
            video_transcoder: video_transcoder.unwrap(),
        })
    }

    pub fn run(&mut self, ring_channel_sender: RingSender<ReadEvent>) {
        let mut v_sync = VideoSync::new(1./90000.);
        for (stream, packet) in self.input_ctx.packets() {
            if let Some(packet_pts) = packet.pts() && v_sync.time_sync(packet_pts) == SyncResult::Drop {
                println!("Drop packet");
                continue;
            }
            let start = Instant::now();
            match stream.parameters().medium() {
                ffmpeg_next::media::Type::Video => {
                    if self.video_transcoder.send_packet_to_decoder(&packet).is_ok() {
                        if let Ok(frame) = self.video_transcoder.receive_decoded_frames() {
                            ring_channel_sender.send(ReadEvent::NewFrames(frame)).unwrap();
                        }
                    }
                },
                ffmpeg_next::media::Type::Audio => {
                    // Add audio support
                },
                media_type => {
                    println!("Non video and audio packet received: {:?}", media_type);
                    // Skip other packets
                }
            }
            if packet.is_corrupt() {
                println!("Got corrupt packet, need rerun reader");
                ring_channel_sender.send(ReadEvent::Failed).expect("Failed to send Fail event through ring channel.");
                break;
            }
            // println!("Packet processing time: {}", start.elapsed().as_secs_f32());
        }
        self.video_transcoder.send_eof_to_decoder();
        self.video_transcoder.receive_decoded_frames().unwrap();
    }
}

#[derive(Clone)]
pub enum ReadEvent {
    NewFrames(Video),
    Failed,
}


#[derive(Clone)]
struct VideoSync {
    start_time: Instant,
    first_pts: i64,
    time_base: f64, // например 1/90000 для 90kHz
}

impl VideoSync {
    fn new(time_base: f64) -> Self {
        Self {
            start_time: Instant::now(),
            first_pts: 0,
            time_base: time_base,
        }
    }

    // Для пропуска отстающих кадров
    fn time_sync(&mut self, frame_pts: i64) -> SyncResult {
        if self.first_pts == 0 {
            self.first_pts = frame_pts;
            return SyncResult::Display
        }
        let relative_pts = frame_pts - self.first_pts;
        let frame_time = relative_pts as f64 * self.time_base;
        let elapsed = self.start_time.elapsed().as_secs_f64();

        if elapsed - frame_time > 0.1 { // Кадр отстал больше чем на 500ms
            println!("DROPPED: {} {}",frame_time, elapsed);
            return SyncResult::Drop
        }
        SyncResult::Display
    }
}

#[derive(PartialEq)]
enum SyncResult {
    Display,
    Drop,
}


// #[derive(Clone)]
// pub struct FramesQueue<T> {
//     capacity: usize,
//     frames: Arc<Mutex<VecDeque<T>>>,
// }

// impl<T> FramesQueue<T> {
//     pub fn new() -> Self {
//         let capacity = 420;
//         FramesQueue {
//             capacity,
//             frames: Arc::new(Mutex::new(VecDeque::with_capacity(capacity)))
//         }
//     }

//     fn count(&self) -> usize {
//         self.frames.lock().unwrap().len()
//     }

//     pub fn with_capacity(capacity: usize) -> Self {
//         FramesQueue {
//             capacity,
//             frames: Arc::new(Mutex::new(VecDeque::with_capacity(capacity)))
//         }
//     }

//     pub fn next_frame(&mut self) -> Option<T> {
//         self.frames.lock().unwrap().pop_front()
//     }

//     pub fn put_new_frame(&mut self, frame: T) {
//         let mut frames = self.frames.lock().unwrap();
//         if frames.len() >= self.capacity {
//             frames.pop_front();
//         }
//         frames.push_back(frame);
//     }
// }
