/// Tests for subtitle support
///
/// We can run these tests on CI infrastructure because they are only downloading modest quantites
/// of data, corresponding to the subtitle files/MP4 fragments. This requires MP4Box (from GPAC) to
/// be installed on CI machines, however.

// To run tests while enabling printing to stdout/stderr, "cargo test -- --show-output" (from the
// root crate directory).


use fs_err as fs;
use std::env;
use std::path::Path;
use tracing::info;
use ffprobe::ffprobe;
use file_format::FileFormat;
use test_log::test;
use pretty_assertions::assert_eq;
use dash_mpd::fetch::DashDownloader;

// This manifest includes subtitles in WVTT (WebVTT) format. We check that these are downloaded to
// the output path with a ".wvtt" extension. Also check that the subtitles are successfully
// converted to SRT format, which is more widely supported, in a file named like the output with a
// ".srt" extension.
//
// Note that these tests will fail if MP4Box (from GPAC) is not installed. MP4Box is used for the
// conversion to SRT format.
#[test(tokio::test)]
async fn test_subtitles_wvtt () {
    let mpd = "https://storage.googleapis.com/shaka-demo-assets/sintel-mp4-wvtt/dash.mpd";
    let outpath = env::temp_dir().join("sintel.mp4");
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
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(subpath_wvtt).is_ok());
    assert!(fs::metadata(subpath_srt).is_ok());
    let _format = FileFormat::from_file(subpath_wvtt).unwrap();
    // For some reason, the file-format crate is not detecting this format correctly (it detects the
    // more generic Mpeg4Part14Video type).
    // assert_eq!(format, FileFormat::WebVideoTextTracks);
    let format = FileFormat::from_file(subpath_srt).unwrap();
    assert_eq!(format, FileFormat::SubripText);
    let srt = fs::read_to_string(subpath_srt).unwrap();
    assert!(srt.contains("land van de poortwachters"));
    if let Err(e) = fs::remove_file(subpath_wvtt) {
        info!("Failed to delete temporary file for wvtt subs: {e}");
    }
    if let Err(e) = fs::remove_file(subpath_srt) {
        info!("Failed to delete temporary file for srt usbs: {e}");
    }
    let _ = fs::remove_file(outpath.clone());

    // Now download the english subtitles and check that we got the expected content.
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_language(String::from("eng"))
        .download_to(outpath.clone()).await
        .unwrap();
    let srt = fs::read_to_string(subpath_srt).unwrap();
    // This time we requested English subtitles.
    assert!(srt.contains("land of the gatekeepers"));
    let _ = fs::remove_file(outpath.clone());
}


#[test(tokio::test)]
async fn test_subtitles_ttml () {
    let mpd = "https://dash.akamaized.net/dash264/TestCases/4b/qualcomm/2/TearsOfSteel_onDem5secSegSubTitles.mpd";
    let outpath = env::temp_dir().join("tears-of-steel.mp4");
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    // We didn't specify a preferred language, so the first available one in the manifest (here
    // English) is downloaded.
    assert!(ttml.contains("You're a jerk"));
    let _ = fs::remove_file(outpath.clone());

    DashDownloader::new(mpd)
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(true)
        .prefer_language(String::from("de"))
        .download_to(outpath.clone()).await
        .unwrap();
    let ttml = fs::read_to_string(subpath).unwrap();
    // This time we requested German subtitles.
    assert!(ttml.contains("Du bist ein Vollidiot"));
    let _ = fs::remove_file(outpath);
}


// We can run this on CI infrastructure because it's only downloading a modest amount of subtitle
// segments.
#[test(tokio::test)]
async fn test_subtitles_vtt () {
    let mpd = "http://dash.edgesuite.net/akamai/test/caption_test/ElephantsDream/elephants_dream_480p_heaac5_1.mpd";
    let outpath = env::temp_dir().join("elephants-dream.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(outpath.clone());
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
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::WebVideoTextTracks);
    // This manifest contains a single subtitle track, available in VTT format via BaseURL addressing.
    let vtt = fs::read_to_string(subpath).unwrap();
    assert!(vtt.contains("Hurry Emo!"));
    let _ = fs::remove_file(outpath);
}


// STPP subtitles are muxed into the output media stream, so we need to download audio and video for
// this type. So we don't run this test on CI infrastructure.
#[test(tokio::test)]
async fn test_subtitles_stpp() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://rdmedia.bbc.co.uk/elephants_dream/1/client_manifest-all.mpd";
    let outpath = env::temp_dir().join("elephants-dream-bbc.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(outpath.clone());
    }
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .verbosity(2)
        .download_to(outpath.clone()).await
        .unwrap();
    let meta = ffprobe(outpath.clone()).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let stpp = &meta.streams[2];
    assert_eq!(stpp.codec_tag_string, "stpp");
    let duration = stpp.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!((620.0 < duration) && (duration < 640.0));
    let _ = fs::remove_file(outpath);
}


// Image-based (IMSC1 CMAF) STPP subtitles.
#[test(tokio::test)]
async fn test_subtitles_stpp_imsc1() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://livesim.dashif.org/dash/vod/testpic_2s/imsc1_img.mpd";
    let outpath = env::temp_dir().join("imsc1-subs.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(outpath.clone());
    }
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .without_content_type_checks()
        .verbosity(2)
        .download_to(outpath.clone()).await
        .unwrap();
    let meta = ffprobe(outpath.clone()).unwrap();
    assert_eq!(meta.streams.len(), 3);
    let stpp = &meta.streams[2];
    // In the MPD it's specified as stpp.ttml.im1i.
    assert_eq!(stpp.codec_tag_string, "stpp");
    let duration = stpp.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!((56.0 < duration) && (duration < 60.0));
    let _ = fs::remove_file(outpath);
}


// MPEG-4 Part 17 (Timed Text), called "mov_text" in ffmpeg.
#[test(tokio::test)]
async fn test_subtitles_tx3g() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mp4-live-subtitle/mp4-live-subtitle-mpd-AVST.mpd";
    let outpath = env::temp_dir().join("tx3g.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(outpath.clone());
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
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::SubripText);
    let srt = fs::read_to_string(subpath).unwrap();
    assert!(srt.contains("Cue #3 Start Time"));
    let meta = ffprobe(outpath.clone()).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let _ = fs::remove_file(outpath);
}


#[test(tokio::test)]
async fn test_subtitles_segmentbase() {
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest contains a subtitle Representation with stpp codec using SegmentBase addressing.
    let mpd = "https://usp-cmaf-test.s3.eu-central-1.amazonaws.com/tears-of-steel-ttml.mpd";
    let outpath = env::temp_dir().join("segbase.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(outpath.clone());
    }
    let mut subpath = outpath.clone();
    subpath.set_extension("ttml");
    let subpath = Path::new(&subpath);
    DashDownloader::new(mpd)
        .fetch_audio(true)
        .fetch_video(true)
        .fetch_subtitles(true)
        .without_content_type_checks()
        .verbosity(2)
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(subpath).is_ok());
    let format = FileFormat::from_file(subpath).unwrap();
    assert_eq!(format, FileFormat::TimedTextMarkupLanguage);
    let ttml = fs::read_to_string(subpath).unwrap();
    assert!(ttml.contains("http://www.w3.org/ns/ttml"));
    assert!(ttml.contains("robot hand"));
    let meta = ffprobe(outpath.clone()).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let _ = fs::remove_file(outpath);
}


// https://media.axprod.net/TestVectors/Cmaf/clear_1080p_h264/manifest.mpd
// has vtt subs in de/en/fr
