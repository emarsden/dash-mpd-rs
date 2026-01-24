// Selecting audio/video streams depending on video resolution/quality
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


#[tokio::test]
async fn test_video_resolution_minq() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://bitmovin-a.akamaihd.net/content/MI201109210084_1/mpds/f08e80da-bf1d-4e3d-8899-f0f6155f6efa.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-minq.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 10_213_437);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(180, video.height.unwrap());
    assert_eq!(video.pix_fmt, Some(String::from("yuv420p")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_180p() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://bitmovin-a.akamaihd.net/content/MI201109210084_1/mpds/f08e80da-bf1d-4e3d-8899-f0f6155f6efa.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-180p.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .prefer_video_height(180)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 10_213_437);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(180, video.height.unwrap());
    assert_eq!(video.pix_fmt, Some(String::from("yuv420p")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_w320() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://bitmovin-a.akamaihd.net/content/MI201109210084_1/mpds/f08e80da-bf1d-4e3d-8899-f0f6155f6efa.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-w320.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .prefer_video_width(320)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 10_213_437);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(320, video.width.unwrap());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_qintermediate() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://bitmovin-a.akamaihd.net/content/MI201109210084_1/mpds/f08e80da-bf1d-4e3d-8899-f0f6155f6efa.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-qintermediate.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .intermediate_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 35_726_813);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(960, video.width.unwrap());
    assert_eq!(540, video.height.unwrap());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_video_resolution_1080p() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://bitmovin-a.akamaihd.net/content/MI201109210084_1/mpds/f08e80da-bf1d-4e3d-8899-f0f6155f6efa.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("selecting-1080p.mp4");
    DashDownloader::new(mpd_url)
        .without_content_type_checks()
        .prefer_video_height(1080)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 130_912_028);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(1080, video.height.unwrap());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}
