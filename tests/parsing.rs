// Tests for the parsing support
//
// To run this test while enabling printing to stdout/stderr
//
//    cargo test --test parsing -- --show-output


// Currently a nightly-only feature
// use std::assert_matches::assert_matches;

#[macro_use]
extern crate approx;

pub mod common;
use fs_err as fs;
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use pretty_assertions::assert_eq;
use dash_mpd::parse;
use dash_mpd::fetch::DashDownloader;
use common::setup_logging;

#[test]
fn test_mpd_parser () {
    setup_logging();
    let case1 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD><Period></Period></MPD>"#;
    let res = parse(case1);
    let mpd = res.unwrap();
    assert_eq!(mpd.periods.len(), 1);
    assert_eq!(mpd.ProgramInformation.len(), 0);

    let case2 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD foo="foo"><Period></Period><foo></foo></MPD>"#;
    let res = parse(case2);
    let mpd = res.unwrap();
    assert_eq!(mpd.periods.len(), 1);
    assert_eq!(mpd.ProgramInformation.len(), 0);

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

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("multiple_supplementals");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let res = parse(&xml);
    let mpd = res.unwrap();
    let mut supplementals_count = 0;
    mpd.periods.iter().for_each(
        |p| p.adaptations.iter().for_each(|a| {
            supplementals_count += a.supplemental_property.len();
            a.representations.iter().for_each(
                |r| supplementals_count += r.supplemental_property.len());
        }));
    assert_eq!(supplementals_count, 6);
}

#[test]
fn test_mpd_failures () {
    setup_logging();
    let case1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT6M16S" minBufferTime="PT1.97S">"#;
    let c1 = parse(case1);
    assert!(c1.is_err());
}


// These tests check that we are able to parse DASH manifests that contain XML elements for which we
// don't have definitions. We want to degrade gracefully and ignore these unknown elements, instead
// of triggering a parse failure.
#[test]
fn test_unknown_elements () {
    setup_logging();
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
    assert!(mpd.ProgramInformation.len() > 0);
    let pi = &mpd.ProgramInformation.first().unwrap();
    assert!(&pi.Title.is_some());
    let title = pi.Title.as_ref().unwrap();
    assert_eq!(title.content.clone().unwrap().clone(), "Foobles");

    let case4 = r#"<MPD><ProgramInformation>
       <Title>Foobles<upfx:UnknownElement/></Title>
     </ProgramInformation></MPD>"#;
    let res = parse(case4);
    let mpd = res.unwrap();
    assert!(mpd.ProgramInformation.len() > 0);
    let pi = &mpd.ProgramInformation.first().unwrap();
    assert!(pi.Title.is_some());
    let title = pi.Title.as_ref().unwrap();
    assert_eq!(title.content.clone().unwrap(), "Foobles");
}

