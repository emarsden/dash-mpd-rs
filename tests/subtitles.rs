//! Tests for subtitle support
//
// We can run these tests on CI infrastructure because they are only downloading modest quantites
// of data, corresponding to the subtitle files/MP4 fragments. This requires MP4Box (from GPAC) to
// be installed on CI machines, however.
//
//
// To run this test while enabling printing to stdout/stderr
//
//    TEST_PERSIST_FILES=1 cargo test --test subtitles --jobs 1 -- --show-output --test-threads=1



pub mod common;
use std::fs;
use std::env;
use std::path::Path;
use std::process::Command;
use ffprobe::ffprobe;
use file_format::FileFormat;
use pretty_assertions::assert_eq;
use dash_mpd::fetch::DashDownloader;
use common::{check_media_duration, setup_logging};


// This manifest includes subtitles in WVTT (WebVTT) format. We check that these are downloaded to
// the output path with a ".wvtt" extension. Also check that the subtitles are successfully
// converted to SRT format, which is more widely supported, in a file named like the output with a
// ".srt" extension.
//
// Note that these tests will fail if MP4Box (from GPAC) is not installed. MP4Box is used for the
// conversion to SRT format.
#[tokio::test]
async fn test_subtitles_wvtt_defaultlang () {
    setup_logging();
    let mpd = "https://storage.googleapis.com/shaka-demo-assets/sintel-mp4-wvtt/dash.mpd";
    let outpath = env::temp_dir().join("sintel-wvtt-defaultlang.mp4");
    let mut subpath_wvtt = outpath.clone();
    subpath_wvtt.set_extension("wvtt");
    let subpath_wvtt = Path::new(&subpath_wvtt);
    let mut subpath_srt = outpath.clone();
    subpath_srt.set_extension("srt");
    let subpath_srt = Path::new(&subpath_srt);
    // First download the subtitles without specifying a preferred language, which means the first
    // one present in the manifest is downloaded (in this case it is in Dutch).
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .verbosity(2)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath_wvtt).is_ok());
    assert!(fs::metadata(subpath_srt).is_ok());
    // let format = FileFormat::from_file(subpath_wvtt).unwrap();
    // For some reason, the file-format crate is not detecting this format correctly (it detects the
    // more generic Mpeg4Part14Subtitles type).
    // assert_eq!(format, FileFormat::WebVideoTextTracks);
    let format = FileFormat::from_file(subpath_srt).unwrap();
    assert_eq!(format, FileFormat::SubripText);
    let srt = fs::read_to_string(subpath_srt).unwrap();
    assert!(srt.contains("land van de poortwachters"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(subpath_wvtt);
        let _ = fs::remove_file(subpath_srt);
        let _ = fs::remove_file(&outpath);
    }
}


#[tokio::test]
async fn test_subtitles_wvtt_en () {
    setup_logging();
    let mpd = "https://storage.googleapis.com/shaka-demo-assets/sintel-mp4-wvtt/dash.mpd";
    let outpath = env::temp_dir().join("sintel-wvtt-en.mp4");
    let mut subpath_wvtt = outpath.clone();
    subpath_wvtt.set_extension("wvtt");
    let subpath_wvtt = Path::new(&subpath_wvtt);
    let mut subpath_srt = outpath.clone();
    subpath_srt.set_extension("srt");
    let subpath_srt = Path::new(&subpath_srt);
    // Download the english subtitles and check that we got the expected content.
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_language(String::from("eng"))
        .download_to(&outpath).await
        .unwrap();
    let srt = fs::read_to_string(subpath_srt).unwrap();
    assert!(srt.contains("land of the gatekeepers"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath_wvtt);
        let _ = fs::remove_file(subpath_srt);
    }
}


