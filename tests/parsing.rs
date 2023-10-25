// Tests for the parsing support
//
// To run this test while enabling printing to stdout/stderr
//
//    cargo test --test parsing -- --show-output


// Currently a nightly-only feature
// use std::assert_matches::assert_matches;

use std::fs;
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use dash_mpd::parse;
use dash_mpd::fetch::DashDownloader;


#[test]
fn test_mpd_parser () {
    let case1 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD><Period></Period></MPD>"#;
    let res = parse(case1);
    let mpd = res.unwrap();
    assert_eq!(mpd.periods.len(), 1);
    assert!(mpd.ProgramInformation.is_none());

    let case2 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD foo="foo"><Period></Period><foo></foo></MPD>"#;
    let res = parse(case2);
    let mpd = res.unwrap();
    assert_eq!(mpd.periods.len(), 1);
    assert!(mpd.ProgramInformation.is_none());

    let case3 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD><Period></PeriodZ></MPD>"#;
    let res = parse(case3);
    assert!(res.is_err());
    // assert_matches!(parse(case3), Err(DashMpdError::Parsing));

    let case4 = r#"<MPD>
                     <BaseURL>http://cdn1.example.com/</BaseURL>
                     <BaseURL>http://cdn2.example.com/</BaseURL>
                   </MPD>"#;
    let res = parse(case4);
    let mpd = res.unwrap();
    assert_eq!(mpd.base_url.len(), 2);

    let case5 = r#"<MPD type="static" minBufferTime="PT1S">
    <Period duration="PT2S">
      <AdaptationSet mimeType="video/mp4">
        <Representation bandwidth="42" id="3"></Representation>
      </AdaptationSet>
    </Period></MPD>"#;
    let res = parse(case5);
    let mpd = res.unwrap();
    assert!(mpd.mpdtype.is_some());
    assert_eq!(mpd.mpdtype.unwrap(), "static");
    assert_eq!(mpd.minBufferTime.unwrap(), Duration::new(1, 0));
    assert_eq!(mpd.periods.len(), 1);
    let p1 = &mpd.periods[0];
    assert_eq!(p1.duration.unwrap(), Duration::new(2, 0));
    assert_eq!(p1.adaptations.len(), 1);
    let a1 = &p1.adaptations[0];
    assert_eq!(a1.mimeType.as_ref().unwrap(), "video/mp4");
    assert_eq!(a1.representations.len(), 1);
    let r1 = &a1.representations[0];
    assert_eq!(r1.bandwidth.unwrap(), 42);

    // This example using single quotes instead of double quotes in XML formatting.
    let case6 = r#"<?xml version='1.0' encoding='UTF-8'?><MPD
       minBufferTime='PT10.00S'
       mediaPresentationDuration='PT3256S'
       type='static' availabilityStartTime='2001-12-17T09:40:57Z'
       profiles='urn:mpeg:dash:profile:isoff-main:2011'>
     <Period start='PT0S' id='1'>
       <AdaptationSet group='1'>
         <Representation mimeType='video/mp4' codecs='avc1.644028, svc1' width='320' height='240' 
           frameRate='15' id='tag0' bandwidth='128000'>
           <SegmentList duration='10'>
             <Initialization sourceURL='seg-s-init.mp4'/>
             <SegmentURL media='seg-s1-128k-1.mp4'/>
             <SegmentURL media='seg-s1-128k-2.mp4'/>
             <SegmentURL media='seg-s1-128k-3.mp4'/>
           </SegmentList>
         </Representation>
       </AdaptationSet>
     </Period></MPD>"#;
    let c6 = parse(case6).unwrap();
    assert_eq!(c6.periods.len(), 1);
}

// These tests check that we are able to parse DASH manifests that contain XML elements for which we
// don't have definitions. We want to degrade gracefully and ignore these unknown elements, instead
// of triggering a parse failure.
#[test]
fn test_unknown_elements () {
    let case1 = r#"<MPD><UnknownElement/></MPD>"#;
    let res = parse(case1);
    assert_eq!(res.unwrap().periods.len(), 0);

    // The same test using an unknown XML namespace prefix.
    let case2 = r#"<MPD><uprefix:UnknownElement></uprefix:UnknownElement></MPD>"#;
    let res = parse(case2);
    assert_eq!(res.unwrap().periods.len(), 0);

    // Here the same check for an XML element which is using the $text "special name" to allow
    // access to the element content (the Title element).
    let case3 = r#"<MPD><ProgramInformation>
       <Title>Foobles<UnknownElement/></Title>
     </ProgramInformation></MPD>"#;
    let res = parse(case3);
    let mpd = res.unwrap();
    assert!(mpd.ProgramInformation.is_some());
    let pi = mpd.ProgramInformation.unwrap();
    assert!(pi.Title.is_some());
    let title = pi.Title.unwrap();
    assert_eq!(title.content.unwrap(), "Foobles");

    let case4 = r#"<MPD><ProgramInformation>
       <Title>Foobles<upfx:UnknownElement/></Title>
     </ProgramInformation></MPD>"#;
    let res = parse(case4);
    let mpd = res.unwrap();
    assert!(mpd.ProgramInformation.is_some());
    let pi = mpd.ProgramInformation.unwrap();
    assert!(pi.Title.is_some());
    let title = pi.Title.unwrap();
    assert_eq!(title.content.unwrap(), "Foobles");
}

