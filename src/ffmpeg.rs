/// Muxing support using ffmpeg as a subprocess.
///
/// Also see the alternative method of using ffmpeg via the shared library API, implemented in file
/// "libav.rs".


use std::fs;
use std::process::Command;
use anyhow::{Result, Context, anyhow};


fn mux_audio_video_ffmpeg(audio_path: &str, video_path: &str, output_path: &str) -> Result<()> {
    let ffmpeg = Command::new("ffmpeg")
        .env_clear() 
        .args(["-hide_banner", "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-i", audio_path,
               "-i", video_path,
               "-c:v", "copy", "-c:a", "copy",
               // select the mp4 muxer explicitly (output_path doesn't necessarily have a .mp4 extension)
               "-f", "mp4",
               output_path])
        .output()
        .context("couldn't run ffmpeg subprocess")?;
    if ffmpeg.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8(ffmpeg.stderr)?;
        Err(anyhow!("Failure running ffmpeg: {}", msg))
    }
}


// See https://wiki.videolan.org/Transcode/
fn mux_audio_video_vlc(audio_path: &str, video_path: &str, output_path: &str) -> Result<()> {
    let vlc = Command::new("vlc")
        .env_clear()
        .args(["--no-repeat", "--no-loop", "-I", "dummy",
               audio_path, video_path,
               "--sout-keep",
               &format!("--sout=#gather:transcode{{{}}}:standard{{access=file,mux=mp4,dst={}}}",
                       "vcodec=h264,vb=1024,scale=1,acodec=mp4a",
                       output_path),
               "vlc://quit"])
        .output()
        .context("couldn't run vlc subprocess")?;
    if vlc.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8(vlc.stderr)?;
        Err(anyhow!("Failure running vlc: {}", msg))
    }
}


// First try ffmpeg subprocess, if that fails try vlc subprocess
pub fn mux_audio_video(audio_path: &str, video_path: &str, output_path: &str) -> Result<()> {
    // eprintln!("Muxing audio {}, video {}", audio_path, video_path);
    if let Err(e) = mux_audio_video_ffmpeg(audio_path, video_path, output_path) {
        log::info!("Muxing with ffmpeg subprocess failed: {}", e);
        log::info!("Retrying mux with vlc subprocess");
        if fs::remove_file(output_path).is_err() {
            // ffmpeg mux attempt didn't create any output
        }
        mux_audio_video_vlc(audio_path, video_path, output_path)
    } else {
        Ok(())
    }
}

