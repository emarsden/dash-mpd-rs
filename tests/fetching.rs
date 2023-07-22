// Tests for MPD download support
//
// To run tests while enabling printing to stdout/stderr, "cargo test -- --show-output"
//
// Testing resources:
//
//   https://testassets.dashif.org/#testvector/list
//   https://ottverse.com/free-mpeg-dash-mpd-manifest-example-test-urls/
//   https://dash.itec.aau.at/dash-dataset/
//   https://github.com/streamlink/streamlink/tree/master/tests/resources/dash


use fs_err as fs;
use std::env;
use std::time::Duration;
use std::process::Command;
use std::path::{Path, PathBuf};
use dash_mpd::fetch::DashDownloader;


#[tokio::test]
async fn test_dl1() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("itec-elephants-dream.mp4");
    assert!(DashDownloader::new(mpd_url)
            .worst_quality()
            .download_to(out.clone())
            .await
            .is_ok());
    println!("DASH content saved to file {}", out.to_string_lossy());
}


// These tests retrieve content from some public MPD manifests and check that the content is
// identical to previous "known good" downloads. These checks are fragile because checksums and
// exact octet counts might change due to version changes in libav, that we use for muxing.
// Running this test downloads several hundred megabytes, so we disable it for CI.
#[tokio::test]
#[allow(dead_code)]
async fn test_downloader() {
    use std::io;
    use sha2::{Digest, Sha256};
    use hex_literal::hex;
    use ffprobe::ffprobe;
    use colored::*;

    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    async fn check_mpd(mpd_url: &str, octets: u64, digest: &[u8]) {
        println!("Checking MPD URL {mpd_url}");
        match DashDownloader::new(mpd_url).download().await {
            Ok(path) => {
                // check that ffprobe identifies this as a media file
                let probed_meta = ffprobe(path.clone());
                if let Ok(meta) = probed_meta {
                    if meta.streams.is_empty() {
                        eprintln!("   {}", "ffprobe finds zero media streams in file".red());
                    } else {
                        let stream = &meta.streams[0];
                        // actually, ffprobe doesn't give a duration for WebM content
                        // assert!(stream.duration.is_some());
                        if let Some(duration) = &stream.duration {
                            if duration.parse::<f64>().unwrap() <= 0.0 {
                                eprintln!("   {}", "ffprobe finds a zero-length stream".red());
                            }
                        }
                    }
                } else {
                    eprintln!("   {} on {mpd_url}", "ffprobe failed".red());
                }
                let mut sha256 = Sha256::new();
                let mut media = std::fs::File::open(path)
                    .expect("opening media file");
                let octets_downloaded = io::copy(&mut media, &mut sha256)
                    .expect("reading media file contents");
                let difference_ratio = (octets_downloaded as f64 - octets as f64) / octets as f64;
                if  difference_ratio.abs() > 0.1 {
                    eprintln!("   {:.1}% difference in download sizes", difference_ratio * 100.0);
                }
                let calculated = sha256.finalize();
                if calculated[..] != digest[..]  {
                    eprintln!("   {}", "incorrect checksum".red());
                }
                // We leave the downloaded file in the filesystem, in case further analysis is needed.
            },
            Err(e) => eprintln!("Failed to fetch MPD {mpd_url}: {e:?}"),
        }
    }

    check_mpd("https://res.cloudinary.com/demo-robert/video/upload/sp_16x9_vp9/yourPublicId.mpd",
              445_758,
              &hex!("7d6533d19a4a60c5c76cead7b2de1664f4ff856916037a574f641aad0324ee36")).await;

    check_mpd("https://storage.googleapis.com/shaka-demo-assets/angel-one/dash.mpd",
              1_786_875,
              &hex!("fc70321b55339d37c6c1ce8303fe357f3b1c83e86bc38fac54eed553cf3a251b")).await;

}

#[tokio::test]
async fn test_follow_redirect() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let redirector = format!("http://httpbin.org/redirect-to?url={mpd_url}");
    let out = env::temp_dir().join("itec-redirected.mp4");
    assert!(DashDownloader::new(&redirector)
            .worst_quality()
            .download_to(out.clone())
            .await
            .is_ok());
    if let Ok(meta) = fs::metadata(Path::new(&out)) {
        let ratio = meta.len() as f64 / 60_939.0;
        assert!(0.95 < ratio && ratio < 1.05);
    }
}


#[tokio::test]
#[should_panic(expected = "invalid digit found in string")]
async fn test_error_parsing() {
    // This DASH manifest is invalid because it contains a presentationDuration="25.7726666667" on a
    // SegmentBase node. The DASH XSD specification states that @presentationDuration is an
    // xs:unsignedLong.
    let url = "https://dash.akamaized.net/dash264/TestCasesHD/MultiPeriod_OnDemand/ThreePeriods/ThreePeriod_OnDemand_presentationDur_AudioTrim.mpd";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}


