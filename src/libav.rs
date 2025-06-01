// libav.rs
//
// Muxing support, using libav (ffmpeg as a shared library) via the ac_ffmpeg crate. This support
// is only compiled in if the "libav" feature is enabled, which is not the default (default is to
// use ffmpeg as a commandline application, see file "ffmpeg.rs").
//
// This use of libav via the library API is a little fiddly, because the ffmpeg commandline
// application implements a number of checks and workarounds to fix invalid input streams that you
// tend to encounter in the wild. We have implemented some of these workarounds here, but not all
// those implemented in the ffmpeg commandline application.
//
// Our code is adapted from the muxing example in the ac_ffmpeg crate
//
//    https://github.com/angelcam/rust-ac-ffmpeg/blob/master/examples/muxing.rs
//
// and muxing examples in ffmpeg/libav in C
//
//    https://github.com/FFmpeg/FFmpeg/blob/master/doc/examples/muxing.c


use std::io;
use std::cmp::{min, max};
use fs_err as fs;
use fs::File;
use std::path::Path;
use std::io::{BufReader, BufWriter};
use tracing::{error, info, trace};
use ac_ffmpeg::codec::CodecParameters;
use ac_ffmpeg::packet::Packet;
use ac_ffmpeg::time::Timestamp;
use ac_ffmpeg::format::io::IO;
use ac_ffmpeg::format::demuxer::Demuxer;
use ac_ffmpeg::format::demuxer::DemuxerWithStreamInfo;
use ac_ffmpeg::format::muxer::Muxer;
use ac_ffmpeg::format::muxer::OutputFormat;
use crate::DashMpdError;
use crate::fetch::DashDownloader;
use crate::media::{audio_container_type, video_container_type, AudioTrack};



fn libav_open_input(path: &str) -> Result<DemuxerWithStreamInfo<File>, DashMpdError> {
    let input = File::open(path)
        .map_err(|_| DashMpdError::Muxing(String::from("opening libav input path")))?;
    let io = IO::from_seekable_read_stream(input);
    Demuxer::builder()
        .build(io)
        .map_err(|_| DashMpdError::Muxing(String::from("building libav demuxer")))?
        .find_stream_info(Some(std::time::Duration::new(2, 0)))
        .map_err(|(_, _e)| DashMpdError::Muxing(String::from("building libav demuxer")))
}

fn libav_open_output(path: &str, elementary_streams: &[CodecParameters]) -> Result<Muxer<File>, DashMpdError> {
    let output_format = OutputFormat::guess_from_file_name(path)
        .or_else(|| OutputFormat::find_by_name("mp4"))
        .ok_or_else(|| DashMpdError::Muxing(String::from("guessing libav output format")))?;
    let output = File::create(path)
        .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
    let io = IO::from_seekable_write_stream(output);
    let mut muxer_builder = Muxer::builder();
    for codec_parameters in elementary_streams {
        muxer_builder.add_stream(codec_parameters)
            .map_err(|_| DashMpdError::Muxing(String::from("adding libav stream to muxer")))?;
    }
    muxer_builder
        // .interleaved(true)
        .build(io, output_format)
        .map_err(|e| DashMpdError::Muxing(
            format!("building libav muxer: {:?}", e)))
}


// The dts is always valid when the last_dts was null.
// The dts is invalid if it's non-monotonic.
fn has_invalid_timestamps(p: &Packet, last_dts: Timestamp) -> bool {
    !last_dts.is_null() && (p.dts().is_null() || p.dts() <= last_dts)
}


