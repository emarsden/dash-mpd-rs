/// Muxing support using mkvmerge/ffmpeg/vlc as a subprocess.
///
/// Also see the alternative method of using ffmpeg via its "libav" shared library API, implemented
/// in file "libav.rs".


use std::io;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use fs_err as fs;
use fs::File;
use log::{trace, info, warn};
use crate::DashMpdError;
use crate::fetch::DashDownloader;
use crate::media::{audio_container_type, video_container_type, container_has_video, container_has_audio};


// ffmpeg can mux to many container types including mp4, mkv, avi
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
               "-i", audio_str,
               "-i", video_str,
               "-c:v", "copy",
               "-c:a", "copy",
               "-movflags", "+faststart", "-preset", "veryfast",
               // select the muxer explicitly (debatable whether this is better than ffmpeg's
               // heuristics based on output filename)
               "-f", muxer,
               tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = String::from_utf8_lossy(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
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
                    "-i", input,
                    "-movflags", "+faststart", "-preset", "veryfast");
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
    let msg = String::from_utf8_lossy(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
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


// See https://wiki.videolan.org/Transcode/
// VLC could also mux to an mkv container if needed
fn mux_audio_video_vlc(
    downloader: &DashDownloader,
    output_path: &Path,
    audio_path: &Path,
    video_path: &Path) -> Result<(), DashMpdError> {
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
    let vlc = Command::new(&downloader.vlc_location)
        .args(["-I", "dummy",
               "--no-repeat", "--no-loop",
               video_str,
               "--input-slave", audio_str,
               "--sout-mp4-faststart",
               &format!("--sout=#std{{access=file,mux=mp4,dst={tmppath}}}"),
               "--sout-keep",
               "vlc://quit"])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning VLC subprocess")))?;
    // VLC is erroneously returning a 0 (success) return code even when it fails to mux, so we need
    // to look for a specific error message to check for failure.
    let msg = String::from_utf8_lossy(&vlc.stderr);
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
        let msg = String::from_utf8_lossy(&vlc.stderr);
        Err(DashMpdError::Muxing(format!("running VLC: {msg}")))
    }
}


// MP4Box from the GPAC suite for muxing audio and video streams
// https://github.com/gpac/gpac/wiki/MP4Box
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
        .args(["-add", video_str,
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
        let msg = String::from_utf8_lossy(&cmd.stderr);
        Err(DashMpdError::Muxing(format!("running MP4Box: {msg}")))
    }
}

// This can be used to package either an audio stream or a video stream into the container format
// that is determined by the extension of output_path.
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
        let msg = String::from_utf8_lossy(&cmd.stderr);
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
    info!("Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        info!("Trying muxer {muxer}");
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_audio_video_mkvmerge(downloader, output_path, audio_path, video_path) {
                warn!("Muxing with mkvmerge subprocess failed: {e}");
            } else {
                info!("Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_audio_video_ffmpeg(downloader, output_path, audio_path, video_path) {
                warn!("Muxing with ffmpeg subprocess failed: {e}");
            } else {
                info!("Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("vlc") {
            if let Err(e) = mux_audio_video_vlc(downloader, output_path, audio_path, video_path) {
                warn!("Muxing with vlc subprocess failed: {e}");
            } else {
                info!("Muxing with vlc subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_audio_video_mp4box(downloader, output_path, audio_path, video_path) {
                warn!("Muxing with MP4Box subprocess failed: {e}");
            } else {
                info!("Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        } else {
            warn!("Ignoring unknown muxer preference {muxer}");
        }
    }
    warn!("All muxers failed");
    Err(DashMpdError::Muxing(String::from("all muxers failed")))
}


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
    // TODO: should allow the user to specify this ordering preference
    let mut muxer_preference = vec![];
    if container.eq("mkv") {
        muxer_preference.push("mkvmerge");
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    } else {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    }
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
    Err(DashMpdError::Muxing(String::from("all available muxers failed")))
}


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
    // TODO: should allow the user to specify this ordering preference
    let mut muxer_preference = vec![];
    if container.eq("mkv") {
        muxer_preference.push("mkvmerge");
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    } else {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("mp4box");
    }
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
    Err(DashMpdError::Muxing(String::from("all available muxers failed")))
}


// Generate an appropriate "complex" filter for the ffmpeg concat filter.
// See https://trac.ffmpeg.org/wiki/Concatenate
//
// Example for n=3: "[0:v:0][0:a:0][1:v:0][1:a:0][2:v:0][2:a:0]concat=n=3:v=1:a=1[outv][outa]"
//
// Example for n=2 with only audio:
//   -i /tmp/audio1 -i /tmp/audio2 -filter_complex "[0:a][1:a] concat=n=2:v=0:a=1 [outa]" -map "[outa]" 
fn make_ffmpeg_concat_filter_args(paths: &Vec<PathBuf>) -> Vec<String> {
    let n = paths.len();
    let mut filter = String::new();
    let mut have_audio = false;
    let mut have_video = false;
    for (i, path) in paths.iter().enumerate().take(n) {
        if container_has_video(path) {
            filter += &format!("[{i}:v:0]");
            have_video = true;
        }
        if container_has_audio(path) {
            filter += &format!("[{i}:a:0]");
            have_audio = true;
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
    let mut args = vec![String::from("-filter_complex"), filter];
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

// Merge all media files named by paths into the file named by the first element of the vector.
// Currently only attempt ffmpeg, with reencoding in case the codecs in the input files are different.
pub(crate) fn concat_output_files(downloader: &DashDownloader, paths: &Vec<PathBuf>) -> Result<(), DashMpdError> {
    if paths.len() < 2 {
        return Ok(());
    }
    // First copy the contents of the first file to a temporary file, as ffmpeg will be overwriting the
    // contents of the first file.
    let container = match paths[0].extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
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
    let mut tmpoutb = BufWriter::new(&tmpout);
    let overwritten = File::open(paths[0].clone())
        .map_err(|e| DashMpdError::Io(e, String::from("opening first container")))?;
    let mut overwritten = BufReader::new(overwritten);
    io::copy(&mut overwritten, &mut tmpoutb)
        .map_err(|e| DashMpdError::Io(e, String::from("copying from overwritten file")))?;
    let mut args = vec!["-hide_banner", "-nostats",
                        "-loglevel", "error",  // or "warning", "info"
                        "-y",
                        "-i", tmppath];
    for p in &paths[1..] {
        args.push("-i");
        args.push(p.to_str().unwrap());
    }
    let filter_args = make_ffmpeg_concat_filter_args(paths);
    filter_args.iter().for_each(|a| args.push(a));
    args.push("-movflags");
    args.push("faststart+omit_tfhd_offset");
    args.push("-f");
    args.push(container);
    let target = paths[0].to_string_lossy();
    args.push(&target);
    trace!("Concatenating with ffmpeg {args:?}");
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = String::from_utf8_lossy(&ffmpeg.stdout);
    if msg.len() > 0 {
        info!("ffmpeg stdout: {msg}");
    }
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    if msg.len() > 0 {
        info!("ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        Ok(())
    } else {
        Err(DashMpdError::Muxing(String::from("running ffmpeg")))
    }
}
