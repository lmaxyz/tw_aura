use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};

use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;

use super::audio_decoder::AudioChunk;

/// Bytes per second for S16 stereo @ 48kHz = 48000 * 2 channels * 2 bytes = 192000
const BYTES_PER_SEC: f64 = 48000.0 * 2.0 * 2.0;

struct AudioStreamInner {
    audio_bytes_consumed: AtomicU64,
    last_audio_pts: AtomicI64,
}

pub struct AudioStream {
    inner: Arc<AudioStreamInner>,
}

impl Clone for AudioStream {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl AudioStream {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AudioStreamInner {
                audio_bytes_consumed: AtomicU64::new(0),
                last_audio_pts: AtomicI64::new(0),
            }),
        }
    }

    pub fn stop(&self) {
        self.inner.audio_bytes_consumed.store(0, Ordering::Relaxed);
        self.inner.last_audio_pts.store(0, Ordering::Relaxed);
    }

    /// PTS последнего аудио-чанка, полученного фидер-потоком.
    /// Используется для A/V sync (в timeline потока, а не playback time).
    pub fn last_audio_pts(&self) -> i64 {
        self.inner.last_audio_pts.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    /// Возвращает текущее время воспроизведения аудио в секундах.
    pub fn audio_clock(&self) -> f64 {
        self.inner.audio_bytes_consumed.load(Ordering::Relaxed) as f64 / BYTES_PER_SEC
    }

    pub fn start(&self, audio_rx: std::sync::mpsc::Receiver<AudioChunk>, _audio_time_base: f64) {
        self.stop();

        let inner = self.inner.clone();

        std::thread::spawn(move || {
            let spec = Spec {
                format: Format::S16le,
                channels: 2,
                rate: 48000,
            };

            if !spec.is_valid() {
                log::error!("AudioStream::start: invalid sample spec");
                return;
            }

            let simple = match create_simple(&spec) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to create PulseAudio connection: {e}");
                    return;
                }
            };

            loop {
                match audio_rx.recv() {
                    Ok(chunk) => {
                        if let Err(e) = simple.write(&chunk.data) {
                            log::error!("PulseAudio write error: {e}");
                            break;
                        }
                        inner
                            .audio_bytes_consumed
                            .fetch_add(chunk.data.len() as u64, Ordering::Relaxed);
                        inner.last_audio_pts.store(chunk.pts, Ordering::Relaxed);
                    }
                    Err(_) => {
                        log::debug!("Audio feed: audio_rx disconnected, exiting");
                        break;
                    }
                }
            }

            if let Err(e) = simple.drain() {
                log::error!("PulseAudio drain error: {e}");
            }
        });
    }
}

fn create_simple(spec: &Spec) -> Result<Simple, libpulse_binding::error::PAErr> {
    match Simple::new(
        None,
        "TwAura",
        Direction::Playback,
        None,
        "Twitch Stream",
        spec,
        None,
        None,
    ) {
        Ok(s) => Ok(s),
        Err(e) => {
            log::warn!("AudioStream: default connection failed ({e}), trying fallback socket...");
            if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
                let path = std::path::PathBuf::from(runtime_dir).join("pulse/native");
                if path.exists() {
                    let server = format!("unix:{}", path.display());
                    Simple::new(
                        Some(&server),
                        "TwAura",
                        Direction::Playback,
                        None,
                        "Twitch Stream",
                        spec,
                        None,
                        None,
                    )
                } else {
                    Err(e)
                }
            } else {
                Err(e)
            }
        }
    }
}
