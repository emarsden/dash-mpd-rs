//! Support for extracting and converting subtitle tracks


use std::io;
use std::fs;
use std::process::Command;
use std::path::Path;
use tracing::{info, warn};
use crate::DashMpdError;
use crate::fetch::{DashDownloader, partial_process_output};


// Extract subtitles from an ISO BMFF/fMP4 WebVTT (wvtt) file and save them to a .vtt file in WebVTT
// format. Also convert the WebVTT file to SubRip (.srt) format. The subtitle files are named after
// the main media content file, with a different extension.
pub async fn wvtt_extract(downloader: &DashDownloader, subs_path: &Path) -> Result<(), DashMpdError> {
    let subs_path_str = subs_path.to_string_lossy();
    let vtt_path = downloader.output_path.as_ref()
        .ok_or_else(|| DashMpdError::Other(String::from("no output_path set")))?
        .with_extension("vtt");
    if downloader.verbosity > 0 {
        info!("  Extracting WebVTT subtitles to {}, using MP4Box", vtt_path.display());
    }
    // MP4Box -noprog -raw "0:output=output.vtt" input.mp4
    let mp4box_arg = format!("0:output={}", vtt_path.to_string_lossy());
    let verbosity = match downloader.verbosity {
        0 => "all@error",
        1 => "all@warning",
        2 => "all@info",
        _ => "all@debug",
    };
    let args = vec![
        "-logs", verbosity,
        "-noprog",
        "-raw", &mp4box_arg, &subs_path_str];
    if downloader.verbosity > 0 {
        info!("  Running MPBox {}", args.join(" "));
    }
    if let Ok(mp4box) = Command::new(&downloader.mp4box_location)
        .args(args)
        .output()
    {
        let msg = partial_process_output(&mp4box.stdout);
        if !msg.is_empty() {
            info!("MP4Box stdout: {msg}");
        }
        let msg = partial_process_output(&mp4box.stderr);
        if !msg.is_empty() {
            info!("MP4Box stderr: {msg}");
        }
        if mp4box.status.success() {
            info!("   Extracted subtitles in WebVTT format");
        } else {
            warn!("Error running MP4Box to extract subtitles");
            return Err(DashMpdError::Io(
                io::Error::other("running MP4Box"),
                String::from("")))
        }
    } else {
        return Err(DashMpdError::Io(
            io::Error::other("spawning MP4Box"),
            String::from("")))
    }
    convert_vtt_srt(downloader, &vtt_path).await
}


// Convert WebVTT subtitles to SubRip format.
//
// Neither the subtitler crate nor the rsubs-lib crates remove cue markup from the VTT inputs
// (markup of the form "<c.white>xxx</c>"). When MP4Box extracts subtitles directly to SubRip format
// (-srt commandline option) it also does not remove cue markup. The subtp crate looks abandoned. So
// we use the (videcoded) oxideav-subtitle crate.
pub async fn convert_vtt_srt(downloader: &DashDownloader, vtt_path: &Path) -> Result<(), DashMpdError> {
    if downloader.verbosity > 0 {
        info!("  Converting subtitles from VTT to SubRip format");
    }
    let srt_path = downloader.output_path.as_ref()
        .ok_or_else(|| DashMpdError::Other(String::from("no output_path set")))?
        .with_extension("srt");
    let vtt = fs::read(vtt_path)
        .map_err(|e| DashMpdError::Io(
            io::Error::other("reading extracted WebVTT subtitles"),
            format!("{e}")))?;
    let srt = oxideav_subtitle::transform::webvtt_to_srt(&vtt)
        .map_err(|e| DashMpdError::Io(
            io::Error::other("converting WebVTT subtitles to SubRip format"),
            format!("{e}")))?;
    fs::write(srt_path, srt)
        .map_err(|e| DashMpdError::Io(
            io::Error::other("writing SRT subtitles"),
            format!("{e}")))
}


// Convert TTML subtitles to SubRip format, using the oxideav-subtitle crate.
//
// The simpler subtitler crate is able to do this conversion, but it includes XML markup in the .srt
// file, so stay with the oxideav library.
pub async fn convert_ttml_srt(downloader: &DashDownloader, ttml_path: &Path) -> Result<(), DashMpdError> {
    use oxideav_subtitle::{srt, ttml};

    if downloader.verbosity > 0 {
        info!("  Converting subtitles from TTML to SubRip format");
    }
    let srt_path = downloader.output_path.as_ref()
        .ok_or_else(|| DashMpdError::Other(String::from("no output_path set")))?
        .with_extension("srt");
    let ttml_octets = fs::read(ttml_path)
        .map_err(|e| DashMpdError::Io(
            io::Error::other("reading TTML subtitles"),
            format!("{e}")))?;
    let track = ttml::parse(&ttml_octets)
        .map_err(|e| DashMpdError::Io(
            io::Error::other("parsing TTML subtitles"),
            format!("{e}")))?;
    let srt_octets = srt::write(&track);
    fs::write(srt_path, srt_octets)
        .map_err(|e| DashMpdError::Io(
            io::Error::other("writing SRT subtitles"),
            format!("{e}")))
}

