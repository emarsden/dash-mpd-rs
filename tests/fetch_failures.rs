// Negative tests for MPD download support
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test fetch_failures -- --show-output

use std::env;
use std::time::Duration;
use dash_mpd::fetch::DashDownloader;


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
#[should_panic(expected = "invalid digit found in string")]
async fn test_error_group_attribute() {
    // This DASH manifest is invalid because it contains an invalid valid "notAnInteger" for the
    // AdaptationSet.group attribute.
    let url = "http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/advanced/invalid_group_string.mpd";
    DashDownloader::new(url)
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
async fn test_error_bad_closing_tag() {
    // This content response is not valid XML because </BaseURl> closes <BaseURL>.
    // Full error from xmltree is Unexpected closing tag: {urn:mpeg:dash:schema:mpd:2011}BaseURl !=
    // {urn:mpeg:dash:schema:mpd:2011}BaseURL
    let url = "https://dash.akamaized.net/akamai/test/isptest.mpd";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "parsing BaseURL: InvalidPort")]
async fn test_error_bad_baseurl() {
    // This DASH manifest contains invalid BaseURLs.
    //   <BaseURL>http://2018-01-30T14:35:19_aa2101c7-b230-4b63-a199-e40886842654</BaseURL>
    let url = "https://dash.akamaized.net/akamai/test/test.mpd";
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
    let url = "https://httpbun.com/html";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "parsing DASH XML")]
async fn test_error_img() {
    // Check that we fail to parse an image response.
    let url = "https://picsum.photos/240/120";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "NetworkConnect")]
async fn test_error_dns() {
    let url = "https://nothere.example.org/";
    DashDownloader::new(url)
        .best_quality()
        .download().await
        .unwrap();
}


// Check that we generate a timeout for network request when setting a low limit on network
// bandwidth (100 Kbps) and retrieving a large file.
#[tokio::test]
#[should_panic(expected = "max_error_count")]
async fn test_error_ratelimit() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        panic!("max_error_count");
    }
    let out = env::temp_dir().join("timeout.mkv");
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .unwrap();
    DashDownloader::new("https://test-speke.s3.eu-west-3.amazonaws.com/tos/clear/manifest.mpd")
        .best_quality()
        .fragment_retry_count(3)
        .max_error_count(1)
        .with_http_client(client)
        .with_rate_limit(100 * 1024)
        .download_to(out.clone()).await
        .unwrap();
}



// Check error reporting for missing DASH manifest
#[tokio::test]
#[should_panic(expected = "requesting DASH manifest")]
async fn test_error_missing_mpd() {
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
    let out = env::temp_dir().join("failure_xlink.mkv");
    DashDownloader::new("https://dash.akamaized.net/dash264/TestCases/5c/nomor/5_1d.mpd")
        .worst_quality()
        .download_to(out.clone()).await
        .unwrap();
}


// Other live streams that could be checked:
//   https://demo.unified-streaming.com/k8s/live/trunk/scte35.isml/.mpd
//   https://tv.nknews.org/tvdash/stream.mpd
//   https://cph-msl.akamaized.net/dash/live/2003285/test/manifest.mpd
//   https://cdn-vos-ppp-01.vos360.video/Content/DASH_DASHCLEAR2/Live/channel(PPP-LL-2DASH)/master.mpd
//   https://livesim.dashif.org/livesim/scte35_2/testpic_2s/Manifest.mpd
//   https://livesim2.dashif.org/livesim2/segtimeline_1/testpic_2s/Manifest.mpd
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
#[should_panic(expected = "NetworkConnect")]
async fn test_error_tls_expired() {
    // Check that the reqwest client refuses to download MPD from an expired TLS certificate
    let mpd = "https://expired.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "NetworkConnect")]
async fn test_error_tls_untrusted_root() {
    // Check that the reqwest client refuses to download from a server with an untrusted root certificate
    let mpd = "https://untrusted-root.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "NetworkConnect")]
async fn test_error_tls_wronghost() {
    // Check that the reqwest client refuses to download MPD from server with incorrect hostname
    let mpd = "https://wrong.host.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "NetworkConnect")]
async fn test_error_tls_self_signed() {
    let mpd = "https://self-signed.badssl.com/ignored.mpd";
    DashDownloader::new(mpd)
        .download().await
        .unwrap();
}

#[tokio::test]
#[should_panic(expected = "NetworkConnect")]
async fn test_error_tls_too_large() {
    // The TLS response message is too large
    DashDownloader::new("https://10000-sans.badssl.com/ignored.mpd")
        .download().await
        .unwrap();
}


#[tokio::test]
#[should_panic(expected = "NetworkConnect")]
async fn test_error_tls_wrong_name() {
    DashDownloader::new("https://wrong.host.badssl.com/ignored.mpd")
        .download().await
        .unwrap();
}

