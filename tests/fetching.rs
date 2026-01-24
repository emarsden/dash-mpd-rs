//! Tests for MPD download support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test fetching -- --show-output
//
// Testing resources:
//
//   https://testassets.dashif.org/#testvector/list
//   https://dash.itec.aau.at/dash-dataset/
//   https://github.com/streamlink/streamlink/tree/master/tests/resources/dash
//   https://github.com/gpac/gpac/wiki/DASH-Sequences
//   https://dash.akamaized.net/


pub mod common;
use fs_err as fs;
use std::env;
use std::process::Command;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, check_media_duration, setup_logging};


#[tokio::test]
async fn test_dl_none() {
    setup_logging();
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cfnone.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(false)
        .download_to(&out).await
        .unwrap();
    assert!(!out.exists());
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_mp4() {
    setup_logging();
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("cf.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .max_error_count(5)
        .record_metainformation(false)
        .with_authentication("user", "dummy")
        .download_to(&out).await
        .unwrap();
    // Curious: this download size changed abruptly from 60_939 to this size early Nov. 2023.
    check_file_size_approx(&out, 410_218);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
async fn test_dl_segmentbase_baseurl() {
    setup_logging();
    let mpd_url = "https://v.redd.it/p5rowtg41iub1/DASHPlaylist.mpd?a=1701104071";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("reddit.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .max_error_count(5)
        .record_metainformation(false)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 62_177);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_dl_segmenttemplate_tiny() {
    setup_logging();
    let mpd_url = "https://github.com/bbc/exoplayer-testing-samples/raw/master/app/src/androidTest/assets/streams/files/redGreenVideo/redGreenOnlyVideo.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("red-green.mp4");
    DashDownloader::new(mpd_url)
        .intermediate_quality()
        .sandbox(true)
        .record_metainformation(false)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 4_546);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_mp4a() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/3a/fraunhofer/aac-lc_stereo_without_video/Sintel/sintel_audio_only_aaclc_stereo_sidx.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("sintel-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 7_456_334);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    assert!(audio.width.is_none());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_flac() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // See http://rdmedia.bbc.co.uk/testcard/vod/
    let mpd_url = "http://rdmedia.bbc.co.uk/testcard/vod/manifests/radio-flac-en.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bbcradio-flac.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 81_603_640);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("flac")));
    assert!(audio.width.is_none());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_eac3() {
    // E-AC-3 is the same as Dolby Digital Plus; it's an improved version of the AC-3 codec that
    // allows higher bitrates.
    setup_logging();
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesMCA/dolby/3/1/ChID_voices_20_128_ddp.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-eac3.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 2_436_607);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("eac3")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// As of 2023-09, ffmpeg v6.0 and VLC v3.0.18 are unable to mux this Dolby AC-4 audio stream into an
// MP4 container, not play the content. mkvmerge is able to mux it into a Matroska container.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_ac4_mkv() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesDolby/2/Living_Room_1080p_20_96k_2997fps.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-ac4.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 11_668_955);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let _audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    // This codec is not currently recogized by ffprobe
    // assert_eq!(stream.codec_name, Some(String::from("ac-4")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_sessionid() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cf-sf-video.wmspanel.com/local/raw/BigBuckBunny_320x180.mp4/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bunny-small.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 64_617_930);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// As of 2023-10, ffmpeg v6.0 (https://trac.ffmpeg.org/ticket/8349) and VLC v3.0.18 are unable to
// mux this Dolby AC-4 audio stream into an MP4 container, nor to play the content. mp4box is able
// to mux it into an MP4 container.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_ac4_mp4() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://ott.dolby.com/OnDelKits/AC-4/Dolby_AC-4_Online_Delivery_Kit_1.5/Test_Signals/muxed_streams/DASH/Live/MPD/Multi_Codec_720p_2997fps_h264.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-ac4.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 8_416_451);
    // Don't attempt to ffprobe, because it generates an error ("no decoder could be found for codec
    // none").
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// As of 2023-09, ffmpeg v6.0 is unable to mux this Dolby DTSC audio codec into an MP4 container. mkvmerge
// is able to mux it into a Matroska container.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_dtsc() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesMCA/dts/1/Paint_dtsc_testA.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-dtsc.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_muxer_preference("mkv", "mkvmerge")
        .content_type_checks(false)
        .conformity_checks(false)
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 35_408_836);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("dts")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// Here a test manifest using MPEG H 3D audio format (mha1 codec), which is not supported by ffmpeg
// 6.0 or mkvmerge.
// https://dash.akamaized.net/dash264/TestCasesMCA/fraunhofer/MPEGH_Stereo_lc_mha1/1/Sintel/sintel_audio_video_mpegh_mha1_stereo_sidx.mpd


#[tokio::test]
async fn test_dl_bok() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://www.bok.net/dash/tears_of_steel/cleartext/stream.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bok-tears.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_muxer_preference("mkv", "mkvmerge")
        .content_type_checks(false)
        .conformity_checks(false)
        .verbosity(0)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 59_936_277);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_hevc_hdr() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesHDR/3a/3/MultiRate.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("hevc-hdr.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 4_052_727);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("hevc")));
    assert!(stream.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_hvc1() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("hvc1.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    assert_eq!(video.width, Some(640));
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    assert!(audio.width.is_none());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_vp9_uhd() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesVP9/vp9-uhd/sintel-vp9-uhd.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("vp9-uhd.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 71_339_734);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("vp9")));
    assert_eq!(stream.width, Some(3840));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// H.266/VVC codec. ffmpeg v7.0 is not able to place this video stream in an MP4 container, but
// muxing to Matroska with mkvmerge works. Neither mplayer nor VLC can play the video.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_vvc() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://ftp.itec.aau.at/datasets/mmsys22/Skateboarding/8sec/vvc/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("vvc.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 9_311_029);
    // ffprobe is not able to read metainformation on the video (it panics)
    // let meta = ffprobe(out).unwrap();
    // let video = meta.streams.iter()
    //     .find(|s| s.codec_type.eq(&Some(String::from("video"))))
    //     .expect("finding video stream");
    // assert_eq!(video.codec_name, Some(String::from("vvc1")));
    // assert_eq!(video.width, Some(384));
    let mkvinfo = Command::new("mkvinfo")
        .env("LANG", "C")
        .arg(out)
        .output()
        .expect("spawning mkvinfo");
    assert!(mkvinfo.status.success());
    let stdout = String::from_utf8_lossy(&mkvinfo.stdout);
    // Note that the "Codec ID" part of this string is locale-dependent.
    assert!(stdout.contains("Codec ID: V_QUICKTIME"), "mkvinfo output missing V_QUICKTIME: got {stdout}");
    // Note that the "Display width" part of this string is locale-dependent.
    assert!(stdout.contains("Display width: 384"), "mkvinfo output missing display width: got {stdout}");
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// MPEG2 TS codec (mostly historical interest).
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_mp2t() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mpeg2-simple/mpeg2-simple-mpd.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("mp2ts.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 9_019_006);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert_eq!(video.width, Some(320));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for SegmentTemplate+SegmentTimeline addressing. Also a test of manifests created with the
// Broadpeak Origin packager.
#[tokio::test]
async fn test_dl_segment_timeline() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://origin.broadpeak.io/bpk-vod/voddemo/default/5min/tearsofsteel/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("broadpeak-tos.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 23_823_326);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for SegmentTemplate+SegmentTimeline addressing with audio-only HE-AACv2 stream.
#[tokio::test]
async fn test_dl_segment_timeline_heaacv2() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/dash264/CTA/ContentModel/SinglePeriod/Fragmented/ToS_HEAACv2_fragmented.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("segment-timeline-heaacv2.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 3_060_741);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Audio);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// A second test for SegmentList+SegmentURL addressing
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_segment_list() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mp4-main-multi/mp4-main-multi-mpd-AV-BS.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("gpac-main-multi-AV-BS.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        // ffmpeg fails to mux this content ("Not yet implemented in FFmpeg")
        .with_muxer_preference("mp4", "mp4box")
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 5_160_973);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(&out).unwrap();
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    check_media_duration(&out, 600.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// A test for SegmentBase@indexRange addressing with a single audio and video fragment that
// is convenient for testing sleep_between_requests()
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_segment_base_indexrange() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://turtle-tube.appspot.com/t/t2/dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("turtle.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(3)
        .sleep_between_requests(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 9_687_251);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_dl_segment_timeline_bbb() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps_320x180_200k.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bbb-segment-timeline.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(1)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 16_033_406);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_media_duration(&out, 634.57);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// This manifest is built using a difficult structure, rarely seen in the wild. To retrieve segments
// it is necessary to combine information from the AdaptationSet.SegmentTemplate element (which has
// the SegmentTimeline) and the Representation.SegmentTemplate element (which has the media
// template). Note that this content is encrypted and we don't have the key.
#[tokio::test]
async fn test_dl_segment_timeline_multilevel() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/akamai/test/bbb_enc/BigBuckBunny_320x180_enc_dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bbb-template-multilevel.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(3)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 52_758_303);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for BaseURL addressing mode.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_baseurl() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/1a/sony/SNE_DASH_SD_CASE1A_REVISED.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("sony.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 38_710_852);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for AdaptationSet>SegmentList + Representation>SegmentList addressing modes.
#[tokio::test]
async fn test_dl_adaptation_segment_list() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://ftp.itec.aau.at/datasets/mmsys13/redbull_4sec.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("redbull.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .without_content_type_checks()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 110_010_161);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// This manifest has video streams with different codecs (avc1 and hev1) in different AdaptationSets.
#[tokio::test]
async fn test_dl_adaptation_set_variants() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesIOP33/adapatationSetSwitching/2/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("adaptation-set-switch.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .without_content_type_checks()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 94_921_878);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert_eq!(video.width, Some(1920));
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// A test for the progress observer functionality.
#[tokio::test]
async fn test_progress_observer() {
    use dash_mpd::fetch::ProgressObserver;
    use std::sync::Arc;

    struct DownloadProgressionTest { }

    impl ProgressObserver for DownloadProgressionTest {
        fn update(&self, percent: u32, _message: &str) {
            assert!(percent <= 100);
        }
    }

    setup_logging();
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("progress.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .add_progress_observer(Arc::new(DownloadProgressionTest{}))
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 410_218);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
}


// These tests retrieve content from some public MPD manifests and check that the content is
// identical to previous "known good" downloads. These checks are fragile because checksums and
// exact octet counts might change due to version changes in libav, that we use for muxing.
// Running this test downloads several hundred megabytes, so we disable it for CI.
#[tokio::test]
#[allow(dead_code)]
async fn test_downloader() {
    use std::io;
    use sha2::{Digest, Sha256};
    use hex_literal::hex;
    use ffprobe::ffprobe;
    use colored::*;

    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    async fn check_mpd(mpd_url: &str, octets: u64, digest: &[u8]) {
        println!("Checking MPD URL {mpd_url}");
        match DashDownloader::new(mpd_url).download().await {
            Ok(path) => {
                // check that ffprobe identifies this as a media file
                let probed_meta = ffprobe(&path);
                if let Ok(meta) = probed_meta {
                    if meta.streams.is_empty() {
                        eprintln!("   {}", "ffprobe finds zero media streams in file".red());
                    } else {
                        let stream = &meta.streams[0];
                        // actually, ffprobe doesn't give a duration for WebM content
                        // assert!(stream.duration.is_some());
                        if let Some(duration) = &stream.duration {
                            if duration.parse::<f64>().unwrap() <= 0.0 {
                                eprintln!("   {}", "ffprobe finds a zero-length stream".red());
                            }
                        }
                    }
                } else {
                    eprintln!("   {} on {mpd_url}", "ffprobe failed".red());
                }
                let mut sha256 = Sha256::new();
                let mut media = std::fs::File::open(path)
                    .expect("opening media file");
                let octets_downloaded = io::copy(&mut media, &mut sha256)
                    .expect("reading media file contents");
                let difference_ratio = (octets_downloaded as f64 - octets as f64) / octets as f64;
                if  difference_ratio.abs() > 0.1 {
                    eprintln!("   {:.1}% difference in download sizes", difference_ratio * 100.0);
                }
                let calculated = sha256.finalize();
                if calculated[..] != digest[..]  {
                    eprintln!("   {}", "incorrect checksum".red());
                }
                // We leave the downloaded file in the filesystem, in case further analysis is needed.
            },
            Err(e) => eprintln!("Failed to fetch MPD {mpd_url}: {e:?}"),
        }
    }

    check_mpd("https://res.cloudinary.com/demo-robert/video/upload/sp_16x9_vp9/yourPublicId.mpd",
              445_758,
              &hex!("7d6533d19a4a60c5c76cead7b2de1664f4ff856916037a574f641aad0324ee36")).await;

    check_mpd("https://storage.googleapis.com/shaka-demo-assets/angel-one/dash.mpd",
              1_786_875,
              &hex!("fc70321b55339d37c6c1ce8303fe357f3b1c83e86bc38fac54eed553cf3a251b")).await;

}


// Testing compatibility with the unified-streaming.com DASH encoder
//
// As of 2024-09 this test is failing with an expired certificate error. This is useful information
// in judging the technical competence of a streaming provider.
#[tokio::test]
async fn test_dl_usp_tos() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel.ism/.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("usp-tos.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 41_621_346);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// Content that uses the HVC1 H265 codec in a CMAF container.
#[tokio::test]
async fn test_dl_h265() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://media.axprod.net/TestVectors/H265/clear_cmaf_1080p_h265/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("h265.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 48_352_569);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// Content packaged using Unified Streaming Platform
//
// Manifest listed at https://reference.dashif.org/dash.js/nightly/samples/dash-if-reference-player/index.html
#[tokio::test]
async fn test_dl_usp_packager() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://s3.eu-west-1.amazonaws.com/origin-prod-lon-v-nova.com/lcevcDualTrack/1080p30_4Mbps_no_dR/master.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("usp-bb.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .download_to(&out).await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 206_901_334);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_dl_arte() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://arteamd1.akamaized.net/GPU/034000/034700/034755-230-A/221125154117/034755-230-A_8_DA_v20221125.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("arte.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 33_188_592);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// Content that is served with an invalid Content-type header.
#[tokio::test]
async fn test_dl_content_type() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cdn.bitmovin.com/content/assets/playhouse-vr/mpds/105560.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("playhouse-content-type.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .download_to(&out).await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 19_639_475);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    check_media_duration(&out, 136.14);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}




// This is a test for "Content Steering" between different CDNs. The library doesn't currently
// provide any support for content steering; when multiple BaseURL elements are present, it will
// simply download from the first one. This test simply checks that this basic "ignore content
// steering" behaviour is operational.
//
// The DASH standard section "5.6.5 Alternative base URLs" says
//
// If alternative base URLs are provided through the BaseURL element at any level, identical
// Segments shall be accessible at multiple locations. In the absence of other criteria, the DASH
// Client may use the first BaseURL element as “base URI". The DASH Client may use base URLs
// provided in the BaseURL element as “base URI” and may implement any suitable algorithm to
// determine which URLs it uses for requests.
#[ignore] // this URL is unreachable in 202512
#[tokio::test]
async fn test_dl_content_steering() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest is bizarre: it announces a worst quality 630x360 video stream, but when
    // downloading it's actually 1920x1080.
    let mpd_url = "https://www.content-steering.com/bbb/playlist_steering_cloudfront_https.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("content-steering.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 267_380_193);
    let meta = ffprobe(&out).unwrap();
    let video = meta.streams.iter()
         .find(|s| s.codec_type.eq(&Some(String::from("video"))))
         .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    check_media_duration(&out, 634.15);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// Test the escaping of a filename that contains '&' due to the &delay=25 in the mpd URL.
// Potentially problematic for our calls to ffmpeg and to mp4decrypt.
// Disable this test because the decryption keys seem to change over time.
#[ignore]
#[tokio::test]
async fn test_dl_filename_ampersand() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://content.uplynk.com/playlist/6c526d97954b41deb90fe64328647a71.mpd?ad=bbbads&delay=25";
    let tmpd = tempfile::tempdir().unwrap();
    env::set_current_dir(&tmpd)
        .expect("changing current directory to tmpdir");
    // We want to check the download() method on DashDownloader, which is going to create a filename
    // that includes an ampersand and an equal sign.
    let out = DashDownloader::new(mpd_url)
        .worst_quality()
        .add_decryption_key(String::from("1f35eaf0cb29406a92888b1097e9a39a"),
                            String::from("da7bc7544d9f5fe3cab7f1d75a8fb9ee"))
        .allow_live_streams(true)
        .download().await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 541_593);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding audio stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// This test doesn't work with libav because we haven't yet implemented copy_video_to_container()
// with a change in container type.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_forced_duration_audio() {
    setup_logging();
    let mpd_url = "https://rdmedia.bbc.co.uk/testcard/vod/manifests/radio-surround-en.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("forced-duration-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .force_duration(8.0)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 281_821);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Audio);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    check_media_duration(&out, 7.7);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_dl_follow_redirect() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let redirector = format!("http://httpbin.org/redirect-to?url={mpd_url}");
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("itec-redirected.mp4");
    DashDownloader::new(&redirector)
        .worst_quality()
        .download_to(&out).await
        .unwrap();
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 410_218);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}



// More possible test streams:
//
//   - https://explo.broadpeak.tv:8343/bpk-tv/spring/lowlat/index_timeline.mpd (live)
//   - https://dash-large-files.akamaized.net/WAVE/Proposed/ToS_Fragmented_AVC_AAC/output.mpd
