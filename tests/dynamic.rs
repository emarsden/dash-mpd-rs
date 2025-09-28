// Tests for downloading from dynamic MPD streams
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test dynamic -- --show-output
//
//
// Note that we can't really do file size checks on dynamic streams, because depending on the
// streamed content when we are downloading the compressed size may differ.

pub mod common;
use fs_err as fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_media_duration, setup_logging};


// This is a "pseudo-live" stream, a dynamic MPD manifest for which all media segments are already
// available at the time of download. Though we are not able to correctly download a genuinely live
// stream (we don't implement the clock functionality needed to wait until segments become
// progressively available), we are able to download pseudo-live stream if the
// allow_live_streaming() method is enabled.
#[tokio::test]
async fn test_dl_dynamic_stream() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://livesim2.dashif.org/livesim2/segtimeline_1/testpic_2s/Manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-manifest.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A second dynamic (live) stream to test.
#[tokio::test]
async fn test_dl_dynamic_vos360() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cdn-vos-ppp-01.vos360.video/Content/DASH_DASHCLEAR2/Live/channel(PPP-LL-2DASH)/master.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-vos360.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A third dynamic (live) stream to test. Disabled because the received content length is unreliable.
// #[ignore]
#[tokio::test]
async fn test_dl_dynamic_5cents() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://wj78dp5elq5r-hls-live.5centscdn.com/72_push_4276_001/streamtester/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-5cents.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .without_content_type_checks()
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// This is a really live stream, for which we only download a certain number of seconds.
// Only a small download, so we can run it on CI infrastructure.
#[tokio::test]
async fn test_dl_dynamic_forced_duration() {
    setup_logging();
    let mpd_url = "https://livesim2.dashif.org/livesim2/ato_inf/testpic_2s/Manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-6s.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .allow_live_streams(true)
        .force_duration(6.5)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(640));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    // FIXME we are seeing 1 here
    check_media_duration(&out, 6.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_dl_lowlatency_forced_duration() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://akamaibroadcasteruseast.akamaized.net/cmaf/live/657078/akasource/out.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-11s-ll.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .force_duration(11.0)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(1280));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    check_media_duration(&out, 11.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// More BBC test streams at http://avcatalogue.cloud.bbc.co.uk/
//
// Possible alternatives:
//  served by Akamai: http://a.files.bbci.co.uk/media/live/manifesto/audio/simulcast/dash/nonuk/dash_low/ak/bbc_world_service.mpd
//  served by Cloudfront: http://a.files.bbci.co.uk/ms6/live/3441A116-B12E-4D2F-ACA8-C1984642FA4B/audio/simulcast/dash/nonuk/pc_hd_abr_v2/cf/bbc_radio_one.mpd
#[tokio::test]
async fn test_dl_bbcws_dynamic() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://a.files.bbci.co.uk/ms6/live/3441A116-B12E-4D2F-ACA8-C1984642FA4B/audio/simulcast/dash/nonuk/pc_hd_abr_v2/ak/bbc_world_service.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-bbcws.mp4");
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.10 Safari/605.1.1")
        .build()
        .expect("creating HTTP client");
    DashDownloader::new(mpd_url)
        .with_http_client(client)
        .worst_quality()
        .allow_live_streams(true)
        .audio_only()
        .force_duration(25.0)
        .sleep_between_requests(7)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Audio);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    check_media_duration(&out, 25.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}