// Check that timeouts on network requests are correctly signalled. This manifest specifies a single
// large video segment (427MB) which should lead to a network timeout with our 0.5s setting, even
// if the test is running with a very large network bandwidth.
#[tokio::test]
#[should_panic(expected = "operation timed out")]
async fn test_error_timeout() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        panic!("operation timed out");
    }
    let out = env::temp_dir().join("timeout.mkv");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    DashDownloader::new("https://test-speke.s3.eu-west-3.amazonaws.com/tos/clear/manifest.mpd")
        .best_quality()
        .with_http_client(client)
        .download_to(out.clone())
        .await
        .unwrap();
}


// Check that we generate a timeout for network request when setting a low limit on network
// bandwidth (100 Kbps) and retrieving a large file.
#[tokio::test]
#[should_panic(expected = "operation timed out")]
async fn test_error_ratelimit() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        panic!("operation timed out");
    }
    let out = env::temp_dir().join("timeout.mkv");
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .unwrap();
    DashDownloader::new("https://test-speke.s3.eu-west-3.amazonaws.com/tos/clear/manifest.mpd")
        .best_quality()
        .with_http_client(client)
        .with_rate_limit(100 * 1024)
        .download_to(out.clone())
        .await
        .unwrap();
}


#[tokio::test]
async fn test_content_protection_parsing() {
    use dash_mpd::{parse, MPD};

    fn known_cp_name(name: &str) -> bool {
        let known = &["cenc", "MSPR 2.0", "Widevine", "ClearKey1.0"];
        known.contains(&name)
    }

    fn known_cp_scheme(scheme: &str) -> bool {
        let known = &["urn:mpeg:dash:mp4protection:2011",
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
            .expect("creating reqwest HTTP client");
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
    use std::process::Command;
    
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let url = "https://storage.googleapis.com/shaka-demo-assets/angel-one-widevine/dash.mpd";
    let out_undecrypted = env::temp_dir().join("angel-undecrypted.mp4");
    let out_decrypted = env::temp_dir().join("angel-decrypted.mp4");
    assert!(DashDownloader::new(url)
            .worst_quality()
            .download_to(out_undecrypted.clone())
            .await
            .is_ok());
    assert!(DashDownloader::new(url)
            .worst_quality()
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
            .download_to(out_decrypted.clone())
            .await
            .is_ok());
    let ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error",
               "-i", &out_decrypted.to_string_lossy(),
               "-f", "null", "-"])
        .output()
        .expect("spawning ffmpeg");
    let msg = String::from_utf8_lossy(&ffmpeg.stderr);
    assert!(msg.len() == 0);
    let ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error",
               "-i", &out_undecrypted.to_string_lossy(),
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

    fn ffmpeg_approval(name: &PathBuf) -> bool {
        let ffmpeg = Command::new("ffmpeg")
            .args(["-v", "error",
                   "-i", &name.to_string_lossy(),
                   "-f", "null", "-"])
        .output()
        .expect("spawning ffmpeg");
        let msg = String::from_utf8_lossy(&ffmpeg.stderr);
        println!("FFMPEG stderr> {msg}");
        msg.len() == 0
    }

    // WideVine ContentProtection with CENC encryption
    let mpd = "https://refapp.hbbtv.org/videos/spring_h265_v8/cenc/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("spring.mp4");
    assert!(DashDownloader::new(mpd)
            .worst_quality()
            .add_decryption_key(String::from("43215678123412341234123412341237"),
                                String::from("12341234123412341234123412341237"))
            .add_decryption_key(String::from("43215678123412341234123412341236"),
                                String::from("12341234123412341234123412341236"))
            .download_to(outpath.clone())
            .await
            .is_ok());
    if let Ok(meta) = fs::metadata(Path::new(&outpath)) {
        let ratio = meta.len() as f64 / 33_746_341.0;
        assert!(0.95 < ratio && ratio < 1.05);
    }
    assert!(ffmpeg_approval(&outpath));

    // WideVine ContentProtection with CBCS encryption
    let mpd = "https://refapp.hbbtv.org/videos/tears_of_steel_h265_v8/cbcs/manifest_wvcenc.mpd";
    let outpath = env::temp_dir().join("tears-steel.mp4");
    assert!(DashDownloader::new(mpd)
            .worst_quality()
            .add_decryption_key(String::from("43215678123412341234123412341237"),
                                String::from("12341234123412341234123412341237"))
            .add_decryption_key(String::from("43215678123412341234123412341236"),
                                String::from("12341234123412341234123412341236"))
            .download_to(outpath.clone())
            .await
            .is_ok());
    if let Ok(meta) = fs::metadata(Path::new(&outpath)) {
        let ratio = meta.len() as f64 / 79_731_116.0;
        assert!(0.95 < ratio && ratio < 1.05);
    }
    // We can't check the validity of this stream using ffmpeg, because ffmpeg complains a lot about
    // various anomalies in the AAC audio stream, though it seems to play the content OK.
    // assert!(ffmpeg_approval(&outpath));

    // PlayReady / CENC
    let mpd = "https://refapp.hbbtv.org/videos/00_llama_h264_v8_8s/cenc/manifest_prcenc.mpd";
    let outpath = env::temp_dir().join("llama.mp4");
    assert!(DashDownloader::new(mpd)
            .worst_quality()
            .add_decryption_key(String::from("43215678123412341234123412341236"),
                                String::from("12341234123412341234123412341236"))
            .download_to(outpath.clone())
            .await
            .is_ok());
    if let Ok(meta) = fs::metadata(Path::new(&outpath)) {
        let ratio = meta.len() as f64 / 26_420_624.0;
        assert!(0.95 < ratio && ratio < 1.05);
    }
    assert!(ffmpeg_approval(&outpath));

    // Marlin / CENC
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cenc/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama.mp4");
    assert!(DashDownloader::new(mpd)
            .worst_quality()
            .add_decryption_key(String::from("43215678123412341234123412341234"),
                                String::from("12341234123412341234123412341234"))
            .download_to(outpath.clone())
            .await
            .is_ok());
    if let Ok(meta) = fs::metadata(Path::new(&outpath)) {
        let ratio = meta.len() as f64 / 14_357_917.0;
        assert!(0.95 < ratio && ratio < 1.05);
    }
    assert!(ffmpeg_approval(&outpath));

    // Marlin / CBCS
    let mpd = "https://refapp.hbbtv.org/videos/agent327_h264_v8/cbcs/manifest_mlcenc.mpd";
    let outpath = env::temp_dir().join("llama.mp4");
    assert!(DashDownloader::new(mpd)
            .worst_quality()
            .add_decryption_key(String::from("43215678123412341234123412341234"),
                                String::from("12341234123412341234123412341234"))
            .download_to(outpath.clone())
            .await
            .is_ok());
    if let Ok(meta) = fs::metadata(Path::new(&outpath)) {
        let ratio = meta.len() as f64 / 14_357_925.0;
        assert!(0.95 < ratio && ratio < 1.05);
    }
    // Also can't test the validity of this stream using ffmpeg, for the same reasons as above
    // (complaints concerning the AAC audio stream).
    // assert!(ffmpeg_approval(&outpath));
}


