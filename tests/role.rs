// Tests for the role_preference functionality
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test role -- --show-output
//
// https://testassets.dashif.org/#feature/details/5a1c4bd87cccbb15567e567e

pub mod common;
use fs_err as fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use test_log::test;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;


// This manifest has role=main and role=alternate but segments are timing out as of May 2024
// https://dash.akamaized.net/microsoft/multiple-adaptation-test.mpd


// This manifest has role=main (HEVC 10 bit) and role=alternate (HEVC 8 bit) streams in different
// AdaptationSets.
#[test(tokio::test)]
async fn test_role_main() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesIOP41/MultiTrack/alternative_content/2/manifest_alternative_content_ondemand.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("role-main.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .prefer_roles(vec!["main".to_string()])
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 189_121_991);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.pix_fmt, Some(String::from("yuv420p10le")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[test(tokio::test)]
async fn test_role_alternate() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesIOP41/MultiTrack/alternative_content/2/manifest_alternative_content_ondemand.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("role-alternate.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .prefer_roles(vec!["alternate".to_string(), "imaginary".to_string()])
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 196_041_016);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.pix_fmt, Some(String::from("yuv420p")));
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

