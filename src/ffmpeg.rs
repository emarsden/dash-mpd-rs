//! Muxing support using mkvmerge/ffmpeg/vlc/mp4box as a subprocess.
//
// Also see the alternative method of using ffmpeg via its "libav" shared library API, implemented
// in file "libav.rs".

// TODO: on Linux we should try to use bubblewrap to execute the muxers in a sandboxed environment,
// along the lines of
//
//  bwrap --ro-bind /usr /usr --ro-bind /lib /lib --ro-bind /lib64 /lib64 --ro-bind /etc /etc --dev /dev --tmpfs /tmp --bind ~/Vidéos/foo.mkv /tmp/video.mkv -- /usr/bin/ffprobe /tmp/video.mkv


use std::env;
use std::io;
use std::io::{Write, BufReader, BufWriter};
use std::path::Path;
use std::process::Command;
use fs_err as fs;
use fs::File;
use ffprobe::ffprobe;
use tracing::{trace, info, warn, error};
use crate::DashMpdError;
use crate::fetch::{DashDownloader, partial_process_output};
use crate::media::{
    audio_container_type,
    video_container_type,
    container_has_video,
    container_has_audio,
    temporary_outpath,
    AudioTrack,
};

fn ffprobe_start_time(input: &Path) -> Result<f64, DashMpdError> {
    match ffprobe(input) {
        Ok(info) => if let Some(st) = info.format.start_time {
            Ok(st.parse::<f64>()
                .map_err(|_| DashMpdError::Io(
                    io::Error::other("reading start_time"),
                    String::from("")))?)
        } else {
            Ok(0.0)
        },
        Err(e) => {
            warn!("Error probing metadata on {}: {e:?}", input.display());
            Ok(0.0)
        },
    }
}

// Mux one video track with multiple audio tracks
#[tracing::instrument(level="trace", skip(downloader))]
pub fn mux_multiaudio_video_ffmpeg(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    if audio_tracks.is_empty() {
        return Err(DashMpdError::Muxing(String::from("no audio tracks")));
    }
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // See output from "ffmpeg -muxers"
    let muxer = match container {
        "mkv" => "matroska",
        "ts" => "mpegts",
        _ => container,
    };
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .disable_cleanup(env::var("DASHMPD_PERSIST_FILES").is_ok())
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining videopath name"),
            String::from("")))?;
    if downloader.verbosity > 0 {
        info!("  Muxing audio ({} track{}) and video content with ffmpeg",
              audio_tracks.len(),
              if audio_tracks.len() == 1 { "" } else { "s" });
        if let Ok(attr) = fs::metadata(video_path) {
            info!("  Video file {} of size {} octets", video_path.display(), attr.len());
        }
    }
    let mut args = vec![
        String::from("-hide_banner"),
        String::from("-nostats"),
        String::from("-loglevel"), String::from("error"),  // or "warning", "info"
        String::from("-y"),  // overwrite output file if it exists
        String::from("-nostdin")];
    let mut mappings = Vec::new();
    mappings.push(String::from("-map"));
    mappings.push(String::from("0:v"));
    args.push(String::from("-i"));
    args.push(String::from(video_str));
    // https://superuser.com/questions/1078298/ffmpeg-combine-multiple-audio-files-and-one-video-in-to-the-multi-language-vid
    for (i, at) in audio_tracks.iter().enumerate() {
        // note that the -map commandline argument counts from 1, whereas the -metadata argument counts from 0
        mappings.push(String::from("-map"));
        mappings.push(format!("{}:a", i+1));
        mappings.push(format!("-metadata:s:a:{i}"));
        let mut lang_sanitized = at.language.clone();
        lang_sanitized.retain(|c: char| c.is_ascii_lowercase());
        mappings.push(format!("language={lang_sanitized}"));
        args.push(String::from("-i"));
        let audio_str = at.path
            .to_str()
            .ok_or_else(|| DashMpdError::Io(
                io::Error::other("obtaining audiopath name"),
                String::from("")))?;
        args.push(String::from(audio_str));
    }
    for m in mappings {
        args.push(m);
    }
    args.push(String::from("-c:v"));
    args.push(String::from("copy"));
    args.push(String::from("-c:a"));
    args.push(String::from("copy"));
    args.push(String::from("-movflags"));
    args.push(String::from("faststart"));
    args.push(String::from("-preset"));
    args.push(String::from("veryfast"));
    // select the muxer explicitly (debateable whether this is better than ffmpeg's
    // heuristics based on output filename)
    args.push(String::from("-f"));
    args.push(String::from(muxer));
    args.push(String::from(tmppath));
    if downloader.verbosity > 0 {
        info!("  Running ffmpeg {}", args.join(" "));
    }
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if !msg.is_empty() {
        info!("  ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if !msg.is_empty() {
        info!("  ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        // local scope so that tmppath is not busy on Windows and can be deleted
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening ffmpeg output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying ffmpeg output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
	    if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary ffmpeg output: {e}");
            }
        }
        return Ok(());
    }
    // TODO: try again without -c:a copy and -c:v copy
    Err(DashMpdError::Muxing(String::from("running ffmpeg")))
}

