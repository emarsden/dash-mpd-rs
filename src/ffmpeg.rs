/// Muxing support using ffmpeg as a subprocess.
///
/// Also see the alternative method of using ffmpeg via the shared library API, implemented in file
/// "libav.rs".


use std::process::Command;
use anyhow::{Result, anyhow};


pub fn mux_audio_video(audio_path: &str, video_path: &str, output_path: &str) -> Result<()> {
    let ffmpeg = Command::new("ffmpeg")
        .env_clear() 
        .args(["-hide_banner", "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-i", audio_path,
               "-i", video_path, 
               "-c:v", "copy", "-c:a", "copy",
               output_path])
        .output()
        .expect("couldn't run ffmpeg subprocess");
    if ffmpeg.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8(ffmpeg.stderr)?;
        Err(anyhow!("Failure running ffmpeg: {}", msg))
    }
}

