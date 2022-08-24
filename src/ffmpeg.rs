/// Muxing support using mkvmerge/ffmpeg/vlc as a subprocess.
///
/// Also see the alternative method of using ffmpeg via its "libav" shared library API, implemented
/// in file "libav.rs".


use std::fs::File;
use std::path::Path;
use std::io;
use std::io::{BufReader, BufWriter};
use std::process::Command;
use anyhow::{Result, Context, anyhow};


// ffmpeg can mux to many container types including mp4, mkv, avi
fn mux_audio_video_ffmpeg(
    audio_path: &str,
    video_path: &str,
    output_path: &Path,
    ffmpeg_location: Option<String>) -> Result<()> {
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    let tmpout = tempfile::Builder::new()
        .prefix("dashmpdrs")
        .suffix(&format!(".{}", container))
        .rand_bytes(5)
        .tempfile()
        .context("creating temporary output file")?;
    let tmppath = tmpout.path().to_str()
        .context("obtaining name of temporary file")?;
    let mut ffmpeg_binary = "ffmpeg".to_string();
    if let Some(loc) = ffmpeg_location {
        ffmpeg_binary = loc;
    }
    let ffmpeg = Command::new(ffmpeg_binary)
        .args(["-hide_banner",
               "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-i", audio_path,
               "-i", video_path,
               "-c:v", "copy", "-c:a", "copy",
               "-movflags", "+faststart", "-preset", "veryfast",
               // select the muxer explicitly
               "-f", container,
               tmppath])
        .output()
        .context("spawning ffmpeg subprocess")?;
    let msg = String::from_utf8_lossy(&ffmpeg.stdout);
    if msg.len() > 0 {
        log::info!("ffmpeg stdout: {}", msg);
    }
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    if msg.len() > 0 {
        log::info!("ffmpeg stderr: {}", msg);
    }
    if ffmpeg.status.success() {
        let tmpfile = File::open(&tmppath).context("opening ffmpeg output")?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path).context("creating output file")?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .context("copying ffmpeg output to output file")?;
        Ok(())
    } else {
        Err(anyhow!("Failure running ffmpeg"))
    }
}


// See https://wiki.videolan.org/Transcode/
// VLC could also mux to an mkv container if needed
fn mux_audio_video_vlc(audio_path: &str, video_path: &str, output_path: &Path) -> Result<()> {
    let tmpout = tempfile::Builder::new()
       .prefix("dashmpdrs")
       .suffix(".mp4")
       .rand_bytes(5)
       .tempfile()
       .context("creating temporary output file")?;
    let tmppath = tmpout.path().to_str()
       .context("obtaining name of temporary file")?;
    let vlc = Command::new("vlc")
        .args(["-I", "dummy",
               "--no-repeat", "--no-loop",
               video_path,
               "--input-slave", audio_path,
               "--sout-mp4-faststart",
               &format!("--sout=#std{{access=file,mux=mp4,dst={}}}", tmppath),
               "--sout-keep",
               "vlc://quit"])
        .output()
        .context("spawning VLC subprocess")?;
    if vlc.status.success() {
        let tmpfile = File::open(&tmppath).context("opening VLC output")?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path).context("creating output file")?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .context("copying VLC output to output file")?;
        Ok(())
    } else {
        let msg = String::from_utf8(vlc.stderr)?;
        Err(anyhow!("Failure running vlc: {}", msg))
    }
}


// mkvmerge on Windows is compiled using MinGW and isn't able to handle native pathnames, so we
// create the temporary file in the current directory. 
#[cfg(target_os = "windows")]
fn temporary_outpath(suffix: &str) -> Result<String> {
    Ok((format!("dashmpdrs-tmp{}", suffix)))
}

#[cfg(not(target_os = "windows"))]
fn temporary_outpath(suffix: &str) -> Result<String> {
    let tmpout = tempfile::Builder::new()
       .prefix("dashmpdrs")
       .suffix(suffix)
       .rand_bytes(5)
       .tempfile()
       .context("creating temporary output file")?;
    match tmpout.path().to_str() {
        Some(s) => Ok(s.to_string()),
        None => Ok(format!("/tmp/dashmpdrs-tmp{}", suffix)),
    }
}

fn mux_audio_video_mkvmerge(audio_path: &str, video_path: &str, output_path: &Path) -> Result<()> {
    let tmppath = temporary_outpath(".mkv")?;
    let mkv = Command::new("mkvmerge")
        .args(["--output", &tmppath,
               "--no-video", audio_path,
               "--no-audio", video_path])
        .output()
        .context("spawning mkvmerge subprocess")?;
    if mkv.status.success() {
        let tmpfile = File::open(&tmppath).context("opening mkvmerge output")?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path).context("creating output file")?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .context("copying mkvmerge output to output file")?;
	#[cfg(target_os = "windows")]
	::std::fs::remove_file(tmppath).ok();
        Ok(())
    } else {
        // mkvmerge writes error messages to stdout, not to stderr
        let msg = String::from_utf8(mkv.stdout)?;
        Err(anyhow!("Failure running mkvmerge: {}", msg))
    }
}


// First try ffmpeg subprocess, if that fails try vlc subprocess
pub fn mux_audio_video(
    audio_path: &str,
    video_path: &str,
    output_path: &Path,
    ffmpeg_location: Option<String>) -> Result<()> {
    log::trace!("Muxing audio {}, video {}", audio_path, video_path);
    let container = match output_path.extension() {
        Some(ext) => ext.to_str().unwrap_or("mp4"),
        None => "mp4",
    };
    let mut muxer_preference = vec![];
    if container.eq("mkv") {
        muxer_preference.push("mkvmerge");
        muxer_preference.push("ffmpeg");
    } else if container.eq("mp4") {
        muxer_preference.push("ffmpeg");
        muxer_preference.push("vlc");
    } else {
        muxer_preference.push("ffmpeg");
    }
    log::info!("Muxer preference for {} is {:?}", container, muxer_preference);
    for muxer in muxer_preference {
        log::info!("Trying muxer {}", muxer);
        if muxer.eq("mkvmerge") {
            if let Err(e) =  mux_audio_video_mkvmerge(audio_path, video_path, output_path) {
                log::warn!("Muxing with mkvmerge subprocess failed: {}", e);
            } else {
                log::info!("Muxing with mkvmerge subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("ffmpeg") {
            if let Err(e) = mux_audio_video_ffmpeg(audio_path, video_path, output_path, ffmpeg_location.clone()) {
                log::warn!("Muxing with ffmpeg subprocess failed: {}", e);
            } else {
                log::info!("Muxing with ffmpeg subprocess succeeded");
                return Ok(());
            }
        } else if muxer.eq("vlc") {
            if let Err(e) = mux_audio_video_vlc(audio_path, video_path, output_path) {
                log::warn!("Muxing with vlc subprocess failed: {}", e);
            } else {
                log::info!("Muxing with vlc subprocess succeeded");
                return Ok(());
            }
        }
    }
    log::warn!("All available muxers failed");
    Err(anyhow!("All available muxers failed"))
}

