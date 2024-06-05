/// Muxing support using mkvmerge/ffmpeg/vlc/mp4box as a subprocess.
///
/// Also see the alternative method of using ffmpeg via its "libav" shared library API, implemented
/// in file "libav.rs".

// TODO: on Linux we should try to use bubblewrap to execute the muxers in a sandboxed environment,
// along the lines of
//
//    bwrap --ro-bind /usr /usr --ro-bind /etc /etc --tmpfs /tmp ffmpeg -i audio.mp4 -i video.mp4 /tmp/muxed.mp4

use std::io;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use fs_err as fs;
use fs::File;
use tracing::{trace, info, warn};
use crate::DashMpdError;
use crate::fetch::{DashDownloader, partial_process_output};
use crate::media::{audio_container_type, video_container_type, container_has_video, container_has_audio};




// ffmpeg can mux to many container types including mp4, mkv, avi
#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_video_ffmpeg(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_path: &Path,
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
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining videopath name"),
            String::from("")))?;
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(["-hide_banner",
               "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-nostdin",
               "-i", audio_str,
               "-i", video_str,
               "-c:v", "copy",
               "-c:a", "copy",
               "-movflags", "faststart", "-preset", "veryfast",
               // select the muxer explicitly (debatable whether this is better than ffmpeg's
               // heuristics based on output filename)
               "-f", muxer,
               tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if msg.len() > 0 {
        info!("ffmpeg stderr: {msg}");
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
	if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary ffmpeg output: {e}");
        }
        return Ok(());
    }
    // The muxing may have failed only due to the "-c:v copy -c:a copy" argument to ffmpeg, which
    // instructs it to copy the audio and video streams without any reencoding. That is not possible
    // for certain output containers; for instance a WebM container must contain video using VP8,
    // VP9 or AV1 codecs and Vorbis or Opus audio codecs. (Unfortunately, ffmpeg doesn't seem to
    // return a recognizable error message in this specific case.)  So we try invoking ffmpeg again,
    // this time allowing reencoding.
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(["-hide_banner",
               "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y", // overwrite output file if it exists
               "-nostdin",
               "-i", audio_str,
               "-i", video_str,
               "-movflags", "faststart", "-preset", "veryfast",
               // select the muxer explicitly (debatable whether this is better than ffmpeg's
               // heuristics based on output filename)
               "-f", muxer,
               tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if msg.len() > 0 {
        info!("ffmpeg stderr: {msg}");
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
	if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary ffmpeg output: {e}");
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
    info!("ffmpeg inserting stream into {container} container named {}", output_path.display());
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
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let input = input_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining input name"),
            String::from("")))?;
    let cn: String;
    let mut args = vec!("-hide_banner",
                        "-nostats",
                        "-loglevel", "error",  // or "warning", "info"
                        "-y",  // overwrite output file if it exists
                        "-nostdin",
                        "-i", input,
                        "-movflags", "faststart", "-preset", "veryfast");
    // We can select the muxer explicitly (otherwise it is determined using heuristics based in the
    // filename extension).
    if let Some(container_name) = ffmpeg_container_name(container) {
        args.push("-f");
        cn = container_name;
        args.push(&cn);
    }
    args.push(tmppath);
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if msg.len() > 0 {
        info!("ffmpeg stderr: {msg}");
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
	if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary ffmpeg output: {e}");
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
    audio_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
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
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    let tmppath = tmpout
        .path()
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining videopath name"),
            String::from("")))?;
    let transcode = if container.eq("webm") {
        "transcode{vcodec=VP90,acodec=vorb}:"
    } else {
        ""
    };
    let vlc = Command::new(&downloader.vlc_location)
        .args(["-I", "dummy",
               "--no-repeat", "--no-loop",
               video_str,
               "--input-slave", audio_str,
               "--sout-mp4-faststart",
               &format!("--sout=#{transcode}std{{access=file,mux={muxer},dst={tmppath}}}"),
               "--sout-keep",
               "vlc://quit"])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning VLC subprocess")))?;
    // VLC is erroneously returning a 0 (success) return code even when it fails to mux, so we need
    // to look for a specific error message to check for failure.
    let msg = partial_process_output(&vlc.stderr);
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
        if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary VLC output: {e}");
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
    audio_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
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
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining videopath name"),
            String::from("")))?;
    let cmd = Command::new(&downloader.mp4box_location)
        .args(["-flat",
               "-add", video_str,
               "-add", audio_str,
               "-new", tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
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
	if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary MP4Box output: {e}");
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
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let input = input_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining input stream name"),
            String::from("")))?;
    let cmd = Command::new(&downloader.mp4box_location)
        .args(["-add", input,
               "-new", tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
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
	if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary MP4Box output: {e}");
        }
        Ok(())
    } else {
        let msg = partial_process_output(&cmd.stderr);
        warn!("MP4Box mux_stream failure: stdout {}", partial_process_output(&cmd.stdout));
        warn!("MP4Box stderr: {msg}");
        Err(DashMpdError::Muxing(format!("running MP4Box: {msg}")))
    }
}

