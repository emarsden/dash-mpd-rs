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
use test_log::test;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_noaudio_ffmpeg() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
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


// mkvmerge cannot concat this stream to MP4: failure with error message
//   Quicktime/MP4 reader: Could not read chunk number 48/62 with size 1060 from position 15936. Aborting.
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all concat helpers failed")]
async fn test_concat_noaudio_mkvmerge_mp4() {
    if env::var("CI").is_ok() {
        panic!("all concat helpers failed");
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-mkvmerge.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let _ = fs::remove_dir_all(tmpd);
}

// mkvmerge fails to concat. Check that the fallback to ffmpeg as a concat helper works 
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_noaudio_mkv_concat_fallback() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/twoperiodsOR.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-noaudio-mkvmerge.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
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


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_singleases_ffmpeg() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/singleases.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-singleases-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
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

// mkvmerge is unable to concatenate these streams: fails with an error message
//
//   The track number 0 from the file '/tmp/.tmpkgxD7n/concat-singleases-mkvmerge-p2.mp4' can
//   probably not be appended correctly to the track number 0 from the file
//   '/tmp/dashmpdrsbnMoM.mkv': The codec's private data does not match. Both have the same length
//   (41) but different content.
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all concat helpers failed")]
async fn test_concat_singleases_mkvmerge() {
    if env::var("CI").is_ok() {
        panic!("all concat helpers failed");
    }
    let mpd_url = "https://dash.akamaized.net/fokus/adinsertion-samples/xlink/singleases.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-singleases-mkvmerge.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "mkvmerge")
        .minimum_period_duration(Duration::new(10, 0))
        .download_to(out.clone()).await
        .unwrap();
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_ffmpeg_mp4() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-ffmpeg.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 42_060);
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

#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_ffmpeg_mkv() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-ffmpeg.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mkv", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    check_file_size_approx(&out, 42_060);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    // ffmpeg decides to reencode the original aac to vorbis for the mkv container
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

// mkvmerge fails to concatenate this stream with error
//
//   Quicktime/MP4 reader: Could not read chunk number 48/62 with size 1060 from position 15936. Aborting.
#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all concat helpers failed")]
async fn test_concat_heliocentrism_mkvmerge_mp4() {
    if env::var("CI").is_ok() {
        panic!("all concat helpers failed");
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-mkvmerge.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 42_060);
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


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_mkvmerge_mkv() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio-mkvmerge.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mkv", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    check_file_size_approx(&out, 42_060);
    let meta = ffprobe(out).unwrap();
    // mkvmerge notices that there is no audio stream, so only includes the video stream in the
    // output file (ffmpeg generates a container with an audio and a video stream).
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


#[test(tokio::test)]
#[cfg(not(feature = "libav"))]
async fn test_concat_heliocentrism_p1p2() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("concat-helio_p1p2.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_concat_preference("mp4", "ffmpeg")
        // here we should be dropping period #3 (id=2) whose duration is 0.701s
        .minimum_period_duration(Duration::new(2, 0))
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    check_file_size_approx(&out, 33_967);
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


