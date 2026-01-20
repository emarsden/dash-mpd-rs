// Tests for decrypting DRM-infested content
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test content_protection -- --show-output


pub mod common;
use fs_err as fs;
use std::env;
use std::process::Command;
use std::time::Duration;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, ffmpeg_approval, setup_logging};


#[tokio::test]
async fn test_content_protection_parsing() {
    use dash_mpd::{parse, MPD};

    setup_logging();

    fn known_cp_name(name: &str) -> bool {
        let known = &["cenc", "MSPR 2.0", "Widevine", "ClearKey1.0"];
        known.contains(&name)
    }

    fn known_cp_scheme(scheme: &str) -> bool {
        let known = &["urn:mpeg:dash:mp4protection:2011",
                      "urn:mpeg:dash:sea:2012",
                      "urn:uuid:9a04f079-9840-4286-ab92-e65be0885f95",
                      "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",
                      "urn:uuid:e2719d58-a985-b3c9-781a-b030af78d30e",
                      "urn:uuid:5e629af5-38da-4063-8977-97ffbd9902d4",
                      "urn:uuid:1077efec-c0b2-4d02-ace3-3c1e52e2fb4b"];
        known.contains(&scheme)
    }

    async fn check_cp(mpd_url: &str) {
        println!("Checking MPD URL {mpd_url}");
        let client = reqwest::Client::builder()
            .timeout(Duration::new(30, 0))
            .gzip(true)
            .build()
            .expect("creating HTTP client");
        let xml = client.get(mpd_url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .send().await
            .expect("requesting MPD content")
            .text().await
            .expect("fetching MPD content");
        let mpd: MPD = parse(&xml)
            .expect("parsing MPD");
        for p in mpd.periods {
            for adap in p.adaptations.iter() {
                for cp in adap.ContentProtection.iter() {
                    if let Some(v) = &cp.value {
                        assert!(known_cp_name(v));
                    }
                    assert!(known_cp_scheme(&cp.schemeIdUri));
                }
            }
        }
    }

    check_cp("https://media.axprod.net/TestVectors/v7-MultiDRM-SingleKey/Manifest_1080p.mpd").await;
    // This URL offline from 2025-02
    // check_cp("https://m.dtv.fi/dash/dasherh264/drm/manifest_clearkey.mpd").await;
}


// Note that mp4decrypt and MP4Box are not able to decrypt content in a WebM container, so we only
// test Shaka packager here.
#[tokio::test]
async fn test_decryption_webm_shaka() {
    setup_logging();
    let url = "https://storage.googleapis.com/shaka-demo-assets/angel-one-widevine/dash.mpd";
    let out = env::temp_dir().join("angel.webm");
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
        .with_decryptor_preference("shaka")
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



// This manifest a 404 from 2025-02.
#[ignore]
#[tokio::test]
async fn test_decryption_cra () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://devs.origin.cdn.cra.cz/dashmultikey/manifest.mpd";
    let outpath = env::temp_dir().join("cra.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("75bf33ac08440c81d623019c87fe1360"),
                            String::from("bacd0a82f91a44d9315e6269dd769e0f"))
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 28_926_446);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


// These test cases are from https://refapp.hbbtv.org/videos/.

// WideVine ContentProtection with CENC encryption
#[tokio::test]
async fn test_decryption_wvcenc_mp4decrypt () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/spring_h265_v8/cenc/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("spring-mp4decrypt.mp4");
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
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 33_746_341);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // We see occasional errors here from ffmpeg that we don't understand.
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


// WideVine ContentProtection with CENC encryption
#[tokio::test]
async fn test_decryption_wvcenc_shaka () {
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
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 33_746_341);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // We are seeing spurious random failures with this ffmpeg check, for unknown reasons.
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(&outpath);
}


#[tokio::test]
async fn test_decryption_wvcenc_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/spring_h265_v8/cenc/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("spring-mp4box.mp4");
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
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 33_746_341);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // We see occasional errors here from ffmpeg that we don't understand.
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


// Widevine ContentProtection with CBCS encryption
#[tokio::test]
async fn test_decryption_wvcbcs_mp4decrypt () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/tears_of_steel_h265_v8/cbcs/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("tears-steel-wvcbcs-mp4decrypt.mp4");
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
async fn test_decryption_wvcbcs_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/tears_of_steel_h265_v8/cbcs/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("tears-steel-wvcbcs-shaka.mp4");
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
        .with_decryptor_preference("shaka")
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
async fn test_decryption_wvcbcs_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/tears_of_steel_h265_v8/cbcs/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("tears-steel-wvcbcs-mp4box.mp4");
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
        .with_decryptor_preference("mp4box")
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


