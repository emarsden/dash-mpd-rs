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



#[test]
fn test_dl1() {
    use dash_mpd::fetch::DashDownloader;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = std::env::temp_dir().join("itec-elephants-dream.mp4");
    assert!(DashDownloader::new(mpd_url)
            .worst_quality()
            .download_to(out.clone()).is_ok());
    println!("DASH content saved to file {}", out.to_string_lossy());
}


// These tests retrieve content from some public MPD manifests and check that the content is
// identical to previous "known good" downloads. These checks are fragile because checksums and
// exact octet counts might change due to version changes in libav, that we use for muxing.
// Running this test downloads several hundred megabytes, so we disable it for CI.
#[test]
#[allow(dead_code)]
fn test_downloader() {
    use std::io;
    use sha2::{Digest, Sha256};
    use hex_literal::hex;
    use ffprobe::ffprobe;
    use dash_mpd::fetch::DashDownloader;
    use colored::*;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    fn check_mpd(mpd_url: &str, octets: u64, digest: &[u8]) {
        println!("Checking MPD URL {}", mpd_url);
        match DashDownloader::new(mpd_url).download() {
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
                    eprintln!("   {} on {}", "ffprobe failed".red(), mpd_url);
                }
                let mut sha256 = Sha256::new();
                let mut media = std::fs::File::open(path)
                    .expect("Can't open media file");
                let octets_downloaded = io::copy(&mut media, &mut sha256)
                    .expect("Couldn't read media file contents");
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
            Err(e) => eprintln!("Failed to fetch MPD {}: {:?}", mpd_url, e),
        }
    }

    check_mpd("https://res.cloudinary.com/demo-robert/video/upload/sp_16x9_vp9/yourPublicId.mpd",
              445_758,
              &hex!("7d6533d19a4a60c5c76cead7b2de1664f4ff856916037a574f641aad0324ee36"));

    check_mpd("https://storage.googleapis.com/shaka-demo-assets/angel-one/dash.mpd",
              1_786_875,
              &hex!("fc70321b55339d37c6c1ce8303fe357f3b1c83e86bc38fac54eed553cf3a251b"));

}



#[test]
fn test_content_protection_parsing() {
    use std::time::Duration;
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

    fn check_cp(mpd_url: &str) {
        println!("Checking MPD URL {}", mpd_url);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::new(30, 0))
            .gzip(true)
            .build()
            .expect("creating reqwest HTTP client");
        let xml = client.get(mpd_url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .send()
            .expect("requesting MPD content")
            .text()
            .expect("fetching MPD content");
        let mpd: MPD = parse(&xml)
            .expect("parsing MPD");
        for p in mpd.periods {
            if let Some(adapts) = p.adaptations {
                for adap in adapts.iter() {
                    if let Some(cpv) = &adap.ContentProtection {
                        for cp in cpv.iter() {
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
        }
    }

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    check_cp("https://media.axprod.net/TestVectors/v7-MultiDRM-SingleKey/Manifest_1080p.mpd");
    check_cp("https://m.dtv.fi/dash/dasherh264/drm/manifest_clearkey.mpd");
}




// Check error reporting for missing DASH manifest
#[test]
#[should_panic(expected = "requesting DASH manifest")]
fn test_error_missing_mpd() {
    use dash_mpd::fetch::DashDownloader;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    let out = std::env::temp_dir().join("failure1.mkv");
    DashDownloader::new("http://httpbin.org/status/404")
        .worst_quality()
        .download_to(out.clone()).unwrap();
}

// Check error reporting when Period element contains a HTTP 404 XLink
// (this URL from DASH test suite)
#[test]
#[should_panic(expected = "fetching XLink")]
fn test_error_xlink_gone() {
    use dash_mpd::fetch::DashDownloader;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    let out = std::env::temp_dir().join("failure1.mkv");
    DashDownloader::new("https://dash.akamaized.net/dash264/TestCases/5c/nomor/5_1d.mpd")
        .worst_quality()
        .download_to(out.clone()).unwrap();
}