// ffmpeg can mux to many container types including mp4, mkv, avi
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_video_ffmpeg(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // See output from "ffmpeg -muxers"
    let muxer = match container {
        "mkv" => "matroska",
        "ts" => "mpegts",
        _ => container,
    };
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .disable_cleanup(env::var("DASHMPD_PERSIST_FILES").is_ok())
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining videopath name"),
            String::from("")))?;
    if downloader.verbosity > 0 {
        info!("  Muxing audio ({} track{}) and video content with ffmpeg",
              audio_tracks.len(),
              if audio_tracks.len() == 1 { "" } else { "s" });
        if let Ok(attr) = fs::metadata(video_path) {
            info!("  Video file {} of size {} octets", video_path.display(), attr.len());
        }
    }
    let mut audio_delay = 0.0;
    let mut video_delay = 0.0;
    if let Ok(audio_start_time) = ffprobe_start_time(&audio_tracks[0].path) {
        if let Ok(video_start_time) = ffprobe_start_time(video_path) {
            if audio_start_time > video_start_time {
                video_delay = audio_start_time - video_start_time;
            } else {
                audio_delay = video_start_time - audio_start_time;
            }
        }
    }
    let mut args = vec![
        String::from("-hide_banner"),
        String::from("-nostats"),
        String::from("-loglevel"), String::from("error"),  // or "warning", "info"
        String::from("-y"),  // overwrite output file if it exists
        String::from("-nostdin")];
    let mut mappings = Vec::new();
    mappings.push(String::from("-map"));
    mappings.push(String::from("0:v"));
    let vd = format!("{video_delay}");
    if video_delay > 0.001 {
        // "-itsoffset", &format!("{}", video_delay),
        args.push(String::from("-ss"));
        args.push(vd);
    }
    args.push(String::from("-i"));
    args.push(String::from(video_str));
    let ad = format!("{audio_delay}");
    if audio_delay > 0.001 {
        // "-itsoffset", &format!("{audio_delay}"),
        args.push(String::from("-ss"));
        args.push(ad);
    }
    // https://superuser.com/questions/1078298/ffmpeg-combine-multiple-audio-files-and-one-video-in-to-the-multi-language-vid
    for (i, at) in audio_tracks.iter().enumerate() {
        // Note that the -map commandline argument counts from 1, whereas the -metadata argument
        // counts from 0.
        mappings.push(String::from("-map"));
        mappings.push(format!("{}:a", i+1));
        mappings.push(format!("-metadata:s:a:{i}"));
        let mut lang_sanitized = at.language.clone();
        lang_sanitized.retain(|c: char| c.is_ascii_lowercase());
        mappings.push(format!("language={lang_sanitized}"));
        args.push(String::from("-i"));
        let audio_str = at.path
            .to_str()
            .ok_or_else(|| DashMpdError::Io(
                io::Error::other("obtaining audiopath name"),
                String::from("")))?;
        args.push(String::from(audio_str));
    }
    for m in mappings {
        args.push(m);
    }
    args.push(String::from("-c:v"));
    args.push(String::from("copy"));
    args.push(String::from("-c:a"));
    args.push(String::from("copy"));
    args.push(String::from("-movflags"));
    args.push(String::from("faststart"));
    args.push(String::from("-preset"));
    args.push(String::from("veryfast"));
    // select the muxer explicitly (debateable whether this is better than ffmpeg's
    // heuristics based on output filename)
    args.push(String::from("-f"));
    args.push(String::from(muxer));
    args.push(String::from(tmppath));
    if downloader.verbosity > 0 {
        info!("  Running ffmpeg {}", args.join(" "));
    }
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args.clone())
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if !msg.is_empty() {
        info!("  ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if !msg.is_empty() {
        info!("  ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        // local scope so that tmppath is not busy on Windows and can be deleted
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening ffmpeg output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying ffmpeg output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
	    if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary ffmpeg output: {e}");
            }
        }
        return Ok(());
    }
    // The muxing may have failed only due to the "-c:v copy -c:a copy" argument to ffmpeg, which
    // instructs it to copy the audio and video streams without any reencoding. That is not possible
    // for certain output containers: for instance a WebM container must contain video using VP8,
    // VP9 or AV1 codecs and Vorbis or Opus audio codecs. (Unfortunately, ffmpeg doesn't seem to
    // return a distinct recognizable error message in this specific case.) So we try invoking
    // ffmpeg again, this time allowing reencoding.
    args.retain(|a| !(a.eq("-c:v") || a.eq("copy") || a.eq("-c:a")));
    if downloader.verbosity > 0 {
        info!("  Running ffmpeg {}", args.join(" "));
    }
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if !msg.is_empty() {
        info!("  ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if !msg.is_empty() {
        info!("  ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        // local scope so that tmppath is not busy on Windows and can be deleted
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening ffmpeg output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying ffmpeg output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
	    if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary ffmpeg output: {e}");
            }
        }
        Ok(())
    } else {
        Err(DashMpdError::Muxing(String::from("running ffmpeg")))
    }
}


// See "ffmpeg -formats"
fn ffmpeg_container_name(extension: &str) -> Option<String> {
    match extension {
        "mkv" => Some(String::from("matroska")),
        "webm" => Some(String::from("webm")),
        "avi" => Some(String::from("avi")),
        "mov" => Some(String::from("mov")),
        "mp4" => Some(String::from("mp4")),
        "ts" => Some(String::from("mpegts")),
        "ogg" => Some(String::from("ogg")),
        "vob" => Some(String::from("vob")),
        _ => None,
    }
}

// This can be used to package either an audio stream or a video stream into the container format
// that is determined by the extension of output_path.
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_stream_ffmpeg(
    downloader: &DashDownloader,
    output_path: &Path,
    input_path: &Path) -> Result<(), DashMpdError> {
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    info!("  ffmpeg inserting stream into {container} container named {}", output_path.display());
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .disable_cleanup(env::var("DASHMPD_PERSIST_FILES").is_ok())
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let input = input_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining input name"),
            String::from("")))?;
    let cn: String;
    let mut args = vec!("-hide_banner",
                        "-nostats",
                        "-loglevel", "error",  // or "warning", "info"
                        "-y",  // overwrite output file if it exists
                        "-nostdin",
                        "-i", input,
                        "-movflags", "faststart", "-preset", "veryfast");
    // We can select the muxer explicitly (otherwise it is determined using heuristics based on the
    // filename extension).
    if let Some(container_name) = ffmpeg_container_name(container) {
        args.push("-f");
        cn = container_name;
        args.push(&cn);
    }
    args.push(tmppath);
    if downloader.verbosity > 0 {
        info!("  Running ffmpeg {}", args.join(" "));
    }
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        // local scope so that tmppath is not busy on Windows and can be deleted
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening ffmpeg output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying ffmpeg output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
	    if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary ffmpeg output: {e}");
            }
        }
        Ok(())
    } else {
        warn!("  unmuxed stream: {input}");
        Err(DashMpdError::Muxing(String::from("running ffmpeg")))
    }
}