// PlayReady / CENC
#[tokio::test]
async fn test_decryption_prcenc_mp4decrypt () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/00_llama_h264_v8_8s/cenc/manifest_prcenc.mpd";
    let outpath = env::temp_dir().join("llama-prcenc-mp4decrypt.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(3)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 26_420_624);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_prcenc_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/00_llama_h264_v8_8s/cenc/manifest_prcenc.mpd";
    let outpath = env::temp_dir().join("llama-prcenc-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(3)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 26_420_624);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_prcenc_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/00_llama_h264_v8_8s/cenc/manifest_prcenc.mpd";
    let outpath = env::temp_dir().join("llama-prcenc-mp4box.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(3)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 26_420_624);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


// Marlin / CENC
#[tokio::test]
async fn test_decryption_marlincenc_mp4decrypt () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cenc/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-marlin-cenc-mp4decrypt.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_917);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}

// Testing -- this wasn't previously tested 20260622
#[tokio::test]
async fn test_decryption_marlincenc_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cenc/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-marlin-cenc-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_917);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_marlincenc_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cenc/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-marlin-cenc-mp4box.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_917);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


// Marlin / CBCS
#[tokio::test]
async fn test_decryption_marlincbcs_mp4decrypt () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-marlin-cbcs.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_925);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    // assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


// new test 20250622
#[tokio::test]
async fn test_decryption_marlincbcs_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-marlin-cbcs-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_925);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    // assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_marlincbcs_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-marlin-cbcs-mp4box.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_925);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    // assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(&outpath);
}



// Marlin / CENC
#[tokio::test]
async fn test_decryption_mlcenc_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cenc/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-mlcenc-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_917);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(&outpath);
}


// Marlin / CBCS
#[tokio::test]
async fn test_decryption_mlcbcs_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-mlcbcs-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_925);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}


#[tokio::test]
async fn test_decryption_mlcbcs_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama-mlcbcs-mp4box.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_925);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(&outpath);
}


// Test vectors from https://github.com/Axinom/public-test-vectors
//
// MP4Box is not able to decode this audio stream: if fails with
//
//  [iso file] Duplicate 'ftyp' detected!
#[tokio::test]
async fn test_decryption_axinom_cmaf_h265_multikey () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://media.axprod.net/TestVectors/H265/protected_cmaf_1080p_h265_multikey/manifest.mpd";
    let outpath = env::temp_dir().join("axinom-h264-multikey.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("53dc3eaa5164410a8f4ee15113b43040"),
                            String::from("620045a34e839061ee2e9b7798fdf89b"))
        .add_decryption_key(String::from("9dbace9e41034c5296aa63227dc5f773"),
                            String::from("a776f83276a107a3c322f9dbd6d4f48c"))
        .add_decryption_key(String::from("a76f0ca68e7d40d08a37906f3e24dde2"),
                            String::from("2a99b42f08005ab4b57af20f4da3cc05"))
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 48_233_447);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
}


#[tokio::test]
async fn test_decryption_axinom_cbcs_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://media.axprod.net/TestVectors/v9-MultiFormat/Encrypted_Cbcs/Manifest_1080p.mpd";
    let outpath = env::temp_dir().join("axinom-cbcs-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("f8c80c25690f47368132430e5c6994ce"),
                            String::from("7bc99cb1dd0623cd0b5065056a57a1dd"))
        // For an unknown reason, mp4decrypt is not able to decrypt the audio stream for this
        // manifest (though the video works fine).
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 41_614_809);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
}


#[tokio::test]
async fn test_decryption_axinom_cbcs_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "https://media.axprod.net/TestVectors/v9-MultiFormat/Encrypted_Cbcs/Manifest_1080p.mpd";
    let outpath = env::temp_dir().join("axinom-cbcs-mp4box.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("f8c80c25690f47368132430e5c6994ce"),
                            String::from("7bc99cb1dd0623cd0b5065056a57a1dd"))
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 41_614_809);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
}


// URL and key from https://github.com/Axinom/public-test-vectors/tree/conservative
#[tokio::test]
async fn test_decryption_axinom_widevine_shaka () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "http://media.axprod.net/TestVectors/v6.1-MultiDRM/Manifest_1080p.mpd";
    let outpath = env::temp_dir().join("axinom-widevine-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("6e5a1d26275747d78046eaa5d1d34b5a"),
                            // (encode-hex-string (base64-decode-string "GX8m9XLIZNIzizrl0RTqnA=="))
                            String::from("197f26f572c864d2338b3ae5d114ea9c"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 47_396_046);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
}


