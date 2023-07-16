// Tests for the parsing support


// Currently a nightly-only feature
// use std::assert_matches::assert_matches;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;


#[test]
fn test_mpd_parser () {
    use dash_mpd::parse;

    let case1 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD><Period></Period></MPD>"#;
    let res = parse(case1);
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert_eq!(mpd.periods.len(), 1);
    assert!(mpd.ProgramInformation.is_none());

    let case2 = r#"<?xml version="1.0" encoding="UTF-8"?><MPD foo="foo"><Period></Period><foo></foo></MPD>"#;
    let res = parse(case2);
    assert!(res.is_ok());
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
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert_eq!(mpd.base_url.len(), 2);

    let case5 = r#"<MPD type="static" minBufferTime="PT1S">
    <Period duration="PT2S">
      <AdaptationSet mimeType="video/mp4">
        <Representation bandwidth="42" id="3"></Representation>
      </AdaptationSet>
    </Period></MPD>"#;
    let res = parse(case5);
    assert!(res.is_ok());
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
}

// These tests check that we are able to parse DASH manifests that contain XML elements for which we
// don't have definitions. We want to degrade gracefully and ignore these unknown elements, instead
// of triggering a parse failure.
#[test]
fn test_unknown_elements () {
    use dash_mpd::parse;

    let case1 = r#"<MPD><UnknownElement/></MPD>"#;
    let res = parse(case1);
    assert!(res.is_ok());
    assert_eq!(res.unwrap().periods.len(), 0);

    // The same test using an unknown XML namespace prefix.
    let case2 = r#"<MPD><uprefix:UnknownElement></uprefix:UnknownElement></MPD>"#;
    let res = parse(case2);
    assert!(res.is_ok());
    assert_eq!(res.unwrap().periods.len(), 0);

    // Here the same check for an XML element which is using the $text "special name" to allow
    // access to the element content (the Title element).
    let case3 = r#"<MPD><ProgramInformation>
       <Title>Foobles<UnknownElement/></Title>
     </ProgramInformation></MPD>"#;
    let res = parse(case3);
    assert!(res.is_ok());
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
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert!(mpd.ProgramInformation.is_some());
    let pi = mpd.ProgramInformation.unwrap();
    assert!(pi.Title.is_some());
    let title = pi.Title.unwrap();
    assert_eq!(title.content.unwrap(), "Foobles");
}

#[test]
fn test_datetime_parsing () {
    use dash_mpd::parse;
    use chrono::{Timelike, Datelike};

    let case1 = r#"<MPD minBufferTime="PT1.500S"></MPD>"#;
    let res = parse(case1);
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert!(mpd.minBufferTime.is_some());
    let mbt = mpd.minBufferTime.unwrap();
    assert_eq!(mbt.as_secs(), 1);
    assert_eq!(mbt.as_millis(), 1500);

    // an xs:datetime without a specified timezone
    let case2 = r#"<MPD availabilityStartTime="2022-12-06T22:27:53"></MPD>"#;
    let res = parse(case2);
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.year(), 2022);
    assert_eq!(ast.hour(), 22);
    assert_eq!(ast.second(), 53);

    // an xs:datetime with a timezone specified
    let case3 = r#"<MPD availabilityStartTime="2021-06-03T13:00:00Z"></MPD>"#;
    let res = parse(case3);
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.year(), 2021);
    assert_eq!(ast.hour(), 13);

    let case4 = r#"<MPD availabilityStartTime="2015-11-03T21:56"></MPD>"#;
    let res = parse(case4);
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.year(), 2015);
    assert_eq!(ast.day(), 3);
    assert_eq!(ast.minute(), 56);

    // an xs:datetime with a timezone specified parses fractional nanoseconds
    let case5 = r#"<MPD availabilityStartTime="2021-06-03T13:00:00.543343989Z"></MPD>"#;
    let res = parse(case5);
    assert!(res.is_ok());
    let mpd = res.unwrap();
    assert!(mpd.availabilityStartTime.is_some());
    let ast = mpd.availabilityStartTime.unwrap();
    assert_eq!(ast.nanosecond(), 543343989);

    // an invalid xs:datetime (month number 14)
    let case6 = r#"<MPD availabilityStartTime="1066-14-03T21:56"></MPD>"#;
    let res = parse(case6);
    assert!(res.is_err());
}


