// Basic tests for the serialization support

use std::time::Duration;
use chrono::prelude::*;
use test_log::test;
use dash_mpd::{MPD, Period};


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
    assert!(dash_mpd::parse(&xml).is_ok());
}

