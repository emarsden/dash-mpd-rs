// Tests for MPD download support
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


pub mod common;
use fs_err as fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use test_log::test;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;


#[test(tokio::test)]
async fn test_dl_none() {
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cfnone.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_audio(false)
        .fetch_video(false)
        .fetch_subtitles(false)
        .download_to(out.clone()).await
        .unwrap();
    assert!(!out.exists());
}

#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_mp4() {
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("cf.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .max_error_count(5)
        .record_metainformation(false)
        .with_authentication("user".to_string(), "dummy".to_string())
        .download_to(out.clone()).await
        .unwrap();
    // Curious: this download size changed abruptly from 60_939 to this size early Nov. 2023.
    check_file_size_approx(&out, 325_334);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[test(tokio::test)]
async fn test_dl_segmentbase_baseurl() {
    let mpd_url = "https://v.redd.it/p5rowtg41iub1/DASHPlaylist.mpd?a=1701104071";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("reddit.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .max_error_count(5)
        .record_metainformation(false)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 62_177);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
async fn test_dl_segmenttemplate_tiny() {
    let mpd_url = "https://github.com/bbc/exoplayer-testing-samples/raw/master/app/src/androidTest/assets/streams/files/redGreenVideo/redGreenOnlyVideo.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("red-green.mp4");
    DashDownloader::new(mpd_url)
        .intermediate_quality()
        .record_metainformation(false)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 4_546);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_mp4a() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/3a/fraunhofer/aac-lc_stereo_without_video/Sintel/sintel_audio_only_aaclc_stereo_sidx.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("sintel-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 7_456_334);
    let meta = ffprobe(out.clone()).unwrap();
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

#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_flac() {
    if env::var("CI").is_ok() {
        return;
    }
    // See http://rdmedia.bbc.co.uk/testcard/vod/
    let mpd_url = "http://rdmedia.bbc.co.uk/testcard/vod/manifests/radio-flac-en.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bbcradio-flac.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 81_603_640);
    let meta = ffprobe(out.clone()).unwrap();
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

#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_eac3() {
    // E-AC-3 is the same as Dolby Digital Plus; it's an improved version of the AC-3 codec that
    // allows higher bitrates.
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesMCA/dolby/3/1/ChID_voices_20_128_ddp.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-eac3.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 2_436_607);
    let meta = ffprobe(out).unwrap();
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
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_ac4_mkv() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesDolby/2/Living_Room_1080p_20_96k_2997fps.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-ac4.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
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


// As of 2023-10, ffmpeg v6.0 (https://trac.ffmpeg.org/ticket/8349) and VLC v3.0.18 are unable to
// mux this Dolby AC-4 audio stream into an MP4 container, nor to play the content. mp4box is able
// to mux it into an MP4 container.
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_ac4_mp4() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://ott.dolby.com/OnDelKits/AC-4/Dolby_AC-4_Online_Delivery_Kit_1.5/Test_Signals/muxed_streams/DASH/Live/MPD/Multi_Codec_720p_2997fps_h264.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-ac4.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(out.clone()).await
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
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_dtsc() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesMCA/dts/1/Paint_dtsc_testA.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dolby-dtsc.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("mkv", "mkvmerge")
        .content_type_checks(false)
        .conformity_checks(false)
        .verbosity(2)
        .download_to(out.clone()).await
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


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_hevc_hdr() {
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
        .download_to(out.clone()).await
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

#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_hvc1() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("hvc1.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
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

#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_vp9_uhd() {
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
        .download_to(out.clone()).await
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

// H.266/VVC codec. ffmpeg v6.0 is not able to place this video stream in an MP4 container, but
// muxing to Matroska with mkvmerge works. Neither mplayer nor VLC can play the video.
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_vvc() {
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
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_311_029);
    let meta = ffprobe(out).unwrap();
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    // This codec is not recognized by ffprobe v6.0
    // assert_eq!(video.codec_name, Some(String::from("vvc1")));
    assert_eq!(video.width, Some(384));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// MPEG2 TS codec (mostly historical interest).
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_mp2t() {
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
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_019_006);
    let meta = ffprobe(out).unwrap();
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
#[test(tokio::test)]
async fn test_dl_segment_timeline() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://origin.broadpeak.io/bpk-vod/voddemo/default/5min/tearsofsteel/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("broadpeak-tos.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 23_823_326);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for SegmentList addressing
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_segment_list() {
    let mpd_url = "https://res.cloudinary.com/demo/video/upload/sp_full_hd/handshake.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("handshake.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 273_629);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for SegmentBase@indexRange addressing with a single audio and video fragment that
// is convenient for testing sleep_between_requests()
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_segment_base_indexrange() {
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
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_687_251);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// This manifest is built using a difficult structure, rarely seen in the wild. To retrieve segments
// it is necessary to combine information from the AdaptationSet.SegmentTemplate element (which has
// the SegmentTimeline) and the Representation.SegmentTemplate element (which has the media
// template).
#[test(tokio::test)]
async fn test_dl_segment_template_multilevel() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/akamai/test/bbb_enc/BigBuckBunny_320x180_enc_dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("bbb-segment-template.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(3)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 52_758_303);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for BaseURL addressing mode.
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_baseurl() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/1a/sony/SNE_DASH_SD_CASE1A_REVISED.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("sony.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 38_710_852);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// A test for AdaptationSet>SegmentList + Representation>SegmentList addressing modes.
#[test(tokio::test)]
async fn test_dl_adaptation_segment_list() {
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
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 110_010_161);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// This manifest has video streams with different codecs (avc1 and hev1) in different AdaptationSets.
#[test(tokio::test)]
async fn test_dl_adaptation_set_variants() {
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
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 94_921_878);
    let format = FileFormat::from_file(out.clone()).unwrap();
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
#[test(tokio::test)]
async fn test_progress_observer() {
    use dash_mpd::fetch::ProgressObserver;
    use std::sync::Arc;

    struct DownloadProgressionTest { }

    impl ProgressObserver for DownloadProgressionTest {
        fn update(&self, percent: u32, _message: &str) {
            assert!(percent <= 100);
        }
    }

    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("progress.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .add_progress_observer(Arc::new(DownloadProgressionTest{}))
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 325_334);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
}


// These tests retrieve content from some public MPD manifests and check that the content is
// identical to previous "known good" downloads. These checks are fragile because checksums and
// exact octet counts might change due to version changes in libav, that we use for muxing.
// Running this test downloads several hundred megabytes, so we disable it for CI.
#[test(tokio::test)]
#[allow(dead_code)]
async fn test_downloader() {
    use std::io;
    use sha2::{Digest, Sha256};
    use hex_literal::hex;
    use ffprobe::ffprobe;
    use colored::*;

    if env::var("CI").is_ok() {
        return;
    }
    async fn check_mpd(mpd_url: &str, octets: u64, digest: &[u8]) {
        println!("Checking MPD URL {mpd_url}");
        match DashDownloader::new(mpd_url).download().await {
            Ok(path) => {
                // check that ffprobe identifies this as a media file
                let probed_meta = ffprobe(path.clone());
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


// Content that uses the HVC1 H265 codec in a CMAF container.
#[test(tokio::test)]
async fn test_dl_h265() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://media.axprod.net/TestVectors/H265/clear_cmaf_1080p_h265/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("h265.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 48_352_569);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// This is a "pseudo-live" stream, a dynamic MPD manifest for which all media segments are already
// available at the time of download. Though we are not able to correctly download a genuinely live
// stream (we don't implement the clock functionality needed to wait until segments become
// progressively available), we are able to download pseudo-live stream if the
// allow_live_streaming() method is enabled.
#[test(tokio::test)]
async fn test_dl_dynamic_stream() {
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
    check_file_size_approx(&out, 1_591_916);
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
#[test(tokio::test)]
async fn test_dl_dynamic_forced_duration() {
    let mpd_url = "https://livesim2.dashif.org/livesim2/ato_inf/testpic_2s/Manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("dynamic-6s.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .force_duration(6.5)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 141_675);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(640));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let duration = stream.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!(5.0 < duration && duration < 7.0, "Expecting duration between 5 and 6, got {duration}");
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
async fn test_dl_lowlatency_forced_duration() {
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
    check_file_size_approx(&out, 2_633_341);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(1280));
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let duration = stream.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!(9.5 < duration && duration < 13.0, "Expecting duration between 9.5 and 13, got {duration}");
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// Test the escaping of a filename that contains '&' due to the &delay=25 in the mpd URL.
// Potentially problematic for our calls to ffmpeg and to mp4decrypt.
// Disable this test because the decryption keys seem to change over time.
#[ignore]
#[test(tokio::test)]
async fn test_dl_filename_ampersand() {
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
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 541_593);
    let meta = ffprobe(out).unwrap();
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
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_dl_forced_duration_audio() {
    let mpd_url = "https://rdmedia.bbc.co.uk/testcard/vod/manifests/radio-surround-en.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("forced-duration-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .allow_live_streams(true)
        .force_duration(8.0)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 281_686);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Audio);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    let duration = stream.duration.as_ref().unwrap().parse::<f64>().unwrap();
    assert!(7.1 < duration && duration < 8.5);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
async fn test_dl_follow_redirect() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let redirector = format!("http://httpbin.org/redirect-to?url={mpd_url}");
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("itec-redirected.mp4");
    DashDownloader::new(&redirector)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 325_334);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

