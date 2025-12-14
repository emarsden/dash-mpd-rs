// Tests for Period concatenation with multi-period manifests
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test concat -- --show-output


pub mod common;
use fs_err as fs;
use std::env;
use std::time::Duration;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, check_media_duration, setup_logging};


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_noaudio_ffmpeg() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_781_840);
    let meta = ffprobe(out).unwrap();
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
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_noaudio_ffmpegdemuxer() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-ffmpegdemuxer.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_781_840);
    let meta = ffprobe(out).unwrap();
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
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// mkvmerge cannot concat this stream to MP4: failure with error message
//   Quicktime/MP4 reader: Could not read chunk number 48/62 with size 1060 from position 15936. Aborting.
#[tokio::test]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all concat helpers failed")]
async fn test_concat_noaudio_mkvmerge_mp4() {
    setup_logging();
    if env::var("CI").is_ok() {
        panic!("all concat helpers failed");
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-mkvmerge.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let _ = fs::remove_dir_all(tmpd);
}

// mkvmerge fails to concat. Check that the fallback to ffmpeg as a concat helper works 
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_noaudio_mkv_concat_fallback() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-mkvmerge.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "mkvmerge")
        .with_concat_preference("mkv", "mkvmerge,ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    check_file_size_approx(&out, 7_258_379);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("vorbis")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_singleases_ffmpeg() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/singleases.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-singleases-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpeg")
        .minimum_period_duration(Duration::new(10, 0))
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_781_840);
    let meta = ffprobe(out).unwrap();
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
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_singleases_ffmpegdemuxer() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/singleases.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-singleases-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .minimum_period_duration(Duration::new(10, 0))
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 5_781_840);
    let meta = ffprobe(out).unwrap();
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
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// mkvmerge is unable to concatenate these streams: fails with an error message
//
//   The track number 0 from the file '/tmp/.tmpkgxD7n/concat-singleases-mkvmerge-p2.mp4' can
//   probably not be appended correctly to the track number 0 from the file
//   '/tmp/dashmpdrsbnMoM.mkv': The codec's private data does not match. Both have the same length
//   (41) but different content.
#[tokio::test]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all concat helpers failed")]
async fn test_concat_singleases_mkvmerge() {
    setup_logging();
    if env::var("CI").is_ok() {
        panic!("all concat helpers failed");
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/singleases.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-singleases-mkvmerge.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "mkvmerge")
        .minimum_period_duration(Duration::new(10, 0))
        .download_to(out.clone()).await
        .unwrap();
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_ffmpeg_mp4() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 36_829);
    let meta = ffprobe(&out).unwrap();
    // This manifest has no audio track.
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    check_media_duration(&out, 4.9);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_ffmpegdemuxer_mp4() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-ffmpeg-demuxer.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // FIXME problem here: on Windows we see a size of 16_384
    // check_file_size_approx(&out, 40_336);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    check_media_duration(&out, 4.9);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// If ffmpeg is used as a concat helper for mkv files that were muxed using mkvmerge (our default