#[test]
fn test_url_parsing () {
    let err1 = r#"<MPD><Period id="1">
       <AdaptationSet group="1">
         <Representation mimeType='video/mp4' width="320" height="240">
           <SegmentList duration="10">
             <Initialization sourceURL="httpunexist://example.com/segment.mp4"/>
             <SegmentURL media="seg1.mp4"/>
           </SegmentList>
         </Representation>
       </AdaptationSet>
     </Period></MPD>"#;
    assert!(parse(err1).is_err());

    let err2 = r#"<MPD><Period id="1">
       <AdaptationSet group="1">
         <Representation mimeType="video/mp4" width="320" height="240">
           <SegmentList duration="10">
             <SegmentURL media="https://example.com:-1/segment.mp4"/>
           </SegmentList>
         </Representation>
       </AdaptationSet>
     </Period></MPD>"#;
    assert!(parse(err2).is_err());

    let err3 = r#"<MPD><ProgramInformation moreInformationURL="https://192.168.1.2.3/segment.mp4" /></MPD>"#;
    assert!(parse(err3).is_err());

    // Yes, this path component is really accepted even if containing characters that are not
    // recommended for use in URLs.
    let err4 = r#"<MPD><Period id="1">
       <AdaptationSet group="1">
         <Representation mimeType="video/mp4">
           <SegmentList duration="10">
             <SegmentURL media="/segment'-??-$$i-<\r-\n-%T%Ae-❌.mp4"/>
           </SegmentList>
         </Representation>
       </AdaptationSet>
     </Period></MPD>"#;
    assert!(parse(err4).is_ok());

    // Check that IPv6 addresses are accepted.
    let ok1 = r#"<MPD><ProgramInformation moreInformationURL="http://[2001:db8::1]/info/" /></MPD>"#;
    parse(ok1).unwrap();
}

#[test]
fn test_datetime_parsing () {
    use chrono::{Timelike, Datelike};

    let case1 = r#"<MPD minBufferTime="PT1.500S"></MPD>"#;
    let res = parse(case1);
    let mpd = res.unwrap();
    assert!(mpd.minBufferTime.is_some());
    let mbt = mpd.minBufferTime.unwrap();
    assert_eq!(mbt.as_secs(), 1);
    assert_eq!(mbt.as_millis(), 1500);

    // an xs:datetime without a specified timezone
    let case2 = r#"<MPD availabilityStartTime="2022-12-06T22:27:53"></MPD>"#;
    let res = parse(case2);
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.year(), 2022);
    assert_eq!(ast.hour(), 22);
    assert_eq!(ast.second(), 53);

    // an xs:datetime with a timezone specified
    let case3 = r#"<MPD availabilityStartTime="2021-06-03T13:00:00Z"></MPD>"#;
    let res = parse(case3);
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.year(), 2021);
    assert_eq!(ast.hour(), 13);

    let case4 = r#"<MPD availabilityStartTime="2015-11-03T21:56"></MPD>"#;
    let res = parse(case4);
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.year(), 2015);
    assert_eq!(ast.day(), 3);
    assert_eq!(ast.minute(), 56);

    // an xs:datetime with a timezone specified parses fractional nanoseconds
    let case5 = r#"<MPD availabilityStartTime="2021-06-03T13:00:00.543343989Z"></MPD>"#;
    let res = parse(case5);
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.nanosecond(), 543343989);

    // an invalid xs:datetime (month number 14)
    let case6 = r#"<MPD availabilityStartTime="1066-14-03T21:56"></MPD>"#;
    let res = parse(case6);
    assert!(res.is_err());
}



#[test]
fn test_file_parsing() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("aws");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    let res = parse(&xml);
    let mpd = res.unwrap();
    assert_eq!(mpd.minBufferTime.unwrap(), Duration::new(30, 0));
    let mp = mpd.periods.iter()
        .filter(|p| p.id.is_some())
        .find(|p| p.id.as_ref().unwrap().eq("8778696_PT0S_0"));
    assert!(mp.is_some());
    let p1 = mp.unwrap();
    assert_eq!(p1.BaseURL.len(), 1);
    assert!(p1.BaseURL[0].base.contains("mediatailor.us-west-2"));

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("mediapackage");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    parse(&xml).unwrap();

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("telestream-elements");
    path.set_extension("xml");
    let tsxml = fs::read_to_string(path).unwrap();
    parse(&tsxml).unwrap();

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("telestream-binary");
    path.set_extension("xml");
    let tsxml = fs::read_to_string(path).unwrap();
    parse(&tsxml).unwrap();

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("dolby-ac4");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    parse(&xml).unwrap();

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("orange");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    parse(&xml).unwrap();
}


