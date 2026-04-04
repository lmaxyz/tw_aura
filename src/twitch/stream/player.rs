use std::{collections::VecDeque, num::{NonZero, NonZeroU32, NonZeroUsize}, sync::{Arc, Mutex, RwLock}, thread::JoinHandle};
use std::time::{Duration, Instant};

use ffmpeg_next::{format::context::Input, frame::Video};
use m3u8_rs::{MasterPlaylist, Resolution, VariantStream};
use ring_channel::{RingReceiver, ring_channel};

use crate::twitch::twitch_legacy;
use super::reader::StreamReader;
use super::utils::print_stream_metadata;
use super::audio::AudioStream;


enum PlayerState {
    Playing,
    Paused,
    Failed,
}


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
    audio_stream: AudioStream,
    pub settings: PlayerSettings,
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
            settings: PlayerSettings {
                master_playlist,
                selected_stream,
            },
            audio_stream,
        }
    }

    pub fn set_stream_variant(&mut self, variant_stream: &VariantStream) {
        self.settings.selected_stream = variant_stream.clone();
    }

    pub fn playlist(&self) -> &MasterPlaylist {
        &self.settings.master_playlist
    }

    pub fn play_audio(&self) {
        self.audio_stream.play_audio();
    }

    pub fn play<F>(&mut self, mut new_frame_cb: F)
    where F: FnMut(Vec<u8>) + Send + 'static
    {
        let stream_uri = self.settings.selected_stream.uri.clone();
        let resolution = self.settings.selected_stream.resolution.unwrap();
        let input_ctx = ffmpeg_next::format::input(&stream_uri).unwrap();
        print_stream_metadata(&input_ctx);
        let stream_frame_rate = get_stream_frame_rate(&input_ctx);
        println!("FRAME RATE: {stream_frame_rate}");
        // Опытным путем выяснилось, что хранить 7 секунд лучше всего, если сделать буфер меньше, то воспроизведение лагает.
        // Возможно, в будущем нужно это решить более правильным решением синхронизации.
        // Сейчас пакеты на стороне стрим-ридера не синхронизируются, поэтому они успевают перезаписать кадры буфера, которые еще не успели отрисоваться,
        // а новые кадры рисовать слишком рано, поэтому.
        let (tx, rx) = ring_channel(NonZeroUsize::new(stream_frame_rate as usize * 7).unwrap());


        let stream_start_time = get_stream_start_time(&input_ctx);
        let mut v_sync = VideoSync::new(1./90000.);
        v_sync.set_stream_start_time(stream_start_time);

        self.stream_reader_handler = Some(std::thread::spawn(move || {
            let mut stream_reader = StreamReader::new(input_ctx, (resolution.width as _, resolution.height as _)).unwrap();
            stream_reader.run(tx);
        }));

        std::thread::spawn(move || {
            loop {
                if let Ok(frame) = rx.recv() {
                    match frame {
                        super::reader::ReadEvent::Failed => {
                            println!("Playing stopped");
                            break
                        },
                        super::reader::ReadEvent::NewFrames(frame) => {
                            if let Some(pts) = frame.pts() {
                                match v_sync.time_sync_and_drop(pts) {
                                    SyncResult::Display => {
                                        let start_time = Instant::now();
                                        let frame_data = frame.data(0);
                                        let stride = frame.stride(0);
                                        let size = [frame.width() as usize, frame.height() as usize];
                                        let width_in_bytes = 3 * size[0];
                                        let frame_target_len = size[1] * width_in_bytes;

                                        if frame_target_len > frame_data.len() {
                                            // Skip corrupted frames
                                            println!("Skipped frame: {} vs {}, size {:?}", frame_data.len(), frame_target_len, size);
                                            // return None;
                                        }

                                        let mut pixels = Vec::with_capacity(frame_target_len);
                                        for line in 0..frame.height() as usize {
                                            let begin = line * stride;
                                            let end = begin + width_in_bytes;
                                            let data_line = &frame_data[begin..end];
                                            pixels.extend(data_line)
                                        }
                                        new_frame_cb(pixels);
                                        // println!("Render frame time elapsed: {}", start_time.elapsed().as_secs_f32());
                                    }
                                    SyncResult::Drop => {
                                        // Пропускаем отстающий кадр
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn stop(&mut self) {

    }

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


fn get_stream_start_time(input_ctx: &Input) -> i64 {
    let mut stream_start_time = 0;
    for stream in input_ctx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Video {
            stream_start_time = stream.start_time();
            break;
        }
    }
    stream_start_time
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

    fn set_stream_start_time(&mut self, stream_start_time: i64) {
        self.first_pts = stream_start_time;
        self.start_time = Instant::now();
    }

    // Для пропуска отстающих кадров
    fn time_sync_and_drop(&mut self, frame_pts: i64) -> SyncResult {
        let relative_pts = frame_pts - self.first_pts;
        let frame_time = relative_pts as f64 * self.time_base;
        let elapsed = self.start_time.elapsed().as_secs_f64();

        if frame_time <= elapsed {
            if elapsed - frame_time > 0.5 { // Кадр отстал больше чем на 500ms
                // println!("DROPPED: {} {}",frame_time, elapsed);
                return SyncResult::Drop
            }
            SyncResult::Display
        } else {
            // Ждем нужное время перед отрисовкой
            let sleep_time = frame_time - elapsed;
            // println!("Sleep: {sleep_time}");
            std::thread::sleep(Duration::from_secs_f64(sleep_time));
            SyncResult::Display
        }
    }
}

enum SyncResult {
    Display,
    Drop,
}