// See https://wiki.videolan.org/Transcode/
// VLC could also mux to an mkv container if needed
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_video_vlc(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    if audio_tracks.len() > 1 {
        error!("Cannot mux more than a single audio track with VLC");
        return Err(DashMpdError::Muxing(String::from("cannot mux more than one audio track with VLC")));
    }
    let audio_path = &audio_tracks[0].path;
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    let muxer = match container {
        "ogg" => "ogg",
        "webm" => "mkv",
        "mp3" => "raw",
        "mpg" => "mpeg1",
        _ => container,
    };
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(".mp4")
        .rand_bytes(5)
        .disable_cleanup(env::var("DASHMPD_PERSIST_FILES").is_ok())
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining videopath name"),
            String::from("")))?;
    let transcode = if container.eq("webm") {
        "transcode{vcodec=VP90,acodec=vorb}:"
    } else {
        ""
    };
    let sout = format!("--sout=#{transcode}std{{access=file,mux={muxer},dst={tmppath}}}");
    let args = vec![
        "-I", "dummy",
        "--no-repeat", "--no-loop",
        video_str,
        "--input-slave", audio_str,
        "--sout-mp4-faststart",
        &sout,
        "--sout-keep",
        "vlc://quit"];
    if downloader.verbosity > 0 {
        info!("  Running vlc {}", args.join(" "));
    }
    let vlc = Command::new(&downloader.vlc_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning VLC subprocess")))?;
    // VLC is erroneously returning a 0 (success) return code even when it fails to mux, so we need
    // to look for a specific error message to check for failure.
    let msg = partial_process_output(&vlc.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  vlc stderr: {msg}");
    }
    if vlc.status.success() && (!msg.contains("mp4 mux error")) {
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening VLC output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying VLC output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
            if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary VLC output: {e}");
            }
        }
        Ok(())
    } else {
        let msg = partial_process_output(&vlc.stderr);
        Err(DashMpdError::Muxing(format!("running VLC: {msg}")))
    }
}


// MP4Box from the GPAC suite for muxing audio and video streams
// https://github.com/gpac/gpac/wiki/MP4Box
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_video_mp4box(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    if audio_tracks.len() > 1 {
        error!("Cannot mux more than a single audio track with MP4Box");
        return Err(DashMpdError::Muxing(String::from("cannot mux more than one audio track with MP4Box")));
    }
    let audio_path = &audio_tracks[0].path;
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .disable_cleanup(env::var("DASHMPD_PERSIST_FILES").is_ok())
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining videopath name"),
            String::from("")))?;
    let args = vec![
        "-flat",
        "-add", video_str,
        "-add", audio_str,
        "-new", tmppath];
    if downloader.verbosity > 0 {
        info!("  Running MP4Box {}", args.join(" "));
    }
    let cmd = Command::new(&downloader.mp4box_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
    let msg = partial_process_output(&cmd.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  MP4Box stderr: {msg}");
    }
    if cmd.status.success() {
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening MP4Box output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying MP4Box output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
	    if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary MP4Box output: {e}");
            }
        }
        Ok(())
    } else {
        let msg = partial_process_output(&cmd.stderr);
        Err(DashMpdError::Muxing(format!("running MP4Box: {msg}")))
    }
}

// This can be used to package either an audio stream or a video stream into the container format
// that is determined by the extension of output_path.
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_stream_mp4box(
    downloader: &DashDownloader,
    output_path: &Path,
    input_path: &Path) -> Result<(), DashMpdError> {
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let input = input_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining input stream name"),
            String::from("")))?;
    let args = vec!["-add", input, "-new", tmppath];
    if downloader.verbosity > 0 {
        info!("  Running MP4Box {}", args.join(" "));
    }
    let cmd = Command::new(&downloader.mp4box_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
    let msg = partial_process_output(&cmd.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  MP4box stderr: {msg}");
    }
    if cmd.status.success() {
        {
            let tmpfile = File::open(tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening MP4Box output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying MP4Box output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
	    if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary MP4Box output: {e}");
            }
        }
        Ok(())
    } else {
        let msg = partial_process_output(&cmd.stderr);
        warn!("  MP4Box mux_stream failure: stdout {}", partial_process_output(&cmd.stdout));
        warn!("  MP4Box stderr: {msg}");
        Err(DashMpdError::Muxing(format!("running MP4Box: {msg}")))
    }
}

#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_video_mkvmerge(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    if audio_tracks.len() > 1 {
        error!("Cannot mux more than a single audio track with mkvmerge");
        return Err(DashMpdError::Muxing(String::from("cannot mux more than one audio track with mkvmerge")));
    }
    let audio_path = &audio_tracks[0].path;
    let tmppath = temporary_outpath(".mkv")?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining videopath name"),
            String::from("")))?;
    let args = vec!["--output", &tmppath,
                    "--no-video", audio_str,
                    "--no-audio", video_str];
    if downloader.verbosity > 0 {
        info!("  Running mkvmerge {}", args.join(" "));
    }
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
    let msg = partial_process_output(&mkv.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  mkvmerge stderr: {msg}");
    }
    if mkv.status.success() {
        {
            let tmpfile = File::open(&tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening mkvmerge output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("opening output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying mkvmerge output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
            if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary mkvmerge output: {e}");
            }
        }
        Ok(())
    } else {
        // mkvmerge writes error messages to stdout, not to stderr
        let msg = String::from_utf8_lossy(&mkv.stdout);
        Err(DashMpdError::Muxing(format!("running mkvmerge: {msg}")))
    }
}

