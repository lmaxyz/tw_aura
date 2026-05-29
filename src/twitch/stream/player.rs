use std::{num::NonZeroUsize, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread::JoinHandle};
use std::time::{Duration, Instant};

use ffmpeg_next::format::context::Input;
use m3u8_rs::{MasterPlaylist, Resolution, VariantStream};
use ring_channel::ring_channel;

use crate::twitch::twitch_legacy;
use super::reader::StreamReader;
use super::audio::AudioStream;


pub struct PlayerSettings {
    master_playlist: MasterPlaylist,
    pub selected_stream: VariantStream,
}

impl PlayerSettings {
    pub fn resolution(&self) -> Resolution {
        self.selected_stream.resolution.unwrap()
    }

    pub fn available_streams(&self) -> &Vec<VariantStream> {
        &self.master_playlist.variants
    }
}

pub struct StreamPlayer {
    stream_reader_handler: Option<JoinHandle<()>>,
    video_thread_handler: Option<JoinHandle<()>>,
    audio_stream: AudioStream,
    pub settings: PlayerSettings,
    cancel: Arc<AtomicBool>,
}

impl StreamPlayer {
    pub fn new(streamer_login: &str, preferred_quality: Option<Resolution>) -> Self {
        let master_playlist = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
            .block_on(async {
                twitch_legacy::get_streamer_playlist(&streamer_login).await.unwrap()
            });

        let selected_stream = if let Some(preferred_quality) = preferred_quality {
            master_playlist.variants.iter()
                .find(|sv| sv.resolution.map_or(false, |r| r == preferred_quality ))
                .unwrap()
                .clone()
        } else {
            master_playlist.variants.iter().last()
                .unwrap()
                .clone()
        };

        let audio_stream = AudioStream::new().expect("Can't create audio streamer");

        StreamPlayer {
            stream_reader_handler: None,
            video_thread_handler: None,
            settings: PlayerSettings {
                master_playlist,
                selected_stream,
            },
            audio_stream,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_stream_variant(&mut self, variant_stream: &VariantStream) {
        self.settings.selected_stream = variant_stream.clone();
    }

    pub fn playlist(&self) -> &MasterPlaylist {
        &self.settings.master_playlist
    }

    pub fn play<F>(&mut self, mut new_frame_cb: F)
    where F: FnMut(Vec<u8>) + Send + 'static
    {
        self.stop();

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();

        let stream_uri = self.settings.selected_stream.uri.clone();
        let resolution = self.settings.selected_stream.resolution.unwrap();
        let (stream_frame_rate, video_time_base, audio_time_base) = {
            let input_ctx = ffmpeg_next::format::input(&stream_uri).unwrap();
            let fr = get_stream_frame_rate(&input_ctx);
            let vtb = get_video_time_base(&input_ctx);
            let atb = get_audio_time_base(&input_ctx);
            println!("FRAME RATE: {fr}, VIDEO_TIME_BASE: {vtb:.6}, AUDIO_TIME_BASE: {atb:.6}");
            (fr, vtb, atb)
        };

        let (video_tx, video_rx) = ring_channel(NonZeroUsize::new(stream_frame_rate as usize * 7).unwrap());

        let v_sync = VideoSync::new(video_time_base, self.audio_stream.clone());
        let v_sync_for_video = v_sync.clone();

        let cancel_video = cancel.clone();

        // Video render thread
        self.video_thread_handler = Some(std::thread::spawn(move || {
            loop {
                if cancel_video.load(Ordering::Relaxed) {
                    println!("Video thread cancelled");
                    return;
                }

                match video_rx.recv() {
                    Ok(frame) => match frame {
                        super::reader::ReadEvent::Failed => {
                            println!("Playing stopped");
                            return;
                        },
                        super::reader::ReadEvent::NewFrames(frame) => {
                            let should_render = if let Some(pts) = frame.pts() {
                                match v_sync_for_video.time_sync_and_drop(pts) {
                                    SyncResult::Display => true,
                                    SyncResult::Drop => false,
                                }
                            } else {
                                true
                            };

                            if should_render {
                                let frame_data = frame.data(0);
                                let stride = frame.stride(0);
                                let size = [frame.width() as usize, frame.height() as usize];
                                let width_in_bytes = 3 * size[0];
                                let frame_target_len = size[1] * width_in_bytes;

                                if frame_target_len > frame_data.len() {
                                    println!("Skipped frame: {} vs {}, size {:?}", frame_data.len(), frame_target_len, size);
                                } else {
                                    let mut pixels = Vec::with_capacity(frame_target_len);
                                    for line in 0..frame.height() as usize {
                                        let begin = line * stride;
                                        let end = begin + width_in_bytes;
                                        let data_line = &frame_data[begin..end];
                                        pixels.extend(data_line)
                                    }
                                    new_frame_cb(pixels);
                                }
                            }
                        }
                    },
                    Err(_) => {
                        println!("video_rx disconnected, exiting video thread");
                        return;
                    }
                }
            }
        }));

        let audio_stream = self.audio_stream.clone();
        let cancel_reader = cancel.clone();

        // Reader thread — restarts automatically on errors
        self.stream_reader_handler = Some(std::thread::spawn(move || {
            loop {
                if cancel_reader.load(Ordering::Relaxed) {
                    println!("Reader thread cancelled");
                    break;
                }

                match ffmpeg_next::format::input_with_interrupt(&stream_uri, || cancel_reader.load(Ordering::Relaxed)) {
                    Ok(input_ctx) => {
                        match StreamReader::new(input_ctx, (resolution.width as _, resolution.height as _)) {
                            Ok(mut stream_reader) => {
                                v_sync.reset();
                                audio_stream.stop();
                                let (new_audio_tx, new_audio_rx) = ring_channel(NonZeroUsize::new(200).unwrap());
                                audio_stream.start(new_audio_rx, audio_time_base);

                                let video_tx_clone = video_tx.clone();
                                match stream_reader.run(video_tx_clone, new_audio_tx) {
                                    Ok(()) => {
                                        println!("StreamReader finished normally, exiting reader loop");
                                        break;
                                    }
                                    Err(e) => {
                                        println!("StreamReader error: {}, restarting in 1s...", e);
                                    }
                                }
                            }
                            Err(e) => {
                                println!("StreamReader::new failed: {:?}, restarting in 1s...", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("Failed to open stream input: {:?}, retrying in 1s...", e);
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }));
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.stream_reader_handler.take();
        self.video_thread_handler.take();
        self.audio_stream.stop();
    }

    #[allow(dead_code)]
    pub fn pause(&mut self) {

    }
}

fn get_stream_frame_rate(input_ctx: &Input) -> i32 {
    let mut frame_rate = 0;
    for stream in input_ctx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Video {
            println!("AVG FRAME RATE: {:?}", stream.rate());
            frame_rate = stream.rate().numerator() / stream.rate().denominator();
            break;
        }
    }
    frame_rate
}

fn get_video_time_base(input_ctx: &Input) -> f64 {
    for stream in input_ctx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Video {
            let tb = stream.time_base();
            return tb.numerator() as f64 / tb.denominator() as f64;
        }
    }
    1.0 / 90000.0
}

fn get_audio_time_base(input_ctx: &Input) -> f64 {
    for stream in input_ctx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Audio {
            let tb = stream.time_base();
            return tb.numerator() as f64 / tb.denominator() as f64;
        }
    }
    1.0 / 90000.0
}


// =============================================================================
// VideoSync — синхронизирует видео по системному времени,
// но старт совпадает с моментом, когда аудио реально пошло в PA.
// =============================================================================

struct VideoSyncInner {
    start_time: Instant,
    first_pts: Option<i64>,
    time_base: f64,
}

#[derive(Clone)]
struct VideoSync {
    inner: Arc<Mutex<VideoSyncInner>>,
    audio_stream: AudioStream,
}

impl VideoSync {
    fn new(time_base: f64, audio_stream: AudioStream) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VideoSyncInner {
                start_time: Instant::now(),
                first_pts: None,
                time_base,
            })),
            audio_stream,
        }
    }

    fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.first_pts = None;
        inner.start_time = Instant::now();
    }

