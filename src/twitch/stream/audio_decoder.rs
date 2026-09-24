use ffmpeg_next::{self as ffmpeg, Stream};
use ffmpeg::software::resampling::context::Context as ResamplerContext;
use ffmpeg::decoder::audio::Audio as AudioDecoder;
use ffmpeg::frame::Audio;

pub struct AudioChunk {
    pub data: Vec<u8>,
    pub pts: i64,
}

pub struct AudioTranscoder {
    decoder: AudioDecoder,
    resampler: ResamplerContext,
}

impl AudioTranscoder {
    pub fn new(input_stream: &Stream) -> Result<Self, ffmpeg::Error> {
        let decoder = ffmpeg::codec::context::Context::from_parameters(input_stream.parameters())?
            .decoder()
            .audio()?;

        let output_format = ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed);
        let output_rate = 48000;

        let resampler = ResamplerContext::get(
            decoder.format(),
            decoder.channel_layout(),
            decoder.rate(),
            output_format,
            ffmpeg::ChannelLayout::STEREO,
            output_rate,
        )?;

        Ok(Self { decoder, resampler })
    }

    pub fn send_packet(&mut self, packet: &ffmpeg::Packet) -> Result<(), ffmpeg::Error> {
        self.decoder.send_packet(packet)
    }

    pub fn receive_and_resample(&mut self) -> Result<AudioChunk, ffmpeg::Error> {
        let mut frame = Audio::empty();
        self.decoder.receive_frame(&mut frame)?;

        let pts = frame.pts().unwrap_or(0);

        let mut out_frame = Audio::empty();
        self.resampler.run(&frame, &mut out_frame)?;

        let data = out_frame.data(0);
        Ok(AudioChunk {
            data: data.to_vec(),
            pts,
        })
    }

    pub fn send_eof(&mut self) {
        let _ = self.decoder.send_eof();
    }

    pub fn drain(&mut self) -> AudioChunk {
        let mut pcm = Vec::new();
        let mut pts = 0i64;
        while let Ok(chunk) = self.receive_and_resample() {
            if pts == 0 {
                pts = chunk.pts;
            }
            pcm.extend(chunk.data);
        }
        AudioChunk { data: pcm, pts }
    }
}
