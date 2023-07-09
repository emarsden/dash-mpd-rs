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


use std::time::Duration;
use dash_mpd::fetch::DashDownloader;


#[tokio::test]
async fn test_dl1() {
    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = std::env::temp_dir().join("itec-elephants-dream.mp4");
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
    if std::env::var("CI").is_ok() {
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


// Check that timeouts on network requests are correctly signalled. This manifest specifies a single
// large video segment (427MB) which should lead to a network timeout with our 0.1s setting, even
// if the test is running with a very large network bandwidth.
#[tokio::test]
#[should_panic(expected = "operation timed out")]
async fn test_error_timeout() {
    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        panic!("operation timed out");
    }
    let out = std::env::temp_dir().join("timeout.mkv");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(100))
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
    if std::env::var("CI").is_ok() {
        panic!("operation timed out");
    }
    let out = std::env::temp_dir().join("timeout.mkv");
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
    if std::env::var("CI").is_ok() {
        return;
    }
    check_cp("https://media.axprod.net/TestVectors/v7-MultiDRM-SingleKey/Manifest_1080p.mpd").await;
    check_cp("https://m.dtv.fi/dash/dasherh264/drm/manifest_clearkey.mpd").await;
}




// Check error reporting for missing DASH manifest
#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_missing_mpd() {
    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        panic!("requesting DASH manifest");
    }
    let out = std::env::temp_dir().join("failure1.mkv");
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
    if std::env::var("CI").is_ok() {
        panic!("fetching XLink");
    }
    let out = std::env::temp_dir().join("failure1.mkv");
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



