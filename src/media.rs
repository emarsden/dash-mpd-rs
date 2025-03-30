// Common code for media handling.
//
// This file contains functions used both by the external subprocess muxing in ffmpeg.rs and the
// libav muxing in libav.rs.


// When building with the libav feature, several functions here are unused.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use file_format::FileFormat;
use tracing::warn;
use crate::DashMpdError;
use crate::fetch::DashDownloader;


#[derive(Debug, Clone)]
pub struct AudioTrack {
    pub language: String,
    pub path: PathBuf,
}


// Returns "mp4", "mkv", "avi" etc. Based on analyzing the media content rather than on the filename
// extension.
#[tracing::instrument(level="trace")]
pub(crate) fn audio_container_type(container: &Path) -> Result<String, DashMpdError> {
    let format = FileFormat::from_file(container)
        .map_err(|e| DashMpdError::Io(e, String::from("determining audio container type")))?;
    Ok(format.extension().to_string())
}

#[tracing::instrument(level="trace")]
pub(crate) fn video_container_type(container: &Path) -> Result<String, DashMpdError> {
    let format = FileFormat::from_file(container)
        .map_err(|e| DashMpdError::Io(e, String::from("determining video container type")))?;
    Ok(format.extension().to_string())
}


// This is the metainformation that we need in order to determine whether two video streams can be
// concatenated using the ffmpeg concat filter.
#[derive(Debug, Clone)]
struct VideoMetainfo {
    width: i64,
    height: i64,
    frame_rate: f64,
    sar: Option<f64>,
}

impl PartialEq for VideoMetainfo {
    fn eq(&self, other: &Self) -> bool {
        if self.width != other.width {
            return false;
        }
        if self.height != other.height {
            return false;
        }
        if (self.frame_rate - other.frame_rate).abs() / self.frame_rate > 0.01 {
            return false;
        }
        // We tolerate missing information concerning the aspect ratio, because in practice it's not
        // always present in video metadata.
        if let Some(sar1) = self.sar {
            if let Some(sar2) = other.sar {
                if (sar1 - sar2).abs() / sar1 > 0.01 {
                    return false;
                }
            }
        }
        true
    }
}

// Frame rate as returned by ffprobe is a rational number serialized as "24/1" for example.
fn parse_frame_rate(s: &str) -> Option<f64> {
    if let Some((num, den)) = s.split_once('/') {
        if let Ok(numerator) = num.parse::<u64>() {
            if let Ok(denominator) = den.parse::<u64>() {
                return Some(numerator as f64 / denominator as f64);
            }
        }
    }
    None
}

// Aspect ratio as returned by ffprobe is a rational number serialized as "1:1" or "16:9" for example.
fn parse_aspect_ratio(s: &str) -> Option<f64> {
    if let Some((num, den)) = s.split_once(':') {
        if let Ok(numerator) = num.parse::<u64>() {
            if let Ok(denominator) = den.parse::<u64>() {
                return Some(numerator as f64 / denominator as f64);
            }
        }
    }
    None
}

// Return metainformation concerning the first stream of the media content at path.
// Uses ffprobe as a subprocess.
#[tracing::instrument(level="trace")]
fn video_container_metainfo(path: &PathBuf) -> Result<VideoMetainfo, DashMpdError> {
    match ffprobe::ffprobe(path) {
        Ok(meta) => {
            if meta.streams.is_empty() {
                return Err(DashMpdError::Muxing(String::from("reading video resolution")));
            }
            if let Some(s) = &meta.streams.iter().find(|s| s.width.is_some() && s.height.is_some()) {
                if let Some(frame_rate) = parse_frame_rate(&s.avg_frame_rate) {
                    let sar = s.sample_aspect_ratio.as_ref()
                        .and_then(|sr| parse_aspect_ratio(sr));
                    if let Some(width) = s.width {
                        if let Some(height) = s.height {
                            return Ok(VideoMetainfo { width, height, frame_rate, sar });
                        }
                    }
                }
            }
        },
        Err(e) => warn!("Error running ffprobe: {e}"),
    }
    Err(DashMpdError::Muxing(String::from("reading video metainformation")))
}

#[tracing::instrument(level="trace")]
pub(crate) fn container_only_audio(path: &PathBuf) -> bool {
    if let Ok(meta) =  ffprobe::ffprobe(path) {
        return meta.streams.iter().all(|s| s.codec_type.as_ref().is_some_and(|typ| typ.eq("audio")));
    }
    false
}


// Does the media container at path contain an audio track (separate from the video track)?
#[tracing::instrument(level="trace")]
pub(crate) fn container_has_audio(path: &PathBuf) -> bool {
    if let Ok(meta) =  ffprobe::ffprobe(path) {
        return meta.streams.iter().any(|s| s.codec_type.as_ref().is_some_and(|typ| typ.eq("audio")));
    }
    false
}

// Does the media container at path contain a video track?
#[tracing::instrument(level="trace")]
pub(crate) fn container_has_video(path: &PathBuf) -> bool {
    if let Ok(meta) =  ffprobe::ffprobe(path) {
        return meta.streams.iter().any(|s| s.codec_type.as_ref().is_some_and(|typ| typ.eq("video")));
    }
    false
}

// Can the video streams in these containers be merged together using the ffmpeg concat filter
// (concatenated, possibly reencoding if the codecs used are different)? They can if:
//   - they have identical resolutions, frame rate and aspect ratio
//   - they all only contain audio content
#[tracing::instrument(level="trace", skip(_downloader))]
pub(crate) fn video_containers_concatable(_downloader: &DashDownloader, paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    if let Some(p0) = &paths.first() {
        if let Ok(p0m) = video_container_metainfo(p0) {
            return paths.iter().all(
                |p| video_container_metainfo(p).is_ok_and(|m| m == p0m));
        }
    }
    paths.iter().all(container_only_audio)
}

// mkvmerge on Windows is compiled using MinGW and isn't able to handle native pathnames, so we
// create the temporary file in the current directory.
#[cfg(target_os = "windows")]
pub fn temporary_outpath(suffix: &str) -> Result<String, DashMpdError> {
    Ok(format!("dashmpdrs-tmp{suffix}"))
}

#[cfg(not(target_os = "windows"))]
pub fn temporary_outpath(suffix: &str) -> Result<String, DashMpdError> {
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