// Copy video stream at video_path into Matroska container at output_path.
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_video_mkvmerge(
    downloader: &DashDownloader,
    output_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
    let tmppath = temporary_outpath(".mkv")?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining videopath name"),
            String::from("")))?;
    let args = vec!["--output", &tmppath, "--no-audio", video_str];
    if downloader.verbosity > 0 {
        info!("  Running mkvmerge {}", args.join(" "));
    }
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
    let msg = partial_process_output(&mkv.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  mkvmerge stderr: {msg}");
    }
    if mkv.status.success() {
        {
            let tmpfile = File::open(&tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening mkvmerge output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("opening output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying mkvmerge output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
            if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary mkvmerge output: {e}");
            }
        }
        Ok(())
    } else {
        // mkvmerge writes error messages to stdout, not to stderr
        let msg = String::from_utf8_lossy(&mkv.stdout);
        Err(DashMpdError::Muxing(format!("running mkvmerge: {msg}")))
    }
}


// Copy audio stream at video_path into Matroska container at output_path.
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_mkvmerge(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_path: &Path) -> Result<(), DashMpdError> {
    let tmppath = temporary_outpath(".mkv")?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining audiopath name"),
            String::from("")))?;
    let args = vec!["--output", &tmppath, "--no-video", audio_str];
    if downloader.verbosity > 0 {
        info!("  Running mkvmerge {}", args.join(" "));
    }
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
    let msg = partial_process_output(&mkv.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  mkvmerge stderr: {msg}");
    }
    if mkv.status.success() {
        {
            let tmpfile = File::open(&tmppath)
                .map_err(|e| DashMpdError::Io(e, String::from("opening mkvmerge output")))?;
            let mut muxed = BufReader::new(tmpfile);
            let outfile = File::create(output_path)
                .map_err(|e| DashMpdError::Io(e, String::from("opening output file")))?;
            let mut sink = BufWriter::new(outfile);
            io::copy(&mut muxed, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying mkvmerge output to output file")))?;
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
            if let Err(e) = fs::remove_file(tmppath) {
                warn!("  Error deleting temporary mkvmerge output: {e}");
            }
        }
        Ok(())
    } else {
        // mkvmerge writes error messages to stdout, not to stderr
        let msg = String::from_utf8_lossy(&mkv.stdout);
        Err(DashMpdError::Muxing(format!("running mkvmerge: {msg}")))
    }
}