// Check error reporting for missing DASH manifest
#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_missing_mpd() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        panic!("requesting DASH manifest");
    }
    let out = env::temp_dir().join("failure1.mkv");
    DashDownloader::new("http://httpbin.org/status/404")
        .worst_quality()
        .download_to(out.clone())
        .await
        .unwrap();
}

// Check error reporting when Period element contains a HTTP 404 XLink
// (this URL from DASH test suite)
#[tokio::test]
#[should_panic(expected = "fetching XLink")]
async fn test_error_xlink_gone() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        panic!("fetching XLink");
    }
    let out = env::temp_dir().join("failure_xlink.mkv");
    DashDownloader::new("https://dash.akamaized.net/dash264/TestCases/5c/nomor/5_1d.mpd")
        .worst_quality()
        .download_to(out.clone())
        .await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "download dynamic MPD")]
async fn test_error_dynamic_mpd() {
    let mpd = "https://akamaibroadcasteruseast.akamaized.net/cmaf/live/657078/akasource/out.mpd";
    DashDownloader::new(mpd)
        .worst_quality()
        .download()
        .await
        .unwrap();
}


// We could try to check that the error message contains "invalid peer certificate" (rustls) or
// "certificate has expired" (native-tls with OpenSSL), but our tests would be platform-dependent
// and fragile.
#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_expired() {
    // Check that the reqwest client refuses to download MPD from an expired TLS certificate
    let mpd = "https://expired.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download()
        .await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_self_signed() {
    let mpd = "https://self-signed.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download()
        .await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_too_large() {
    // The TLS response message is too large
    DashDownloader::new("https://10000-sans.badssl.com/ignored.mpd")
        .download()
        .await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_wrong_name() {
    DashDownloader::new("https://wrong.host.badssl.com/ignored.mpd")
        .download()
        .await
        .unwrap();
}

