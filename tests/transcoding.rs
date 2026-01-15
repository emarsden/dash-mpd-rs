// Tests for MPD download/transcoding support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test transcoding -- --show-output

pub mod common;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use pretty_assertions::assert_eq;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, setup_logging};


// We can't check file size for this test, as depending on whether mkvmerge or ffmpeg or mp4box are
// used to copy the video stream into the Matroska container (depending on which one is installed),
// the output file size varies quite a lot.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_transcode_mkv() {
    setup_logging();
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cf.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(3)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_transcode_webm() {
    setup_logging();
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cf.webm");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    // The file size is unreliable: in 2026-01 has chagned to 410218 octets...
    // check_file_size_approx(&out, 69_243);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Webm);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_transcode_avi() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cf.avi");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 513_308);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::AudioVideoInterleave);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_transcode_av1() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // from demo page at https://bitmovin.com/demos/av1
    let mpd_url = "https://storage.googleapis.com/bitmovin-demos/av1/stream.mpd";
    let out = env::temp_dir().join("mango.webm");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("webm", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 12_987_188);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    // The order of streams in the WebM container is unreliable.
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("opus")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("av1")));
    assert!(video.width.is_some());
}


// Test transcoding audio from mp4a/aac to Ogg Vorbis
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_transcode_audio_vorbis() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/3a/fraunhofer/aac-lc_stereo_without_video/Sintel/sintel_audio_only_aaclc_stereo_sidx.mpd";
    let out = env::temp_dir().join("sintel-audio.ogg");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_880_500);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::OggVorbis);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("vorbis")));
}

// Test transcoding multiperiod audio from mp4a/aac to MP3
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_transcode_audio_multiperiod_mp3() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://media.axprod.net/TestVectors/v7-Clear/Manifest_MultiPeriod_AudioOnly.mpd";
    let out = env::temp_dir().join("multiperiod-audio.mp3");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 23_362_703);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg12AudioLayer3);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("mp3")));
}


