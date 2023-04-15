/// Muxing support using mkvmerge/ffmpeg/vlc as a subprocess.
///
/// Also see the alternative method of using ffmpeg via its "libav" shared library API, implemented
/// in file "libav.rs".


use std::fs::File;
use std::io;
use std::io::{BufReader, BufWriter};
use std::process::Command;
use crate::DashMpdError;
use crate::fetch::DashDownloader;


// ffmpeg can mux to many container types including mp4, mkv, avi
fn mux_audio_video_ffmpeg(
    downloader: &DashDownloader,
    audio_path: &str,
    video_path: &str) -> Result<(), DashMpdError> {
    let output_path = downloader.output_path.as_ref()
              .expect("muxer called without specifying output_path");
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
    let ffmpeg = Command::new(&downloader.ffmpeg_location)
        .args(["-hide_banner",
               "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-i", audio_path,
               "-i", video_path,
               "-c:v", "copy",
               "-c:a", "copy",
               "-movflags", "+faststart", "-preset", "veryfast",
               // select the muxer explicitly
               "-f", container,
               tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning ffmpeg subprocess")))?;
    let msg = String::from_utf8_lossy(&ffmpeg.stdout);
    if msg.len() > 0 {
        log::info!("ffmpeg stdout: {msg}");
    }
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    if msg.len() > 0 {
        log::info!("ffmpeg stderr: {msg}");
    }
    if ffmpeg.status.success() {
        let tmpfile = File::open(tmppath)
            .map_err(|e| DashMpdError::Io(e, String::from("opening ffmpeg output")))?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying ffmpeg output to output file")))?;
	#[cfg(target_os = "windows")]
	::std::fs::remove_file(tmppath).ok();
        Ok(())
    } else {
        Err(DashMpdError::Muxing(String::from("running ffmpeg")))
    }
}


// See https://wiki.videolan.org/Transcode/
// VLC could also mux to an mkv container if needed
fn mux_audio_video_vlc(
    downloader: &DashDownloader,
    audio_path: &str,
    video_path: &str) -> Result<(), DashMpdError> {
    let output_path = downloader.output_path.as_ref()
              .expect("muxer called without specifying output_path");
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
    let vlc = Command::new(&downloader.vlc_location)
        .args(["-I", "dummy",
               "--no-repeat", "--no-loop",
               video_path,
               "--input-slave", audio_path,
               "--sout-mp4-faststart",
               &format!("--sout=#std{{access=file,mux=mp4,dst={tmppath}}}"),
               "--sout-keep",
               "vlc://quit"])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning VLC subprocess")))?;
    if vlc.status.success() {
        let tmpfile = File::open(tmppath)
            .map_err(|e| DashMpdError::Io(e, String::from("opening VLC output")))?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying VLC output to output file")))?;
	#[cfg(target_os = "windows")]
	::std::fs::remove_file(tmppath).ok();
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
    audio_path: &str,
    video_path: &str) -> Result<(), DashMpdError> {
    let output_path = downloader.output_path.as_ref()
              .expect("muxer called without specifying output_path");
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
    let cmd = Command::new(&downloader.mp4box_location)
        .args(["-add", video_path,
               "-add", audio_path,
               "-new", tmppath])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning MP4Box subprocess")))?;
    if cmd.status.success() {
        let tmpfile = File::open(tmppath)
            .map_err(|e| DashMpdError::Io(e, String::from("opening MP4Box output")))?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("creating output file")))?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying MP4Box output to output file")))?;
	#[cfg(target_os = "windows")]
	::std::fs::remove_file(tmppath).ok();
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
    Ok(format!("dashmpdrs-tmp{}", suffix))
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
    audio_path: &str,
    video_path: &str) -> Result<(), DashMpdError> {
    let output_path = downloader.output_path.as_ref()
              .expect("muxer called without specifying output_path");
    let tmppath = temporary_outpath(".mkv")?;
    let mkv = Command::new(&downloader.mkvmerge_location)
        .args(["--output", &tmppath,
               "--no-video", audio_path,
               "--no-audio", video_path])
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mkvmerge subprocess")))?;
    if mkv.status.success() {
        let tmpfile = File::open(&tmppath)
            .map_err(|e| DashMpdError::Io(e, String::from("opening mkvmerge output")))?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path)
            .map_err(|e| DashMpdError::Io(e, String::from("opening output file")))?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .map_err(|e| DashMpdError::Io(e, String::from("copying mkvmerge output to output file")))?;
	#[cfg(target_os = "windows")]
	::std::fs::remove_file(tmppath).ok();
        Ok(())
    } else {
        // mkvmerge writes error messages to stdout, not to stderr
        let msg = String::from_utf8_lossy(&mkv.stdout);
        Err(DashMpdError::Muxing(format!("running mkvmerge: {msg}")))
    }
}


// First try ffmpeg subprocess, if that fails try vlc subprocess
pub fn mux_audio_video(
    downloader: &DashDownloader,
    audio_path: &str,
    video_path: &str) -> Result<(), DashMpdError> {
    log::trace!("Muxing audio {audio_path}, video {video_path}");
    let output_path = downloader.output_path.as_ref()
              .expect("muxer called without specifying output_path");
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    // TODO: should probably allow the user to specify this ordering preference
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
    log::info!("Muxer preference for {container} is {muxer_preference:?}");
    for muxer in muxer_preference {
        log::info!("Trying muxer {}", muxer);
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_audio_video_mkvmerge(downloader, audio_path, video_path) {
                log::warn!("Muxing with mkvmerge subprocess failed: {e}");
            } else {
                log::info!("Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_audio_video_ffmpeg(downloader, audio_path, video_path) {
                log::warn!("Muxing with ffmpeg subprocess failed: {e}");
            } else {
                log::info!("Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("vlc") {
            if let Err(e) = mux_audio_video_vlc(downloader, audio_path, video_path) {
                log::warn!("Muxing with vlc subprocess failed: {e}");
            } else {
                log::info!("Muxing with vlc subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("mp4box") {
            if let Err(e) = mux_audio_video_mp4box(downloader, audio_path, video_path) {
                log::warn!("Muxing with MP4Box subprocess failed: {e}");
            } else {
                log::info!("Muxing with MP4Box subprocess succeeded");
                return Ok(());
            }
        }
    }
    log::warn!("All available muxers failed");
    Err(DashMpdError::Muxing(String::from("all available muxers failed")))
}

