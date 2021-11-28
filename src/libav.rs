// Muxing support, using libav / ffmpeg via the ac_ffmpeg crate. 



use anyhow::{Context, Result, anyhow};
use std::fs::File;
use ac_ffmpeg::codec::CodecParameters;
use ac_ffmpeg::format::io::IO;
use ac_ffmpeg::format::demuxer::Demuxer;
use ac_ffmpeg::format::demuxer::DemuxerWithStreamInfo;
use ac_ffmpeg::format::muxer::Muxer;
use ac_ffmpeg::format::muxer::OutputFormat;



// adapted from https://github.com/angelcam/rust-ac-ffmpeg/blob/master/examples/muxing.rs
fn libav_open_input(path: &str) -> Result<DemuxerWithStreamInfo<File>> {
    let input = File::open(path)?;
    let io = IO::from_seekable_read_stream(input);
    Demuxer::builder()
        .build(io)?
        .find_stream_info(None)
        .map_err(|(_, err)| anyhow!("Demuxer build error: {:?}", err))
}

// adapted from https://github.com/angelcam/rust-ac-ffmpeg/blob/master/examples/muxing.rs
fn libav_open_output(path: &str, elementary_streams: &[CodecParameters]) -> Result<Muxer<File>> {
    let output_format = OutputFormat::guess_from_file_name(path)
        .or_else(|| OutputFormat::find_by_name("mp4"))
        .expect("libav can't guess output format");
    let output = File::create(path)?;
    let io = IO::from_seekable_write_stream(output);
    let mut muxer_builder = Muxer::builder();
    for codec_parameters in elementary_streams {
        muxer_builder.add_stream(codec_parameters)?;
    }
    muxer_builder
        .interleaved(true)
        .build(io, output_format)
        .map_err(|e| anyhow!("Error building libav muxer: {:?}", e))
}

// http://gimite.net/en/index.php?Run%20native%20executable%20in%20Android%20App
//
// libavformat muxing example in C, https://github.com/FFmpeg/FFmpeg/blob/master/doc/examples/muxing.c
// and in Rust ac-ffmpeg https://github.com/angelcam/rust-ac-ffmpeg/issues/25
pub fn mux_audio_video_libav(audio_path: &str, video_path: &str, output_path: &str) -> Result<()> {
    let mut video_demuxer = libav_open_input(video_path)?;
    let (video_pos, video_codec) = video_demuxer
        .streams()
        .iter()
        .enumerate()
        .find_map(|(pos, stream)| {
            let params = stream.codec_parameters();
            if params.is_video_codec() {
                return Some((pos, params));
            }
            None
        })
        .context("libav video codec not found")?;
    let mut audio_demuxer = libav_open_input(audio_path)?;
    let (audio_pos, audio_codec) = audio_demuxer
        .streams()
        .iter()
        .enumerate()
        .find_map(|(pos, stream)| {
            let params = stream.codec_parameters();
            if params.is_audio_codec() {
                return Some((pos, params));
            }
            None
        })
        .context("libav audio codec not found")?;

    let mut muxer = libav_open_output(output_path, &[video_codec, audio_codec])?;
    // wonder about memory consumption here, should we interleave the pushes?
    while let Some(packet) = video_demuxer.take()? {
        if packet.stream_index() == video_pos {
            muxer.push(packet.with_stream_index(0))?;
        }
    }
    while let Some(packet) = audio_demuxer.take()? {
        if packet.stream_index() == audio_pos {
            muxer.push(packet.with_stream_index(1))?;
        }
    }
    muxer.flush()?;
    muxer.close()?;
    Ok(())
}