// Mux (merge) audio and video using an external tool, selecting the tool based on the output
// container format and on the user-specified muxer preference ordering (e.g. "ffmpeg,vlc,mp4box")
// or our hardcoded container-dependent preference ordering.
#[tracing::instrument(level="trace", skip(downloader))]
pub fn mux_audio_video(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_tracks: &Vec<AudioTrack>,
    video_path: &Path) -> Result<(), DashMpdError> {
    trace!("Muxing {} audio tracks with video {}", audio_tracks.len(), video_path.display());
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    let mut muxer_preference = vec![];
    if container.eq("mkv") {
        muxer_preference.push("mkvmerge");
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    } else if container.eq("webm") {
        // VLC is a better default than ffmpeg, because ffmpeg (with the options we supply) doesn't
        // automatically reencode the vidoe and audio streams when they are incompatible with the
        // container format requested, whereas VLC does do so.
        muxer_preference.push("vlc");
        muxer_preference.push("ffmpeg");
    } else if container.eq("mp4") {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("vlc");
        muxer_preference.push("mp4box");
    } else {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    }
    if let Some(ordering) = downloader.muxer_preference.get(container) {
        muxer_preference.clear();
        for m in ordering.split(',') {
            muxer_preference.push(m);
        }
    }
    info!("  Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        info!("  Trying muxer {muxer}");
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_audio_video_mkvmerge(downloader, output_path, audio_tracks, video_path) {
                warn!("  Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("  Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_audio_video_ffmpeg(downloader, output_path, audio_tracks, video_path) {
                warn!("  Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("  Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("vlc") {
            if let Err(e) = mux_audio_video_vlc(downloader, output_path, audio_tracks, video_path) {
                warn!("  Muxing with vlc subprocess failed: {e}");
            } else {
                info!("  Muxing with vlc subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_audio_video_mp4box(downloader, output_path, audio_tracks, video_path) {
                warn!("  Muxing with MP4Box subprocess failed: {e}");
            } else {
                info!("  Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        } else {
            warn!("  Ignoring unknown muxer preference {muxer}");
        }
    }
    warn!("All muxers failed");
    warn!("  unmuxed audio streams: {}", audio_tracks.len());
    warn!("  unmuxed video stream: {}", video_path.display());
    Err(DashMpdError::Muxing(String::from("all muxers failed")))
}


#[tracing::instrument(level="trace", skip(downloader))]
pub fn copy_video_to_container(
    downloader: &DashDownloader,
    output_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
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
    let mut muxer_preference = vec![];
    if container.eq("mkv") {
        muxer_preference.push("mkvmerge");
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    } else {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    }
    if let Some(ordering) = downloader.muxer_preference.get(container) {
        muxer_preference.clear();
        for m in ordering.split(',') {
            muxer_preference.push(m);
        }
    }
    info!("  Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        info!("  Trying muxer {muxer}");
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_video_mkvmerge(downloader, output_path, video_path) {
                warn!("  Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("  Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_stream_ffmpeg(downloader, output_path, video_path) {
                warn!("  Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("  Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_stream_mp4box(downloader, output_path, video_path) {
                warn!("  Muxing with MP4Box subprocess failed: {e}");
            } else {
                info!("  Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        }
    }
    warn!("  All available muxers failed");
    warn!("    unmuxed video stream: {}", video_path.display());
    Err(DashMpdError::Muxing(String::from("all available muxers failed")))
}


#[tracing::instrument(level="trace", skip(downloader))]
pub fn copy_audio_to_container(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_path: &Path) -> Result<(), DashMpdError> {
    trace!("Copying audio {} to output container {}", audio_path.display(), output_path.display());
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // If the audio stream is already in the desired container format, we can just copy it to the
    // output file.
    if audio_container_type(audio_path)?.eq(container) {
        let tmpfile_video = File::open(audio_path)
            .map_err(|e| DashMpdError::Io(e, String::from("opening temporary output file")))?;
        let mut video = BufReader::new(tmpfile_video);
        let output_file = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
        let mut sink = BufWriter::new(output_file);
        io::copy(&mut video, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying audio stream to output file")))?;
        return Ok(());
    }
    let mut muxer_preference = vec![];
    if container.eq("mkv") {
        muxer_preference.push("mkvmerge");
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    } else {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    }
    if let Some(ordering) = downloader.muxer_preference.get(container) {
        muxer_preference.clear();
        for m in ordering.split(',') {
            muxer_preference.push(m);
        }
    }
    info!("  Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        info!("  Trying muxer {muxer}");
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_audio_mkvmerge(downloader, output_path, audio_path) {
                warn!("  Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("  Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_stream_ffmpeg(downloader, output_path, audio_path) {
                warn!("  Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("  Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_stream_mp4box(downloader, output_path, audio_path) {
                warn!("  Muxing with MP4Box subprocess failed: {e}");
            } else {
                info!("  Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        }
    }
    warn!("  All available muxers failed");
    warn!("    unmuxed audio stream: {}", audio_path.display());
    Err(DashMpdError::Muxing(String::from("all available muxers failed")))
}


// Generate an appropriate "complex" filter for the ffmpeg concat filter.
// See https://trac.ffmpeg.org/wiki/Concatenate and
//  https://ffmpeg.org/ffmpeg-filters.html#concat
//
// Example for n=3: "[0:v:0][0:a:0][1:v:0][1:a:0][2:v:0][2:a:0]concat=n=3:v=1:a=1[outv][outa]"
//
// Example for n=2 with only audio:
//   -i /tmp/audio1 -i /tmp/audio2 -filter_complex "[0:a][1:a] concat=n=2:v=0:a=1 [outa]" -map "[outa]" 
#[tracing::instrument(level="trace")]
fn make_ffmpeg_concat_filter_args(paths: &[&Path]) -> Vec<String> {
    let n = paths.len();
    let mut args = Vec::new();
    let mut anullsrc = String::new();
    let mut link_labels = Vec::new();
    let mut have_audio = false;
    let mut have_video = false;
    for (i, path) in paths.iter().enumerate().take(n) {
        let mut included = false;
        if container_has_video(path) {
            included = true;
            args.push(String::from("-i"));
            args.push(path.display().to_string());
            have_video = true;
            link_labels.push(format!("[{i}:v]"));
        }
        if container_has_audio(path) {
            if !included {
                args.push(String::from("-i"));
                args.push(path.display().to_string());
            }
            link_labels.push(format!("[{i}:a]"));
            have_audio = true;
        } else {
            // Use a null audio src. Without this null audio track the concat filter is generating
            // errors, with ffmpeg version 6.1.1.
            anullsrc += &format!("anullsrc=r=48000:cl=mono:d=1[anull{i}:a];{anullsrc}");
            link_labels.push(format!("[anull{i}:a]"));
        }
    }
    let mut filter = String::new();
    // Only include the null audio track and the audio link labels to the concat filter when at
    // least one of our component segments has a audio component.
    if have_audio {
        filter += &anullsrc;
        filter += &link_labels.join("");
    } else {
        // We need to delete the link_labels of the form [anull{i}] that refer to null audio sources
        // that we aren't including in the filter graph.
        for ll in link_labels {
            if ! ll.starts_with("[anull") {
                filter += &ll;
            }
        }
    }
    filter += &format!(" concat=n={n}");
    if have_video {
        filter += ":v=1";
    } else {
        filter += ":v=0";
    }
    if have_audio {
        filter += ":a=1";
    } else {
        filter += ":a=0";
    }
    if have_video {
        filter += "[outv]";
    }
    if have_audio {
        filter += "[outa]";
    }
    args.push(String::from("-filter_complex"));
    args.push(filter);
    if have_video {
        args.push(String::from("-map"));
        args.push(String::from("[outv]"));
    }
    if have_audio {
        args.push(String::from("-map"));
        args.push(String::from("[outa]"));
    }
    args
}


/// This function concatenates files using the ffmpeg "concat filter". This reencodes all streams so
/// is slow, but works in situations where the concat protocol doesn't work.
#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files_ffmpeg_filter(
    downloader: &DashDownloader,
    paths: &[&Path]) -> Result<(), DashMpdError>
{
    if paths.len() < 2 {
        return Err(DashMpdError::Muxing(String::from("need at least two files")));
    }
    let container = match paths[0].extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // See output from "ffmpeg -muxers"
    let output_format = match container {
        "mkv" => "matroska",
        "ts" => "mpegts",
        _ => container,
    };
    // First copy the contents of the first file to a temporary file, as ffmpeg will be overwriting the
    // contents of the first file.
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = &tmpout.path();
    fs::copy(paths[0], tmppath)
        .map_err(|e| DashMpdError::Io(e, String::from("copying first input path")))?;
    let mut args = vec!["-hide_banner", "-nostats",
                        "-loglevel", "error",  // or "warning", "info"
                        "-y",
                        "-nostdin"];
    let mut inputs = Vec::<&Path>::new();
    inputs.push(tmppath);
    for p in &paths[1..] {
        inputs.push(p);
    }
    let filter_args = make_ffmpeg_concat_filter_args(&inputs);
    filter_args.iter().for_each(|a| args.push(a));
    args.push("-movflags");
    args.push("faststart+omit_tfhd_offset");
    args.push("-f");
    args.push(output_format);
    let target = paths[0].to_string_lossy();
    args.push(&target);
    if downloader.verbosity > 0 {
        info!("  Concatenating with ffmpeg concat filter {}", args.join(" "));
    }
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        Ok(())
    } else {
        warn!("  unconcatenated input files:");
        for p in paths {
            warn!("      {}", p.display());
        }
        Err(DashMpdError::Muxing(String::from("running ffmpeg")))
    }
}

// This function concatenates files using the ffmpeg concat demuxer. All files must have the same
// streams (same codecs, same time base, etc.) but can be wrapped in different container formats.
// This concatenation helper is very fast because it copies the media streams, rather than
// reencoding them.
//
// In a typical use case of a multi-period DASH manifest with DAI (where Periods containing
// advertising have been intermixed with Periods of content), where it is possible to drop the
// advertising segments (using minimum_period_duration() or using an XSLT filter on Period
// elements), the content segments are likely to all use the same codecs and encoding parameters, so
// this helper should work well.
#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files_ffmpeg_demuxer(
    downloader: &DashDownloader,
    paths: &[&Path]) -> Result<(), DashMpdError>
{
    if paths.len() < 2 {
        return Err(DashMpdError::Muxing(String::from("need at least two files")));
    }
    let container = match paths[0].extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // See output from "ffmpeg -muxers"
    let output_format = match container {
        "mkv" => "matroska",
        "ts" => "mpegts",
        _ => container,
    };
    // First copy the contents of the first file to a temporary file, as ffmpeg will be overwriting the
    // contents of the first file.
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{container}"))
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = &tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    fs::copy(paths[0], tmppath)
        .map_err(|e| DashMpdError::Io(e, String::from("copying first input path")))?;
    let mut args = vec!["-hide_banner", "-nostats",
                        "-loglevel", "error",  // or "warning", "info"
                        "-y",
                        "-nostdin"];
    // https://trac.ffmpeg.org/wiki/Concatenate
    let demuxlist = tempfile::Builder::new()
        .prefix("dashmpddemux")
        .suffix(".txt")
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    // https://ffmpeg.org/ffmpeg-formats.html#concat
    writeln!(&demuxlist, "ffconcat version 1.0")
        .map_err(|e| DashMpdError::Io(e, String::from("writing to demuxer cmd file")))?;
    let canonical = fs::canonicalize(tmppath)
        .map_err(|e| DashMpdError::Io(e, String::from("canonicalizing temporary filename")))?;
    writeln!(&demuxlist, "file '{}'", canonical.display())
        .map_err(|e| DashMpdError::Io(e, String::from("writing to demuxer cmd file")))?;
    for p in &paths[1..] {
        let canonical = fs::canonicalize(p)
            .map_err(|e| DashMpdError::Io(e, String::from("canonicalizing temporary filename")))?;
        writeln!(&demuxlist, "file '{}'", canonical.display())
            .map_err(|e| DashMpdError::Io(e, String::from("writing to demuxer cmd file")))?;
    }
    let demuxlistpath = &demuxlist
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    args.push("-f");
    args.push("concat");
    // We can't use "safe" file paths because our input files have names that are absolute, rather
    // than relative.
    args.push("-safe");
    args.push("0");
    args.push("-i");
    args.push(demuxlistpath);
    args.push("-c");
    args.push("copy");
    args.push("-movflags");
    args.push("faststart+omit_tfhd_offset");
    args.push("-f");
    args.push(output_format);
    let target = String::from("file:") + &paths[0].to_string_lossy();
    args.push(&target);
    if downloader.verbosity > 0 {
        info!("  Concatenating with ffmpeg concat demuxer {}", args.join(" "));
    }
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        Ok(())
    } else {
        warn!("  unconcatenated input files:");
        for p in paths {
            warn!("      {}", p.display());
        }
        Err(DashMpdError::Muxing(String::from("running ffmpeg")))
    }
}


// Merge all media files named by paths into the file named by the first element of the vector.
//
// This concat helper does not seem to work in a satisfactory manner.
#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files_mp4box(
    downloader: &DashDownloader,
    paths: &[&Path]) -> Result<(), DashMpdError>
{
    if paths.len() < 2 {
        return Err(DashMpdError::Muxing(String::from("need at least two files")));
    }
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(".mp4")
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = &tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let mut tmpoutb = BufWriter::new(&tmpout);
    let overwritten = File::open(paths[0])
        .map_err(|e| DashMpdError::Io(e, String::from("opening first container")))?;
    let mut overwritten = BufReader::new(overwritten);
    io::copy(&mut overwritten, &mut tmpoutb)
        .map_err(|e| DashMpdError::Io(e, String::from("copying from overwritten file")))?;
    // MP4Box -add file1.mp4 -cat file2.mp4 -cat file3.mp4 output.mp4"
    let out = paths[0].to_string_lossy();
    let mut args = vec!["-flat", "-add", &tmppath];
    for p in &paths[1..] {
        if let Some(ps) = p.to_str() {
            args.push("-cat");
            args.push(ps);
        } else {
            warn!("  Ignoring non-Unicode pathname {:?}", p);
        }
    }
    args.push(&out);
    if downloader.verbosity > 0 {
        info!("  Concatenating with MP4Box {}", args.join(" "));
    }
    let mp4box = Command::new(&downloader.mp4box_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
    let msg = partial_process_output(&mp4box.stdout);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  MP4Box stdout: {msg}");
    }
    let msg = partial_process_output(&mp4box.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  MP4Box stderr: {msg}");
    }
    if mp4box.status.success() {
        Ok(())
    } else {
        warn!("  unconcatenated input files:");
        for p in paths {
            warn!("      {}", p.display());
        }
        Err(DashMpdError::Muxing(String::from("running MP4Box")))
    }
}

#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files_mkvmerge(
    downloader: &DashDownloader,
    paths: &[&Path]) -> Result<(), DashMpdError>
{
    if paths.len() < 2 {
        return Err(DashMpdError::Muxing(String::from("need at least two files")));
    }
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(".mkv")
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = &tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::other("obtaining tmpfile name"),
            String::from("")))?;
    let mut tmpoutb = BufWriter::new(&tmpout);
    let overwritten = File::open(paths[0])
        .map_err(|e| DashMpdError::Io(e, String::from("opening first container")))?;
    let mut overwritten = BufReader::new(overwritten);
    io::copy(&mut overwritten, &mut tmpoutb)
        .map_err(|e| DashMpdError::Io(e, String::from("copying from overwritten file")))?;
    // https://mkvtoolnix.download/doc/mkvmerge.html
    let mut args = Vec::new();
    if downloader.verbosity < 1 {
        args.push("--quiet");
    }
    args.push("-o");
    let out = paths[0].to_string_lossy();
    args.push(&out);
    args.push("[");
    args.push(tmppath);
    if let Some(inpaths) = paths.get(1..) {
        for p in inpaths {
            if let Some(ps) = p.to_str() {
                args.push(ps);
            }
        }
    }
    args.push("]");
    if downloader.verbosity > 1 {
        info!("  Concatenating with mkvmerge {}", args.join(" "));
    }
    let mkvmerge = Command::new(&downloader.mkvmerge_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge")))?;
    let msg = partial_process_output(&mkvmerge.stdout);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  mkvmerge stdout: {msg}");
        println!("  mkvmerge stdout: {msg}");
    }
    let msg = partial_process_output(&mkvmerge.stderr);
    if downloader.verbosity > 0 && !msg.is_empty() {
        info!("  mkvmerge stderr: {msg}");
        println!("  mkvmerge stderr: {msg}");
    }
    if mkvmerge.status.success() {
        Ok(())
    } else {
        warn!("  unconcatenated input files:");
        for p in paths {
            warn!("      {}", p.display());
        }
        Err(DashMpdError::Muxing(String::from("running mkvmerge")))
    }
}

// Merge all media files named by paths into the file named by the first element of the vector.
#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files(
    downloader: &DashDownloader,
    paths: &[&Path]) -> Result<(), DashMpdError> {
    if paths.len() < 2 {
        return Ok(());
    }
    let container = if let Some(p0) = paths.first() {
        match p0.extension() {
            Some(ext) => ext.to_str().unwrap_or("mp4"),
            None => "mp4",
        }
    } else {
        "mp4"
    };
    let mut concat_preference = vec![];
    if container.eq("mp4") ||
        container.eq("mkv") ||
        container.eq("webm")
    {
        // We will probably make ffmpegdemuxer the default concat helper in a future release; it's
        // much more robust than mkvmerge and much faster than ffmpeg ("concat filter"). But wait
        // until it gets more testing.
        // concat_preference.push("ffmpegdemuxer");
        concat_preference.push("mkvmerge");
        concat_preference.push("ffmpeg");
    } else {
        concat_preference.push("ffmpeg");
    }
    if let Some(ordering) = downloader.concat_preference.get(container) {
        concat_preference.clear();
        for m in ordering.split(',') {
            concat_preference.push(m);
        }
    }
    info!("  Concat helper preference for {container} is {concat_preference:?}");
    for concat in concat_preference {
        info!("  Trying concat helper {concat}");
        if concat.eq("mkvmerge") {
            if let Err(e) = concat_output_files_mkvmerge(downloader, paths) {
                warn!("  Concatenation with mkvmerge failed: {e}");
            } else {
                info!("  Concatenation with mkvmerge succeeded");
                return Ok(());
            }
        } else if concat.eq("ffmpeg") {
            if let Err(e) = concat_output_files_ffmpeg_filter(downloader, paths) {
                warn!("  Concatenation with ffmpeg filter failed: {e}");
            } else {
                info!("  Concatenation with ffmpeg filter succeeded");
                return Ok(());
            }
        } else if concat.eq("ffmpegdemuxer") {
            if let Err(e) = concat_output_files_ffmpeg_demuxer(downloader, paths) {
                warn!("  Concatenation with ffmpeg demuxer failed: {e}");
            } else {
                info!("  Concatenation with ffmpeg demuxer succeeded");
                return Ok(());
            }
        } else if concat.eq("mp4box") {
            if let Err(e) = concat_output_files_mp4box(downloader, paths) {
                warn!("  Concatenation with MP4Box failed: {e}");
            } else {
                info!("  Concatenation with MP4Box succeeded");
                return Ok(());
            }
        } else {
            warn!("  Ignoring unknown concat helper preference {concat}");
        }
    }
    warn!("  All concat helpers failed");
    Err(DashMpdError::Muxing(String::from("all concat helpers failed")))
}


// Run these tests with "cargo test -- --nocapture" to see all tracing logs.
#[cfg(test)]
mod tests {
    use std::path::Path;
    use assert_cmd::Command;
    use fs_err as fs;

    fn generate_mp4_hue_tone(filename: &Path, color: &str, tone: &str) {
        Command::new("ffmpeg")
            .args(["-y",  // overwrite output file if it exists
                   "-nostdin",
                   "-lavfi", &format!("color=c={color}:duration=5:size=50x50:rate=1;sine=frequency={tone}:sample_rate=48000:duration=5"),
                   // Force the use of the libx264 encoder. ffmpeg defaults to platform-specific
                   // encoders (which may allow hardware encoding) on certain builds, which may have
                   // stronger restrictions on acceptable frame rates and so on. For example, the
                   // h264_mediacodec encoder on Android has more constraints than libx264 regarding the
                   // number of keyframes.
                   "-c:v", "libx264",
                   "-pix_fmt", "yuv420p",
                   "-profile:v", "baseline",
                   "-framerate", "25",
                   "-movflags", "faststart",
                   filename.to_str().unwrap()])
            .assert()
            .success();
    }

    // Generate 3 5-second dummy MP4 files, one with a red background color, the second with green,
    // the third with blue. Concatenate them into the first red file. Check that at second 2.5 we
    // have a red background, at second 7.5 a green background, and at second 12.5 a blue
    // background.
    //
    // We run this test once for each of the concat helpers: ffmpeg, ffmpegdemuxer, mkvmerge.
    #[test]
    fn test_concat_helpers() {
        use crate::fetch::DashDownloader;
        use crate::ffmpeg::{
            concat_output_files_ffmpeg_filter,
            concat_output_files_ffmpeg_demuxer,
            concat_output_files_mkvmerge
        };
        use image::ImageReader;
        use image::Rgb;

        // Check that the media file merged contains a first sequence with red background, then with
        // green background, then with blue background.
        fn check_color_sequence(merged: &Path) {
            let tmpd = tempfile::tempdir().unwrap();
            let capture_red = tmpd.path().join("capture-red.png");
            Command::new("ffmpeg")
                .args(["-ss", "2.5",
                       "-i", merged.to_str().unwrap(),
                       "-frames:v", "1",
                       capture_red.to_str().unwrap()])
                .assert()
                .success();
            let img = ImageReader::open(&capture_red).unwrap()
                .decode().unwrap()
                .into_rgb8();
            for pixel in img.pixels() {
                match pixel {
                    Rgb(rgb) => {
                        assert!(rgb[0] > 250);
                        assert!(rgb[1] < 5);
                        assert!(rgb[2] < 5);
                    },
                };
            }
            fs::remove_file(&capture_red).unwrap();
            // The green color used by ffmpeg is Rgb(0,127,0)
            let capture_green = tmpd.path().join("capture-green.png");
            Command::new("ffmpeg")
                .args(["-ss", "7.5",
                       "-i", merged.to_str().unwrap(),
                       "-frames:v", "1",
                       capture_green.to_str().unwrap()])
                .assert()
                .success();
            let img = ImageReader::open(&capture_green).unwrap()
                .decode().unwrap()
                .into_rgb8();
            for pixel in img.pixels() {
                match pixel {
                    Rgb(rgb) => {
                        assert!(rgb[0] < 5);
                        assert!(rgb[1].abs_diff(127) < 5);
                        assert!(rgb[2] < 5);
                    },
                };
            }
            fs::remove_file(&capture_green).unwrap();
            // The "blue" color chosen by ffmpeg is Rgb(0,0,254)
            let capture_blue = tmpd.path().join("capture-blue.png");
            Command::new("ffmpeg")
                .args(["-ss", "12.5",
                       "-i", merged.to_str().unwrap(),
                       "-frames:v", "1",
                       capture_blue.to_str().unwrap()])
                .assert()
                .success();
            let img = ImageReader::open(&capture_blue).unwrap()
                .decode().unwrap()
                .into_rgb8();
            for pixel in img.pixels() {
                match pixel {
                    Rgb(rgb) => {
                        assert!(rgb[0] < 5);
                        assert!(rgb[1] < 5);
                        assert!(rgb[2] > 250);
                    },
                };
            }
            fs::remove_file(&capture_blue).unwrap();
        }

        let tmpd = tempfile::tempdir().unwrap();
        let red = tmpd.path().join("concat-red.mp4");
        let green = tmpd.path().join("concat-green.mp4");
        let blue = tmpd.path().join("concat-blue.mp4");
        generate_mp4_hue_tone(&red, "red", "400");
        generate_mp4_hue_tone(&green, "green", "600");
        generate_mp4_hue_tone(&blue, "blue", "800");
        let ddl = DashDownloader::new("https://www.example.com/")
            .verbosity(2);

        let output_ffmpeg_filter = tmpd.path().join("output-ffmpeg-filter.mp4");
        fs::copy(&red, &output_ffmpeg_filter).unwrap();
        concat_output_files_ffmpeg_filter(
            &ddl,
            &[output_ffmpeg_filter.clone(), green.clone(), blue.clone()]).unwrap();
        check_color_sequence(&output_ffmpeg_filter);
        fs::remove_file(&output_ffmpeg_filter).unwrap();

        let output_ffmpeg_demuxer = tmpd.path().join("output-ffmpeg-demuxer.mp4");
        fs::copy(&red, &output_ffmpeg_demuxer).unwrap();
        concat_output_files_ffmpeg_demuxer(
            &ddl,
            &[output_ffmpeg_demuxer.clone(), green.clone(), blue.clone()]).unwrap();
        check_color_sequence(&output_ffmpeg_demuxer);
        fs::remove_file(&output_ffmpeg_demuxer).unwrap();

        // mkvmerge fails to concatenate our test MP4 files generated with ffmpeg (its Quicktime/MP4
        // reader complains about "Could not read chunk number XX/YY with size XX from position
        // XX"). So test it instead with Matroska files for which it should be more robust. We
        // also test the ffmpeg_filter and ffmpeg_demuxer concat helpers on the Matroska files.
        let red = tmpd.path().join("concat-red.mkv");
        let green = tmpd.path().join("concat-green.mkv");
        let blue = tmpd.path().join("concat-blue.mkv");
        generate_mp4_hue_tone(&red, "red", "400");
        generate_mp4_hue_tone(&green, "green", "600");
        generate_mp4_hue_tone(&blue, "blue", "800");

        let output_mkvmerge = tmpd.path().join("output-mkvmerge.mkv");
        fs::copy(&red, &output_mkvmerge).unwrap();
        concat_output_files_mkvmerge(
            &ddl,
            &[output_mkvmerge.clone(), green.clone(), blue.clone()]).unwrap();
        check_color_sequence(&output_mkvmerge);
        fs::remove_file(&output_mkvmerge).unwrap();

        let output_ffmpeg_filter = tmpd.path().join("output-ffmpeg-filter.mkv");
        fs::copy(&red, &output_ffmpeg_filter).unwrap();
        concat_output_files_ffmpeg_filter(
            &ddl,
            &[output_ffmpeg_filter.clone(), green.clone(), blue.clone()]).unwrap();
        check_color_sequence(&output_ffmpeg_filter);
        fs::remove_file(&output_ffmpeg_filter).unwrap();

        let output_ffmpeg_demuxer = tmpd.path().join("output-ffmpeg-demuxer.mkv");
        fs::copy(&red, &output_ffmpeg_demuxer).unwrap();
        concat_output_files_ffmpeg_demuxer(
            &ddl,
            &[output_ffmpeg_demuxer.clone(), green.clone(), blue.clone()]).unwrap();
        check_color_sequence(&output_ffmpeg_demuxer);
        fs::remove_file(&output_ffmpeg_demuxer).unwrap();

        let _ = fs::remove_dir_all(tmpd);
    }
}