    fn time_sync_and_drop(&self, frame_pts: i64) -> SyncResult {
        let mut inner = self.inner.lock().unwrap();

        if inner.first_pts.is_none() {
            inner.first_pts = Some(frame_pts);
            drop(inner);

            // Ждём, пока аудио реально начнёт идти в PA.
            // Это синхронизирует старт видео и аудио.
            let mut waited = 0;
            while self.audio_stream.audio_start_time().is_none() && waited < 200 {
                std::thread::sleep(Duration::from_millis(10));
                waited += 1;
            }
            let audio_start = self.audio_stream.audio_start_time().unwrap_or_else(Instant::now);

            let mut inner = self.inner.lock().unwrap();
            inner.start_time = audio_start;
            println!(
                "VideoSync: first frame pts={}, start aligned with audio (waited={}ms)",
                frame_pts, waited * 10
            );
            return SyncResult::Display;
        }

        let first_pts = inner.first_pts.unwrap();
        let frame_time = (frame_pts - first_pts) as f64 * inner.time_base;
        let elapsed = inner.start_time.elapsed().as_secs_f64();
        drop(inner);

        // Сброс при резком скачке PTS (discontinuity)
        if (frame_time - elapsed).abs() > 5.0 {
            println!(
                "AV sync reset: video={:.3}s elapsed={:.3}s diff={:.3}s",
                frame_time, elapsed, frame_time - elapsed
            );
            self.reset();
            return SyncResult::Display;
        }

        if frame_time <= elapsed {
            if elapsed - frame_time > 0.5 {
                return SyncResult::Drop;
            }
            SyncResult::Display
        } else {
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