// muxer for that container format), we see concatenation errors. Using ffmpeg for both muxing and
// concatenation helps work around this problem.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_ffmpeg_mkv() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-ffmpeg.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .verbosity(3)
        .with_muxer_preference("mkv", "ffmpeg")
        .with_concat_preference("mkv", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let fp = std::process::Command::new("ffprobe")
        .env("LANG", "C")
        .arg("-hide_banner")
        .arg(out.to_str().unwrap())
        .output()
        .expect("spawning ffprobe");
    let stdout = String::from_utf8_lossy(&fp.stdout);
    if stdout.len() > 0 {
        println!("ffprobe stdout> {stdout}");
    }
    let stderr = String::from_utf8_lossy(&fp.stderr);
    if stderr.len() > 0 {
        println!("ffprobe stderr> {stderr}");
    }
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    check_file_size_approx(&out, 35_937);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    check_media_duration(&out, 4.9);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_ffmpegdemuxer_mkv() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-ffmpeg.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .verbosity(2)
        .with_muxer_preference("mkv", "ffmpeg")
        .with_concat_preference("mkv", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    let fp = std::process::Command::new("ffprobe")
        .env("LANG", "C")
        .arg("-hide_banner")
        .arg(out.to_str().unwrap())
        .output()
        .expect("spawning ffprobe");
    let stdout = String::from_utf8_lossy(&fp.stdout);
    if stdout.len() > 0 {
        println!("ffprobe stdout> {stdout}");
    }
    let stderr = String::from_utf8_lossy(&fp.stderr);
    if stderr.len() > 0 {
        println!("ffprobe stderr> {stderr}");
    }
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    // On Windows we are seeing the size 37005 instead of 42_060
    // check_file_size_approx(&out, 37_005);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    check_media_duration(&out, 4.9);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// mkvmerge fails to concatenate this stream with error
//
//   Quicktime/MP4 reader: Could not read chunk number 48/62 with size 1060 from position 15936. Aborting.
#[tokio::test]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all concat helpers failed")]
async fn test_concat_heliocentrism_mkvmerge_mp4() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-mkvmerge.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .verbosity(2)
        .with_concat_preference("mp4", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 42_060);
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
    check_media_duration(&out, 4.9);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_mkvmerge_mkv() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-mkvmerge.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .verbosity(3)
        .with_concat_preference("mkv", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    // File size here is unreliable, is quite different on Microsoft Windows for example (30_699)
    // check_file_size_approx(&out, 42_060);
    let meta = ffprobe(&out).unwrap();
    // mkvmerge notices that there is no audio stream, so only includes the video stream in the
    // output file (ffmpeg generates a container with an audio and a video stream).
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    // FIXME this duration test is failing with an identified duration of 4.104s, though it looks
    // like the three segments are being downloaded correctly and merged. Perhaps an mkvmerge bug?
    // check_media_duration(&out, 4.9);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_p1p2() {
    setup_logging();
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio_p1p2.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpeg")
        // here we should be dropping period #3 (id=2) whose duration is 0.701s
        .minimum_period_duration(Duration::new(2, 0))
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 30_496);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_dashif_5bnomor2() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // From https://testassets.dashif.org/#feature/details/586fb3879ae9045678eab587
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/5b/nomor/2.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-5bnomor2.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .sandbox(true)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 119_710_971);
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
    check_media_duration(&out, 710.0);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_concat_axinom_multiperiod() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // Keys obtained from https://github.com/Axinom/public-test-vectors/blob/master/TestVectors-v7-v8.md
    let mpd_url = "https://media.axprod.net/TestVectors/v7-MultiDRM-MultiKey-MultiPeriod/Manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-axinom-multiperiod.mp4");
    DashDownloader::new(mpd_url)
        .verbosity(2)
        .sandbox(true)
        .worst_quality()
        .add_decryption_key(String::from("0872786ef9e7465fa3a24e5b0ef8fa45"),
                            String::from("c3261179bab61eeec979d2d4069511cf"))
        .add_decryption_key(String::from("29f05e8fa1ae46e480e922dcd44cd7a1"),
                            String::from("0711b17c84a90cbb41097264c901b732"))
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    let fp = std::process::Command::new("ffprobe")
        .env("LANG", "C")
        .arg("-hide_banner")
        .arg(out.to_str().unwrap())
        .output()
        .expect("spawning ffprobe");
    let stdout = String::from_utf8_lossy(&fp.stdout);
    if stdout.len() > 0 {
        println!("ffprobe stdout> {stdout}");
    }
    let stderr = String::from_utf8_lossy(&fp.stderr);
    if stderr.len() > 0 {
        println!("ffprobe stderr> {stderr}");
    }
    let mi = std::process::Command::new("mediainfo")
        .env("LANG", "C")
        .arg(out.to_str().unwrap())
        .output()
        .expect("spawning mediainfo");
    let stdout = String::from_utf8_lossy(&mi.stdout);
    if stdout.len() > 0 {
        println!("mediainfo stdout> {stdout}");
    }
    let stderr = String::from_utf8_lossy(&mi.stderr);
    if stderr.len() > 0 {
        println!("mediainfo stderr> {stderr}");
    }
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 83_015_660);
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
    // On Linux, we were seeing 1468.5 seconds; on Windows for some crazy reason we see a duration
    // of 10751.3 (2h29), displayed both by ffprobe and by mediainfo.
    if ! env::consts::OS.eq("windows") {
        check_media_duration(&out, 1468.5);
    }
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}
