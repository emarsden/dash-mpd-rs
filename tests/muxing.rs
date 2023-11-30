// Tests for MPD muxing support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test muxing -- --show-output

pub mod common;
use fs_err as fs;
use std::env;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;


// We can't check file size for this test, as depending on whether mkvmerge or ffmpeg or mp4box are
// used to copy the video stream into the Matroska container (depending on which one is installed),
// the output file size varies quite a lot.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_mkvmerge() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        // Some useless arguments to increase test coverage
        .with_ffmpeg("/usr/bin/ffmpeg")
        .with_vlc("/usr/bin/vlc")
        .with_mkvmerge("/usr/bin/mkvmerge")
        .with_mp4box("/usr/bin/MP4Box")
        .with_mp4decrypt("/usr/bin/mp4decrypt")
        .with_shaka_packager("shaka-packager")
        .with_muxer_preference("mkv", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    check_file_size_approx(&out, 6_652_846);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    assert!(video.width.is_some());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_mkvmerge_audio() {
    let mpd_url = "http://yt-dash-mse-test.commondatastorage.googleapis.com/media/car-20120827-manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("audio-only.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_video(false)
        .fetch_subtitles(false)
        .with_muxer_preference("mkv", "mkvmerge")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 720_986);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaAudio);
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


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_ffmpeg_avi() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.avi");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("avi", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::AudioVideoInterleave);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_ffmpeg_mkv() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("mkv", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 6_629_479);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

// Expect this to print INFO diagnostics to stderr complaining that the audio and video codecs are
// not compatible with a WebM container (we are running ffmpeg with "-c:v copy -c:a copy" which
// prevents re-encoding), then the second ffmpeg run (with reencoding allowed) should succeed.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_ffmpeg_webm() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.webm");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("webm", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    // Note that encoded to VP9/opus it's much smaller than the HEVC/AAC original...
    check_file_size_approx(&out, 3_511_874);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Webm);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("opus")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("vp9")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_ffmpeg_audio() {
    // This manifest has segments in fragmented WebM and fragmented MP4 formats, in different
    // representations using different encodings. The audio Representation with the lowest bandwidth
    // uses vorbis codec and a WebM container, so it's the one which is selected here. This means
    // that instead of simply copying the appended segments (that are already in an MP4 container),
    // we need to mux the WebM container into an MP4 container (in the function
    // copy_audio_to_container in ffmpeg.rs), here using ffmpeg.
    let mpd_url = "https://turtle-tube.appspot.com/t/t2/dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("audio-only.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_video(false)
        .fetch_subtitles(false)
        .with_muxer_preference("mp4", "ffmpeg")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 2_661_567);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Audio);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert!(audio.width.is_none());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_vlc_mp4() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("mp4", "vlc")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_vlc_mkv() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("mkv", "vlc")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_vlc_webm() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.webm");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("webm", "vlc")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 3_509_566);
    let format = FileFormat::from_file(out.clone()).unwrap();
    // Yes, VLC's webm muxer generates a Matroska container that isn't recognized as WebM...
    assert_eq!(format, FileFormat::MatroskaVideo);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("vorbis")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("vp9")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_mp4box() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://refapp.hbbtv.org/videos/01_llama_drama_2160p_25f75g6sv3/manifest.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("muxing-llama.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_muxer_preference("mp4", "mp4box")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 6_652_846);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("hevc")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_muxing_mp4box_audio() {
    // This manifest has segments in fragmented WebM and fragmented MP4 formats, in different
    // representations using different encodings. The audio Representation with the lowest bandwidth
    // uses vorbis codec and a WebM container, so it's the one which is selected here. This means
    // that instead of simply copying the appended segments (that are already in an MP4 container),
    // we need to mux the WebM container into an MP4 container (in the function
    // copy_audio_to_container in ffmpeg.rs), here using mp4box.
    let mpd_url = "https://turtle-tube.appspot.com/t/t2/dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("audio-only.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_video(false)
        .fetch_subtitles(false)
        .with_muxer_preference("mp4", "mp4box")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 2_221_430);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Audio);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("vorbis")));
    assert!(audio.width.is_none());
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


// Test failure case if we request muxing applications that aren't installed. We should also see two
// warnings printed to stderr "Ignoring unknown muxer preference unavailable", but can't currently
// test for that.
#[tokio::test]
#[cfg(not(feature = "libav"))]
#[should_panic(expected = "all muxers failed")]
async fn test_muxing_unavailable() {
    let mpd_url = "https://m.dtv.fi/dash/dasherh264/manifest.mpd";
    let out = env::temp_dir().join("unexist.mp3");
    DashDownloader::new(mpd_url)
        .with_muxer_preference("mp3", "unavailable,nothere")
        .download_to(out.clone()).await
        .unwrap();

}