// Test some of the example DASH manifests provided by the MPEG Group
// at https://github.com/MPEGGroup/DASHSchema
#[tokio::test]
async fn test_parsing_online() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }

    async fn check_mpd(client: reqwest::Client, url: &str) {
        let xml = client.get(url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .send().await
            .expect("requesting MPD content")
            .text().await
            .expect("fetching MPD content");
        parse(&xml).unwrap();
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    check_mpd(client.clone(),
              "https://raw.githubusercontent.com/MPEGGroup/DASHSchema/5th-Ed-AMD1/example_H3.mpd").await;
    check_mpd(client.clone(),
              "https://raw.githubusercontent.com/MPEGGroup/DASHSchema/5th-Ed-AMD1/example_G4.mpd").await;
    check_mpd(client.clone(),
              "https://raw.githubusercontent.com/MPEGGroup/DASHSchema/5th-Ed-AMD1/example_G22.mpd").await;
    check_mpd(client.clone(),
              "https://cph-msl.akamaized.net/dash/live/2003285/test/manifest.mpd").await;
}


#[tokio::test]
async fn test_parsing_subrepresentations() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "https://raw.githubusercontent.com/MPEGGroup/DASHSchema/5th-Ed-AMD1/example_G6.mpd";
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    let mpd = parse(&xml);
    assert!(mpd.is_ok());
    let mpd = mpd.unwrap();
    // Check that every Representation in this manifest contains three SubRepresentation nodes.
    mpd.periods.iter().for_each(
        |p| p.adaptations.iter().for_each(
            |a| a.representations.iter().for_each(
                |r| assert_eq!(3, r.SubRepresentation.len()))));
}


#[tokio::test]
async fn test_parsing_eventstream() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "https://raw.githubusercontent.com/MPEGGroup/DASHSchema/5th-Ed-AMD1/example_G9.mpd";
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    let mpd = parse(&xml);
    assert!(mpd.is_ok());
    let mpd = mpd.unwrap();
    mpd.periods.iter().for_each(
        |p| p.event_streams.iter().for_each(
            |es| assert_eq!(4, es.event.len())));
    assert!(mpd.periods.iter().any(
        |p| p.adaptations.iter().any(
            |a| a.representations.iter().any(
                |r| r.InbandEventStream.len() == 2))));
}


#[tokio::test]
async fn test_parsing_supplementalproperty() {
    // Don't run download tests on CI infrastructure
    if env::var("CI").is_ok() {
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "https://raw.githubusercontent.com/MPEGGroup/DASHSchema/5th-Ed-AMD1/example_H2.mpd";
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    let mpd = dash_mpd::parse(&xml);
    let mpd = mpd.unwrap();
    assert!(mpd.periods.iter().any(
        |p| p.adaptations.iter().any(
            |a| a.supplemental_property.iter().any(
                |sp| sp.value.as_ref().is_some_and(|v| v.eq("0,1,1,1,1,2,2"))))));
    assert!(mpd.periods.iter().all(
        |p| p.adaptations.iter().all(
            |a| a.supplemental_property.iter().all(
                |sp| sp.value.as_ref().is_some_and(|v| !v.eq("42"))))));
}


// This manifest is invalid because it contains a subsegmentStartsWithSAP="true", whereas the DASH
// specification states that this should be an SAPType, an integer (checked with
// https://conformance.dashif.org/).
#[tokio::test]
#[should_panic(expected = "invalid digit found in string")]
async fn test_parsing_fail_invalid_int() {
    DashDownloader::new("https://dash.akamaized.net/akamai/test/jurassic-compact.mpd")
        .best_quality()
        .download().await
        .unwrap();
}

// This manifest has <BaseURL> closed by <BaseURl>
#[tokio::test]
#[should_panic(expected = "parsing DASH XML")]
async fn test_parsing_fail_incorrect_tag() {
    DashDownloader::new("https://dash.akamaized.net/akamai/test/isptest.mpd")
        .best_quality()
        .download().await
        .unwrap();
}

// This a an exemple DASH manifest from a commercial ad management platform which is not spec
// compliant. The MPD specifies maxSegmentDuration="PT2S", but the SegmentTimeline contains segments
// of duration 132300 / 44100 (3 seconds).
#[tokio::test]
#[should_panic(expected = "segment@d > @maxSegmentDuration")]
async fn test_parsing_invalid_maxsegmentduration() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("admanager");
    path.set_extension("xml");
    let amxml = fs::read_to_string(path).unwrap();
    parse(&amxml).unwrap();
}