#[test]
fn test_url_parsing () {
    setup_logging();
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

    setup_logging();
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


// This test manifest from https://livesim2.dashif.org/livesim2/ato_inf/testpic_2s/Manifest.mpd It
// includes an attribute @availabilityTimeOffset="INF" for floating point infinity, which we want to
// check is correctly deserialized into our Option<f64> field.
#[test]
fn test_timeoffset_inf() {
    setup_logging();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("dashif-live-atoinf");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let res = parse(&xml);
    let mpd = res.unwrap();
    assert_eq!(mpd.mpdtype, Some(String::from("dynamic")));
    assert!(mpd.UTCTiming.iter().all(
        |utc| utc.value.as_ref().is_some_and(
            |v| v.contains("time.akamai.com"))));
    assert!(mpd.periods.iter().all(
        |p| p.adaptations.iter().all(
            |a| a.SegmentTemplate.as_ref().is_some_and(
                |st| st.availabilityTimeOffset.is_some_and(
                    |ato| ato.is_infinite())))));
}

// Check that our serialization of f64 struct elements matches the XML Schema specification for type
// xsd:double.
#[test]
fn test_parse_f64() {
    setup_logging();
    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="-3E5"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(relative_eq!(bu.availability_time_offset.unwrap(), -3E5));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="4268.22752E11"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(relative_eq!(bu.availability_time_offset.unwrap(), 4268.22752E11));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="+24.3e-3"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(relative_eq!(bu.availability_time_offset.unwrap(), 24.3e-3));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="12"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(relative_eq!(bu.availability_time_offset.unwrap(), 12.0));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="+3.5"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(relative_eq!(bu.availability_time_offset.unwrap(), 3.5));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="-0"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(relative_eq!(bu.availability_time_offset.unwrap(), -0.0));
}


#[test]
fn test_parse_f64_infnan() {
    setup_logging();
    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="INF"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(bu.availability_time_offset.is_some_and(|f| f.is_infinite()));
    assert_eq!(bu.availability_time_offset, Some(f64::INFINITY));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="-INF"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(bu.availability_time_offset.is_some_and(|f| f.is_infinite()));
    assert!(! bu.availability_time_offset.is_some_and(|f| f.is_nan()));
    assert_eq!(bu.availability_time_offset, Some(f64::NEG_INFINITY));

    let xml = r#"<MPD><Period id="1"><BaseURL availabilityTimeOffset="NaN"/></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let bu = &mpd.periods[0].BaseURL[0];
    assert!(bu.availability_time_offset.is_some_and(|f| f.is_nan()));
    assert!(! bu.availability_time_offset.is_some_and(|f| f.is_infinite()));
}


// This test manifest from https://livesim2.dashif.org/livesim2/chunkdur_1/ato_7/testpic4_8s/Manifest300.mpd
// Includes features of the DASH Low Latency specification.
#[test]
fn test_parse_low_latency() {
    setup_logging();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("dashif-low-latency");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let res = parse(&xml);
    let mpd = res.unwrap();
    assert_eq!(mpd.mpdtype, Some(String::from("dynamic")));
    assert!(mpd.ServiceDescription.first().as_ref().is_some_and(
        |sd| sd.Latency.iter().all(
            |l| l.max.is_some_and(
                |m| 6999.9 < m && m < 7000.1))));
    assert!(mpd.ServiceDescription.first().as_ref().is_some_and(
        |sd| sd.PlaybackRate.iter().all(
            |pbr| 0.95 < pbr.min.unwrap() && pbr.min.unwrap() < 0.97)));
    assert!(mpd.UTCTiming.iter().all(
        |utc| utc.value.as_ref().is_some_and(
            |v| v.contains("time.akamai.com"))));
    assert!(mpd.periods.iter().all(
        |p| p.adaptations.iter().all(
            |a| a.SegmentTemplate.as_ref().is_some_and(
                |st| st.availabilityTimeComplete.is_some_and(
                    |atc| !atc)))));
}


#[test]
fn test_file_parsing() {
    setup_logging();
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

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("a2d-tv");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let mpd = parse(&xml).unwrap();
    assert_eq!(mpd.periods[0].event_streams[0].event.len(), 3);

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("st-sl");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let mpd = parse(&xml).unwrap();
    let sl = &mpd.periods[0].adaptations[0].representations[0].SegmentList.as_ref().unwrap();
    assert!(sl.Initialization.as_ref().is_some_and(
        |i| i.sourceURL.as_ref().is_some_and(
            |su| su.contains("foobar"))));
    assert_eq!(sl.segment_urls.len(), 3);
    assert!(sl.SegmentTimeline.as_ref().is_some_and(
        |st| st.segments[0].t.is_some_and(|t| t == 0)));
}


#[test]
fn test_parsing_patch_location() {
    setup_logging();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("patch-location");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let mpd = parse(&xml).unwrap();
    assert_eq!(mpd.mpdtype.unwrap(), "dynamic");
    assert_eq!(mpd.PatchLocation.len(), 1);
    assert!(mpd.PatchLocation[0].content.contains("patch.mpp"));
}

// Test fixture is from https://standards.iso.org/iso-iec/23009/-1/ed-5/en/
#[test]
fn test_parsing_failover_content() {
    setup_logging();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("example_G22");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let mpd = parse(&xml).unwrap();
    assert_eq!(mpd.mpdtype.unwrap(), "dynamic");
    assert_eq!(mpd.minBufferTime.unwrap(), Duration::new(4, 0));
    assert_eq!(mpd.base_url.len(), 2);
    assert_eq!(mpd.periods[0].adaptations[0]
               .SegmentTemplate.as_ref().unwrap()
               .SegmentTimeline.as_ref().unwrap()
               .segments.len(), 3);
    let rep = &mpd.periods[0].adaptations[0].representations.iter()
        .find(|r| r.id.as_ref().is_some_and(|id| id == "A"))
        .unwrap();
    assert_eq!(rep.SegmentTemplate.as_ref().unwrap()
               .failover_content.as_ref().unwrap()
               .fcs_list[0].d, Some(180180));
}


#[test]
fn test_parsing_xsd_uintvector() {
    setup_logging();
    let xml = r#"<MPD><Period><Subset contains=""></Subset></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    assert!(&mpd.periods[0].subsets[0].contains.is_empty());

    let xml = r#"<MPD><Period><Subset contains="56"></Subset></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let ss = &mpd.periods[0].subsets[0];
    assert_eq!(ss.contains.len(), 1);
    assert_eq!(ss.contains[0], 56);

    let xml = r#"<MPD><Period><Subset contains="99 33 1 56789 44"></Subset></Period></MPD>"#;
    let mpd = parse(xml).unwrap();
    let ss = &mpd.periods[0].subsets[0];
    assert_eq!(ss.contains.len(), 5);
    assert_eq!(ss.contains[0], 99);
    assert_eq!(ss.contains[4], 44);

    let xml = r#"<MPD><Period><Subset contains="-4 5"></Subset></Period></MPD>"#;
    assert!(parse(xml).is_err());
}


#[tokio::test]
async fn test_content_protection() {
    setup_logging();
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "https://test.playready.microsoft.com/media/profficialsite/tearsofsteel_4k.ism/manifest.mpd";
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    let mpd = parse(&xml);
    assert!(mpd.is_ok());
    let mpd = mpd.unwrap();
    let cp = mpd.ContentProtection;
    assert_eq!(cp.len(), 1);
    let prcp = cp[0].clone();
    assert!(prcp.schemeIdUri.eq("urn:uuid:9A04F079-9840-4286-AB92-E65BE0885F95"));
}


// Test some of the example DASH manifests provided by the MPEG Group
// at https://github.com/MPEGGroup/DASHSchema
#[tokio::test]
async fn test_parsing_online() {
    // Don't run download tests on CI infrastructure
    setup_logging();
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
        parse(&xml)
            .expect(&format!("Failed to parse {}", url));
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
    check_mpd(client.clone(),
              "https://raw.githubusercontent.com/Blazemeter/mpd-tools/0c7b3eabfdab6c66100c0218df09f430dc72c802/parser/src/test/resources/random.mpd").await;

    // from nice collection at https://github.com/Eyevinn/dash-mpd/
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/adaptationset_switching.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/events.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/live_profile_multi_base_url.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/multiple_supplementals.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/newperiod.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/segment_list.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/segment_timeline.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/segment_timeline_multi_period.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/truncate.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/truncate_short.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/go-dash-fixtures/audio_channel_configuration.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/livesim/multi-drm.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/Eyevinn/dash-mpd/raw/226078de966af6b72b9da6b3f7fd2b2d8c2a1c79/mpd/testdata/schema-mpds/example_H2.mpd").await;

    // Additional test files from https://github.com/claudiuolteanu/mpd-parser
    check_mpd(client.clone(),
              "https://github.com/claudiuolteanu/mpd-parser/raw/refs/heads/master/examples/car-20120827-manifest.mpd").await;
    check_mpd(client.clone(),
              "https://github.com/claudiuolteanu/mpd-parser/raw/refs/heads/master/examples/feelings_vp9-20130806-manifest.mpd").await;

    check_mpd(client.clone(),
              "https://github.com/claudiuolteanu/mpd-parser/raw/refs/heads/master/examples/oops-20120802-manifest.mpd").await;

    // 2025-11 this site is down
    // check_mpd(client.clone(),
    //           "https://content.media24.link/drm/manifest.mpd").await;

    check_mpd(client.clone(),
              "http://vod-dash-ww-rd-stage.akamaized.net/dash/ondemand/testcard/1/client_manifest-nosurround-ctv-events_on_both.mpd").await;

    check_mpd(client.clone(),
              "https://vod-dash-ww-rd-stage.akamaized.net/testcard/2/manifests/avc-full-events_both-en-rel.mpd").await;

    check_mpd(client.clone(),
              "http://vod-dash-ww-rd-stage.akamaized.net/dash/ondemand/testcard/1/client_manifest-pto_both-events.mpd").await;

    check_mpd(client.clone(),
              "http://vs-dash-ww-rd-live.akamaized.net/wct/A00/client_manifest.mpd").await;

    check_mpd(client.clone(),
              "https://test.playready.microsoft.com/media/dash/APPLEENC_CBCS_BBB_1080p/1080p.mpd").await;
}


#[tokio::test]
async fn test_parsing_subrepresentations() {
    // Don't run download tests on CI infrastructure
    setup_logging();
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
    setup_logging();
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
    setup_logging();
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


#[tokio::test]
async fn test_parsing_essentialproperty() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "http://dash.edgesuite.net/dash264/TestCasesNegative/1/1.mpd";
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
            |r| r.representations.iter().any(
                |a| a.essential_property.iter().any(
                    |ep| ep.value.as_ref().is_some_and(|v| v.eq("Negative Test EssentialProperty 1")))))));
}