// TODO: the manifest at https://refplayer-dev.cloud.digitaluk.co.uk/dynamic/stl-dashads-dartest.mpd
// has scte35:Binary content that doesn't parse as Base64, for some reason.
#[test]
fn test_scte35_binary() {
    use dash_mpd::parse;

    let bin1 = r#"<MPD><Period>
          <EventStream timescale="10000000" schemeIdUri="urn:scte:scte35:2014:xml+bin">
            <Event>
                <scte35:Signal>
                    <scte35:Binary>/DAnAAAAAAAAAP/wBQb+0cr/PQARAg9DVUVJAAA0Q3+/AAAjAAAG6c2q</scte35:Binary>
                </scte35:Signal>
            </Event>
          </EventStream>
     </Period></MPD>"#;
    let res = parse(bin1);
    assert!(res.is_ok());

    let bin2 = r#"<MPD><Period id="2470350023" start="PT27448.333589S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2014:xml+bin">
      <Event duration="21690000" id="1">
       <scte35:Signal>
         <scte35:Binary>
           /DAlAAAAAAAAAP/wFAUAAAAEf+/+kybGyP4BSvaQAAEBAQAArky/3g==
         </scte35:Binary>
       </scte35:Signal>
      </Event>
    </EventStream>
    </Period></MPD>"#;
    let res = parse(bin2);
    assert!(res.is_ok());

    // from DVB spec, https://dvb.org/wp-content/uploads/2022/08/A178-3_Dynamic-substitution-of-content-in-linear-broadcast_Part3_Signalling-in-DVB-DASH_Interim_Draft-TS-103-752-3v111_Aug-2022.pdf
    let bin3 = r#"<MPD><Period id="1519" start="PT451209H39M31.000S">
       <EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="1" presentationTimeOffset="1624354771">
         <Event presentationTime="1624354848" duration="19" id="760">
           <Signal xmlns="http://www.scte.org/schemas/35/2016">
             <Binary>/DAgAAAAAAAAAP/wDwUAAAL4f//+ABoXsMAAAAAAAPF20V0=</Binary>
           </Signal>
         </Event>
       </EventStream>
      </Period></MPD>"#;
    let res = parse(bin3);
    assert!(res.is_ok());

    let bin4 = r#"<MPD><Period start="PT444806.040S" id="123586" duration="PT15.000S">
    <EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="1">
      <Event presentationTime="1541436240" duration="24" id="29">
        <scte35:Signal xmlns="http://www.scte.org/schemas/35/2016">
          <scte35:Binary>/DAhAAAAAAAAAP/wEAUAAAHAf+9/fgAg9YDAAAAAAAA25aoh</scte35:Binary>
        </scte35:Signal>
      </Event>
      <Event presentationTime="1541436360" duration="24" id="30">
        <scte35:Signal xmlns="http://www.scte.org/schemas/35/2016">
          <scte35:Binary>QW5vdGhlciB0ZXN0IHN0cmluZyBmb3IgZW5jb2RpbmcgdG8gQmFzZTY0IGVuY29kZWQgYmluYXJ5Lg==</scte35:Binary>
        </scte35:Signal>
      </Event></EventStream>
    </Period></MPD>"#;
    assert!(parse(bin4).is_ok());

    let bin5 = r#"<MPD><Period><EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="1">
       <Event presentationTime="1540809120" id="1999">
          <Signal xmlns="http://www.scte.org/schemas/35/2016"><Binary>/DAhAAAAAAAAAP/wEAUAAAfPf+9/fgAg9YDAAAAAAAA/APOv</Binary>
          </Signal>
       </Event>
      </EventStream>
     </Period></MPD>"#;
    assert!(parse(bin5).is_ok());

    // from https://developers.broadpeak.io/docs/input-streaming-formats
    let bin6 = r#"<MPD><Period><EventStream
      schemeIdUri="urn:scte:scte35:2014:xml+bin"
      timescale="10000000">
      <Event
        presentationTime="15516962501159406"
        id="3999785549">
        <Signal xmlns="http://www.scte.org/schemas/35/2016"> 
           <Binary>/DAnAAAAAAAAAP/wBQb/Y/SedwARAg9DVUVJAAAAPH+/AAAjAQEGLc/Q
           </Binary>
        </Signal>
      </Event>
      </EventStream></Period></MPD>"#;
    assert!(parse(bin6).is_ok());

}

