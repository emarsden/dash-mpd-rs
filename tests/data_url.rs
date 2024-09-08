// Testing for correct handling of data: URLs (sometimes used for init segment in a DASH manifest).
//
//
// To run this test while enabling printing to stdout/stderr
//
//    cargo test --test data_url -- --show-output
//
//
// We create a test video of duration 15s that contains 5 seconds of solid red, then 5 seconds of
// solid green, the 5 seconds of solid blue. The test video is segmented into an init segment and
// two media fragments. The three media fragments are embedded into a DASH manifest as data URLs
// (basically, the media content encoding in base64). We download from the DASH manifest, which
// causes the media fragments to be reassembled (concatenated). We check that the reassembled media
// file contains firstly solid red, then solid green, then solid blue, indicating that the data urls
// were correctly encoded then decoded, and that the media fragments were correctly reassembled.

pub mod common;
use fs_err as fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::Builder;
use axum::{routing::get, Router};
use axum::http::header;
use ffprobe::ffprobe;
use dash_mpd::{MPD, Period, AdaptationSet, Representation, Initialization, SegmentList, SegmentURL};
use dash_mpd::fetch::DashDownloader;
use anyhow::Result;
use common::setup_logging;


// Check that the video at timestamp has a solid color of expected_rgb.
fn check_frame_color(video: &Path, timestamp: &str, expected_rgb: &[u8; 3]) {
    use image::GenericImageView;
    
    let out = Builder::new().suffix(".png").tempfile().unwrap();
    let ffmpeg = Command::new("ffmpeg")
        .env("LANG", "C")
        .args(["-y",
               "-nostdin",
               "-ss", timestamp,
               "-i", &video.to_string_lossy(),
               "-frames:v", "1",
               "-update", "1",
               out.path().to_str().unwrap()])
        .output()
        .expect("spawning ffmpeg");
    if !ffmpeg.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg.stderr);
        eprintln!("ffmpeg stderr: {stderr}");
    }
    assert!(ffmpeg.status.success());
    let img = image::ImageReader::open(out.path())
        .unwrap().decode().unwrap();
    // We are satisfied with a simple non-perceptual distance in RGB color space here.
    for (_x, _y, rgba) in img.pixels() {
        let dr: i32 = rgba[0] as i32 - expected_rgb[0] as i32;
        let dg: i32 = rgba[1] as i32 - expected_rgb[1] as i32;
        let db: i32 = rgba[2] as i32 - expected_rgb[2] as i32;
        assert!(dr*dr + dg*dg + db*db < 20);
    }
}

// The format of a data URL is specifed by https://www.rfc-editor.org/rfc/rfc2397.
fn as_data_url(video: &Path) -> String {
    use base64::prelude::{Engine as _, BASE64_STANDARD};

    let bytes = fs::read(video).unwrap();
    "data:video/x-matroska;base64,".to_owned() + &BASE64_STANDARD.encode(bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_data_url() -> Result<()> {
    // Use ffmpeg to create a test MP4 file with 5 seconds of solid red, 5 seconds of solid green then 5
    // seconds of solid blue. Segment this file to create an initialization segment and two fragmented
    // MP4 segments.
    setup_logging();
    let tmpd = Builder::new().prefix("dash-mpd-ffmpeg").tempdir().unwrap();
    let tmpdp = tmpd.path();
    let ffmpeg = Command::new("ffmpeg")
        .env("LANG", "C")
        .current_dir(tmpdp)
        .args(["-y",
               "-nostdin",
               "-f", "lavfi", "-i", "color=c=0xff0000:size=100x100:r=10:duration=5",
               "-f", "lavfi", "-i", "color=c=0x00ff00:size=100x100:r=10:duration=5",
               "-f", "lavfi", "-i", "color=c=0x0000ff:size=100x100:r=10:duration=5",
               // Force the use of the libx264 encoder. ffmpeg defaults to platform-specific
               // encoders (which may allow hardware encoding) on certain builds, which may have
               // stronger restrictions on acceptable frame rates and so on. For example, the
               // h264_mediacodec encoder on Android has more constraints than libx264 regarding the
               // number of keyframes.
               "-c:v", "libx264",
               "-filter_complex", "[0:v:0][1:v:0][2:v:0]concat=n=3:v=1:a=0[outv]",
               "-map", "[outv]", "concat.mp4"])
        .output()
        .expect("spawning ffmpeg");
    if !ffmpeg.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg.stderr);
        eprintln!("ffmpeg stderr: {stderr}");
    }
    assert!(ffmpeg.status.success());
    let ffmpeg = Command::new("ffmpeg")
        .env("LANG", "C")
        .current_dir(tmpdp)
        .args(["-y",
               "-nostdin",
               "-i", "concat.mp4",
               "-single_file", "0",
               "-init_seg_name", "init.mp4",
               "-media_seg_name", "fragment-$Number$.mp4",
               "-seg_duration", "5", "-frag_duration", "5",
               "-f", "dash", "manifest.mpd"])
        .output()
        .expect("spawning ffmpeg");
    if !ffmpeg.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg.stderr);
        eprintln!("ffmpeg stderr: {stderr}");
    }
    assert!(ffmpeg.status.success());
    let init_bytes = tmpdp.join("init.mp4");
    let frag1_bytes = tmpdp.join("fragment-1.mp4");
    let frag2_bytes = tmpdp.join("fragment-2.mp4");
    let initialization = Initialization {
        sourceURL: Some(as_data_url(&init_bytes)),
        ..Default::default()
    };
    let seg1 = SegmentURL {
        media: Some(as_data_url(&frag1_bytes)),
        ..Default::default()
    };
    let seg2 = SegmentURL {
        media: Some(as_data_url(&frag2_bytes)),
        ..Default::default()
    };
    let segment_list = SegmentList {
        Initialization: Some(initialization),
        segment_urls: vec!(seg1, seg2),
        ..Default::default()
    };
    let rep1 = Representation {
        id: Some("1".to_string()),
        mimeType: Some("video/mp4".to_string()),
        width: Some(100),
        height: Some(100),
        SegmentList: Some(segment_list),
        ..Default::default()
    };
    let adap = AdaptationSet {
        id: Some("1".to_string()),
        contentType: Some("video".to_string()),
        representations: vec!(rep1),
        ..Default::default()
    };
    let period = Period {
        id: Some("p1".to_string()),
        duration: Some(Duration::new(15, 0)),
        adaptations: vec!(adap),
        ..Default::default()
    };
    let mpd = MPD {
        mpdtype: Some("static".to_string()),
        periods: vec!(period),
        ..Default::default()
    };
    let xml = mpd.to_string();
    let app = Router::new()
        .route("/mpd", get(|| async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml) }));
    let server_handle = hyper_serve::Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        hyper_serve::bind("127.0.0.1:6666".parse().unwrap())
            .handle(backend_handle)
            .serve(app.into_make_service()).await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let out = tmpdp.join("data-url.mp4");
    DashDownloader::new("http://localhost:6666/mpd")
        .intermediate_quality()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(100));

    // Check that the background colors in the reassembled video at timestamps located respectively
    // in the first, second and third 5-second part correspond to the initial input video which
    // contained red, green and blue.
    check_frame_color(&out.clone(), "00:00:03", &[255, 0, 0]);
    check_frame_color(&out.clone(), "00:00:08", &[0, 255, 0]);
    check_frame_color(&out.clone(), "00:00:13", &[0, 0, 255]);
    server_handle.shutdown();
    Ok(())
}
