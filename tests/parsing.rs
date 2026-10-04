// Tests for the parsing support
//
// To run this test while enabling printing to stdout/stderr
//
//    cargo test --test parsing -- --show-output


// Currently a nightly-only feature
// use std::assert_matches::assert_matches;

// #[macro_use]
// extern crate approx;

pub mod common;
use std::time::Duration;
use dash_mpd_core::parse;
use dash_mpd::DashDownloader;
use common::setup_logging;



#[tokio::test]
async fn test_mpd_failures () {
    setup_logging();
    let case1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT6M16S" minBufferTime="PT1.97S">"#;
    let c1 = parse(case1);
    assert!(c1.is_err());

    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/invalid.mpd";
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    assert!(parse(&xml).is_err());
}



// This manifest is invalid because it contains a subsegmentStartsWithSAP="true", whereas the DASH
// specification states that this should be an SAPType, an integer (checked with
// https://conformance.dashif.org/).
#[tokio::test]
#[should_panic(expected = "Parsing")]
async fn test_parsing_fail_invalid_int() {
    setup_logging();
    DashDownloader::new("https://dash.akamaized.net/akamai/test/jurassic-compact.mpd")
        .best_quality()
        .download().await
        .unwrap();
}

// This manifest has <BaseURL> closed by <BaseURl>
#[tokio::test]
#[should_panic(expected = "parsing DASH XML")]
async fn test_parsing_fail_incorrect_tag() {
    setup_logging();
    DashDownloader::new("https://dash.akamaized.net/akamai/test/isptest.mpd")
        .best_quality()
        .download().await
        .unwrap();
}


// Possible additions:
//
//  Fail with https://akm.eu.prd.media.max.com/bolt-glo-prod/78cccdce-2592-4a2b-a023-91034c43366e/packager-mp4-cenc/main.mpd (cenc: XML namespace is not defined correctly)