#[test]
fn test_scte35_elements() {
    use dash_mpd::parse;

    let elem1 = r#"<MPD><Period start="PT444806.040S" id="123586" duration="PT15.000S">
      <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
        <Event duration="1350000">
          <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="180832" tier="4095">
            <scte35:SpliceInsert spliceEventId="4026531855" spliceEventCancelIndicator="false" outOfNetworkIndicator="true" spliceImmediateFlag="false" uniqueProgramId="1" availNum="1" availsExpected="1">
              <scte35:Program><scte35:SpliceTime ptsTime="5672624400"/></scte35:Program>
              <scte35:BreakDuration autoReturn="true" duration="1350000"/>
            </scte35:SpliceInsert>
          </scte35:SpliceInfoSection>
        </Event>
      </EventStream>
    </Period></MPD>"#;
    assert!(parse(elem1).is_ok());

    let elem2 = r#"<MPD><Period start="PT444806.040S" id="123586" duration="PT15.000S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
      <Event duration="1350000">
        <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="180832" tier="4095">
          <scte35:SpliceInsert spliceEventId="4026531855" spliceEventCancelIndicator="false" outOfNetworkIndicator="true" spliceImmediateFlag="false" uniqueProgramId="1" availNum="1" availsExpected="1">
            <scte35:Program><scte35:SpliceTime ptsTime="5672624400"/></scte35:Program>
            <scte35:BreakDuration autoReturn="true" duration="1350000"/>
          </scte35:SpliceInsert>
        </scte35:SpliceInfoSection>
      </Event></EventStream>
    </Period></MPD>"#;
    assert!(parse(elem2).is_ok());

    let elem3 = r#"<MPD><Period start="PT346530.250S" id="178443" duration="PT61.561S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
      <Event duration="5310000">
        <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="183003" tier="4095">
          <scte35:TimeSignal>
            <scte35:SpliceTime ptsTime="3442857000"/>
          </scte35:TimeSignal>
          <scte35:SegmentationDescriptor segmentationEventId="1414668" segmentationEventCancelIndicator="false" segmentationDuration="8100000">
            <scte35:DeliveryRestrictions webDeliveryAllowedFlag="false" noRegionalBlackoutFlag="false" archiveAllowedFlag="false" deviceRestrictions="3"/>
            <scte35:SegmentationUpid segmentationUpidType="12" segmentationUpidLength="2" segmentationTypeId="52" segmentNum="0" segmentsExpected="0">0100</scte35:SegmentationUpid>
          </scte35:SegmentationDescriptor>
        </scte35:SpliceInfoSection>
      </Event></EventStream>
     </Period></MPD>"#;
    assert!(parse(elem3).is_ok());

    let elem4 = r#"<MPD>  <Period start="PT346530.250S" id="178443" duration="PT61.561S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
      <Event duration="5310000">
        <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="183003" tier="4095">
          <scte35:TimeSignal>
            <scte35:SpliceTime ptsTime="3442857000"/>
          </scte35:TimeSignal>
          <scte35:SegmentationDescriptor segmentationEventId="1414668" segmentationEventCancelIndicator="false" segmentationDuration="8100000">
            <scte35:DeliveryRestrictions webDeliveryAllowedFlag="false" noRegionalBlackoutFlag="false" archiveAllowedFlag="false" deviceRestrictions="3"/>
            <scte35:SegmentationUpid segmentationUpidType="12" segmentationUpidLength="2" segmentationTypeId="52" segmentNum="0" segmentsExpected="0">0100</scte35:SegmentationUpid>
          </scte35:SegmentationDescriptor>
        </scte35:SpliceInfoSection>
      </Event></EventStream></Period></MPD>"#;
    assert!(parse(elem4).is_ok());
}


#[test]
fn test_file_parsing() {
    use dash_mpd::parse;

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("aws");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    let res = parse(&xml);
    assert!(res.is_ok());
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
    let res = parse(&xml);
    assert!(res.is_ok());

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("telestream-elements");
    path.set_extension("xml");
    let tsxml = fs::read_to_string(path).unwrap();
    let tx = parse(&tsxml);
    assert!(tx.is_ok());

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("telestream-binary");
    path.set_extension("xml");
    let tsxml = fs::read_to_string(path).unwrap();
    let tx = parse(&tsxml);
    assert!(tx.is_ok());

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("admanager");
    path.set_extension("xml");
    let amxml = fs::read_to_string(path).unwrap();
    let am = parse(&amxml);
    assert!(am.is_ok());

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("dolby-ac4");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    let db = parse(&xml);
    assert!(db.is_ok());

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("orange");
    path.set_extension("xml");
    let xml = fs::read_to_string(path).unwrap();
    let db = parse(&xml);
    assert!(db.is_ok());

}


// Test some of the example DASH manifests provided by the MPEG Group
// at https://github.com/MPEGGroup/DASHSchema
#[tokio::test]
async fn test_parsing_online() {
    use dash_mpd::parse;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
        return;
    }

    async fn check_mpd(client: reqwest::Client, url: &str) {
        let xml = client.get(url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .send().await
            .expect("requesting MPD content")
            .text().await
            .expect("fetching MPD content");
        let p = parse(&xml);
        assert!(p.is_ok());
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
}


#[tokio::test]
async fn test_parsing_subrepresentations() {
    use dash_mpd::parse;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
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
    use dash_mpd::parse;

    // Don't run download tests on CI infrastructure
    if std::env::var("CI").is_ok() {
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
    if std::env::var("CI").is_ok() {
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
    assert!(mpd.is_ok());
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

