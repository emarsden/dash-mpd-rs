// Tests for MPD download support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test content_protection -- --show-output


pub mod common;
use std::env;
use std::process::Command;
use std::time::Duration;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, ffmpeg_approval};


#[tokio::test]
async fn test_content_protection_parsing() {
    use dash_mpd::{parse, MPD};

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
                    assert!(cp.schemeIdUri.is_some());
                    if let Some(s) = &cp.schemeIdUri {
                        assert!(known_cp_scheme(s));
                    }
                }
            }
        }
    }

    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    check_cp("https://media.axprod.net/TestVectors/v7-MultiDRM-SingleKey/Manifest_1080p.mpd").await;
    check_cp("https://m.dtv.fi/dash/dasherh264/drm/manifest_clearkey.mpd").await;
}


// Download a stream with ContentProtection and check that it generates many decoding errors when
// "played" (to a null output device) with ffmpeg. Also check that when the same stream is
// downloaded and decryption keys are provided, there are no playback errors.
#[tokio::test]
async fn test_decryption() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let url = "https://storage.googleapis.com/shaka-demo-assets/angel-one-widevine/dash.mpd";
    let out_encrypted = env::temp_dir().join("angel-encrypted.mp4");
    let out_decrypted = env::temp_dir().join("angel-decrypted.mp4");
    DashDownloader::new(url)
        .worst_quality()
        .download_to(out_encrypted.clone()).await
        .unwrap();
    DashDownloader::new(url)
        .worst_quality()
        .verbosity(2)
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
        .download_to(out_decrypted.clone()).await
        .unwrap();
    let ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error",
               "-i", &out_decrypted.to_string_lossy(),
               "-f", "null", "-"])
        .output()
        .expect("spawning ffmpeg");
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    if msg.len() > 0 {
        eprintln!("FFMPEG stderr {msg}");
    }
    assert!(msg.len() == 0);
    let ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error",
               "-i", &out_encrypted.to_string_lossy(),
               "-f", "null", "-"])
        .output()
        .expect("spawning ffmpeg");
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    assert!(msg.len() > 10_000);
}


// These test cases are from https://refapp.hbbtv.org/videos/.
#[tokio::test]
async fn test_decryption_variants () {
    if env::var("CI").is_ok() {
        return;
    }
    
    // WideVine ContentProtection with CENC encryption
    let mpd = "https://refapp.hbbtv.org/videos/spring_h265_v8/cenc/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("spring.mp4");
    DashDownloader::new(mpd)
        .worst_quality()
        .add_decryption_key(String::from("43215678123412341234123412341237"),
                            String::from("12341234123412341234123412341237"))
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .download_to(outpath.clone()).await
        .unwrap();
    check_file_size_approx(&outpath, 33_746_341);
    assert!(ffmpeg_approval(&outpath));
    
    // Widevine ContentProtection with CBCS encryption
    let mpd = "https://refapp.hbbtv.org/videos/tears_of_steel_h265_v8/cbcs/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("tears-steel.mp4");
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(2)
        .add_decryption_key(String::from("43215678123412341234123412341237"),
                            String::from("12341234123412341234123412341237"))
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .download_to(outpath.clone()).await
        .unwrap();
    check_file_size_approx(&outpath, 79_731_116);
    // We can't check the validity of this stream using ffmpeg, because ffmpeg complains a lot about
    // various anomalies in the AAC audio stream, though it seems to play the content OK.
    // assert!(ffmpeg_approval(&outpath));

    // PlayReady / CENC
    let mpd = "https://refapp.hbbtv.org/videos/00_llama_h264_v8_8s/cenc/manifest_prcenc.mpd";
    let outpath = env::temp_dir().join("llama.mp4");
    DashDownloader::new(mpd)
        .worst_quality()
        .verbosity(3)
        .add_decryption_key(String::from("43215678123412341234123412341236"),
                            String::from("12341234123412341234123412341236"))
        .download_to(outpath.clone()).await
        .unwrap();
    check_file_size_approx(&outpath, 26_420_624);
    assert!(ffmpeg_approval(&outpath));

    // Marlin / CENC
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cenc/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama.mp4");
    DashDownloader::new(mpd)
        .worst_quality()
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .download_to(outpath.clone()).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_917);
    assert!(ffmpeg_approval(&outpath));

    // Marlin / CBCS
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama.mp4");
    DashDownloader::new(mpd)
        .worst_quality()
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .download_to(outpath.clone()).await
        .unwrap();
    check_file_size_approx(&outpath, 14_357_925);
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    // assert!(ffmpeg_approval(&outpath));
}



// A small decryption test case that we can run on the CI infrastructure.
#[tokio::test]
async fn test_decryption_small () {
    let mpd = "https://m.dtv.fi/dash/dasherh264/drm/manifest_clearkey.mpd";
    let outpath = env::temp_dir().join("caminandes.mp4");
    DashDownloader::new(mpd)
        .worst_quality()
        .add_decryption_key(String::from("43215678123412341234123412341234"),
                            String::from("12341234123412341234123412341234"))
        .download_to(outpath.clone()).await
        .unwrap();
    check_file_size_approx(&outpath, 6_975_147);
    assert!(ffmpeg_approval(&outpath));
}