// From a list of streams at
//   https://garfnet.org.uk/cms/tables/radio-frequencies/internet-radio-player/bbc-national-and-local-radio-dash-streams/
//
// For dynamic MPDs, you shall "never" start to play with startNumber, but the latest available
// segment is LSN = floor( (now - (availabilityStartTime+PST))/segmentDuration + startNumber - 1).
#[tokio::test]
async fn test_parsing_servicelocation() {
    setup_logging();
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.10 Safari/605.1.1")
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let url = "https://a.files.bbci.co.uk/ms6/live/3441A116-B12E-4D2F-ACA8-C1984642FA4B/audio/simulcast/dash/nonuk/pc_hd_abr_v2/aks/bbc_world_service.mpd";
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    let mpd = dash_mpd::parse(&xml);
    let mpd = mpd.unwrap();
    assert!(mpd.publishTime.is_some());
    assert!(mpd.UTCTiming.len() > 0);
    assert_eq!(mpd.base_url.len(), 1);
    let base_url = mpd.base_url.first().unwrap();
    assert!(base_url.priority.is_some());
    assert!(base_url.weight.is_some());
    assert!(base_url.serviceLocation.is_some());
}


// The aim of this test is to check that the ordering of elements in the XML serialization of
// ServiceDescription elements respects the order specified by the DASH XSD.
#[tokio::test]
async fn test_parsing_servicedescription() {
    setup_logging();
    let fragment = r#"
<ServiceDescription id="0">
  <Scope schemeIdUri="urn:dvb:dash:lowlatency:scope:2019"/>
  <Latency min="3000" max="5000" target="4000"/>
  <PlaybackRate min="0.95" max="1.05"/>
</ServiceDescription>"#;
    let sd: dash_mpd::ServiceDescription = quick_xml::de::from_str(&fragment).unwrap();
    let serialized = quick_xml::se::to_string(&sd).unwrap();
    let pos1 = serialized.find("Scope").unwrap();
    let pos2 = serialized.find("Latency").unwrap();
    let pos3 = serialized.find("PlaybackRate").unwrap();
    assert!(pos1 < pos2 && pos2 < pos3);
}


// This manifest has some unusual use of XML namespacing
//   <g1:MPD xmlns="urn:MPEG:ns:DASH" xmlns:g1="urn:mpeg:DASH:schema:MPD:2011"
#[tokio::test]
async fn test_parsing_namespacing() {
    setup_logging();
    let url = "https://dash.akamaized.net/qualcomm/cloud/cloudology_new_dash.mpd";
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("creating HTTP client");
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .expect("requesting MPD content")
        .text().await
        .expect("fetching MPD content");
    let mpd = dash_mpd::parse(&xml);
    let mpd = mpd.unwrap();
    assert_eq!(mpd.periods.len(), 1);
    assert_eq!(mpd.periods[0].adaptations.len(), 2);
    // This manifest uses SegmentList+SegmentURL addressing for both Adapations
    assert!(mpd.periods.iter().all(
        |p| p.adaptations.iter().all(
            |a| a.representations.iter().all(
                |r| r.SegmentList.iter().all(
                    |sl| sl.segment_urls.len() == 1)))));
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

