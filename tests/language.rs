// Tests for language preferences in download support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test language -- --show-output
//

pub mod common;
use fs_err as fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use test_log::test;
use pretty_assertions::assert_eq;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;



#[test(tokio::test)]
async fn test_lang_prefer_spa() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://refapp.hbbtv.org/videos/02_gran_dillama_1080p_ma_25f75g6sv5/manifest.mpd";
    let out = env::temp_dir().join("dillama-spa.mp4");
    if out.exists() {
        let _ = fs::remove_file(out.clone());
    }
    DashDownloader::new(mpd_url)
        .worst_quality()
        .max_error_count(5)
        .record_metainformation(true)
        .prefer_language(String::from("spa"))
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 11_809_117);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let tags = stream.tags.as_ref().unwrap();
    assert_eq!(tags.language, Some(String::from("spa")));
    let _ = fs::remove_file(out);
}


#[test(tokio::test)]
async fn test_subtitle_lang_stpp_im1t() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "http://rdmedia.bbc.co.uk/testcard/vod/manifests/avc-mobile.mpd";
    let outpath = env::temp_dir().join("im1t-subs.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(outpath.clone());
    }
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .prefer_language(String::from("fra"))
        .verbosity(2)
        .download_to(outpath.clone()).await
        .unwrap();
    let meta = ffprobe(outpath.clone()).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let audio = &meta.streams[1];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let tags = audio.tags.as_ref().unwrap();
    assert_eq!(tags.language, Some(String::from("fra")));
    let stpp = &meta.streams[2];
    assert_eq!(stpp.codec_tag_string, "stpp");
    let subtags = stpp.tags.as_ref().unwrap();
    assert_eq!(subtags.language, Some(String::from("fra")));
    let duration = stpp.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!((3598.0 < duration) && (duration < 3599.0));
    let _ = fs::remove_file(outpath);
}
