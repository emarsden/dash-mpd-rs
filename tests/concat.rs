// Tests for Period concatenation with multi-period manifests
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test concat -- --show-output

pub mod common;
use fs_err as fs;
use std::env;
use std::time::Duration;
use ffprobe::ffprobe;
use file_format::FileFormat;
use test_log::test;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_noaudio() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_781_840);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_singleases() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/singleases.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-singleases.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .minimum_period_duration(Duration::new(10, 0))
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_781_840);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 42_060);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_p1p2() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio_p1p2.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        // here we should be dropping period #3 (id=2) whose duration is 0.701s
        .minimum_period_duration(Duration::new(2, 0))
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 33_967);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


