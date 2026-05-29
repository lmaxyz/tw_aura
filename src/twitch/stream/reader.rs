use std::time::Instant;

use ring_channel::*;

use ffmpeg_next as ffmpeg;
use ffmpeg::format::context::Input;
use ffmpeg::frame::Video;

use crate::twitch::stream::video::Transcoder;
use crate::twitch::stream::audio_decoder::{AudioChunk, AudioTranscoder};
use crate::twitch::stream::utils::print_stream_metadata;

pub struct StreamReader {
    input_ctx: Input,
    video_transcoder: Transcoder,
    audio_transcoder: Option<AudioTranscoder>,
}

impl StreamReader {
    pub fn new(input_ctx: Input, target_resolution: (usize, usize)) -> Result<Self, ffmpeg::Error> {
        let mut video_transcoder = None;
        let mut audio_transcoder = None;

        println!("=== StreamReader::new ===");
        print_stream_metadata(&input_ctx);
        println!("=========================");

        for stream in input_ctx.streams() {
            match stream.parameters().medium() {
                ffmpeg_next::media::Type::Video => {
                    video_transcoder = Some(Transcoder::new(&stream, target_resolution)?);
                },
                ffmpeg_next::media::Type::Audio => {
                    audio_transcoder = Some(AudioTranscoder::new(&stream)?);
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
            audio_transcoder,
        })
    }

    pub fn run(&mut self, video_tx: RingSender<ReadEvent>, audio_tx: RingSender<AudioChunk>) -> Result<(), String> {
        let mut video_pkt_count = 0u64;
        let mut video_frame_count = 0u64;
        let mut audio_pkt_count = 0u64;
        let mut audio_chunk_count = 0u64;

        for (stream, packet) in self.input_ctx.packets() {
            if packet.is_corrupt() {
                println!("Got corrupt packet, restarting stream reader");
                return Err("Corrupt packet".to_string());
            }
            let _start = Instant::now();
            match stream.parameters().medium() {
                ffmpeg_next::media::Type::Video => {
                    video_pkt_count += 1;

                    match self.video_transcoder.send_packet_to_decoder(&packet) {
                        Ok(()) => {
                            loop {
                                match self.video_transcoder.receive_decoded_frames() {
                                    Ok(frame) => {
                                        video_frame_count += 1;
                                        if video_tx.send(ReadEvent::NewFrames(frame)).is_err() {
                                            println!("video_tx disconnected, stopping reader");
                                            return Ok(());
                                        }
                                    }
                                    Err(e) => {
                                        let msg = e.to_string();
                                        if !msg.contains("temporarily unavailable") {
                                            println!("Video decode error: {} (key={})", msg, packet.is_key());
                                            self.video_transcoder.flush_decoder();
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("Video send_packet error: {:?} (key={})", e, packet.is_key());
                            self.video_transcoder.flush_decoder();
                        }
                    }
                },
                ffmpeg_next::media::Type::Audio => {
                    audio_pkt_count += 1;
                    if let Some(ref mut transcoder) = self.audio_transcoder {
                        if transcoder.send_packet(&packet).is_ok() {
                            while let Ok(chunk) = transcoder.receive_and_resample() {
                                audio_chunk_count += 1;
                                if audio_tx.send(chunk).is_err() {
                                    println!("audio_tx disconnected, stopping reader");
                                    return Ok(());
                                }
                            }
                        }
                    }
                },
                _ => {
                    // Skip other packets
                }
            }
            // println!("Packet processing time: {}", start.elapsed().as_secs_f32());
        }
        println!(
            "StreamReader finished. video_packets={}, video_frames={}, audio_packets={}",
            video_pkt_count, video_frame_count, audio_pkt_count
        );
        println!("Audio chunks sent: {}", audio_chunk_count);

        self.video_transcoder.send_eof_to_decoder();
        while let Ok(frame) = self.video_transcoder.receive_decoded_frames() {
            let _ = video_tx.send(ReadEvent::NewFrames(frame));
        }

        if let Some(ref mut transcoder) = self.audio_transcoder {
            transcoder.send_eof();
            let remaining = transcoder.drain();
            if !remaining.data.is_empty() {
                let _ = audio_tx.send(remaining);
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub enum ReadEvent {
    NewFrames(Video),
    #[allow(dead_code)]
    Failed,
}
