// Basic tests for the serialization support

use fs_err as fs;
use std::path::PathBuf;
use std::time::Duration;
use chrono::prelude::*;
use test_log::test;
use dash_mpd::{parse, MPD, Period, BaseURL, Subset};


#[test]
fn test_serialize () {
    let period = Period {
        id: Some("randomcookie".to_string()),
        duration: Some(Duration::new(420, 69)),
        ..Default::default()
    };
    let mpd = MPD {
        mpdtype: Some("static".to_string()),
        xmlns: Some("urn:mpeg:dash:schema:mpd:2011".to_string()),
        periods: vec!(period),
        publishTime: Some(Utc.with_ymd_and_hms(2017, 5, 25, 11, 11, 0).unwrap()),
        ..Default::default()
    };
    let xml = mpd.to_string();
    assert!(xml.contains("MPD"));
    assert!(xml.contains("urn:mpeg:dash:schema"));
    assert!(xml.contains("randomcookie"));
    assert!(xml.contains("2017-05-25T11:11"));
    assert!(parse(&xml).is_ok());
}


// See https://github.com/emarsden/dash-mpd-rs/issues/49
#[test]
fn test_serialize_inf() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("f64-inf");
    path.set_extension("mpd");
    let xml = fs::read_to_string(path).unwrap();
    let mpd = parse(&xml).unwrap();
    let p1 = &mpd.periods[0];
    let a1 = &p1.adaptations[0];
    assert_eq!(a1.contentType.as_ref().unwrap(), "video");
    assert_eq!(a1.SegmentTemplate.as_ref().unwrap().availabilityTimeOffset, Some(f64::INFINITY));

    let serialized = mpd.to_string();
    let roundtripped = parse(&serialized).unwrap();
    let p1 = &roundtripped.periods[0];
    let a1 = &p1.adaptations[0];
    assert_eq!(a1.contentType.as_ref().unwrap(), "video");
    assert_eq!(a1.SegmentTemplate.as_ref().unwrap().availabilityTimeOffset, Some(f64::INFINITY));
    // http://www.datypic.com/sc/xsd/t-xsd_double.html
    println!("+Inf> {serialized}");
    assert!(serialized.contains("availabilityTimeOffset=\"INF\""));
}


#[test]
fn test_serialize_f64_infnan() {
    let period = Period {
        id: Some("randomcookie".to_string()),
        duration: Some(Duration::new(420, 69)),
        ..Default::default()
    };
    let mut bu = BaseURL {
        base: String::from("http://www.example.com/"),
        availability_time_offset: Some(f64::INFINITY),
        ..Default::default()
    };
    let mut mpd = MPD {
        mpdtype: Some(String::from("static")),
        xmlns: Some("urn:mpeg:dash:schema:mpd:2011".to_string()),
        periods: vec!(period),
        base_url: vec!(bu.clone()),
        ..Default::default()
    };
    let serialized = mpd.to_string();
    assert!(serialized.contains("availabilityTimeOffset=\"INF\""));

    bu.availability_time_offset = Some(f64::NEG_INFINITY);
    mpd.base_url = vec!(bu.clone());
    let serialized = mpd.to_string();
    assert!(serialized.contains("availabilityTimeOffset=\"-INF\""));

    bu.availability_time_offset = Some(f64::NAN);
    mpd.base_url = vec!(bu);
    let serialized = mpd.to_string();
    // http://www.datypic.com/sc/xsd/t-xsd_double.html
    assert!(serialized.contains("availabilityTimeOffset=\"NaN\""));
}



#[test]
fn test_serialize_xsd_uintvector() {
    let subset = Subset {
        id: Some("sub1".to_string()),
        contains: vec![22, 33, 44],
    };
    let period = Period {
        id: Some("66".to_string()),
        duration: Some(Duration::new(420, 69)),
        subsets: vec![subset],
        ..Default::default()
    };
    let mpd = MPD {
        mpdtype: Some(String::from("dynamic")),
        xmlns: Some("urn:mpeg:dash:schema:mpd:2011".to_string()),
        periods: vec!(period),
        ..Default::default()
    };
    let serialized = mpd.to_string();
    assert!(serialized.contains("22 33 44"));
}


