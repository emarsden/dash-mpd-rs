// Dedicated tests for multiperiod manifests.
//
// To run only these tests while enabling printing to stdout/stderr
//
//    cargo test --test multiperiod -- --show-output

pub mod common;
use fs_err as fs;
use std::env;
use file_format::FileFormat;
use ffprobe::ffprobe;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, check_media_duration, setup_logging};



#[tokio::test]
async fn test_multiperiod_helio() {
    setup_logging();
    // This test generates large CPU usage by reencoding a multiperiod media file, so don't run it
    // on CI infrastructure.
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest has three periods, each with only a video stream, identical resolutions,
    // encoding in VP9. Check that we concat this into a single media file. This media content is
    // very small (40kB) if it stays encoded in VP9 (when we select a WebM output container), but
    // blows up into 150MB if we save to an MP4 container. ffmpeg v6.0 shows an error message
    // "matroska,webm @ 0x5631d8198700] File ended prematurely" while concatenating, but the output
    // file is playable.
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("heliocentrism-multiperiod.webm");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    // We see different file sizes for content from this manifest, for unknown reasons.
    // check_file_size_approx(&out, 36_000);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Webm);
    // The three periods should have been merged into a single output file, and the other temporary
    // media files should be been explicitly deleted.
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_multiperiod_nomor5a_ffmpeg() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest is a 92MB file with 2 periods, identical video resolution and codecs in the two periods.
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/5a/nomor/1.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("nomor.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .concatenate_periods(true)
        // The mkvmerge concat helper fails on this manifest
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 95_623_359);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_multiperiod_nomor5b_ffmpeg() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest has 3 periods, with different resolutions. We will therefore save the media
    // content to three separate files.
    let mpd_url = "http://dash.edgesuite.net/dash264/TestCases/5b/1/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-5b.mp4");
    let p2 = tmpd.path().join("multiperiod-5b-p2.mp4");
    let p3 = tmpd.path().join("multiperiod-5b-p3.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 28_755_275);
    check_file_size_approx(&p2, 4_383_256);
    check_file_size_approx(&p3, 31_215_605);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 3, "Expecting 3 output files, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
async fn test_multiperiod_nomor5b_mkvmerge() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest has 3 periods, with different resolutions. We will therefore save the media
    // content to three separate files.
    let mpd_url = "http://dash.edgesuite.net/dash264/TestCases/5b/1/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-5b.mp4");
    let p2 = tmpd.path().join("multiperiod-5b-p2.mp4");
    let p3 = tmpd.path().join("multiperiod-5b-p3.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 28_755_275);
    check_file_size_approx(&p2, 4_383_256);
    check_file_size_approx(&p3, 31_215_605);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 3, "Expecting 3 output files, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
async fn test_multiperiod_withsubs_ffmpeg() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest has 2 periods, each containing audio, video and WVTT subtitle streams. The
    // periods should be concatenated into a single output file.
    let mpd_url = "http://media.axprod.net/TestVectors/v6-Clear/MultiPeriod_Manifest_1080p.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-withsubs.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 94_818_672);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
async fn test_multiperiod_withsubs_ffmpegdemuxer() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This manifest has 2 periods, each containing audio, video and subtitle streams. The periods
    // should be concatenated into a single output file.
    let mpd_url = "http://media.axprod.net/TestVectors/v6-Clear/MultiPeriod_Manifest_1080p.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-withsubs.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 94_818_672);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// This manifest has two periods, each only containing audio content.
#[tokio::test]
async fn test_multiperiod_audio_ffmpeg() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://media.axprod.net/TestVectors/v7-Clear/Manifest_MultiPeriod_AudioOnly.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-audio.mp3");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp3", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 23_868_589);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg12AudioLayer3);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
async fn test_multiperiod_audio_ffmpegdemuxer() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://media.axprod.net/TestVectors/v7-Clear/Manifest_MultiPeriod_AudioOnly.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-audio.mp3");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp3", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 23_868_589);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg12AudioLayer3);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// This manifest contains three Periods, each with a different BaseURL (which could be pointing to
// different CDNs). We disable it due to the size of the output file.
#[ignore]
#[tokio::test]
async fn test_multiperiod_diffbase() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesIOP33/multiplePeriods/3/manifest_multiple_Periods_Content_Offering_CDN.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("multiperiod-diffbase.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 245_287_205);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}



// This manifest contains two periods that are tricky to merge: the first period has audio and video
// whereas the second period has video but not audio.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_multiperiod_witha_withouta_ffmpegfilter() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // There is a first period of duration 20s, then a second period that resolves to zero, the a
    // third period of duration 20s.
    let mpd_url = "http://dash.edgesuite.net/fokus/adinsertion-samples/xlink/twoperiods.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("twoperiods.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_973_570);
    let meta = ffprobe(&out).unwrap();
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
    check_media_duration(&out, 40.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_multiperiod_witha_withouta_ffmpegdemuxer() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/fokus/adinsertion-samples/xlink/twoperiods.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("twoperiods.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_973_570);
    let meta = ffprobe(&out).unwrap();
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
    check_media_duration(&out, 40.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_multiperiod_witha_withouta_witha() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/fokus/adinsertion-samples/xlink/threeperiods.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("threeperiods.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 14_435_150);
    let meta = ffprobe(&out).unwrap();
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
    check_media_duration(&out, 72.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

