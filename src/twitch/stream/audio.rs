use std::{env::home_dir, ffi::CString, fs::File};

use pulseaudio::{
    Client as PAClient,
    protocol,
    ClientError,
    PlaybackStream,
    AsPlaybackSource,
    protocol::PlaybackStreamParams,
};
use tokio::runtime::Runtime;

pub struct AudioStream {
    client: PAClient,
    async_rt: Runtime,
    // out_stream: OutputStream,
}

impl AudioStream {
    pub fn new() -> Result<Self, ClientError> {
        let name = CString::new("TwAura").unwrap();
        let client = PAClient::from_env(name)?;
        let async_rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        Ok(Self {client, async_rt})
    }

    pub fn play_audio(&self) {
        println!("Playing");

        let sinks = self.async_rt.block_on(self.client.list_sinks()).expect("Can't get pa sinks.");
        let mut params_list = Vec::new();
        for sink in sinks {
            println!("Sink {}: {:?} ({:?})\n\t{:?}\n\t{:?}\n\t{:?}\n\tvol: {:?}", sink.index, sink.name, sink.state, sink.sample_spec, sink.props, sink.channel_map, sink.cvolume);
            let params = PlaybackStreamParams {
                sample_spec: sink.sample_spec,
                channel_map: sink.channel_map,
                props: sink.props,
                sink_name: Some(sink.name),
                ..Default::default()
            };
            params_list.push(params);
        }

        let file = File::open(home_dir().unwrap().join("Downloads/simple.wav")).unwrap();
            let mut wav_reader = hound::WavReader::new(file).unwrap();
            let spec = wav_reader.spec();

            // let format = match (spec.bits_per_sample, spec.sample_format) {
            //     (16, hound::SampleFormat::Int) => protocol::SampleFormat::S16Le,
            //     _ => {
            //         println!("unsupported sample format: {}bit {:?}", spec.bits_per_sample, spec.sample_format);
            //         return
            //     }
            // };

            // let channel_map = match spec.channels {
            //     1 => protocol::ChannelMap::mono(),
            //     2 => protocol::ChannelMap::stereo(),
            //     _ => {
            //         println!("unsupported channel count: {}", spec.channels);
            //         return
            //     }
            // };

            let last_sink_params = params_list.get(2).unwrap();

            let params = protocol::PlaybackStreamParams {
                sample_spec: protocol::SampleSpec {
                    format: protocol::SampleFormat::S16Le,
                    channels: last_sink_params.sample_spec.channels as u8,
                    sample_rate: spec.sample_rate,
                },
                channel_map: protocol::ChannelMap::stereo(),
                cvolume: Some(last_sink_params.cvolume.unwrap_or(protocol::ChannelVolume::norm(2))),
                sink_name: Some(CString::new("sink.deep_buffer").unwrap()),
                ..Default::default()
            };

            // Create a callback function, which is called by the client to write data
            // to the stream.
            let callback = move |data: &mut [u8]| copy_chunk(&mut wav_reader, data);
            let async_handle = self.async_rt.handle();
            let pa_client = self.client.clone();
            async_handle.spawn(async move {
                let stream = pa_client
                    .create_playback_stream(params, callback.as_playback_source())
                    .await
                    .expect("Failed to create playback stream");
                stream.play_all().await.expect("Failed to play stream");
            });

        // let last_sink_params = params_list.pop().unwrap();
        // let cb = move |buffer: &mut [u8]| {copy_chunk(buffer)};
        // self.client.create_playback_stream(last_sink_params, cb.as_playback_source()).await.unwrap();
        println!("End");
    }
}

fn copy_chunk<T: std::io::Read>(wav_reader: &mut hound::WavReader<T>, buf: &mut [u8]) -> usize {
    use byteorder::WriteBytesExt;
    let len = buf.len();
    assert!(len % 2 == 0);

    let mut cursor = std::io::Cursor::new(buf);
    for sample in wav_reader.samples::<i16>().filter_map(Result::ok) {
        if cursor.write_i16::<byteorder::LittleEndian>(sample).is_err() {
            break;
        }

        if cursor.position() == len as u64 {
            break;
        }
    }

    cursor.position() as usize
}