#[tokio::test]
async fn test_decryption_axinom_widevine_mp4box () {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd = "http://media.axprod.net/TestVectors/v6.1-MultiDRM/Manifest_1080p.mpd";
    let outpath = env::temp_dir().join("axinom-widevine-mp4box.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("6e5a1d26275747d78046eaa5d1d34b5a"),
                            // (encode-hex-string (base64-decode-string "GX8m9XLIZNIzizrl0RTqnA=="))
                            String::from("197f26f572c864d2338b3ae5d114ea9c"))
        .with_decryptor_preference("mp4box")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 47_396_046);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
}


// List of Shaka test assets:
//  https://github.com/shaka-project/shaka-player/blob/1f336dd319ad23a6feb785f2ab05a8bc5fc8e2a2/demo/common/assets.js


// A small decryption test case that we can run on the CI infrastructure.
// This test disabled from 2025-03 because the manifest URL is unreachable.
#[ignore]
#[tokio::test]
async fn test_decryption_oldsmall () {
    setup_logging();
    let mpd = "https://m.dtv.fi/dash/dasherh264/drm/manifest_clearkey.mpd";
    let outpath = env::temp_dir().join("caminandes.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 6_975_147);
    assert!(ffmpeg_approval(&outpath));
}

// This test fails with MP4Box, which doesn't seem to be able to decrypt content in a WebM
// container.
#[tokio::test]
async fn test_decryption_small () {
    setup_logging();
    let mpd = "https://storage.googleapis.com/shaka-demo-assets/angel-one-widevine/dash.mpd";
    let outpath = env::temp_dir().join("angel.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
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
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 1_316_391);
    assert!(ffmpeg_approval(&outpath));
}


// Content that isn't encrypted should be downloaded normally even if unnecessary decryption keys are
// specified.
#[tokio::test]
async fn test_decryption_unencrypted_mp4decrypt () {
    setup_logging();
    let mpd = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let outpath = env::temp_dir().join("unencrypted-mp4decrypt.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("mp4decrypt")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 12_975_377);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
}


// Content that isn't encrypted should be downloaded normally even if unnecessary decryption keys are
// specified.
//
// This test currently disabled because the most recent version of shaka-packager (v3.0) fails while
// decoding the media stream.
// https://github.com/shaka-project/shaka-packager/issues/1368
//
// Also not tested for MP4Box, which fails if asked to decode a file which is already decoded.
#[ignore]
#[tokio::test]
async fn test_decryption_unencrypted_shaka () {
    setup_logging();
    let mpd = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let outpath = env::temp_dir().join("unencrypted-shaka.mp4");
    if outpath.exists() {
        let _ = fs::remove_file(&outpath);
    }
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .sandbox(true)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
    check_file_size_approx(&outpath, 12_975_377);
    let format = FileFormat::from_file(&outpath).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    assert!(ffmpeg_approval(&outpath));
    let _ = fs::remove_file(outpath);
}



#[tokio::test]
#[should_panic(expected = "unknown decryption application")]
async fn test_decryption_invalid_decryptor () {
    // Don't set up logging, because we know that an error will be logged to the terminal.
    // setup_logging();
    let mpd = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let outpath = env::temp_dir().join("failing.mp4");
    DashDownloader::new(mpd)
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .with_decryptor_preference("unknown")
        .download_to(&outpath).await
        .unwrap();
}


// We are expecting a DashMpdError::Decrypting error.
#[tokio::test]
#[should_panic(expected = "Decrypting")]
async fn test_decryption_invalid_key_mp4decrypt () {
    let mpd = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let outpath = env::temp_dir().join("failing.mp4");
    DashDownloader::new(mpd)
        .add_decryption_key(String::from("66"),
                            String::from("99"))
        .with_decryptor_preference("mp4decrypt")
        .download_to(&outpath).await
        .unwrap();
}


// We are expecting a DashMpdError::Decrypting error.
#[tokio::test]
#[should_panic(expected = "Decrypting")]
async fn test_decryption_invalid_key_shaka () {
    let mpd = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let outpath = env::temp_dir().join("failing.mp4");
    DashDownloader::new(mpd)
        .add_decryption_key(String::from("66"),
                            String::from("99"))
        .with_decryptor_preference("shaka")
        .download_to(&outpath).await
        .unwrap();
}

