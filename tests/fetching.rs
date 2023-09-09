// Tests for MPD download support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test fetching -- --show-output
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
use std::path::PathBuf;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;


// We tolerate significant differences in final output file size, because as encoder performance
// changes in newer versions of ffmpeg, the resulting file size when reencoding may change
// significantly.
fn check_file_size_approx(p: &PathBuf, expected: u64) {
    let meta = fs::metadata(p).unwrap();
    let ratio = meta.len() as f64 / expected as f64;
    assert!(0.9 < ratio && ratio < 1.1, "File sizes: expected {}, got {}", expected, meta.len());
}


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_mp4() {
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cf.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 60_939);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
    println!("DASH content saved to MP4 container at {}", out.to_string_lossy());
}

// We can't check file size for this test, as depending on whether mkvmerge or ffmpeg or mp4box are
// used to copy the video stream into the Matroska container (depending on which one is installed),
// the output file size varies quite a lot.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_mkv() {
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cf.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbose(3)
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mkv");
    println!("DASH content saved to MKV container at {}", out.to_string_lossy());
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_webm() {
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cf.webm");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 65_798);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "webm");
    println!("DASH content saved to WebM container at {}", out.to_string_lossy());
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


// Content that uses the HVC1 H265 codec in a CMAF container.
#[tokio::test]
async fn test_h265() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://media.axprod.net/TestVectors/H265/clear_cmaf_1080p_h265/manifest.mpd";
    let out = env::temp_dir().join("h265.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
    check_file_size_approx(&out, 48_352_569);
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
    DashDownloader::new(&redirector)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 60_939);
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

#[tokio::test]
#[should_panic(expected = "parsing DASH XML")]
async fn test_error_invalidxml() {
    // This content response is not valid XML because the processing instruction ("<?xml...>") is
    // not at the beginning of the content.
    let url = "https://httpbin.dmuth.org/xml";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "parsing DASH XML")]
async fn test_error_html() {
    // Check that we fail to parse an HTML response.
    let url = "https://httpbun.org/html";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "dns error")]
async fn test_error_dns() {
    let url = "https://nothere.example.org/";
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
        .download_to(out.clone()).await
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
        .download_to(out.clone()).await
        .unwrap();
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
        .download_to(out.clone()).await
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
        .download_to(out.clone()).await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "download dynamic MPD")]
async fn test_error_dynamic_mpd() {
    let mpd = "https://akamaibroadcasteruseast.akamaized.net/cmaf/live/657078/akasource/out.mpd";
    DashDownloader::new(mpd)
        .worst_quality()
        .download().await
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
        .download().await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_self_signed() {
    let mpd = "https://self-signed.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_too_large() {
    // The TLS response message is too large
    DashDownloader::new("https://10000-sans.badssl.com/ignored.mpd")
        .download().await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_tls_wrong_name() {
    DashDownloader::new("https://wrong.host.badssl.com/ignored.mpd")
        .download().await
        .unwrap();
}

