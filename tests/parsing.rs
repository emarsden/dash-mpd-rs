// Tests for the parsing support


// Currently a nightly-only feature
// use std::assert_matches::assert_matches;


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

    // an invalid xs:datetime (month number 14)
    let case5 = r#"<MPD availabilityStartTime="1066-14-03T21:56"></MPD>"#;
    let res = parse(case5);
    assert!(res.is_err());
}

