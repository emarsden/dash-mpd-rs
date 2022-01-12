/// Muxing support using ffmpeg as a subprocess.
///
/// Also see the alternative method of using ffmpeg via the shared library API, implemented in file
/// "libav.rs".


use std::fs::File;
use std::path::Path;
use std::io;
use std::io::{BufReader, BufWriter};
use std::process::Command;
use tempfile::NamedTempFile;
use anyhow::{Result, Context, anyhow};


fn mux_audio_video_ffmpeg(audio_path: &str, video_path: &str, output_path: &Path) -> Result<()> {
    let tmpout = NamedTempFile::new()
       .context("creating temporary output file")?;
    let tmppath = tmpout.path().to_str()
       .context("obtaining name of temporary file")?;
    let ffmpeg = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-i", audio_path,
               "-i", video_path,
               "-c:v", "copy", "-c:a", "copy",
               // select the mp4 muxer explicitly (output_path doesn't necessarily have a .mp4 extension)
               "-f", "mp4",
               tmppath])
        .output()
        .context("running ffmpeg subprocess")?;
    if ffmpeg.status.success() {
        let tmpfile = File::open(&tmppath).context("opening ffmpeg output")?;
        let mut muxed = BufReader::new(tmpfile);
        let outfile = File::create(output_path).context("creating output file")?;
        let mut sink = BufWriter::new(outfile);
        io::copy(&mut muxed, &mut sink)
            .context("copying ffmpeg output to output file")?;
        Ok(())
    } else {
        let msg = String::from_utf8(ffmpeg.stderr)?;
        Err(anyhow!("Failure running ffmpeg: {}", msg))
    }
}


// See https://wiki.videolan.org/Transcode/
fn mux_audio_video_vlc(audio_path: &str, video_path: &str, output_path: &Path) -> Result<()> {
    let tmpout = NamedTempFile::new()
       .context("creating temporary output file")?;
    let tmppath = tmpout.path().to_str()
       .context("obtaining name of temporary file")?;
    let vlc = Command::new("vlc")
        .args(["--no-repeat", "--no-loop", "-I", "dummy",
               audio_path, video_path,
               "--sout-keep",
               &format!("--sout=#gather:transcode{{{}}}:standard{{access=file,mux=mp4,dst={}}}",
                       "vcodec=h264,vb=1024,scale=1,acodec=mp4a",
                       tmppath),
               "vlc://quit"])
        .output()
        .context("running VLC subprocess")?;
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


// First try ffmpeg subprocess, if that fails try vlc subprocess
pub fn mux_audio_video(audio_path: &str, video_path: &str, output_path: &Path) -> Result<()> {
    // eprintln!("Muxing audio {}, video {}", audio_path, video_path);
    if let Err(e) = mux_audio_video_ffmpeg(audio_path, video_path, output_path) {
        log::warn!("Muxing with ffmpeg subprocess failed: {}", e);
        log::info!("Retrying mux with vlc subprocess");
        if let Err(e) = mux_audio_video_vlc(audio_path, video_path, output_path) {
            log::warn!("Muxing with vlc subprocess failed: {}", e);
            Err(e)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}

