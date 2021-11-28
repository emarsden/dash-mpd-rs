// Tests for the parsing support



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
}