pub fn mux_audio_video(
    _downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    ac_ffmpeg::set_log_callback(|_count, msg: &str| info!("ffmpeg: {msg}"));
    if audio_tracks.len() > 1 {
        error!("Cannot mux more than a single audio track with libav");
        return Err(DashMpdError::Muxing(String::from("cannot mux more than one audio track with libav")));
    }
    let audio_str = &audio_tracks[0].path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining videopath name"),
            String::from("")))?;
    let mut video_demuxer = libav_open_input(video_str)
        .map_err(|_| DashMpdError::Muxing(String::from("opening input video stream")))?;
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
        .ok_or_else(|| DashMpdError::Muxing(String::from("finding libav video codec")))?;
    let mut audio_demuxer = libav_open_input(audio_str)?;
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
        .ok_or_else(|| DashMpdError::Muxing(String::from("finding libav audio codec")))?;

    let out = output_path.to_str()
        .ok_or_else(|| DashMpdError::Muxing(String::from("converting output path")))?;
    let mut muxer = libav_open_output(out, &[video_codec, audio_codec])?;
    let mut last_dts: Timestamp = Timestamp::null();

    // wonder about memory consumption here, should we interleave the pushes?
    while let Some(mut pkt) = video_demuxer.take()
        .map_err(|_| DashMpdError::Muxing(String::from("fetching video packet from libav demuxer")))? {
        if pkt.stream_index() == video_pos {
            // We try to work around malformed media streams with fluctuating dts (decompression timestamp).
            // The dts must be strictly increasing according to av_write_frame(), but some streams (eg
            // Vevo) have buggy inputs. There is special handling in ffmpeg.c around line 814 to avoid
            // the error "Application provided invalid, non monotonically increasing dts to muxer", and
            // also in videostreamer (see patch
            // https://github.com/horgh/videostreamer/commit/c2aa2d30b59332a7257e4c0c57a09bc8a0358b96 )
            // We rewrite the dts in this situation.
            if has_invalid_timestamps(&pkt, last_dts) {
                let next_dts = Timestamp::new(last_dts.timestamp() + 1, last_dts.time_base());
                if !pkt.pts().is_null() && pkt.pts() > pkt.dts() {
                    // we can't use std::cmp::max because only partial order available on Timestamp
                    let mut max = next_dts;
                    if pkt.pts() > max {
                        max = pkt.pts();
                    }
                    pkt = pkt.with_pts(max);
                }
                if pkt.pts().is_null() {
                    pkt = pkt.with_pts(next_dts);
                }
                pkt = pkt.with_dts(next_dts);
            }
            // Here a workaround for invalid media streams where dts is sometimes larger than pts.
            // Reproducing workaround from this patch to ffmpeg.c
            // http://git.videolan.org/?p=ffmpeg.git;a=commitdiff;h=22844132069ebd2c0b2ac4e7b41c93c33890bfb9
            if !pkt.pts().is_null() && !pkt.dts().is_null() && pkt.dts() > pkt.pts() {
                info!("Fixing invalid DTS (dts > pts) in DASH video stream");
                let pts_ts = pkt.pts().timestamp();
                let dts_ts = pkt.dts().timestamp();
                let next_ts = last_dts.timestamp() + 1;
                let fixed_dts_ts = pts_ts + dts_ts + next_ts
                    - min(pts_ts, min(dts_ts, next_ts))
                    - max(pts_ts, max(dts_ts, next_ts));
                let fixed_dts = Timestamp::new(fixed_dts_ts, last_dts.time_base());
                pkt = pkt.with_dts(fixed_dts).with_pts(fixed_dts);
            }
            last_dts = pkt.dts();
            muxer.push(pkt.with_stream_index(0))
                .map_err(|_| DashMpdError::Muxing(String::from("pushing video packet to libav muxer")))?;
        }
    }
    muxer.flush()
        .map_err(|_| DashMpdError::Muxing(String::from("flushing libav muxer")))?;
    last_dts = Timestamp::null();
    while let Some(mut pkt) = audio_demuxer.take()
        .map_err(|_| DashMpdError::Muxing(String::from("fetching audio packet from libav demuxer")))? {
        if pkt.stream_index() == audio_pos {
            // See comments concerning workarounds for invalid media streams in the code for the
            // video stream, above.
            if has_invalid_timestamps(&pkt, last_dts) {
                let next_dts = Timestamp::new(last_dts.timestamp() + 1, last_dts.time_base());
                if !pkt.pts().is_null() && (pkt.pts() > pkt.dts()) {
                    let mut max = next_dts;
                    if pkt.pts() > max {
                        max = pkt.pts();
                    }
                    pkt = pkt.with_pts(max);
                }
                if pkt.pts().is_null() {
                    pkt = pkt.with_pts(next_dts);
                }
                pkt = pkt.with_dts(next_dts);
            }
            if !pkt.pts().is_null() && !pkt.dts().is_null() && pkt.dts() > pkt.pts() {
                info!("Fixing invalid DTS (dts > pts) in DASH audio stream");
                let pts_ts = pkt.pts().timestamp();
                let dts_ts = pkt.dts().timestamp();
                let next_ts = last_dts.timestamp() + 1;
                let fixed_dts_ts = pts_ts + dts_ts + next_ts
                    - min(pts_ts, min(dts_ts, next_ts))
                    - max(pts_ts, max(dts_ts, next_ts));
                let fixed_dts = Timestamp::new(fixed_dts_ts, last_dts.time_base());
                pkt = pkt.with_dts(fixed_dts).with_pts(fixed_dts);
            }
            last_dts = pkt.dts();
            muxer.push(pkt.with_stream_index(1))
                .map_err(|_| DashMpdError::Muxing(String::from("pushing audio packet to libav muxer")))?;
        }
    }
    muxer.flush()
        .map_err(|_| DashMpdError::Muxing(String::from("flushing libav muxer")))?;
    muxer.close()
        .map_err(|_| DashMpdError::Muxing(String::from("closing libav muxer")))?;
    Ok(())
}


pub fn copy_video_to_container(
    _downloader: &DashDownloader,
    output_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError>
{
    trace!("Copying video {} to output container {}", video_path.display(), output_path.display());
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // If the video stream is already in the desired container format, we can just copy it to the
    // output file.
    if video_container_type(video_path)?.eq(container) {
        let tmpfile_video = File::open(video_path)
            .map_err(|e| DashMpdError::Io(e, String::from("opening temporary video output file")))?;
        let mut video = BufReader::new(tmpfile_video);
        let output_file = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("creating output file for video")))?;
        let mut sink = BufWriter::new(output_file);
        io::copy(&mut video, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying video stream to output file")))?;
        return Ok(());
    }
    todo!()
}


pub fn copy_audio_to_container(
    _downloader: &DashDownloader,
    output_path: &Path,
    audio_path: &Path) -> Result<(), DashMpdError>
{
    trace!("Copying audio {} to output container {}", audio_path.display(), output_path.display());
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // If the audio stream is already in the desired container format, we can just copy it to the
    // output file.
    if audio_container_type(audio_path)?.eq(container) {
        let tmpfile_audio = File::open(audio_path)
            .map_err(|e| DashMpdError::Io(e, String::from("opening temporary audio output file")))?;
        let mut audio = BufReader::new(tmpfile_audio);
        let output_file = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("creating output file for audio")))?;
        let mut sink = BufWriter::new(output_file);
        io::copy(&mut audio, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying audio stream to output file")))?;
        return Ok(());
    }
    todo!()
}
