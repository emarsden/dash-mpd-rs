//! Testing for correct handling of data: URLs (sometimes used for init segment in a DASH manifest).
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
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::Builder;
use axum::{routing::get, Router};
use axum::http::header;
use axum_server::{Handle, bind};
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::{MPD, Period, AdaptationSet, Representation, Initialization, SegmentList, SegmentURL};
use dash_mpd::fetch::DashDownloader;
use anyhow::Result;
use common::{check_file_size_approx, check_media_duration, setup_logging};



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
        xmlns: Some("urn:mpeg:dash:schema:mpd:2011".to_string()),
        mpdtype: Some("static".to_string()),
        periods: vec!(period),
        ..Default::default()
    };
    let xml = mpd.to_string();
    let app = Router::new()
        .route("/mpd", get(|| async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml) }));
    let server_handle: Handle<SocketAddr> = Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        bind("127.0.0.1:6666".parse().unwrap())
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
        .download_to(&out).await
        .unwrap();
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(100));

    // Check that the background colors in the reassembled video at timestamps located respectively
    // in the first, second and third 5-second part correspond to the initial input video which
    // contained red, green and blue.
    check_frame_color(&out, "00:00:03", &[255, 0, 0]);
    check_frame_color(&out, "00:00:08", &[0, 255, 0]);
    check_frame_color(&out, "00:00:13", &[0, 0, 255]);
    server_handle.shutdown();
    Ok(())
}



// This is https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd converted to a data URL
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dataurl_big_bunny() {
    setup_logging();
    let data_url = "data:application/dash+xml;charset=utf-8,%3CMPD%20mediaPresentationDuration=%22PT634.566S%22%20minBufferTime=%22PT2.00S%22%20profiles=%22urn:hbbtv:dash:profile:isoff-live:2012,urn:mpeg:dash:profile:isoff-live:2011%22%20type=%22static%22%20xmlns=%22urn:mpeg:dash:schema:mpd:2011%22%20xmlns:xsi=%22http://www.w3.org/2001/XMLSchema-instance%22%20xsi:schemaLocation=%22urn:mpeg:DASH:schema:MPD:2011%20DASH-MPD.xsd%22%3E%20%3CBaseURL%3Ehttps://dash.akamaized.net/akamai/bbb_30fps/%3C/BaseURL%3E%20%3CPeriod%3E%20%20%3CAdaptationSet%20mimeType=%22video/mp4%22%20contentType=%22video%22%20subsegmentAlignment=%22true%22%20subsegmentStartsWithSAP=%221%22%20par=%2216:9%22%3E%20%20%20%3CSegmentTemplate%20duration=%22120%22%20timescale=%2230%22%20media=%22$RepresentationID$/$RepresentationID$_$Number$.m4v%22%20startNumber=%221%22%20initialization=%22$RepresentationID$/$RepresentationID$_0.m4v%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_1024x576_2500k%22%20codecs=%22avc1.64001f%22%20bandwidth=%223134488%22%20width=%221024%22%20height=%22576%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_1280x720_4000k%22%20codecs=%22avc1.64001f%22%20bandwidth=%224952892%22%20width=%221280%22%20height=%22720%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_1920x1080_8000k%22%20codecs=%22avc1.640028%22%20bandwidth=%229914554%22%20width=%221920%22%20height=%221080%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_320x180_200k%22%20codecs=%22avc1.64000d%22%20bandwidth=%22254320%22%20width=%22320%22%20height=%22180%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_320x180_400k%22%20codecs=%22avc1.64000d%22%20bandwidth=%22507246%22%20width=%22320%22%20height=%22180%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_480x270_600k%22%20codecs=%22avc1.640015%22%20bandwidth=%22759798%22%20width=%22480%22%20height=%22270%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_640x360_1000k%22%20codecs=%22avc1.64001e%22%20bandwidth=%221254758%22%20width=%22640%22%20height=%22360%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_640x360_800k%22%20codecs=%22avc1.64001e%22%20bandwidth=%221013310%22%20width=%22640%22%20height=%22360%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_768x432_1500k%22%20codecs=%22avc1.64001e%22%20bandwidth=%221883700%22%20width=%22768%22%20height=%22432%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_30fps_3840x2160_12000k%22%20codecs=%22avc1.640033%22%20bandwidth=%2214931538%22%20width=%223840%22%20height=%222160%22%20frameRate=%2230%22%20sar=%221:1%22%20scanType=%22progressive%22/%3E%20%20%3C/AdaptationSet%3E%20%20%3CAdaptationSet%20mimeType=%22audio/mp4%22%20contentType=%22audio%22%20subsegmentAlignment=%22true%22%20subsegmentStartsWithSAP=%221%22%3E%20%20%20%3CAccessibility%20schemeIdUri=%22urn:tva:metadata:cs:AudioPurposeCS:2007%22%20value=%226%22/%3E%20%20%20%3CRole%20schemeIdUri=%22urn:mpeg:dash:role:2011%22%20value=%22main%22/%3E%20%20%20%3CSegmentTemplate%20duration=%22192512%22%20timescale=%2248000%22%20media=%22$RepresentationID$/$RepresentationID$_$Number$.m4a%22%20startNumber=%221%22%20initialization=%22$RepresentationID$/$RepresentationID$_0.m4a%22/%3E%20%20%20%3CRepresentation%20id=%22bbb_a64k%22%20codecs=%22mp4a.40.5%22%20bandwidth=%2267071%22%20audioSamplingRate=%2248000%22%3E%20%20%20%20%3CAudioChannelConfiguration%20schemeIdUri=%22urn:mpeg:dash:23003:3:audio_channel_configuration:2011%22%20value=%222%22/%3E%20%20%20%3C/Representation%3E%20%20%3C/AdaptationSet%3E%20%3C/Period%3E%3C/MPD%3E";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("data-url-bigbunny.mp4");
    DashDownloader::new(data_url)
        .worst_quality()
        .max_error_count(5)
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 21_205_589);
    check_media_duration(&out, 634.57);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}

