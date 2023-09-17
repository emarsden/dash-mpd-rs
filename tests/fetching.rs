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
use ffprobe::ffprobe;
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
        .max_error_count(5)
        .record_metainformation(false)
        .with_authentication("user".to_string(), "dummy".to_string())
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 60_939);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    println!("DASH content saved to MP4 container at {}", out.to_string_lossy());
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_mp4a() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/3a/fraunhofer/aac-lc_stereo_without_video/Sintel/sintel_audio_only_aaclc_stereo_sidx.mpd";
    let out = env::temp_dir().join("sintel-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 7_456_334);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let audio = &meta.streams[0];
    assert_eq!(audio.codec_type, Some(String::from("audio")));
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    assert!(audio.width.is_none());
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_eac3() {
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesMCA/dolby/3/1/ChID_voices_20_128_ddp.mpd";
    let out = env::temp_dir().join("dolby-eac3.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 2_436_607);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("eac3")));
}

// As of 2023-09, ffmpeg v6.0 is unable to mux this ac-4 audio codec into an MP4 container. mkvmerge
// is able to mux it into a Matroska container.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_eac4() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesDolby/2/Living_Room_1080p_20_96k_2997fps.mpd";
    let out = env::temp_dir().join("dolby-eac4.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 11_668_955);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    // This codec is not currently recogized by ffprobe
    // assert_eq!(stream.codec_name, Some(String::from("ac-4")));
}

// As of 2023-09, ffmpeg v6.0 is unable to mux this Dolby DTSC audio codec into an MP4 container. mkvmerge
// is able to mux it into a Matroska container.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_dolby_dtsc() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesMCA/dts/1/Paint_dtsc_testA.mpd";
    let out = env::temp_dir().join("dolby-dtsc.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 35_408_884);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[1];
    assert_eq!(stream.codec_type, Some(String::from("audio")));
    assert_eq!(stream.codec_name, Some(String::from("dts")));
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_hevc_hdr() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesHDR/3a/3/MultiRate.mpd";
    let out = env::temp_dir().join("hevc-hdr.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 4_052_727);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("hevc")));
    assert!(stream.width.is_some());
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_vp9_uhd() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCasesVP9/vp9-uhd/sintel-vp9-uhd.mpd";
    let out = env::temp_dir().join("vp9-uhd.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 71_339_734);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("vp9")));
    assert_eq!(stream.width, Some(3840));
}

// H.266/VVC codec. ffmpeg v6.0 is not able to place this video stream in an MP4 container, but
// muxing to Matroska with mkvmerge works. Neither mplayer nor VLC can play the video.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_vvc() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://ftp.itec.aau.at/datasets/mmsys22/Skateboarding/8sec/vvc/manifest.mpd";
    let out = env::temp_dir().join("vvc.mkv");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_311_029);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("vvc1")));
    assert_eq!(stream.width, Some(384));
}

// MPEG2 TS codec (mostly historical interest).
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_mp2t() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mpeg2-simple/mpeg2-simple-mpd.mpd";
    let out = env::temp_dir().join("mp2ts.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .without_content_type_checks()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_019_006);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let stream = &meta.streams[0];
    assert_eq!(stream.codec_type, Some(String::from("video")));
    assert_eq!(stream.codec_name, Some(String::from("h264")));
    assert_eq!(stream.width, Some(320));
}

http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mpeg2-simple/mpeg2-simple-mpd.mpd


// A test for SegmentList addressing
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_segment_list() {
    let mpd_url = "https://res.cloudinary.com/demo/video/upload/sp_full_hd/handshake.mpd";
    let out = env::temp_dir().join("handshake.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 273_629);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
}

// A test for SegmentBase@indexRange addressing with a single audio and video fragment that
// is convenient for testing sleep_between_requests()
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_segment_base_indexrange() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://turtle-tube.appspot.com/t/t2/dash.mpd";
    let out = env::temp_dir().join("turtle.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(3)
        .sleep_between_requests(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 9_687_251);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
}

// A test for BaseURL addressing mode.
#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_baseurl() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/1a/sony/SNE_DASH_SD_CASE1A_REVISED.mpd";
    let out = env::temp_dir().join("sony.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 38_710_852);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
}

// A test for AdaptationSet>SegmentList + Representation>SegmentList addressing modes.
#[tokio::test]
async fn test_dl_adaptation_segment_list() {
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://ftp.itec.aau.at/datasets/mmsys13/redbull_4sec.mpd";
    let out = env::temp_dir().join("redbull.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .without_content_type_checks()
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 110_010_161);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
}

// A test for the progress observer functionality.
#[tokio::test]
async fn test_progress_observer() {
    use dash_mpd::fetch::ProgressObserver;
    use std::sync::Arc;

    struct DownloadProgressionTest { }

    impl ProgressObserver for DownloadProgressionTest {
        fn update(&self, percent: u32, _message: &str) {
            assert!(percent <= 100);
        }
    }

    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("progress.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .add_progress_observer(Arc::new(DownloadProgressionTest{}))
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 60_939);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format.extension(), "mp4");
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
async fn test_error_smoothstreaming() {
    // SmoothStreamingMedia manifests are an XML format, but not the same schema as DASH (root
    // element is "SmoothStreamingMedia").
    let url = "http://playready.directtaps.net/smoothstreaming/SSWSS720H264/SuperSpeedway_720.ism/Manifest";
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
#[should_panic(expected = "parsing DASH XML")]
async fn test_error_img() {
    // Check that we fail to parse an image response.
    let url = "https://placekitten.com/240/120";
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

