// tests for MPD download support
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
fn test_itec1() {
    use std::time::Duration;
    use dash_mpd::fetch_mpd;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(10, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let manifest_url = "http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd";
    assert!(fetch_mpd(&client, manifest_url, "/tmp/test-dash-out.mp4").is_ok());
}


// These tests retrieve content from some public MPD manifests and check that the content is
// identical to previous "known good" downloads. These checks are fragile because checksums and
// exact octet counts might change due to version changes in libav, that we use for muxing.
// Running this test downloads several hundred megabytes, so we disable it for CI. 
// #[test]
#[allow(dead_code)]
fn test_downloader() {
    use std::io;
    use std::time::Duration;
    use sha2::{Digest, Sha256};
    use hex_literal::hex;
    use ffprobe::ffprobe;
    use dash_mpd::{fetch_mpd, HttpClient};
    use colored::*;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.183 Safari/537.36")
        .timeout(Duration::new(10, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");

    fn check_mpd(client: &HttpClient, mpd_url: &str, octets: u64, digest: &[u8]) {
        use tempfile::NamedTempFile;

        println!("Checking MPD URL {}", mpd_url);
        let tmp = NamedTempFile::new()
            .expect("Can't create tempfile");
        let (tmpfile, tmppath) = tmp.keep()
            .expect("Can't keep tempfile");
        let os_path = tmppath.to_str().expect("tmpfile path");
        if let Err(e) = fetch_mpd(client, mpd_url, os_path) {
            eprintln!("Failed to fetch MPD {}: {:?}", mpd_url, e);
        }
        tmpfile.sync_all()
            .expect("Syncing data to disk");
        // check that ffprobe identifies this as a media file
        let probed_meta = ffprobe(tmppath.clone());
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
        // we reopen because rewinding the tmpfile doesn't seem to work
        let mut reopened = std::fs::File::open(os_path)
            .expect("Can't open media file");
        let octets_downloaded = io::copy(&mut reopened, &mut sha256)
            .expect("Couldn't read media file contents");
        let difference_ratio = (octets_downloaded as f64 - octets as f64) / octets as f64;
        if  difference_ratio.abs() > 0.1 {
            eprintln!("   {:.1}% difference in download sizes", difference_ratio * 100.0);
        }
        let calculated = sha256.finalize();
        if calculated[..] != digest[..]  {
            eprintln!("   {}", "incorrect checksum".red());
        }
    }

    check_mpd(&client,
              "https://res.cloudinary.com/demo-robert/video/upload/sp_16x9_vp9/yourPublicId.mpd",
              445_758,
              &hex!("7d6533d19a4a60c5c76cead7b2de1664f4ff856916037a574f641aad0324ee36"));

    check_mpd(&client,
              "https://storage.googleapis.com/shaka-demo-assets/angel-one/dash.mpd",
              1_786_875,
              &hex!("fc70321b55339d37c6c1ce8303fe357f3b1c83e86bc38fac54eed553cf3a251b"));

}

