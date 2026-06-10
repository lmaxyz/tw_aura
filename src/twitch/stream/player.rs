use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use m3u8_rs::{MasterPlaylist, Resolution, VariantStream};

use super::audio::AudioStream;
use super::video::VideoStream;
use super::YuvFrame;
use crate::twitch::twitch_legacy;

pub struct PlayerSettings {
    master_playlist: MasterPlaylist,
    pub selected_stream: VariantStream,
}

impl PlayerSettings {
    pub fn available_streams(&self) -> &Vec<VariantStream> {
        &self.master_playlist.variants
    }
}

pub struct StreamPlayer {
    stream_reader_handler: Option<JoinHandle<()>>,
    audio_stream: AudioStream,
    pub settings: PlayerSettings,
    cancel: Arc<AtomicBool>,
    #[cfg(feature = "aurora")]
    display_wakelock_handler: Option<JoinHandle<()>>,
}

impl StreamPlayer {
    pub fn new(streamer_login: &str, preferred_quality: Option<String>) -> Self {
        let master_playlist = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                twitch_legacy::get_streamer_playlist(&streamer_login)
                    .await
                    .unwrap()
            });

        let selected_stream = if let Some(preferred) = preferred_quality {
            master_playlist
                .variants
                .iter()
                .find(|sv| sv.video.as_ref() == Some(&preferred))
                .cloned()
                .unwrap_or_else(|| master_playlist.variants.first().unwrap().clone())
        } else {
            master_playlist.variants.iter().last().unwrap().clone()
        };

        let audio_stream = AudioStream::new().expect("Can't create audio streamer");

        StreamPlayer {
            stream_reader_handler: None,
            settings: PlayerSettings {
                master_playlist,
                selected_stream,
            },
            audio_stream,
            cancel: Arc::new(AtomicBool::new(false)),
            #[cfg(feature = "aurora")]
            display_wakelock_handler: None,
        }
    }

    pub fn set_stream_variant(&mut self, variant_stream: &VariantStream) {
        self.settings.selected_stream = variant_stream.clone();
    }

    pub fn play<F>(&mut self, new_frame_cb: F)
    where
        F: FnMut(YuvFrame) + Send + 'static,
    {
        self.stop();

        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();

        let stream_uri = self.settings.selected_stream.uri.clone();
        let preferred_resolution = self.settings.selected_stream.resolution;

        #[cfg(feature = "aurora")]
        {
            let wakelock_cancel = cancel.clone();
            self.display_wakelock_handler = Some(std::thread::spawn(move || {
                match aurora_services::DisplayService::new() {
                    Ok(display_service) => loop {
                        if wakelock_cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        display_service.pause_display_blanking().unwrap();
                        std::thread::sleep(std::time::Duration::from_secs(30));
                    },
                    Err(e) => {
                        println!("Unable to init display service: {:?}", e);
                        // ToDo: Log wakelock service is not available.
                    }
                }
            }));
        }

        let audio_stream = self.audio_stream.clone();
        let cancel_reader = cancel.clone();

        let callback = Arc::new(std::sync::Mutex::new(new_frame_cb));

        self.stream_reader_handler = Some(std::thread::spawn(move || {
            loop {
                if cancel_reader.load(Ordering::Relaxed) {
                    println!("Reader thread cancelled");
                    break;
                }

                match ffmpeg_next::format::input_with_interrupt(&stream_uri, || {
                    cancel_reader.load(Ordering::Relaxed)
                }) {
                    Ok(mut input_ctx) => {
                        let mut video_stream_idx = None;
                        let mut audio_stream_idx = None;

                        for stream in input_ctx.streams() {
                            match stream.parameters().medium() {
                                ffmpeg_next::media::Type::Video => {
                                    video_stream_idx = Some(stream.index());
                                }
                                ffmpeg_next::media::Type::Audio => {
                                    audio_stream_idx = Some(stream.index());
                                }
                                _ => {}
                            }
                        }

                        let video_stream_idx = match video_stream_idx {
                            Some(idx) => idx,
                            None => {
                                println!("No video stream found");
                                std::thread::sleep(Duration::from_secs(1));
                                continue;
                            }
                        };

                        let video_stream = input_ctx.stream(video_stream_idx).unwrap();
                        let mut frame_rate =
                            video_stream.rate().numerator() / video_stream.rate().denominator();
                        if frame_rate < 5 {
                            println!("Suspicious frame rate {frame_rate}, falling back to 30");
                            frame_rate = 30;
                        }
                        let video_time_base = {
                            let tb = video_stream.time_base();
                            tb.numerator() as f64 / tb.denominator() as f64
                        };
                        let audio_time_base = if let Some(idx) = audio_stream_idx {
                            let stream = input_ctx.stream(idx).unwrap();
                            let tb = stream.time_base();
                            tb.numerator() as f64 / tb.denominator() as f64
                        } else {
                            1.0 / 90000.0
                        };

                        println!(
                            "FRAME RATE: {frame_rate}, VIDEO_TIME_BASE: {video_time_base:.6}, AUDIO_TIME_BASE: {audio_time_base:.6}"
                        );

                        let resolution = preferred_resolution.or_else(|| {
                            let codec = ffmpeg_next::codec::context::Context::from_parameters(
                                video_stream.parameters(),
                            )
                            .ok()?;
                            let decoder = codec.decoder().video().ok()?;
                            Some(Resolution {
                                width: decoder.width() as u64,
                                height: decoder.height() as u64,
                            })
                        });

                        let cb = callback.clone();
                        let audio_for_sync = audio_stream.clone();
                        let video_player = match VideoStream::new(
                            &video_stream,
                            resolution,
                            frame_rate,
                            video_time_base,
                            move |yuv| {
                                if let Ok(mut guard) = cb.lock() {
                                    guard(yuv);
                                }
                            },
                            move || audio_for_sync.last_audio_pts() as f64 * audio_time_base,
                        ) {
                            Ok(vp) => vp,
                            Err(e) => {
                                println!("Failed to create VideoStream: {:?}", e);
                                std::thread::sleep(Duration::from_secs(1));
                                continue;
                            }
                        };

                        let (audio_pkt_tx, audio_pkt_rx) = std::sync::mpsc::sync_channel(120);
                        let (audio_frame_tx, audio_frame_rx) = std::sync::mpsc::sync_channel(20);

                        let audio_transcoder = if let Some(idx) = audio_stream_idx {
                            match super::audio_decoder::AudioTranscoder::new(
                                &input_ctx.stream(idx).unwrap(),
                            ) {
                                Ok(at) => Some(at),
                                Err(e) => {
                                    println!("Failed to create audio transcoder: {:?}", e);
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        audio_stream.stop();
                        if audio_transcoder.is_some() {
                            audio_stream.start(audio_frame_rx, audio_time_base);
                        }

                        let cancel_audio_decode = cancel_reader.clone();
                        if let Some(mut transcoder) = audio_transcoder {
                            std::thread::spawn(move || {
                                for packet in audio_pkt_rx {
                                    if cancel_audio_decode.load(Ordering::Relaxed) {
                                        break;
                                    }
                                    if transcoder.send_packet(&packet).is_ok() {
                                        while let Ok(chunk) = transcoder.receive_and_resample() {
                                            if audio_frame_tx.send(chunk).is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }
                                transcoder.send_eof();
                                let remaining = transcoder.drain();
                                if !remaining.data.is_empty() {
                                    let _ = audio_frame_tx.send(remaining);
                                }
                            });
                        }

                        for (stream, packet) in input_ctx.packets() {
                            if cancel_reader.load(Ordering::Relaxed) {
                                break;
                            }
                            if packet.is_corrupt() {
                                println!("Got corrupt packet, restarting stream reader");
                                break;
                            }

                            match stream.parameters().medium() {
                                ffmpeg_next::media::Type::Video => {
                                    video_player.try_send_packet(packet);
                                }
                                ffmpeg_next::media::Type::Audio => {
                                    if audio_pkt_tx.send(packet).is_err() {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }

                        drop(video_player);
                    }
                    Err(e) => {
                        println!("Failed to open stream input: {:?}, retrying in 1s...", e);
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        }));
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.stream_reader_handler.take();
        #[cfg(feature = "aurora")]
        self.display_wakelock_handler.take();
        self.audio_stream.stop();
    }

    #[allow(dead_code)]
    pub fn pause(&mut self) {}
}
