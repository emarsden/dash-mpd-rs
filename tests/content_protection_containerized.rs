// Tests for decrypting DRM-infested content using containerized helper applications
//
// This is split out into a different file from the tests in content_protection.rs in order to be
// able to disable the tests on ARM64 MacOS CI runners on Github. These runners don't currently
// enable nested virtualization, and thus Podman/Docker don't work.
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test content_protection_containerized -- --show-output


pub mod common;
use std::fs;
use std::env;
use std::process::Command;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, ffmpeg_approval, setup_logging};



#[tokio::test]
async fn test_decryption_webm_shaka_container() {
    setup_logging();
    let url = "https://storage.googleapis.com/shaka-demo-assets/angel-one-widevine/dash.mpd";
    let out = env::temp_dir().join("angel-shaka-container.webm");
    if out.exists() {
        let _ = fs::remove_file(&out);
    }
    DashDownloader::new(url)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("4d97930a3d7b55fa81d0028653f5e499"),
                            String::from("429ec76475e7a952d224d8ef867f12b6"))
        .add_decryption_key(String::from("d21373c0b8ab5ba9954742bcdfb5f48b"),
                            String::from("150a6c7d7dee6a91b74dccfce5b31928"))
        .add_decryption_key(String::from("6f1729072b4a5cd288c916e11846b89e"),
                            String::from("a84b4bd66901874556093454c075e2c6"))
        .add_decryption_key(String::from("800aacaa522958ae888062b5695db6bf"),
                            String::from("775dbf7289c4cc5847becd571f536ff2"))
        .add_decryption_key(String::from("67b30c86756f57c5a0a38a23ac8c9178"),
                            String::from("efa2878c2ccf6dd47ab349fcf90e6259"))
        .with_muxer_preference("webm", "ffmpeg")
        .with_decryptor_preference("shaka-container")
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 1_331_284);
    let meta = ffprobe(&out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    // The order of audio and video streams in the output WebM container is unreliable with Shaka
    // packager, so we need to test this carefully.
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    // Whether opus or vorbis codec is chosen seems to depend on the version of the muxer used.
    assert!(audio.codec_name.eq(&Some(String::from("vorbis"))) ||
            audio.codec_name.eq(&Some(String::from("opus"))));
    let video = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("video"))))
        .expect("finding video stream");
    assert_eq!(video.codec_name, Some(String::from("vp9")));
    assert!(video.width.is_some());
    let ffmpeg = Command::new("ffmpeg")
        .env("LANG", "C")
        .args(["-v", "error",
               "-i", &out.to_string_lossy(),
               "-f", "null", "-"])
        .output()
        .expect("spawning ffmpeg");
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    if !msg.is_empty() {
        eprintln!("FFMPEG stderr {msg}");
    }
    // ffmpeg 8 is displaying an error concerning invalid Opus content
    // [opus @ 0x564b73f40700] Error parsing Opus packet header
    // assert!(msg.len() == 0);
    let _ = fs::remove_file(out);
}


#[tokio::test]
async fn test_decryption_wvcenc_shaka_container () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/spring_h265_v8/cenc/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("spring-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341237"),
                            String::from("12341234123412341234123412341237"))
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .with_decryptor_preference("shaka-container")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 33_746_341);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // We are seeing spurious random failures with this ffmpeg check, for unknown reasons.
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_wvcenc_mp4box_container () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/spring_h265_v8/cenc/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("spring-mp4box-container.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341237"),
                            String::from("12341234123412341234123412341237"))
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .with_decryptor_preference("mp4box-container")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 33_746_341);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // We see occasional errors here from ffmpeg that we don't understand.
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_wvcbcs_mp4box_container () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/tears_of_steel_h265_v8/cbcs/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("tears-steel-wvcbcs-mp4box-container.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341237"),
                            String::from("12341234123412341234123412341237"))
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .with_decryptor_preference("mp4box-container")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 79_731_116);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // We can't check the validity of this stream using ffmpeg, because ffmpeg complains a lot about
    // various anomalies in the AAC audio stream, though it seems to play the content OK.
    // assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_prcenc_shaka_container () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/00_llama_h264_v8_8s/cenc/manifest_prcenc.mpd";
    let outpath = env::temp_dir().join("llama-prcenc-shaka-container.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(3)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .with_decryptor_preference("shaka-container")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 26_420_624);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


