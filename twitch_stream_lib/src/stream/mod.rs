mod transcoder;
pub mod player;

use anyhow::Result;
use ffmpeg_next::format::context;
use ffmpeg_next as ffmpeg;
use ffmpeg::format::{input, output};
use ffmpeg::{format, encoder, codec, media, Rational};

use transcoder::Transcoder;
use player::FrameQueue;


pub fn start_streaming(stream_uri: &str, resolution: (u64, u64), frames_queue: FrameQueue) {
    ffmpeg::init().unwrap();
    let mut input_ctx = input(stream_uri).unwrap();
    print_stream_metadata(&input_ctx);
    save_video_stream_to_mp4(&mut input_ctx, "captured.mp4", resolution, frames_queue.clone()).unwrap();

    let mut input_ctx = input(stream_uri).unwrap();
    print_stream_metadata(&input_ctx);
    save_video_stream_to_mp4(&mut input_ctx, "captured2.mp4", resolution, frames_queue).unwrap();
}


fn save_video_stream_to_mp4(
    input_ctx: &mut format::context::Input,
    output_path: &str,
    stream_res: (u64, u64),
    mut frames_queue: FrameQueue,
) -> Result<()> {
    // Открываем входной поток
    // let mut input_ctx = input(input_url)?;
    let mut out_ctx = output(output_path)?;

    let mut stream_mapping: Vec<isize> = vec![0; input_ctx.nb_streams() as _];
    let mut ist_time_bases = vec![Rational(0, 0); input_ctx.nb_streams() as _];
    let mut ost_time_bases = vec![Rational(0, 0); input_ctx.nb_streams() as _];
    let mut transcoders = std::collections::HashMap::new();
    let mut ost_index = 0;

    for (ist_index, ist) in input_ctx.streams().enumerate() {
        let ist_medium = ist.parameters().medium();
        if ist_medium != media::Type::Audio
            && ist_medium != media::Type::Video
            && ist_medium != media::Type::Subtitle
        {
            stream_mapping[ist_index] = -1;
            continue;
        }
        stream_mapping[ist_index] = ost_index;
        ist_time_bases[ist_index] = ist.time_base();
        if ist_medium == media::Type::Video {
            // Initialize transcoder for video stream.
            transcoders.insert(
                ist_index,
                Transcoder::new(
                    &ist,
                    &mut out_ctx,
                    ost_index as _,
                    stream_res,
                )
                .unwrap(),
            );
        } else {
            // Set up for stream copy for non-video stream.
            let mut ost = out_ctx.add_stream(encoder::find(codec::Id::None)).unwrap();
            ost.set_parameters(ist.parameters());
            // We need to set codec_tag to 0 lest we run into incompatible codec tag
            // issues when muxing into a different container format. Unfortunately
            // there's no high level API to do this (yet).
            unsafe {
                (*ost.parameters().as_mut_ptr()).codec_tag = 0;
            }
        }
        ost_index += 1;
    }

    out_ctx.set_metadata(input_ctx.metadata().to_owned());
    format::context::output::dump(&out_ctx, 0, Some(output_path));
    out_ctx.write_header().unwrap();

    for (ost_index, _) in out_ctx.streams().enumerate() {
        ost_time_bases[ost_index] = out_ctx.stream(ost_index as _).unwrap().time_base();
    }

    let start_time = std::time::Instant::now();
    let mut stream_id = -1;
    for (stream, mut packet) in input_ctx.packets() {
        let ist_index = stream.index();
        let ost_index = stream_mapping[ist_index];
        if ost_index < 0 {
            continue;
        }
        let ost_time_base = ost_time_bases[ost_index as usize];
        match transcoders.get_mut(&ist_index) {
            Some(transcoder) => {
                if stream_id != stream.id() {
                    println!("New video stream id: {}", stream.id());
                    stream_id = stream.id();
                }

                if stream.parameters().medium() != media::Type::Video {
                    println!("THRER IS NON-VIDEO STREAM WITH TRANSCODER");
                    break
                }
                if transcoder.send_packet_to_decoder(&packet).is_ok() {
                    transcoder.receive_and_process_decoded_frames(&mut out_ctx, ost_time_base, &mut frames_queue);
                }
            }
            None => {
                if stream.parameters().medium() == media::Type::Video {
                    println!("THRER IS VIDEO STREAM WITHOUT TRANSCODER");
                    break
                }

                // Do stream copy on non-video streams.
                packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
                packet.set_position(-1);
                packet.set_stream(ost_index as _);
                packet.write_interleaved(&mut out_ctx).unwrap();
            }
        }
        if std::time::Instant::now().duration_since(start_time).as_secs() >= 20 {
            println!("BREAK");
            break
        }
    }

    println!("Time elapsed: {}", std::time::Instant::now().duration_since(start_time).as_secs());

    // Flush encoders and decoders.
    for (ost_index, transcoder) in transcoders.iter_mut() {
        let ost_time_base = ost_time_bases[*ost_index];
        transcoder.send_eof_to_decoder();
        transcoder.receive_and_process_decoded_frames(&mut out_ctx, ost_time_base, &mut frames_queue);
        transcoder.send_eof_to_encoder();
        transcoder.receive_and_process_encoded_packets(&mut out_ctx, ost_time_base);
    }

    out_ctx.write_trailer().unwrap();

    Ok(())
}