// fragmented text/vtt subtitles using SegmentTemplate>SegmentTimeline addressing
#[tokio::test]
async fn test_subtitles_vtt_fragments () {
    setup_logging();
    let mpd = "https://gcp.emea-free.prd.media.max.com/global/6703e8d6-055c-5897-9006-d6d75f11e4b8/11_dbbb02_fallback.mpd";
    let outpath = env::temp_dir().join("cnn-subs.mp4");
    let mut subpath_vtt = outpath.clone();
    subpath_vtt.set_extension("vtt");
    let subpath_vtt = Path::new(&subpath_vtt);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .verbosity(1)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath_vtt).is_ok());
    let format = FileFormat::from_file(subpath_vtt).unwrap();
    assert_eq!(format, FileFormat::WebVideoTextTracks);
    let vtt = fs::read_to_string(subpath_vtt).unwrap();
    assert!(vtt.contains("unhinged behavior"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(&outpath);
        let _ = fs::remove_file(subpath_vtt);
    }
}


// This manifest contains two TTML "sidecars" (an AdaptationSet with contentType=text and
// mimeType=application/ttml+xml and BaseURL addressing). We start by downloading the default
// subtitles (the first to appear in the MPD file, which are in English here), then request
// explicitly the German subtitles.
#[tokio::test]
async fn test_subtitles_ttml_sidecar () {
    setup_logging();
    let mpd = "https://dash.akamaized.net/dash264/TestCases/4b/qualcomm/2/TearsOfSteel_onDem5secSegSubTitles.mpd";
    let outpath = env::temp_dir().join("ttml-tears-of-steel.mp4");
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    // We didn't specify a preferred language, so the first available one in the manifest (here
    // English) is downloaded.
    assert!(ttml.contains("You're a jerk"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(&outpath);
        let _ = fs::remove_file(subpath);
    }

    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_language(String::from("de"))
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let ttml = fs::read_to_string(subpath).unwrap();
    // This time we requested German subtitles.
    assert!(ttml.contains("Du bist ein Vollidiot"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


// We can run this on CI infrastructure because it's only downloading a modest amount of subtitle
// segments.
#[tokio::test]
async fn test_subtitles_vtt () {
    setup_logging();
    let mpd = "http://dash.edgesuite.net/akamai/test/caption_test/ElephantsDream/elephants_dream_480p_heaac5_1.mpd";
    let outpath = env::temp_dir().join("vtt-elephants-dream.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("vtt");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .verbosity(2)
        .prefer_language(String::from("de"))
        .download_to(&outpath).await
        .unwrap();
    let meta = fs::metadata(subpath).unwrap();
    assert!(meta.len() > 0);
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::WebVideoTextTracks);
    // This manifest contains a single subtitle track, available in VTT format via BaseURL addressing.
    let vtt = fs::read_to_string(subpath).unwrap();
    assert!(vtt.contains("Hurry Emo!"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


#[tokio::test]
async fn test_subtitles_stpp() {
    setup_logging();
    let mpd = "https://rdmedia.bbc.co.uk/elephants_dream/1/client_manifest-all.mpd";
    let outpath = env::temp_dir().join("stpp-elephants-dream-bbc.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .verbosity(2)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("just for you Proog."));
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let stpp = &meta.streams[2];
    assert_eq!(stpp.codec_tag_string, "stpp");
    check_media_duration(&outpath, 632.0);
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


// In addition to checking that we can extract TTML from the STPP subtitle track with the expected
// language, check that the TTML is also converted to SubRip format.
#[tokio::test]
async fn test_subtitles_stpp_multilang() {
    setup_logging();
    let mpd = "https://livesim2.dashif.org/vod/testpic_2s/multi_subs.mpd";
    let outpath = env::temp_dir().join("stpp-lang-swe.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath_ttml = outpath.clone();
    subpath_ttml.set_extension("ttml");
    let mut subpath_srt = outpath.clone();
    subpath_srt.set_extension("srt");
    let subpath_ttml = Path::new(&subpath_ttml);
    let subpath_srt = Path::new(&subpath_srt);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .prefer_subtitle_language(String::from("sv"))
        .verbosity(2)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath_ttml).is_ok());
    assert!(fs::metadata(subpath_srt).is_ok());
    let format = FileFormat::from_file(subpath_ttml).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let format = FileFormat::from_file(subpath_srt).unwrap();
    assert_eq!(format, FileFormat::SubripText);
    let ttml = fs::read_to_string(subpath_ttml).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("swe : 00:00:49.000"));
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let stpp = &meta.streams[2];
    assert_eq!(stpp.codec_tag_string, "stpp");
    check_media_duration(&outpath, 632.0);
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath_ttml);
        let _ = fs::remove_file(subpath_srt);
    }
}


// Image-based (IMSC1 CMAF) STPP subtitles (codec = stpp.ttml.im1i). We don't have support for
// extracting the content from these subtitles, so the associated .srt file is empty.
#[tokio::test]
async fn test_subtitles_stpp_imsc1() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://livesim.dashif.org/dash/vod/testpic_2s/imsc1_img.mpd";
    let outpath = env::temp_dir().join("stpp-imsc1-subs.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&outpath).await
        .unwrap();
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let stpp = &meta.streams[2];
    // In the MPD it's specified as stpp.ttml.im1i.
    assert_eq!(stpp.codec_tag_string, "stpp");
    check_media_duration(&outpath, 60.0);
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
    }
}


// MPEG-4 Part 17 (Timed Text), called "mov_text" in ffmpeg.
#[tokio::test]
async fn test_subtitles_tx3g() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mp4-live-subtitle/mp4-live-subtitle-mpd-AVST.mpd";
    let outpath = env::temp_dir().join("tx3g.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("srt");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::SubripText);
    let srt = fs::read_to_string(subpath).unwrap();
    assert!(srt.contains("Cue #3 Start Time"));
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 2);
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


#[tokio::test]
async fn test_subtitles_usp_ttml_sidecar() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest contains a TTML "sidecar" (an AdaptationSet with contentType=text and
    // mimeType=application/ttml+xml and BaseURL addressing
    let mpd = "https://demo.unified-streaming.com/k8s/features/stable/no-handler-origin/tears-of-steel/tears-of-steel-ttml-sidecar.mpd";
    let outpath = env::temp_dir().join("ttml-sidecar-fr.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_subtitle_language(String::from("fr"))
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("vos culs"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


#[tokio::test]
async fn test_subtitles_usp_ttml_fmp4() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest contains fragmented TTML subs (AdaptationSets with contentType=text and
    // mimeType=application/mp4 with stpp.ttml.im1t codec and .m4s fragments)
    let mpd = "https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel-ttml.ism/.mpd";
    let outpath = env::temp_dir().join("ttml-fragmented-fr.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_subtitle_language(String::from("fr"))
        .without_content_type_checks()
        .verbosity(1)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("vos culs"));
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 1);
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}



// This manifest contains fragmented TTML subtitles and captions (labeled hard of hearing). It
// has AdaptationSets with contentType=text and mimeType=application/mp4 with stpp.ttml.im1t
// codec and .m4s fragments and uses SegmentTemplate>SegmentTimeline addressing.
#[tokio::test]
async fn test_subtitles_usp_ttml_hoh() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel-hoh-subs.ism/.mpd";
    let outpath = env::temp_dir().join("ttml-hoh-fr.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_subtitle_language(String::from("fr"))
        .without_content_type_checks()
        .verbosity(1)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("vos culs"));
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 1);
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}



// FIXME here we have an stpp.ttml.etd1|im1t subtitle track, but we are retrieving no useful content from it

// Useful MP4 box analysis tool: https://media-analyzer.pro/analyzer
#[tokio::test]
async fn test_subtitles_bbc_lowlat_sthdbox() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://rdmedia.bbc.co.uk/testcard/lowlatency/manifests/ll-hevc-ctv-stereo-en.mpd";
    let outpath = env::temp_dir().join("ttml-bbc-lowlat.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .allow_live_streams(true)
        .force_duration(55.0)
        .verbosity(1)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    let meta = ffprobe(&outpath).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let meta = ffprobe(&outpath).unwrap();
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


// Manifest from https://refapp.hbbtv.org/videos/index_2019.html
// In-band TTML subs using STPP codec in fragmented MP4 segments.
#[tokio::test]
async fn test_subtitles_hbbtv_dillama_subib() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/02_gran_dillama_1080p_25f75g6sv5/manifest_subib.mpd";
    let outpath = env::temp_dir().join("ttml-hbbtv-dillama-subib.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .without_content_type_checks()
        .verbosity(1)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("Subtitle: 0:15 – 0:20(eng)"));
    let meta = ffprobe(&outpath).unwrap();
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}


// Out-of-band (sidecar) TTML subs.
#[tokio::test]
async fn test_subtitles_hbbtv_dillama_subob() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/02_gran_dillama_1080p_25f75g6sv5/manifest_subob.mpd";
    let outpath = env::temp_dir().join("ttml-hbbtv-dillama-subob.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .prefer_subtitle_language(String::from("fin"))
        .without_content_type_checks()
        .verbosity(1)
        .download_to(&outpath).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("Subtitle: 0:20 – 0:25(fin) ABCÅÄÖ"));
    let meta = ffprobe(&outpath).unwrap();
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(outpath);
        let _ = fs::remove_file(subpath);
    }
}



// A stream with two in-band CEA-608 closed captions embedded in the video track: English and Swedish.
// The closed captions can be played by VLC for example.
//
// We check that the captions can be extracted using ffmpeg and contain the expected text. Note that
// the ffmpeg extraction is including font and styling information in the srt file, which is
// non-standard but is supported by some players.
#[tokio::test]
async fn test_subtitles_cea608() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://livesim2.dashif.org/vod/testpic_2s/cea608.mpd";
    let out_path = env::temp_dir().join("subs-cea608.mp4");
    if out_path.exists() {
        let _ = fs::remove_file(&out_path);
    }
    let caption_path = env::temp_dir().join("subs-cea608-extracted.srt");
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(true)
        .fetch_subtitles(true)
        .without_content_type_checks()
        .verbosity(1)
        .download_to(&out_path).await
        .unwrap();
    let ffmpeg = Command::new("ffmpeg")
        .env("LANG", "C")
        .args(["-hide_banner",
               "-nostats",
               "-loglevel", "error",  // or "warning", "info"
               "-y",  // overwrite output file if it exists
               "-nostdin",
               "-f", "lavfi",
               "-i", &format!("movie={}[out+subcc]", &out_path.to_string_lossy()),
               "-map", "0:s:0",
               "-c:s", "srt",
               "-sub_charenc", "UTF-8",
               &caption_path.to_string_lossy()])
        .output()
        .expect("spawning ffmpeg");
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    if !msg.is_empty() {
        eprintln!("FFMPEG stderr {msg}");
    }
    assert!(fs::metadata(&out_path).is_ok());
    assert!(fs::metadata(&caption_path).is_ok());
    let format = FileFormat::from_file(&caption_path).unwrap();
    assert_eq!(format, FileFormat::SubripText);
    let srt = fs::read_to_string(&caption_path).unwrap();
    assert!(srt.contains("eng: 00:55:44:00"));
    if !env::var("TEST_PERSIST_FILES").is_ok() {
        let _ = fs::remove_file(&out_path);
        let _ = fs::remove_file(&caption_path);
    }
}





// TODO: try also
//
//   https://livesim2.dashif.org/vod/testpic_2s/multi_subs.mpd
//   https://livesim.dashif.org/vod/testpic_2s/Manifest_stpp.mpd
//   https://media.axprod.net/TestVectors/Cmaf/clear_1080p_h264/manifest.mpd (vtt subs in de/en/fr)
