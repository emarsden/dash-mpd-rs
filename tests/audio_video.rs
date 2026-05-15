// Tests for particular audio/video aspects of download support
//
// To run these tests while enabling printing to stdout/stderr
//
//    cargo test --test audio_video -- --show-output

pub mod common;
use std::fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, check_media_duration, setup_logging};


// This test is too slow to run; disable it.
#[ignore]
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_video_only_slow() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://nimbuspm.origin.mediaservices.windows.net/aed33834-ec2d-4788-88b5-a4505b3d032c/Microsoft's HoloLens Live Demonstration.ism/manifest(format=mpd-time-csf)";
    let out = env::temp_dir().join("hololens-video.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .video_only()
        .download_to(&out).await
        .unwrap();
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert!(stream.width.is_some());
}

#[tokio::test]
async fn test_dl_video_only() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let out = env::temp_dir().join("envivio-video.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .video_only()
        .download_to(&out).await
        .unwrap();
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert!(stream.width.is_some());
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_only() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let out = env::temp_dir().join("envivio-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .audio_only()
        .download_to(&out).await
        .unwrap();
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    assert!(stream.width.is_none());
}

#[tokio::test]
async fn test_dl_keep_audio_video() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let out = env::temp_dir().join("envivio.mp4");
    let out_audio = env::temp_dir().join("envivio-audio.mp4");
    let out_video = env::temp_dir().join("envivio-video.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .keep_audio_as(&out_audio)
        .keep_video_as(&out_video)
        .download_to(&out).await
        .unwrap();
    let meta = ffprobe(out_audio).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("aac")));
    assert!(stream.width.is_none());

    let meta = ffprobe(out_video).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert!(stream.width.is_some());
}

#[tokio::test]
async fn test_dl_keep_segments() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let out = env::temp_dir().join("envivio-segments.mp4");
    let fragments_dir = tempfile::tempdir().unwrap();
    let audio_fragments_dir = fragments_dir.path().join("audio");
    let video_fragments_dir = fragments_dir.path().join("video");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .save_fragments_to(fragments_dir.path())
        .download_to(&out).await
        .unwrap();
    let audio_entries = fs::read_dir(audio_fragments_dir).unwrap();
    assert!(audio_entries.count() > 3);
    let video_entries = fs::read_dir(video_fragments_dir).unwrap();
    assert!(video_entries.count() > 3);
}

#[ignore]
#[tokio::test]
async fn test_dl_cea608_captions_slow() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://livesim.dashif.org/dash/vod/testpic_2s/cea608.mpd";
    let out = env::temp_dir().join("cea-closed-captions.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .without_content_type_checks()
        .download_to(&out).await
        .unwrap();
    // Downloaded file size on this is variable.
    // check_file_size_approx(&out, 11_809_117);
    // The closed captions are embedded in the video stream.
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
}



// This manifest contains three AdaptationSets with video content, with different codecs. We want to
// check that when selecting the video stream to download (criterion = lowest bandwidth), we are
// analyzing all Representation elements in the manifest, and not just the Representations in the
// first AdaptationSet.

// The MPD URL generates an HTTP 403 error from 2024-08.
#[ignore]
#[tokio::test]
async fn test_dl_video_stream_selection_defunct() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://quick.vidalytics.com/video/InPR2EKH/LuZkcIBwHO1N1Pk_/57721/48948/stream.mpd";
    let out = env::temp_dir().join("vidalytics-multiple-video-adaptations.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 105_187_936);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    // This manifest contains a video AdaptationSet with codec of hevc and another with codec vp9
    // with exactly the same bandwidth, so we could chose either one.
    assert!(video.codec_name.eq(&Some(String::from("hevc"))) ||
            video.codec_name.eq(&Some(String::from("vp9"))));
    assert_eq!(video.width, Some(480));
}



#[tokio::test]
async fn test_dl_video_stream_selection() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // Fun fact: the URL below contains invalid segment paths for the non-h264 segments.
    // https://ftp.itec.aau.at/datasets/mmsys18/DrivingPOV/DrivingPOV_2sec/multi-codec.mpd

    let mpd_url = "https://ftp.itec.aau.at/datasets/mmsys22/Skateboarding/4sec/multi-codecs-manifest.mpd";
    
    // First check the smallest AV1 stream
    let out = env::temp_dir().join("mmsys22-multiple-video-adaptations-av1.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .prefer_video_codecs(vec![String::from("av01")])
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 2_031_045);
    check_media_duration(&out, 236.0);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert!(video.codec_name.eq(&Some(String::from("av1"))));
    assert_eq!(video.width, Some(320));

    // Then check the smallest HEV1 stream
    let out = env::temp_dir().join("mmsys22-multiple-video-adaptations-hevc.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .prefer_video_codecs(vec![String::from("inexistent"), String::from("hev1"), String::from("vp09")])
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 3_601_579);
    check_media_duration(&out, 236.0);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert!(video.codec_name.eq(&Some(String::from("hevc"))));
    assert_eq!(video.width, Some(320));

    // Then check the biggest VVC stream (unfortunately, this is very large)
    let out = env::temp_dir().join("mmsys22-multiple-video-adaptations-vvc.mp4");
    DashDownloader::new(mpd_url)
        .best_quality()
        .prefer_video_codecs(vec![String::from("vvc1"), String::from("h264")])
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 827_435_116);
    check_media_duration(&out, 236.0);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert!(video.codec_name.eq(&Some(String::from("vvc"))));
    assert_eq!(video.width, Some(7680));

    // Then check the id substring filtering
    let out = env::temp_dir().join("mmsys22-multiple-video-adaptations-id.mp4");
    DashDownloader::new(mpd_url)
        .want_video_id_substring(String::from("34"))
        .verbosity(2)
        .download_to(&out).await
        .unwrap();
    check_media_duration(&out, 236.0);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert!(video.codec_name.eq(&Some(String::from("hevc"))));
    assert_eq!(video.width, Some(640));

}
