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
    let res = parse(&case1);
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

#[test]
fn test_datetime_parsing () {
    use dash_mpd::parse;
    use chrono::{Timelike, Datelike};

    let case1 = r#"<MPD minBufferTime="PT1.500000000S"></MPD>"#;
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

    // an invalid xs:datetime (month number 14)
    let case5 = r#"<MPD availabilityStartTime="1066-14-03T21:56"></MPD>"#;
    let res = parse(case5);
    assert!(res.is_err());
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
}