// mkvmerge on Windows is compiled using MinGW and isn't able to handle native pathnames, so we
// create the temporary file in the current directory.
#[cfg(target_os = "windows")]
fn temporary_outpath(suffix: &str) -> Result<String, DashMpdError> {
    Ok(format!("dashmpdrs-tmp{suffix}"))
}

#[cfg(not(target_os = "windows"))]
fn temporary_outpath(suffix: &str) -> Result<String, DashMpdError> {
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(suffix)
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary output file")))?;
    match tmpout.path().to_str() {
        Some(s) => Ok(s.to_string()),
        None => Ok(format!("/tmp/dashmpdrs-tmp{suffix}")),
    }
}

#[tracing::instrument(level="trace", skip(downloader))]
fn mux_audio_video_mkvmerge(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
    let tmppath = temporary_outpath(".mkv")?;
    let audio_str = audio_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining audiopath name"),
            String::from("")))?;
    let video_str = video_path
        .to_str()
        .ok_or_else(|| DashMpdError::Io(
            io::Error::new(io::ErrorKind::Other, "obtaining videopath name"),
            String::from("")))?;
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(["--output", &tmppath,
               "--no-video", audio_str,
               "--no-audio", video_str])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
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
        if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary mkvmerge output: {e}");
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
            io::Error::new(io::ErrorKind::Other, "obtaining videopath name"),
            String::from("")))?;
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(["--output", &tmppath,
               "--no-audio", video_str])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
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
        if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary mkvmerge output: {e}");
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
            io::Error::new(io::ErrorKind::Other, "obtaining audiopath name"),
            String::from("")))?;
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(["--output", &tmppath,
               "--no-video", audio_str])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
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
        if let Err(e) = fs::remove_file(tmppath) {
            warn!("Error deleting temporary mkvmerge output: {e}");
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
    audio_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
    trace!("Muxing audio {}, video {}", audio_path.display(), video_path.display());
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
            if let Err(e) =  mux_audio_video_mkvmerge(downloader, output_path, audio_path, video_path) {
                warn!("  Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("  Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_audio_video_ffmpeg(downloader, output_path, audio_path, video_path) {
                warn!("  Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("  Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("vlc") {
            if let Err(e) = mux_audio_video_vlc(downloader, output_path, audio_path, video_path) {
                warn!("  Muxing with vlc subprocess failed: {e}");
            } else {
                info!("  Muxing with vlc subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_audio_video_mp4box(downloader, output_path, audio_path, video_path) {
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
    warn!("  unmuxed audio stream: {}", audio_path.display());
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
    info!("Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        info!("Trying muxer {muxer}");
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_video_mkvmerge(downloader, output_path, video_path) {
                warn!("Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_stream_ffmpeg(downloader, output_path, video_path) {
                warn!("Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_stream_mp4box(downloader, output_path, video_path) {
                warn!("Muxing with MP4Box subprocess failed: {e}");
            } else {
                info!("Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        }
    }
    warn!("All available muxers failed");
    warn!("  unmuxed video stream: {}", video_path.display());
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
    info!("Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        info!("Trying muxer {muxer}");
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_audio_mkvmerge(downloader, output_path, audio_path) {
                warn!("Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_stream_ffmpeg(downloader, output_path, audio_path) {
                warn!("Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_stream_mp4box(downloader, output_path, audio_path) {
                warn!("Muxing with MP4Box subprocess failed: {e}");
            } else {
                info!("Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        }
    }
    warn!("All available muxers failed");
    warn!("  unmuxed audio stream: {}", audio_path.display());
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
fn make_ffmpeg_concat_filter_args(paths: &[PathBuf]) -> Vec<String> {
    let n = paths.len();
    let mut args = Vec::new();
    let mut filter = String::new();
    let mut link_labels = Vec::new();
    let mut have_audio = false;
    let mut have_video = false;
    for (i, path) in paths.iter().enumerate().take(n) {
        let mut included = false;
        if container_has_video(path) {
            included = true;
            args.push(String::from("-i"));
            args.push(path.display().to_string());
            // filter = format!("streams=dv[v{i}];{filter}");
            have_video = true;
            link_labels.push(format!("[{i}:v]"));
        }
        if container_has_audio(path) {
            if !included {
                args.push(String::from("-i"));
                args.push(path.display().to_string());
            }
            // filter = format!("streams=da[a{i}];{filter}");
            link_labels.push(format!("[{i}:a]"));
            have_audio = true;
        } else {
            // Use a null audio src. Without this null audio track the concat filter is generating
            // errors, with ffmpeg version 6.1.1.
            filter = format!("anullsrc=r=48000:cl=mono:d=1[audio{i}];{filter}");
            link_labels.push(format!("[audio{i}]"));
            have_audio = true;
        }
        // link_labels.push(format!("[a{i}]"));
    }
    filter += &link_labels.join("");
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


#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files_ffmpeg(
    downloader: &DashDownloader,
    paths: &[PathBuf]) -> Result<(), DashMpdError>
{
    assert!(paths.len() >= 2);
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
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    fs::copy(paths[0].clone(), tmppath)
        .map_err(|e| DashMpdError::Io(e, String::from("copying first input path")))?;
    let mut args = vec!["-hide_banner", "-nostats",
                        "-loglevel", "error",  // or "warning", "info"
                        "-y",
                        "-nostdin"];
    let mut inputs = Vec::<PathBuf>::new();
    inputs.push(tmppath.into());
    for p in &paths[1..] {
        inputs.push(p.to_path_buf());
    }
    let filter_args = make_ffmpeg_concat_filter_args(&inputs);
    filter_args.iter().for_each(|a| args.push(a));
    args.push("-movflags");
    args.push("faststart+omit_tfhd_offset");
    args.push("-f");
    args.push(output_format);
    let target = paths[0].to_string_lossy();
    args.push(&target);
    trace!("Concatenating with ffmpeg {args:?}");
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg")))?;
    let msg = partial_process_output(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = partial_process_output(&ffmpeg.stderr);
    if msg.len() > 0 {
        info!("ffmpeg stderr: {msg}");
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
#[tracing::instrument(level="trace", skip(downloader))]
pub(crate) fn concat_output_files_mp4box(
    downloader: &DashDownloader,
    paths: &[PathBuf]) -> Result<(), DashMpdError>
{
    assert!(paths.len() >= 2);
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
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let mut tmpoutb = BufWriter::new(&tmpout);
    let overwritten = File::open(paths[0].clone())
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
            warn!("Ignoring non-Unicode pathname {:?}", p);
        }
    }
    args.push(&out);
    trace!("Concatenating with MP4Box {args:?}");
    let mp4box = Command::new(&downloader.mp4box_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
    let msg = partial_process_output(&mp4box.stdout);
    if msg.len() > 0 {
        info!("MP4Box stdout: {msg}");
    }
    let msg = partial_process_output(&mp4box.stderr);
    if msg.len() > 0 {
        info!("MP4Box stderr: {msg}");
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
    paths: &[PathBuf]) -> Result<(), DashMpdError>
{
    assert!(paths.len() >= 2);
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
            io::Error::new(io::ErrorKind::Other, "obtaining tmpfile name"),
            String::from("")))?;
    let mut tmpoutb = BufWriter::new(&tmpout);
    let overwritten = File::open(paths[0].clone())
        .map_err(|e| DashMpdError::Io(e, String::from("opening first container")))?;
    let mut overwritten = BufReader::new(overwritten);
    io::copy(&mut overwritten, &mut tmpoutb)
        .map_err(|e| DashMpdError::Io(e, String::from("copying from overwritten file")))?;
    // https://mkvtoolnix.download/doc/mkvmerge.html
    let mut args = Vec::new();
    args.push("--quiet");
    args.push("-o");
    let out = paths[0].to_string_lossy();
    args.push(&out);
    args.push("[");
    args.push(tmppath);
    for p in &paths[1..] {
        if let Some(ps) = p.to_str() {
            args.push(ps);
        }
    }
    args.push("]");
    trace!("Concatenating with mkvmerge {args:?}");
    let mkvmerge = Command::new(&downloader.mkvmerge_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge")))?;
    let msg = partial_process_output(&mkvmerge.stdout);
    if msg.len() > 0 {
        info!("mkvmerge stdout: {msg}");
    }
    let msg = partial_process_output(&mkvmerge.stderr);
    if msg.len() > 0 {
        info!("mkvmerge stderr: {msg}");
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
    paths: &[PathBuf]) -> Result<(), DashMpdError> {
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
            if let Err(e) = concat_output_files_ffmpeg(downloader, paths) {
                warn!("  Concatenation with ffmpeg failed: {e}");
            } else {
                info!("  Concatenation with ffmpeg succeeded");
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
    warn!("All concat helpers failed");
    Err(DashMpdError::Muxing(String::from("all concat helpers failed")))
}



#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;
    use test_log::test;
    use fs_err as fs;
    use super::concat_output_files_ffmpeg;

    fn generate_mp4_hue_tone(filename: &Path, color: &str, tone: &str) {
        let ffmpeg = Command::new("ffmpeg")
            .args(["-y",  // overwrite output file if it exists
                   "-nostdin",
                   "-lavfi", &format!("color=c={color}:duration=5:size=50x50:rate=1;sine=frequency={tone}:sample_rate=48000:duration=5"),
                   // Force the use of the libx264 encoder. ffmpeg defaults to platform-specific
                   // encoders (which may allow hardware encoding) on certain builds, which may have
                   // stronger restrictions on acceptable frame rates and so on. For example, the
                   // h264_mediacodec encoder on Android has more constraints than libx264 regarding the
                   // number of keyframes.
                   "-c:v", "libx264",
                   filename.to_str().unwrap()])
            .output()
            .expect("spawning ffmpeg");
        assert!(ffmpeg.status.success());
    }

    // Generate 3 5-second dummy MP4 files, one with a red background color, the second with green,
    // the third with blue. Concatenate them into the first red file. Check that at second 2.5 we
    // have a red background, at second 7.5 a green background, and at second 12.5 a blue
    // background.
    #[test]
    fn test_concat() {
        use crate::fetch::DashDownloader;
        use image::io::Reader as ImageReader;
        use image::Rgb;

        let tmpd = tempfile::tempdir().unwrap();
        let red = tmpd.path().join("concat-red.mp4");
        let green = tmpd.path().join("concat-green.mp4");
        let blue = tmpd.path().join("concat-blue.mp4");
        generate_mp4_hue_tone(&red, "red", "400");
        generate_mp4_hue_tone(&green, "green", "600");
        generate_mp4_hue_tone(&blue, "blue", "800");
        let ddl = DashDownloader::new("https://www.example.com/");
        let _ = concat_output_files_ffmpeg(&ddl, &[red.clone(), green, blue]);
        let capture_red = tmpd.path().join("capture-red.png");
        Command::new("ffmpeg")
            .args(["-ss", "2.5",
                   "-i", &red.to_str().unwrap(),
                   "-frames:v", "1",
                   &capture_red.to_str().unwrap()])
            .output()
            .expect("extracting red frame");
        let img = ImageReader::open(capture_red).unwrap()
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
        // The green color used by ffmpeg is Rgb(0,127,0)
        let capture_green = tmpd.path().join("capture-green.png");
        Command::new("ffmpeg")
            .args(["-ss", "7.5",
                   "-i", &red.to_str().unwrap(),
                   "-frames:v", "1",
                   &capture_green.to_str().unwrap()])
            .output()
            .expect("extracting green frame");
        let img = ImageReader::open(capture_green).unwrap()
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
        // The "blue" color chosen by ffmpeg is Rgb(0,0,254)
        let capture_blue = tmpd.path().join("capture-blue.png");
        Command::new("ffmpeg")
            .args(["-ss", "12.5",
                   "-i", &red.to_str().unwrap(),
                   "-frames:v", "1",
                   &capture_blue.to_str().unwrap()])
            .output()
            .expect("extracting blue frame");
        let img = ImageReader::open(capture_blue).unwrap()
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
        let _ = fs::remove_dir_all(tmpd);
    }
}
