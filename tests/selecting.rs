//! Selecting audio/video streams depending on video resolution/quality
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test selecting -- --show-output

pub mod common;
use std::fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, setup_logging};


// The bitmovin-a.akamaihd.net CDN is sending video segments with an incorrect Content-type header,
// so we need to ignore that in all the tests here.
//
// Apparently same streams are available from
//    https://cdn.bitmovin.com/content/assets/art-of-motion-dash-hls-progressive/mpds/f08e80da-bf1d-4e3d-8899-f0f6155f6efa.mpd


// TODO find another test manifest; this one has multiple periods

#[tokio::test]
async fn test_video_resolution_minq() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-minq.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(360, video.height.unwrap());
    assert_eq!(video.pix_fmt, Some(String::from("yuv420p")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_360p() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-360p.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .prefer_video_height(360)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(360, video.height.unwrap());
    assert_eq!(video.pix_fmt, Some(String::from("yuv420p")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_w1280() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-w1280.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .prefer_video_width(1280)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 16_481_175);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(1280, video.width.unwrap());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// This manifest has video at three resolutions; we want to check that with .intermediate_quality()
// we select the middle one.
#[tokio::test]
async fn test_video_resolution_qintermediate() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/2b/qualcomm/1/MultiResMPEG2.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-qintermediate.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .intermediate_quality()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 112_423_976);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(768, video.width.unwrap());
    assert_eq!(432, video.height.unwrap());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_2160p() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-2160p.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .prefer_video_height(2160)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 36_411_820);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(2160, video.height.unwrap());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}