fn print_stream_metadata(istream: &context::Input) {
    println!("Nb chapters: {}", istream.nb_chapters());
    println!("Duration: {}", istream.duration());
    println!("Bit rate: {}", istream.bit_rate());
    println!("Format: {} ({})", istream.format().name(), istream.format().description());
    println!("Probe score: {}", istream.probe_score());

    for chapter in istream.chapters() {
        println!("chapter id {}:", chapter.id());
        println!("\ttime_base: {}", chapter.time_base());
        println!("\tstart: {}", chapter.start());
        println!("\tend: {}", chapter.end());

        for (k, v) in chapter.metadata().iter() {
            println!("\t{}: {}", k, v);
        }
    }

    for stream in istream.streams() {
        println!("stream index {}:", stream.index());
        println!("\ttime_base: {}", stream.time_base());
        println!("\tstart_time: {}", stream.start_time());
        println!("\tduration (stream timebase): {}", stream.duration());
        println!(
            "\tduration (seconds): {:.2}",
            stream.duration() as f64 * f64::from(stream.time_base())
        );
        println!("\tframes: {}", stream.frames());
        println!("\tdisposition: {:?}", stream.disposition());
        println!("\tdiscard: {:?}", stream.discard());
        println!("\trate: {}", stream.rate());

        let codec = ffmpeg::codec::context::Context::from_parameters(stream.parameters()).unwrap();
        println!("\tmedium: {:?}", codec.medium());
        println!("\tid: {:?}", codec.id());

        if codec.medium() == ffmpeg::media::Type::Video {
            if let Ok(video) = codec.decoder().video() {
                println!("\tbit_rate: {}", video.bit_rate());
                println!("\tmax_rate: {}", video.max_bit_rate());
                println!("\tdelay: {}", video.delay());
                println!("\tvideo.width: {}", video.width());
                println!("\tvideo.height: {}", video.height());
                println!("\tvideo.format: {:?}", video.format());
                println!("\tvideo.has_b_frames: {}", video.has_b_frames());
                println!("\tvideo.aspect_ratio: {}", video.aspect_ratio());
                println!("\tvideo.color_space: {:?}", video.color_space());
                println!("\tvideo.color_range: {:?}", video.color_range());
                println!("\tvideo.color_primaries: {:?}", video.color_primaries());
                println!(
                    "\tvideo.color_transfer_characteristic: {:?}",
                    video.color_transfer_characteristic()
                );
                println!("\tvideo.chroma_location: {:?}", video.chroma_location());
                println!("\tvideo.references: {}", video.references());
                println!("\tvideo.intra_dc_precision: {}", video.intra_dc_precision());
            }
        } else if codec.medium() == ffmpeg::media::Type::Audio {
            if let Ok(audio) = codec.decoder().audio() {
                println!("\tbit_rate: {}", audio.bit_rate());
                println!("\tmax_rate: {}", audio.max_bit_rate());
                println!("\tdelay: {}", audio.delay());
                println!("\taudio.rate: {}", audio.rate());
                println!("\taudio.channels: {}", audio.channels());
                println!("\taudio.format: {:?}", audio.format());
                println!("\taudio.frames: {}", audio.frames());
                println!("\taudio.align: {}", audio.align());
                println!("\taudio.channel_layout: {:?}", audio.channel_layout());
            }
        }
    }
}
