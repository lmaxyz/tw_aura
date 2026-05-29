use std::{
    collections::VecDeque,
    ffi::CString,
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}},
    time::{Duration, Instant},
};

use pulseaudio::{AsPlaybackSource, Client as PAClient, ClientError, PlaybackStream, protocol};
use ring_channel::RingReceiver;
use tokio::runtime::Handle;

use super::audio_decoder::AudioChunk;

/// Bytes per second for S16 stereo @ 48kHz = 48000 * 2 channels * 2 bytes = 192000
const BYTES_PER_SEC: f64 = 48000.0 * 2.0 * 2.0;
/// 100 ms в байтах
const TARGET_BUFFER_BYTES: u32 = (BYTES_PER_SEC * 0.1) as u32;

struct AudioStreamInner {
    client: PAClient,
    stream_handle: Mutex<Option<PlaybackStream>>,
    pcm_buffer: Mutex<VecDeque<u8>>,
    bytes_played: AtomicU64,
    audio_start_time: Mutex<Option<Instant>>,
}

pub struct AudioStream {
    inner: Arc<AudioStreamInner>,
    _runtime: Option<tokio::runtime::Runtime>,
    handle: Handle,
}

impl Clone for AudioStream {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _runtime: None,
            handle: self.handle.clone(),
        }
    }
}

impl AudioStream {
    pub fn new() -> Result<Self, ClientError> {
        println!("AudioStream::new: connecting to PulseAudio...");
        let name = CString::new("TwAura").unwrap();
        let client = match PAClient::from_env(name.clone()) {
            Ok(client) => {
                println!("AudioStream::new: connected via env");
                client
            }
            Err(ClientError::ServerUnavailable) => {
                println!("AudioStream::new: from_env failed, trying fallback socket...");
                let socket_path = std::env::var("XDG_RUNTIME_DIR")
                    .ok()
                    .map(|d| std::path::PathBuf::from(d).join("pulse/native"))
                    .filter(|p| p.exists());

                if let Some(path) = socket_path {
                    let socket = std::os::unix::net::UnixStream::connect(&path)
                        .map_err(|_| ClientError::ServerUnavailable)?;
                    let cookie =
                        pulseaudio::cookie_path_from_env().and_then(|p| std::fs::read(p).ok());
                    PAClient::new_unix(name, socket, cookie)?
                } else {
                    return Err(ClientError::ServerUnavailable);
                }
            }
            Err(e) => return Err(e),
        };
        let async_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let handle = async_rt.handle().clone();
        Ok(Self {
            inner: Arc::new(AudioStreamInner {
                client,
                stream_handle: Mutex::new(None),
                pcm_buffer: Mutex::new(VecDeque::with_capacity(262144)),
                bytes_played: AtomicU64::new(0),
                audio_start_time: Mutex::new(None),
            }),
            _runtime: Some(async_rt),
            handle,
        })
    }

    pub fn stop(&self) {
        let old_stream = self.inner.stream_handle.lock().unwrap().take();
        drop(old_stream);
        self.inner.pcm_buffer.lock().unwrap().clear();
        self.inner.bytes_played.store(0, Ordering::Relaxed);
        *self.inner.audio_start_time.lock().unwrap() = None;
        println!("AudioStream::stop: stream dropped, buffer cleared");
    }

    /// Момент, когда первые реальные аудио-данные ушли в PA.
    pub fn audio_start_time(&self) -> Option<Instant> {
        *self.inner.audio_start_time.lock().unwrap()
    }

    pub fn start(&self, audio_rx: RingReceiver<AudioChunk>, _audio_time_base: f64) {
        println!("AudioStream::start: beginning start sequence");
        self.stop();

        let inner = self.inner.clone();

        // Поток, перекачивающий PCM из ring channel в локальный буфер
        std::thread::spawn(move || {
            let mut tick = 0u64;
            loop {
                match audio_rx.try_recv() {
                    Ok(chunk) => {
                        inner.pcm_buffer.lock().unwrap().extend(chunk.data);
                    }
                    Err(ring_channel::TryRecvError::Empty) => {
                        tick += 1;
                        if tick % 500 == 0 {
                            let buf_len = inner.pcm_buffer.lock().unwrap().len();
                            println!("Audio feed: pcm_buffer={} bytes (Empty)", buf_len);
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(ring_channel::TryRecvError::Disconnected) => {
                        println!("Audio feed: audio_rx disconnected, exiting");
                        break;
                    }
                }
            }
        });

        let sample_spec = protocol::SampleSpec {
            format: protocol::SampleFormat::S16Le,
            channels: 2,
            sample_rate: 48000,
        };

        let params = protocol::PlaybackStreamParams {
            sample_spec,
            channel_map: protocol::ChannelMap::stereo(),
            cvolume: Some(protocol::ChannelVolume::muted(2)),
            flags: protocol::stream::StreamFlags {
                adjust_latency: true,
                ..Default::default()
            },
            buffer_attr: protocol::stream::BufferAttr {
                max_length: TARGET_BUFFER_BYTES,
                target_length: TARGET_BUFFER_BYTES,
                pre_buffering: 0,
                minimum_request_length: (TARGET_BUFFER_BYTES / 4).max(1),
                ..Default::default()
            },
            ..Default::default()
        };

        let inner = self.inner.clone();
        let callback = move |data: &mut [u8]| {
            let mut pcm = inner.pcm_buffer.lock().unwrap();
            let to_copy = std::cmp::min(pcm.len(), data.len());
            for i in 0..to_copy {
                data[i] = pcm.pop_front().unwrap();
            }
            if to_copy < data.len() {
                data[to_copy..].fill(0);
            }
            // Запоминаем момент, когда первые реальные данные ушли в PA
            if to_copy > 0 {
                let mut start = inner.audio_start_time.lock().unwrap();
                if start.is_none() {
                    *start = Some(Instant::now());
                    println!("AudioStream: first real data sent to PA");
                }
            }
            inner.bytes_played.fetch_add(to_copy as u64, Ordering::Relaxed);
            data.len()
        };

        let inner = self.inner.clone();
        self.handle.spawn(async move {
            println!("AudioStream::start: creating playback stream...");
            match tokio::time::timeout(
                Duration::from_secs(5),
                inner.client.create_playback_stream(params, callback.as_playback_source()),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    println!("AudioStream::start: playback stream created successfully");
                    *inner.stream_handle.lock().unwrap() = Some(stream);
                }
                Ok(Err(e)) => eprintln!("Failed to create playback stream: {}", e),
                Err(_) => eprintln!("Timeout creating playback stream"),
            }
        });
    }
}
