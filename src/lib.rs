//! A Rust library for parsing, serializing and downloading media content from a DASH MPD manifest,
//! as used for on-demand replay of TV content and video streaming services. Allows both parsing of
//! a DASH manifest (XML format) to Rust structs (deserialization) and programmatic generation of an
//! MPD manifest (serialization). The library also allows you to download media content from a
//! streaming server.

//! [DASH](https://en.wikipedia.org/wiki/Dynamic_Adaptive_Streaming_over_HTTP) (dynamic adaptive
//! streaming over HTTP), also called MPEG-DASH, is a technology used for media streaming over the
//! web, commonly used for video on demand (VOD) services. The Media Presentation Description (MPD)
//! is a description of the resources (manifest or “playlist”) forming a streaming service, that a
//! DASH client uses to determine which assets to request in order to perform adaptive streaming of
//! the content. DASH MPD manifests can be used both with content encoded as MPEG and as WebM.
//!

//! This library provides a serde-based parser (deserializer) and serializer for the DASH MPD
//! format, as formally defined in ISO/IEC standard 23009-1:2022. This version of the standard is
//! [available for free online](https://standards.iso.org/ittf/PubliclyAvailableStandards/c083314_ISO_IEC%2023009-1_2022(en).zip). XML schema files are [available for no cost from
//! ISO](https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/). When
//! MPD files in practical use diverge from the formal standard, this library prefers to
//! interoperate with existing practice.
//!
//! The library does not yet provide full coverage of the fifth edition of the specification. All
//! elements and attributes in common use are supported, however.
//!
//! The library also provides experimental support for downloading content (audio or video)
//! described by an MPD manifest. This involves selecting the alternative with the most appropriate
//! encoding (in terms of bitrate, codec, etc.), fetching segments of the content using HTTP or
//! HTTPS requests (this functionality depends on the `reqwest` crate) and muxing audio and video
//! segments together (using ffmpeg via the `ac_ffmpeg` crate).
//!
//!
//! ## DASH features supported
//!
//! - VOD (static) stream manifests
//! - Multi-period content
//! - XLink elements (only with actuate=onLoad semantics, resolve-to-zero supported)
//! - All forms of segment index info: SegmentBase@indexRange, SegmentTimeline,
//!   SegmentTemplate@duration, SegmentTemplate@index, SegmentList
//! - Media containers of types supported by mkvmerge, ffmpeg, VLC and MP4Box (this includes
//!   Matroska, ISO-BMFF / CMAF / MP4, WebM, MPEG-2 TS)
//! - Subtitles: preliminary support for WebVTT and TTML streams
//!
//!
//! ## Limitations / unsupported features
//!
//! - Dynamic MPD manifests, that are used for live streaming/OTT TV
//! - XLink with actuate=onRequest semantics
//! - Application of MPD patches
//
//
//
// Reference libdash library: https://github.com/bitmovin/libdash
//   https://github.com/bitmovin/libdash/blob/master/libdash/libdash/source/xml/Node.cpp
// Reference dash.js library: https://github.com/Dash-Industry-Forum/dash.js
// Google Shaka player: https://github.com/google/shaka-player
// The DASH code in VLC: https://code.videolan.org/videolan/vlc/-/tree/master/modules/demux/dash
// Streamlink source code: https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/dash_manifest.py

// TODO: handle dynamic MPD as per https://livesim.dashif.org/livesim/mup_30/testpic_2s/Manifest.mpd
// TODO: handle indexRange attribute, as per https://dash.akamaized.net/dash264/TestCasesMCA/dolby/2/1/ChID_voices_71_768_ddp.mpd
// TODO: implement MPD Patch support when downloading, with test cases from https://github.com/ab2022/mpddiffs/tree/main


#![allow(non_snake_case)]

/// If library feature `libav` is enabled, muxing support (combining audio and video streams, which
/// are often separated out in DASH streams) is provided by ffmpeg's libav library, via the
/// `ac_ffmpeg` crate. Otherwise, muxing is implemented by calling `mkvmerge`, `ffmpeg` or `vlc` as
/// a subprocess. The muxing support is only compiled when the fetch feature is enabled.
#[cfg(feature = "fetch")]
mod media;
#[cfg(all(feature = "fetch", feature = "libav"))]
mod libav;
#[cfg(all(feature = "fetch", not(feature = "libav")))]
mod ffmpeg;
#[cfg(feature = "fetch")]
pub mod sidx;
#[cfg(feature = "fetch")]
pub mod fetch;
// Support for the SCTE-35 standard for insertion of alternate content
#[cfg(feature = "scte35")]
pub mod scte35;
#[cfg(feature = "scte35")]
use crate::scte35::{Signal, SpliceInfoSection};

#[cfg(all(feature = "fetch", feature = "libav"))]
use crate::libav::{mux_audio_video, copy_video_to_container, copy_audio_to_container};
#[cfg(all(feature = "fetch", not(feature = "libav")))]
use crate::ffmpeg::{mux_audio_video, copy_video_to_container, copy_audio_to_container};
use std::cell::OnceCell;
use serde::{Serialize, Serializer, Deserialize};
use serde::de;
use serde_with::skip_serializing_none;
use regex::Regex;
use std::time::Duration;
use chrono::DateTime;
use url::Url;
#[allow(unused_imports)]
use tracing::warn;


/// Type representing an xs:dateTime, as per <https://www.w3.org/TR/xmlschema-2/#dateTime>
// Something like 2021-06-03T13:00:00Z or 2022-12-06T22:27:53
pub type XsDatetime = DateTime<chrono::offset::Utc>;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum DashMpdError {
    #[error("parse error {0:?}")]
    Parsing(String),
    #[error("invalid Duration: {0:?}")]
    InvalidDuration(String),
    #[error("invalid DateTime: {0:?}")]
    InvalidDateTime(String),
    #[error("invalid media stream: {0:?}")]
    UnhandledMediaStream(String),
    #[error("I/O error {1} ({0:?})")]
    Io(#[source] std::io::Error, String),
    #[error("network error {0:?}")]
    Network(String),
    #[error("network timeout: {0:?}")]
    NetworkTimeout(String),
    #[error("network connection: {0:?}")]
    NetworkConnect(String),
    #[error("muxing error {0:?}")]
    Muxing(String),
    #[error("decryption error {0:?}")]
    Decrypting(String),
    #[error("{0:?}")]
    Other(String),
}


// Serialize an xsd:double parameter. We can't use the default serde serialization for f64 due to
// the difference in handling INF, -INF and NaN values.
//
// Reference: http://www.datypic.com/sc/xsd/t-xsd_double.html
fn serialize_xsd_double<S>(xsd: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let formatted = if xsd.is_nan() {
        String::from("NaN")
    } else if xsd.is_infinite() {
        if xsd.is_sign_positive() {
            // Here serde returns "inf", which doesn't match the XML Schema definition.
            String::from("INF")
        } else {
            String::from("-INF")
        }
    } else {
        xsd.to_string()
    };
    serializer.serialize_str(&formatted)
}

// Serialize an Option<f64> as an xsd:double.
fn serialize_opt_xsd_double<S>(oxsd: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(xsd) = oxsd {
        serialize_xsd_double(xsd, serializer)
    } else {
        // in fact this won't be called because of the #[skip_serializing_none] annotation
        serializer.serialize_none()
    }
}


// Parse an XML duration string, as per https://www.w3.org/TR/xmlschema-2/#duration
//
// The lexical representation for duration is the ISO 8601 extended format PnYn MnDTnH nMnS, where
// nY represents the number of years, nM the number of months, nD the number of days, 'T' is the
// date/time separator, nH the number of hours, nM the number of minutes and nS the number of
// seconds. The number of seconds can include decimal digits to arbitrary precision.
//
// Examples: "PT0H0M30.030S", "PT1.2S", PT1004199059S, PT130S
// P2Y6M5DT12H35M30S  => 2 years, 6 months, 5 days, 12 hours, 35 minutes, 30 seconds
// P1DT2H => 1 day, 2 hours
// P0Y20M0D => 20 months (0 is permitted as a number, but is not required)
// PT1M30.5S => 1 minute, 30.5 seconds
//
// Limitations: we can't represent negative durations (leading "-" character) due to the choice of a
// std::time::Duration. We only accept fractional parts of seconds, and reject for example "P0.5Y" and "PT2.3H".
fn parse_xs_duration(s: &str) -> Result<Duration, DashMpdError> {
    use std::cmp::min;
    let xs_duration_regex = OnceCell::new();

    match xs_duration_regex.get_or_init(
        || Regex::new(concat!(r"^(?P<sign>[+-])?P",
                              r"(?:(?P<years>\d+)Y)?",
                              r"(?:(?P<months>\d+)M)?",
                              r"(?:(?P<weeks>\d+)W)?",
                              r"(?:(?P<days>\d+)D)?",
                              r"(?:(?P<hastime>T)", // time part must begin with a T
                              r"(?:(?P<hours>\d+)H)?",
                              r"(?:(?P<minutes>\d+)M)?",
                              r"(?:(?P<seconds>\d+)(?:(?P<nanoseconds>[.,]\d+)?)S)?",
                              r")?")).unwrap()).captures(s)
    {
        Some(m) => {
            if m.name("hastime").is_none() &&
               m.name("years").is_none() &&
               m.name("months").is_none() &&
               m.name("weeks").is_none() &&
               m.name("days").is_none() {
                  return Err(DashMpdError::InvalidDuration("empty".to_string()));
            }
            let mut secs: u64 = 0;
            let mut nsecs: u32 = 0;
            if let Some(nano) = m.name("nanoseconds") {
                // We drop the initial "." and limit precision in the fractional seconds to 9 digits
                // (nanosecond precision)
                let lim = min(nano.as_str().len(), 9 + ".".len());
                if let Some(ss) = &nano.as_str().get(1..lim) {
                    let padded = format!("{ss:0<9}");
                    nsecs = padded.parse::<u32>()
                        .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                }
            }
            if let Some(mseconds) = m.name("seconds") {
                let seconds = mseconds.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += seconds;
            }
            if let Some(mminutes) = m.name("minutes") {
                let minutes = mminutes.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += minutes * 60;
            }
            if let Some(mhours) = m.name("hours") {
                let hours = mhours.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += hours * 60 * 60;
            }
            if let Some(mdays) = m.name("days") {
                let days = mdays.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += days * 60 * 60 * 24;
            }
            if let Some(mweeks) = m.name("weeks") {
                let weeks = mweeks.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += weeks * 60 * 60 * 24 * 7;
            }
            if let Some(mmonths) = m.name("months") {
                let months = mmonths.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += months * 60 * 60 * 24 * 30;
            }
            if let Some(myears) = m.name("years") {
                let years = myears.as_str().parse::<u64>()
                    .map_err(|_| DashMpdError::InvalidDuration(String::from(s)))?;
                secs += years * 60 * 60 * 24 * 365;
            }
            if let Some(msign) = m.name("sign") {
                if msign.as_str() == "-" {
                    return Err(DashMpdError::InvalidDuration("can't represent negative durations".to_string()));
                }
            }
            Ok(Duration::new(secs, nsecs))
        },
        None => Err(DashMpdError::InvalidDuration(String::from("couldn't parse XS duration"))),
    }
}


// Note bug in current version of the iso8601 crate which incorrectly parses
// strings like "PT344S" (seen in a real MPD) as a zero duration. However, ISO 8601 standard as
// adopted by Indian Bureau of Standards includes p29 an example "PT72H", as do various MPD
// manifests in the wild. https://archive.org/details/gov.in.is.7900.2007/
// fn parse_xs_duration_buggy(s: &str) -> Result<Duration> {
//     match iso8601::duration(s) {
//         Ok(iso_duration) => {
//             match iso_duration {
//                 iso8601::Duration::Weeks(w) => Ok(Duration::new(w as u64*60 * 60 * 24 * 7, 0)),
//                 iso8601::Duration::YMDHMS {year, month, day, hour, minute, second, millisecond } => {
//                     // note that if year and month are specified, we are not going to do a very
//                     // good conversion here
//                     let mut secs: u64 = second.into();
//                     secs += minute as u64 * 60;
//                     secs += hour   as u64 * 60 * 60;
//                     secs += day    as u64 * 60 * 60 * 24;
//                     secs += month  as u64 * 60 * 60 * 24 * 31;
//                     secs += year   as u64 * 60 * 60 * 24 * 31 * 365;
//                     Ok(Duration::new(secs, millisecond * 1000_000))
//                 },
//             }
//         },
//         Err(e) => Err(anyhow!("Couldn't parse XS duration {}: {:?}", s, e)),
//     }
// }

// The iso8601_duration crate can't handle durations with fractional seconds
// fn parse_xs_duration_buggy(s: &str) -> Result<Duration> {
//     match iso8601_duration::Duration::parse(s) {
//         Ok(d) => {
//             let nanos: u32 = 1000_000 * d.second.fract() as u32;
//             let mut secs: u64 = d.second.trunc() as u64;
//             secs += d.minute as u64 * 60;
//             secs += d.hour   as u64 * 60 * 60;
//             secs += d.day    as u64 * 60 * 60 * 24;
//             secs += d.month  as u64 * 60 * 60 * 24 * 31;
//             secs += d.year   as u64 * 60 * 60 * 24 * 31 * 365;
//             Ok(Duration::new(secs, nanos))
//         },
//         Err(e) => Err(anyhow!("Couldn't parse XS duration {}: {:?}", s, e)),
//     }
// }



// Deserialize an optional XML duration string to an Option<Duration>. This is a little trickier
// than deserializing a required field with serde.
fn deserialize_xs_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: de::Deserializer<'de>,
{
    match <Option<String>>::deserialize(deserializer) {
        Ok(optstring) => match optstring {
            Some(xs) => match parse_xs_duration(&xs) {
                Ok(d) => Ok(Some(d)),
                Err(e) => Err(de::Error::custom(e)),
            },
            None => Ok(None),
        },
        // the field isn't present, return an Ok(None)
        Err(_) => Ok(None),
    }
}

// There are many possible correct ways of serializing a Duration in xs:duration (ISO 8601) format.
// We choose to serialize to a perhaps-canonical xs:duration format including hours and minutes
// (instead of representing them as a large number of seconds). Hour and minute count are not
// included when the duration is less than a minute. Trailing zeros are omitted. Fractional seconds
// are included to a nanosecond precision.
//
// Example: Duration::new(3600, 40_000_000) => "PT1H0M0.04S"
fn serialize_xs_duration<S>(oxs: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(xs) = oxs {
        let seconds = xs.as_secs();
        let nanos = xs.subsec_nanos();
        let minutes = seconds / 60;
        let hours = if minutes > 59 { minutes / 60 } else { 0 };
        let fractional_maybe = if nanos > 0 {
            format!(".{nanos:09}").trim_end_matches('0').to_string()
        } else {
            "".to_string()
        };
        let formatted_duration = if hours > 0 {
            let mins = minutes % 60;
            let secs = seconds % 60;
            format!("PT{hours}H{mins}M{secs}{fractional_maybe}S")
        } else if minutes > 0 {
            let secs = seconds % 60;
            format!("PT{minutes}M{secs}{fractional_maybe}S")
        } else {
            format!("PT{seconds}{fractional_maybe}S")
        };
        serializer.serialize_str(&formatted_duration)
    } else {
        // in fact this won't be called because of the #[skip_serializing_none] annotation
        serializer.serialize_none()
    }
}


// We can't use the parsing functionality from the chrono crate, because that assumes RFC 3339
// format (including a timezone), whereas the xs:dateTime type (as per
// <https://www.w3.org/TR/xmlschema-2/#dateTime>) allows the timezone to be omitted. For more on the
// complicated relationship between ISO 8601 and RFC 3339, see
// <https://ijmacd.github.io/rfc3339-iso8601/>.
fn parse_xs_datetime(s: &str) -> Result<XsDatetime, DashMpdError> {
    use iso8601::Date;
    use chrono::{LocalResult, NaiveDate, TimeZone};
    use num_traits::cast::FromPrimitive;
    match DateTime::<chrono::offset::FixedOffset>::parse_from_rfc3339(s) {
        Ok(dt) => Ok(dt.into()),
        Err(_) => match iso8601::datetime(s) {
            Ok(dt) => {
                let nd = match dt.date {
                    Date::YMD { year, month, day } =>
                        NaiveDate::from_ymd_opt(year, month, day)
                        .ok_or(DashMpdError::InvalidDateTime(s.to_string()))?,
                    Date::Week { year, ww, d } => {
                        let d = chrono::Weekday::from_u32(d)
                            .ok_or(DashMpdError::InvalidDateTime(s.to_string()))?;
                        NaiveDate::from_isoywd_opt(year, ww, d)
                            .ok_or(DashMpdError::InvalidDateTime(s.to_string()))?
                    },
                    Date::Ordinal { year, ddd } =>
                        NaiveDate::from_yo_opt(year, ddd)
                        .ok_or(DashMpdError::InvalidDateTime(s.to_string()))?,
                };
                let nd = nd.and_hms_nano_opt(dt.time.hour, dt.time.minute, dt.time.second, dt.time.millisecond*1000*1000)
                    .ok_or(DashMpdError::InvalidDateTime(s.to_string()))?;
                let tz_secs = dt.time.tz_offset_hours * 3600 + dt.time.tz_offset_minutes * 60;
                match chrono::FixedOffset::east_opt(tz_secs)
                    .ok_or(DashMpdError::InvalidDateTime(s.to_string()))?
                    .from_local_datetime(&nd)
                {
                    LocalResult::Single(local) => Ok(local.with_timezone(&chrono::Utc)),
                    _ => Err(DashMpdError::InvalidDateTime(s.to_string())),
                }
            },
            Err(_) => Err(DashMpdError::InvalidDateTime(s.to_string())),
        }
    }
}

// Deserialize an optional XML datetime string (type xs:datetime) to an Option<XsDatetime>.
fn deserialize_xs_datetime<'de, D>(deserializer: D) -> Result<Option<XsDatetime>, D::Error>
where
    D: de::Deserializer<'de>,
{
    match <Option<String>>::deserialize(deserializer) {
        Ok(optstring) => match optstring {
            Some(xs) => match parse_xs_datetime(&xs) {
                Ok(d) => Ok(Some(d)),
                Err(e) => Err(de::Error::custom(e)),
            },
            None => Ok(None),
        },
        // the field isn't present; return an Ok(None)
        Err(_) => Ok(None),
    }
}

// XSD type is "UIntVectorType", or whitespace-separated list of unsigned integers.
// It's a <xs:list itemType="xs:unsignedInt"/>.
fn serialize_xsd_uintvector<S>(v: &Vec<u64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut formatted = String::new();
    for u in v {
        formatted += &format!("{u} ");
    }
    serializer.serialize_str(&formatted)
}

fn deserialize_xsd_uintvector<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let mut out = Vec::<u64>::new();
    for uint64_str in s.split_whitespace() {
        match uint64_str.parse::<u64>() {
            Ok(val) => out.push(val),
            Err(e) => return Err(de::Error::custom(e)),
        }
    }
    Ok(out)
}

// These serialization functions are need to serialize correct default values for various optional
// namespaces specified as attributes of the root MPD struct (e.g. xmlns:xsi, xmlns:xlink). If a
// value is present in the struct field (specified in the parsed XML or provided explicitly when
// building the MPD struct) then we use that, and otherwise default to the well-known URLs for these
// namespaces.
//
// The quick-xml support for #[serde(default = "fn")] (which would allow a less heavyweight solution
// to this) does not seem to work.

fn serialize_xmlns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("urn:mpeg:dash:schema:mpd:2011")
    }
}

fn serialize_xsi_ns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("http://www.w3.org/2001/XMLSchema-instance")
    }
}

fn serialize_cenc_ns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("urn:mpeg:cenc:2013")
    }
}

fn serialize_mspr_ns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("urn:microsoft:playready")
    }
}

fn serialize_xlink_ns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("http://www.w3.org/1999/xlink")
    }
}

fn serialize_dvb_ns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("urn:dvb:dash-extensions:2014-1")
    }
}


// These default_* functions are needed to provide defaults for serde deserialization of certain
// elements, where the Default function for that type doesn't return a value compatible with the
// default specified in the XSD specification.
fn default_optstring_on_request() -> Option<String> {
    Some("onRequest".to_string())
}

fn default_optstring_one() -> Option<String> {
    Some(String::from("1"))
}

fn default_optstring_encoder() -> Option<String> {
    Some(String::from("encoder"))
}

fn default_optbool_false() -> Option<bool> {
    Some(false)
}

fn default_optu64_zero() -> Option<u64> {
    Some(0)
}


// The MPD format is documented by ISO using an XML Schema at
// https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/DASH-MPD-edition2.xsd
// Historical spec: https://ptabdata.blob.core.windows.net/files/2020/IPR2020-01688/v67_EXHIBIT%201067%20-%20ISO-IEC%2023009-1%202019(E)%20-%20Info.%20Tech.%20-%20Dynamic%20Adaptive%20Streaming%20Over%20HTTP%20(DASH).pdf
// We occasionally diverge from the standard when in-the-wild implementations do.
// Some reference code for DASH is at https://github.com/bitmovin/libdash
//
// We are using the quick_xml + serde crates to deserialize the XML content to Rust structs, and the
// reverse serialization process of programmatically generating XML from Rust structs. Note that
// serde will ignore unknown fields when deserializing, so we don't need to cover every single
// possible field.

/// The title of the media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Title {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

/// The original source of the media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Source {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

/// Copyright information concerning the media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Copyright {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

/// Metainformation concerning the media stream (title, language, etc.)
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct ProgramInformation {
    /// Language in RFC 5646 format
    #[serde(rename = "@lang")]
    pub lang: Option<String>,
    #[serde(rename = "@moreInformationURL")]
    pub moreInformationURL: Option<String>,
    pub Title: Option<Title>,
    pub Source: Option<Source>,
    pub Copyright: Option<Copyright>,
    #[serde(rename(serialize = "scte214:ContentIdentifier", deserialize = "ContentIdentifier"))]
    pub scte214ContentIdentifier: Option<Scte214ContentIdentifier>,
}

/// DASH specification MPEG extension (SCTE 214) program identification type.
///
/// Indicates how the program content is identified.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Scte214ContentIdentifier {
    #[serde(rename = "@type")]
    pub idType: Option<String>,
    #[serde(rename = "@value")]
    pub idValue: Option<String>,
}

/// Describes a sequence of contiguous Segments with identical duration.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct S {
    /// Time
    #[serde(rename = "@t")]
    pub t: Option<u64>,
    #[serde(rename = "@n")]
    pub n: Option<u64>,
    /// The duration (shall not exceed the value of MPD@maxSegmentDuration).
    #[serde(rename = "@d")]
    pub d: u64,
    /// The repeat count (number of contiguous Segments with identical MPD duration minus one),
    /// defaulting to zero if not present.
    #[serde(rename = "@r")]
    pub r: Option<i64>,
    #[serde(rename = "@k")]
    pub k: Option<u64>,
}

/// Contains a sequence of `S` elements, each of which describes a sequence of contiguous segments of
/// identical duration.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct SegmentTimeline {
    /// There must be at least one S element.
    #[serde(rename = "S")]
    pub segments: Vec<S>,
}

/// Information on the bitstream switching capabilities for Representations.
///
/// When bitstream switching is enabled, the player can seamlessly switch between Representations in
/// the manifest without reinitializing the media decoder. This means fewer perturbations for the
/// viewer when the network conditions change. It requires the media segments to have been encoded
/// respecting a certain number of constraints.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct BitstreamSwitching {
    #[serde(rename = "@sourceURL")]
    pub source_url: Option<String>,
    #[serde(rename = "@range")]
    pub range: Option<String>,
}

/// The first media segment in a sequence of Segments.
///
/// Subsequent segments can be concatenated to this segment to produce a media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Initialization {
    #[serde(rename = "@sourceURL")]
    pub sourceURL: Option<String>,
    #[serde(rename = "@range")]
    pub range: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct RepresentationIndex {
    #[serde(rename = "@range")]
    pub range: Option<String>,
    #[serde(rename = "@sourceURL")]
    pub sourceURL: Option<String>,
}

/// Allows template-based `SegmentURL` construction. Specifies various substitution rules using
/// dynamic values such as `$Time$` and `$Number$` that map to a sequence of Segments.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SegmentTemplate {
    #[serde(rename = "@media")]
    pub media: Option<String>,
    #[serde(rename = "@index")]
    pub index: Option<String>,
    #[serde(rename = "@initialization")]
    pub initialization: Option<String>,
    #[serde(rename = "@bitstreamSwitching")]
    pub bitstreamSwitching: Option<String>,
    #[serde(rename = "@indexRange")]
    pub indexRange: Option<String>,
    #[serde(rename = "@indexRangeExact")]
    pub indexRangeExact: Option<bool>,
    #[serde(rename = "@startNumber")]
    pub startNumber: Option<u64>,
    // note: the spec says this is an unsigned int, not an xs:duration. In practice, some manifests
    // use a floating point value (eg.
    // https://dash.akamaized.net/akamai/bbb_30fps/bbb_with_multiple_tiled_thumbnails.mpd)
    #[serde(rename = "@duration")]
    pub duration: Option<f64>,
    #[serde(rename = "@timescale")]
    pub timescale: Option<u64>,
    /// Indicates a possible offset between media segment start/end points and period start/end points.
    #[serde(rename = "@eptDelta")]
    pub eptDelta: Option<i64>,
    /// Specifies the difference between the presentation duration of this Representation and the
    /// Period duration. Expressed in units of @timescale.
    #[serde(rename = "@pdDelta")]
    pub pbDelta: Option<i64>,
    #[serde(rename = "@presentationTimeOffset")]
    pub presentationTimeOffset: Option<u64>,
    #[serde(rename = "@availabilityTimeOffset", serialize_with="serialize_opt_xsd_double")]
    pub availabilityTimeOffset: Option<f64>,
    #[serde(rename = "@availabilityTimeComplete")]
    pub availabilityTimeComplete: Option<bool>,
    pub Initialization: Option<Initialization>,
    #[serde(rename = "RepresentationIndex")]
    pub representation_index: Option<RepresentationIndex>,
    // The XSD included in the DASH specification only includes a FailoverContent element on the
    // SegmentBase element, but also includes it on a SegmentTemplate element in one of the
    // examples. Even if examples are not normative, we choose to be tolerant in parsing.
    #[serde(rename = "FailoverContent")]
    pub failover_content: Option<FailoverContent>,
    pub SegmentTimeline: Option<SegmentTimeline>,
    pub BitstreamSwitching: Option<BitstreamSwitching>,
}

/// A URI string to which a new request for an updated manifest should be made.
///
/// This feature is intended for servers and clients that can't use sticky HTTP redirects.
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Location {
    #[serde(rename = "$text")]
    pub url: String,
}

/// A URI string that specifies one or more common locations for Segments and other resources.
///
/// Used as a prefix for SegmentURLs. Can be specified at the level of the MPD node, or Period,
/// AdaptationSet, Representation, and can be nested (the client should combine the prefix on MPD
/// and on Representation, for example).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct BaseURL {
    #[serde(rename = "@serviceLocation")]
    pub serviceLocation: Option<String>,
    #[serde(rename = "@byteRange")]
    pub byte_range: Option<String>,
    /// Elements with the same `@serviceLocation` value are likely to have their URLs resolve to
    /// services at a common network location, for example the same CDN.
    #[serde(rename = "@availabilityTimeOffset", serialize_with="serialize_opt_xsd_double")]
    pub availability_time_offset: Option<f64>,
    #[serde(rename = "@availabilityTimeComplete")]
    pub availability_time_complete: Option<bool>,
    #[serde(rename = "@timeShiftBufferDepth",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub timeShiftBufferDepth: Option<Duration>,
    /// Lowest value indicates the highest priority.
    #[serde(rename = "@dvb:priority", alias = "@priority")]
    pub priority: Option<u64>,
    /// For load balancing between different base urls with the same @priority. The BaseURL to use
    /// is chosen at random by the player, with the weight of any given BaseURL being its @weight
    /// value divided by the sum of all @weight values.
    #[serde(rename = "@dvb:weight", alias = "@weight")]
    pub weight: Option<i64>,
    #[serde(rename = "$text")]
    pub base: String,
}

/// Failover Content Segment (FCS).
///
/// The time and optional duration for which a representation does not represent the main content
/// but a failover version. It can and is also used to represent gaps where no segments are present
/// at all - used within the `FailoverContent` element.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Fcs {
    /// The time at which no/failover segments for this representation starts (if the valid
    /// flag is set to `true` in `FailoverContent`).
    #[serde(rename = "@t")]
    pub t: u64,

    /// The optional duration for which there is failover or no content.  If `None` then
    /// the duration is for the remainder of the `Period` the parent `Representation` is in.
    #[serde(rename = "@d")]
    pub d: Option<u64>,
}

/// Period of time for which either failover content or no content/segments exist for the
/// parent `Representation`.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct FailoverContent {
    // If true, the FCS represents failover content; if false, it represents a gap
    // where there are no segments at all.
    #[serde(rename = "@valid")]
    pub valid: Option<bool>,
    #[serde(rename = "FCS")]
    pub fcs_list: Vec<Fcs>,
}

/// Specifies some common information concerning media segments.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SegmentBase {
    #[serde(rename = "@timescale")]
    pub timescale: Option<u64>,
    #[serde(rename = "@presentationTimeOffset")]
    pub presentationTimeOffset: Option<u64>,
    #[serde(rename = "@indexRange")]
    pub indexRange: Option<String>,
    #[serde(rename = "@indexRangeExact")]
    pub indexRangeExact: Option<bool>,
    #[serde(rename = "@availabilityTimeOffset", serialize_with="serialize_opt_xsd_double")]
    pub availabilityTimeOffset: Option<f64>,
    #[serde(rename = "@availabilityTimeComplete")]
    pub availabilityTimeComplete: Option<bool>,
    #[serde(rename = "@presentationDuration")]
    pub presentationDuration: Option<u64>,
    /// Indicates a possible offset between media segment start/end points and period start/end points.
    #[serde(rename = "@eptDelta")]
    pub eptDelta: Option<i64>,
    /// Specifies the difference between the presentation duration of this Representation and the
    /// Period duration. Expressed in units of @timescale.
    #[serde(rename = "@pdDelta")]
    pub pbDelta: Option<i64>,
    pub Initialization: Option<Initialization>,
    #[serde(rename = "RepresentationIndex")]
    pub representation_index: Option<RepresentationIndex>,
    #[serde(rename = "FailoverContent")]
    pub failover_content: Option<FailoverContent>,
}

/// The URL of a media segment.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct SegmentURL {
    #[serde(rename = "@media")]
    pub media: Option<String>, // actually an URI
    #[serde(rename = "@mediaRange")]
    pub mediaRange: Option<String>,
    #[serde(rename = "@index")]
    pub index: Option<String>, // actually an URI
    #[serde(rename = "@indexRange")]
    pub indexRange: Option<String>,
}

/// Contains a sequence of SegmentURL elements.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct SegmentList {
    // note: the spec says this is an unsigned int, not an xs:duration
    #[serde(rename = "@duration")]
    pub duration: Option<u64>,
    #[serde(rename = "@timescale")]
    pub timescale: Option<u64>,
    #[serde(rename = "@indexRange")]
    pub indexRange: Option<String>,
    #[serde(rename = "@indexRangeExact")]
    pub indexRangeExact: Option<bool>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "@xlink:href", alias = "@href")]
    pub href: Option<String>,
    #[serde(rename = "@xlink:actuate", alias = "@actuate", default="default_optstring_on_request")]
    pub actuate: Option<String>,
    #[serde(rename = "@xlink:type", alias = "@type")]
    pub sltype: Option<String>,
    #[serde(rename = "@xlink:show", alias = "@show")]
    pub show: Option<String>,
    pub Initialization: Option<Initialization>,
    pub SegmentTimeline: Option<SegmentTimeline>,
    pub BitstreamSwitching: Option<BitstreamSwitching>,
    #[serde(rename = "SegmentURL")]
    pub segment_urls: Vec<SegmentURL>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Resync {
    #[serde(rename = "@type")]
    pub rtype: Option<String>,
    #[serde(rename = "@dT")]
    pub dT: Option<u64>,
    #[serde(rename = "@dImax")]
    pub dImax: Option<f64>,
    #[serde(rename = "@dImin")]
    pub dImin: Option<f64>,
    #[serde(rename = "@marker")]
    pub marker: Option<bool>,
}

/// Specifies information concerning the audio channel (e.g. stereo, multichannel).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct AudioChannelConfiguration {
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
}

// This element is not specified in ISO/IEC 23009-1:2022; exact format is unclear.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Language {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

/// A Preselection is a personalization option to produce a “complete audio experience”.
///
/// Used for audio signaling in the context of the ATSC 3.0 standard for advanced IP-based
/// television broadcasting. Details are specified by the “DASH-IF Interoperability Point for ATSC
/// 3.0” document.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Preselection {
    #[serde(rename = "@id", default = "default_optstring_one")]
    pub id: Option<String>,
    /// Specifies the ids of the contained elements/content components of this Preselection list as
    /// white space separated list in processing order. The first id defines the main element.
    #[serde(rename = "@preselectionComponents")]
    pub preselectionComponents: String,
    #[serde(rename = "@lang")]
    pub lang: Option<String>,
    #[serde(rename = "@audioSamplingRate")]
    pub audioSamplingRate: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381>
    #[serde(rename = "@codecs")]
    pub codecs: String,
    #[serde(rename = "@selectionPriority")]
    pub selectionPriority: Option<u64>,
    #[serde(rename = "@tag")]
    pub tag: String,
    pub FramePacking: Vec<FramePacking>,
    pub AudioChannelConfiguration: Vec<AudioChannelConfiguration>,
    pub ContentProtection: Vec<ContentProtection>,
    pub OutputProtection: Option<OutputProtection>,
    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,
    pub InbandEventStream: Vec<InbandEventStream>,
    pub Switching: Vec<Switching>,
    // TODO: missing RandomAccess element
    #[serde(rename = "GroupLabel")]
    pub group_label: Vec<Label>,
    pub Label: Vec<Label>,
    pub ProducerReferenceTime: Option<ProducerReferenceTime>,
    // TODO: missing ContentPopularityRate element
    pub Resync: Option<Resync>,
    #[serde(rename = "Accessibility")]
    pub accessibilities: Vec<Accessibility>,
    #[serde(rename = "Role")]
    pub roles: Vec<Role>,
    #[serde(rename = "Rating")]
    pub ratings: Vec<Rating>,
    #[serde(rename = "Viewpoint")]
    pub viewpoints: Vec<Viewpoint>,
    // end PreselectionType specific elements
    #[serde(rename = "Language")]
    pub languages: Vec<Language>,
}

/// Specifies that content is suitable for presentation to audiences for which that rating is known to be
/// appropriate, or for unrestricted audiences.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Rating {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
}

/// Specifies frame-packing arrangement information of the video media component type.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct FramePacking {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
}

/// Information used to allow Adaptation Set Switching (for instance, allowing the player to switch
/// between camera angles).
///
/// This is different from "bitstream switching".
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Switching {
    #[serde(rename = "@interval")]
    pub interval: Option<u64>,
    /// Valid values are "media" and "bitstream".
    #[serde(rename = "@type")]
    pub stype: Option<String>,
}

/// Specifies the accessibility scheme used by the media content.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Accessibility {
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
}

/// Scope of a namespace.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Scope {
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
}

/// A SubRepresentation contains information that only applies to one media stream in a Representation.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SubRepresentation {
    #[serde(rename = "@level")]
    pub level: Option<u32>,
    #[serde(rename = "@dependencyLevel")]
    pub dependencyLevel: Option<String>,
    /// If present, a whitespace-separated list of values of ContentComponent@id values.
    #[serde(rename = "@contentComponent")]
    pub contentComponent: Option<String>,
    #[serde(rename = "@mimeType")]
    pub mimeType: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381>
    #[serde(rename = "@codecs")]
    pub codecs: Option<String>,
    #[serde(rename = "@contentType")]
    pub contentType: Option<String>,
    #[serde(rename = "@profiles")]
    pub profiles: Option<String>,
    #[serde(rename = "@segmentProfiles")]
    /// Specifies the profiles of Segments that are essential to process the Representation. The
    /// semantics depend on the value of the @mimeType attribute.
    pub segmentProfiles: Option<String>,
    /// If present, this attribute is expected to be set to "progressive".
    #[serde(rename = "@scanType")]
    pub scanType: Option<String>,
    #[serde(rename = "@frameRate")]
    pub frameRate: Option<String>, // can be something like "15/2"
    /// The Sample Aspect Ratio, eg. "1:1"
    #[serde(rename = "@sar")]
    pub sar: Option<String>,
    /// The average bandwidth of the Representation.
    #[serde(rename = "@bandwidth")]
    pub bandwidth: Option<u64>,
    #[serde(rename = "@audioSamplingRate")]
    pub audioSamplingRate: Option<String>,
    /// Indicates the possibility for accelerated playout allowed by this codec profile and level.
    #[serde(rename = "@maxPlayoutRate", serialize_with="serialize_opt_xsd_double")]
    pub maxPlayoutRate: Option<f64>,
    #[serde(rename = "@codingDependency")]
    pub codingDependency: Option<bool>,
    #[serde(rename = "@width")]
    pub width: Option<u64>,
    #[serde(rename = "@height")]
    pub height: Option<u64>,
    #[serde(rename = "@startWithSAP")]
    pub startWithSAP: Option<u64>,
    #[serde(rename = "@maximumSAPPeriod", serialize_with="serialize_opt_xsd_double")]
    pub maximumSAPPeriod: Option<f64>,
    pub FramePacking: Vec<FramePacking>,
    pub AudioChannelConfiguration: Vec<AudioChannelConfiguration>,
    pub ContentProtection: Vec<ContentProtection>,
    pub OutputProtection: Option<OutputProtection>,
    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,
    pub InbandEventStream: Vec<InbandEventStream>,
    pub Switching: Vec<Switching>,
    // TODO: missing RandomAccess element
    #[serde(rename = "GroupLabel")]
    pub group_label: Vec<Label>,
    pub Label: Vec<Label>,
    pub ProducerReferenceTime: Option<ProducerReferenceTime>,
    // TODO: missing ContentPopularityRate element
    pub Resync: Option<Resync>,
}

/// A Representation describes a version of the content, using a specific encoding and bitrate.
///
/// Streams often have multiple representations with different bitrates, to allow the client to
/// select that most suitable to its network conditions (adaptive bitrate or ABR streaming).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Representation {
    // no id for a linked Representation (with xlink:href), so this attribute is optional
    #[serde(rename = "@id")]
    pub id: Option<String>,
    /// The average bandwidth of the Representation.
    #[serde(rename = "@bandwidth")]
    pub bandwidth: Option<u64>,
    /// Specifies a quality ranking of this Representation relative to others in the same
    /// AdaptationSet. Lower values represent higher quality content. If not present, then no
    /// ranking is defined.
    #[serde(rename = "@qualityRanking")]
    pub qualityRanking: Option<u8>,
    /// Identifies the base layer representation of this enhancement layer representation.
    /// Separation between a base layer and a number of enhancement layers is used by certain
    /// content encoding mechanisms, such as HEVC Scalable and Dolby Vision.
    #[serde(rename = "@dependencyId")]
    pub dependencyId: Option<String>,
    #[serde(rename = "@associationId")]
    pub associationId: Option<String>,
    #[serde(rename = "@associationType")]
    pub associationType: Option<String>,
    #[serde(rename = "@mediaStreamStructureId")]
    pub mediaStreamStructureId: Option<String>,
    #[serde(rename = "@profiles")]
    pub profiles: Option<String>,
    #[serde(rename = "@width")]
    pub width: Option<u64>,
    #[serde(rename = "@height")]
    pub height: Option<u64>,
    /// The Sample Aspect Ratio, eg. "1:1".
    #[serde(rename = "@sar")]
    pub sar: Option<String>,
    #[serde(rename = "@frameRate")]
    pub frameRate: Option<String>, // can be something like "15/2"
    #[serde(rename = "@audioSamplingRate")]
    pub audioSamplingRate: Option<String>,
    // The specification says that @mimeType is mandatory, but it's not always present on
    // akamaized.net MPDs
    #[serde(rename = "@mimeType")]
    pub mimeType: Option<String>,
    /// Specifies the profiles of Segments that are essential to process the Representation. The
    /// semantics depend on the value of the @mimeType attribute.
    #[serde(rename = "@segmentProfiles")]
    pub segmentProfiles: Option<String>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381>
    #[serde(rename = "@codecs")]
    pub codecs: Option<String>,
    #[serde(rename = "@containerProfiles")]
    pub containerProfiles: Option<String>,
    #[serde(rename = "@maximumSAPPeriod")]
    pub maximumSAPPeriod: Option<f64>,
    #[serde(rename = "@startWithSAP")]
    pub startWithSAP: Option<u64>,
    /// Indicates the possibility for accelerated playout allowed by this codec profile and level.
    #[serde(rename = "@maxPlayoutRate", serialize_with="serialize_opt_xsd_double")]
    pub maxPlayoutRate: Option<f64>,
    #[serde(rename = "@codingDependency")]
    pub codingDependency: Option<bool>,
    /// If present, this attribute is expected to be set to "progressive".
    #[serde(rename = "@scanType")]
    pub scanType: Option<String>,
    #[serde(rename = "@selectionPriority")]
    pub selectionPriority: Option<u64>,
    #[serde(rename = "@tag")]
    pub tag: Option<String>,
    #[serde(rename = "@contentType")]
    pub contentType: Option<String>,
    /// Language in RFC 5646 format.
    #[serde(rename = "@lang")]
    pub lang: Option<String>,
    #[serde(rename = "@sampleRate")]
    pub sampleRate: Option<u64>,
    #[serde(rename = "@numChannels")]
    pub numChannels: Option<u32>,
    #[serde(rename = "@xlink:href", alias = "@href")]
    pub href: Option<String>,
    #[serde(rename = "@xlink:actuate", alias = "@actuate", default = "default_optstring_on_request")]
    pub actuate: Option<String>,
    #[serde(rename = "@scte214:supplementalProfiles", alias = "@supplementalProfiles")]
    pub scte214_supplemental_profiles: Option<String>,
    #[serde(rename = "@scte214:supplementalCodecs", alias = "@supplementalCodecs")]
    pub scte214_supplemental_codecs: Option<String>,
    pub FramePacking: Vec<FramePacking>,
    pub AudioChannelConfiguration: Vec<AudioChannelConfiguration>,
    pub ContentProtection: Vec<ContentProtection>,
    pub OutputProtection: Option<OutputProtection>,
    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,
    pub InbandEventStream: Vec<InbandEventStream>,
    pub Switching: Vec<Switching>,
    // TODO: missing RandomAccess element
    #[serde(rename = "GroupLabel")]
    pub group_label: Vec<Label>,
    pub Label: Vec<Label>,
    pub ProducerReferenceTime: Vec<ProducerReferenceTime>,
    // TODO: missing ContentPopularityRate element
    pub Resync: Vec<Resync>,
    pub BaseURL: Vec<BaseURL>,
    // TODO: missing ExtendedBandwidth element
    pub SubRepresentation: Vec<SubRepresentation>,
    pub SegmentBase: Option<SegmentBase>,
    pub SegmentList: Option<SegmentList>,
    pub SegmentTemplate: Option<SegmentTemplate>,
    #[serde(rename = "RepresentationIndex")]
    pub representation_index: Option<RepresentationIndex>,
}

/// Describes a media content component.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct ContentComponent {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    /// Language in RFC 5646 format (eg. "fr-FR", "en-AU").
    #[serde(rename = "@lang")]
    pub lang: Option<String>,
    #[serde(rename = "@contentType")]
    pub contentType: Option<String>,
    #[serde(rename = "@par")]
    pub par: Option<String>,
    #[serde(rename = "@tag")]
    pub tag: Option<String>,
    pub Accessibility: Vec<Accessibility>,
    pub Role: Vec<Role>,
    pub Rating: Vec<Rating>,
    pub Viewpoint: Vec<Viewpoint>,
}

/// A Common Encryption "Protection System Specific Header" box. Content is typically base64 encoded.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct CencPssh {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

/// Licence acquisition URL for content using Microsoft PlayReady DRM.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Laurl {
    #[serde(rename = "@Lic_type")]
    pub lic_type: Option<String>,
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

/// Initialization data that is specific to the Microsoft PlayReady DRM.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct MsprPro {
    #[serde(rename = "@xmlns", serialize_with="serialize_xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct MsprIsEncrypted {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct MsprIVSize {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct MsprKid {
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct OutputProtection {
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
}

/// Contains information on DRM (rights management / encryption) mechanisms used in the stream.
///
/// If this node is not present, no content protection (such as Widevine and Playready) is applied
/// by the source.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct ContentProtection {
    /// The robustness level required for this content protection scheme.
    #[serde(rename = "@robustness")]
    pub robustness: Option<String>,
    #[serde(rename = "@refId")]
    pub refId: Option<String>,
    /// An xs:IDREF that references an identifier in this MPD.
    #[serde(rename = "@ref")]
    pub r#ref: Option<String>,
    /// References an identifier in this MPD.
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
    /// The DRM initialization data (Protection System Specific Header).
    #[serde(rename="cenc:pssh", alias="pssh")]
    pub cenc_pssh: Vec<CencPssh>,
    /// The DRM key identifier.
    #[serde(rename = "@cenc:default_KID", alias = "@default_KID")]
    pub default_KID: Option<String>,
    /// License acquisition URL.
    #[serde(rename = "dashif:laurl", alias = "laurl")]
    pub laurl: Option<Laurl>,
    /// License acquisition URL. The name clearkey:Laurl is obsolete and replaced by dashif:laurl.
    /// Some manifests in the wild include both, and the parser does not allow for duplicate fields,
    /// so we need to allow for this field using a distinct name.
    #[serde(rename = "clearkey:Laurl", alias = "Laurl")]
    pub clearkey_laurl: Option<Laurl>,
    /// Content specific to initialization data using Microsoft PlayReady DRM.
    #[serde(rename = "mspr:pro", alias = "pro")]
    pub msprpro: Option<MsprPro>,
    #[serde(rename = "mspr:IsEncrypted", alias = "IsEncrypted")]
    pub mspr_is_encrypted: Option<MsprIsEncrypted>,
    #[serde(rename = "mspr:IV_Size", alias = "IV_Size")]
    pub mspr_iv_size: Option<MsprIVSize>,
    #[serde(rename = "mspr:kid", alias = "kid")]
    pub mspr_kid: Option<MsprKid>,
}

/// The Role specifies the purpose of this media stream (caption, subtitle, main content, etc.).
///
/// Possible values include "caption", "subtitle", "main", "alternate", "supplementary",
/// "commentary", and "dub" (this is the attribute scheme for @value when the schemeIdUri is
/// "urn:mpeg:dash:role:2011").
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Role {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Viewpoint {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Selection {
    #[serde(rename = "@dataEncoding")]
    pub dataEncoding: Option<String>,
    #[serde(rename = "@parameter")]
    pub parameter: Option<String>,
    #[serde(rename = "@data")]
    pub data: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct SelectionInfo {
    #[serde(rename = "@selectionInfo")]
    pub selectionInfo: Option<String>,
    #[serde(rename = "@contactURL")]
    pub contactURL: Option<String>,
    pub Selection: Vec<Selection>,
}

/// A mechanism allowing the server to send additional information to the DASH client which is
/// synchronized with the media stream.
///
/// DASH Events are Used for various purposes such as dynamic ad insertion, providing additional
/// metainformation concerning the actors or location at a point in the media stream, providing
/// parental guidance information, or sending custom data to the DASH player application.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Event {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@presentationTime", default = "default_optu64_zero")]
    pub presentationTime: Option<u64>,
    #[serde(rename = "@presentationTimeOffset")]
    pub presentationTimeOffset: Option<u64>,
    #[serde(rename = "@duration")]
    pub duration: Option<u64>,
    #[serde(rename = "@timescale")]
    pub timescale: Option<u64>,
    /// Possible encoding (e.g. "base64") for the Event content or the value of the @messageData
    /// attribute.
    #[serde(rename = "@contentEncoding")]
    pub contentEncoding: Option<String>,
    /// The value for this event stream element. This attribute is present for backward
    /// compatibility; message content should be included in the Event element instead.
    #[serde(rename = "@messageData")]
    pub messageData: Option<String>,
    pub SelectionInfo: Option<SelectionInfo>,
    #[cfg(feature = "scte35")]
    #[serde(rename = "scte35:Signal", alias="Signal")]
    #[cfg(feature = "scte35")]
    pub signal: Vec<Signal>,
    #[cfg(feature = "scte35")]
    #[serde(rename = "scte35:SpliceInfoSection", alias="SpliceInfoSection")]
    #[cfg(feature = "scte35")]
    pub splice_info_section: Vec<SpliceInfoSection>,
    // #[serde(rename = "@schemeIdUri")]
    // pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    // The content may be base64 encoded, but may also be text. See for example
    // https://refapp.hbbtv.org/videos/00_llama_multiperiod_v1/manifest.mpd
    #[serde(rename = "$text")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct EventStream {
    #[serde(rename = "@xlink:href")]
    #[serde(alias = "@href")]
    pub href: Option<String>,
    #[serde(rename = "@xlink:actuate", alias = "@actuate", default = "default_optstring_on_request")]
    pub actuate: Option<String>,
    #[serde(rename = "@messageData")]
    // actually an xs:anyURI
    pub messageData: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@timescale")]
    pub timescale: Option<u64>,
    #[serde(rename = "@presentationTimeOffset")]
    pub presentationTimeOffset: Option<u64>,
    #[serde(rename = "Event")]
    pub event: Vec<Event>,
}

/// "Inband" events are materialized by the presence of DASHEventMessageBoxes (emsg) in the media
/// segments.
///
/// The client is informed of their presence by the inclusion of an InbandEventStream element in the
/// AdaptationSet or Representation element.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct InbandEventStream {
    #[serde(rename = "@timescale")]
    pub timescale: Option<u64>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "Event")]
    pub event: Vec<Event>,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "@xlink:href")]
    #[serde(alias = "@href")]
    pub href: Option<String>,
    #[serde(rename = "@xlink:actuate", alias = "@actuate")]
    pub actuate: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct EssentialProperty {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct SupplementalProperty {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename(serialize = "scte214:ContentIdentifier"))]
    #[serde(rename(deserialize = "ContentIdentifier"))]
    pub scte214ContentIdentifiers: Vec<Scte214ContentIdentifier>,
}

/// Provides a textual description of the content, which can be used by the client to allow
/// selection of the desired media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Label {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@lang")]
    pub lang: Option<String>,
    #[serde(rename = "$text")]
    pub content: String,
}

/// Contains a set of Representations.
///
/// For example, if multiple language streams are available for the audio content, each one can be
/// in its own AdaptationSet. DASH implementation guidelines indicate that "representations in the
/// same video adaptation set should be alternative encodings of the same source content, encoded
/// such that switching between them does not produce visual glitches due to picture size or aspect
/// ratio differences".
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct AdaptationSet {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "@xlink:href", alias = "@href")]
    pub href: Option<String>,
    #[serde(rename = "@xlink:actuate", alias = "@actuate", default = "default_optstring_on_request")]
    pub actuate: Option<String>,
    #[serde(rename = "@group")]
    pub group: Option<i64>,
    #[serde(rename = "@selectionPriority")]
    pub selectionPriority: Option<u64>,
    // e.g. "audio", "video", "text"
    #[serde(rename = "@contentType")]
    pub contentType: Option<String>,
    #[serde(rename = "@profiles")]
    pub profiles: Option<String>,
    /// Content language, in RFC 5646 format.
    #[serde(rename = "@lang")]
    pub lang: Option<String>,
    /// The Sample Aspect Ratio, eg. "1:1".
    #[serde(rename = "@sar")]
    pub sar: Option<String>,
    /// The Pixel Aspect Ratio, eg. "16:9".
    #[serde(rename = "@par")]
    pub par: Option<String>,
    /// If present, this attribute is expected to be set to "progressive".
    #[serde(rename = "@scanType")]
    pub scanType: Option<String>,
    #[serde(rename = "@segmentAlignment")]
    pub segmentAlignment: Option<bool>,
    #[serde(rename = "@segmentProfiles")]
    /// Specifies the profiles of Segments that are essential to process the Representation. The
    /// semantics depend on the value of the @mimeType attribute.
    pub segmentProfiles: Option<String>,
    #[serde(rename = "@subsegmentAlignment")]
    pub subsegmentAlignment: Option<bool>,
    #[serde(rename = "@subsegmentStartsWithSAP")]
    pub subsegmentStartsWithSAP: Option<u64>,
    #[serde(rename = "@bitstreamSwitching")]
    pub bitstreamSwitching: Option<bool>,
    #[serde(rename = "@audioSamplingRate")]
    pub audioSamplingRate: Option<String>,
    #[serde(rename = "@width")]
    pub width: Option<u64>,
    #[serde(rename = "@height")]
    pub height: Option<u64>,
    // eg "video/mp4"
    #[serde(rename = "@mimeType")]
    pub mimeType: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381> (eg. "avc1.4D400C").
    #[serde(rename = "@codecs")]
    pub codecs: Option<String>,
    #[serde(rename = "@minBandwidth")]
    pub minBandwidth: Option<u64>,
    #[serde(rename = "@maxBandwidth")]
    pub maxBandwidth: Option<u64>,
    #[serde(rename = "@minWidth")]
    pub minWidth: Option<u64>,
    #[serde(rename = "@maxWidth")]
    pub maxWidth: Option<u64>,
    #[serde(rename = "@minHeight")]
    pub minHeight: Option<u64>,
    #[serde(rename = "@maxHeight")]
    pub maxHeight: Option<u64>,
    #[serde(rename = "@frameRate")]
    pub frameRate: Option<String>, // it can be something like "15/2"
    #[serde(rename = "@maxFrameRate")]
    pub maxFrameRate: Option<String>, // it can be something like "15/2"
    /// Indicates the possibility for accelerated playout allowed by this codec profile and level.
    #[serde(rename = "@maxPlayoutRate", serialize_with="serialize_opt_xsd_double")]
    pub maxPlayoutRate: Option<f64>,
    #[serde(rename = "@maximumSAPPeriod", serialize_with="serialize_opt_xsd_double")]
    pub maximumSAPPeriod: Option<f64>,
    #[serde(rename = "@startWithSAP")]
    pub startWithSAP: Option<u64>,
    #[serde(rename = "@codingDependency")]
    pub codingDependency: Option<bool>,
    pub FramePacking: Vec<FramePacking>,
    pub AudioChannelConfiguration: Vec<AudioChannelConfiguration>,
    pub ContentProtection: Vec<ContentProtection>,
    // TODO OutputProtection element
    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,
    pub InbandEventStream: Vec<InbandEventStream>,
    pub Switching: Vec<Switching>,
    // TODO RandomAccess element
    pub GroupLabel: Vec<Label>,
    pub Label: Vec<Label>,
    pub ProducerReferenceTime: Vec<ProducerReferenceTime>,
    // TODO ContentPopularityRate element
    pub Resync: Vec<Resync>,
    pub Accessibility: Vec<Accessibility>,
    pub Role: Vec<Role>,
    pub Rating: Vec<Rating>,
    pub Viewpoint: Vec<Viewpoint>,
    pub ContentComponent: Vec<ContentComponent>,
    pub BaseURL: Vec<BaseURL>,
    pub SegmentBase: Option<SegmentBase>,
    pub SegmentList: Option<SegmentList>,
    pub SegmentTemplate: Option<SegmentTemplate>,
    #[serde(rename = "Representation")]
    pub representations: Vec<Representation>,
    #[serde(rename = "@scte214:supplementalProfiles", alias = "@supplementalProfiles")]
    pub scte214_supplemental_profiles: Option<String>,
    #[serde(rename = "@scte214:supplementalCodecs", alias = "@supplementalCodecs")]
    pub scte214_supplemental_codecs: Option<String>,
}

/// Identifies the asset to which a given Period belongs.
///
/// Can be used to implement client functionality that depends on distinguishing between ads and
/// main content.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct AssetIdentifier {
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename(serialize = "scte214:ContentIdentifier"))]
    #[serde(rename(deserialize = "ContentIdentifier"))]
    pub scte214ContentIdentifiers: Vec<Scte214ContentIdentifier>,
}

/// Subsets provide a mechanism to restrict the combination of active Adaptation Sets.
///
/// An active Adaptation Set is one for which the DASH Client is presenting at least one of the
/// contained Representations.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Subset {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    /// Specifies the AdaptationSets contained in a Subset by providing a whitespace separated
    /// list of the @id values of the contained AdaptationSets.
    #[serde(rename = "@contains",
            deserialize_with = "deserialize_xsd_uintvector",
            serialize_with = "serialize_xsd_uintvector",
            default)]
    pub contains: Vec<u64>,
}

/// Describes a chunk of the content with a start time and a duration. Content can be split up into
/// multiple periods (such as chapters, advertising segments).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Period {
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "@xlink:href", alias = "@href")]
    pub href: Option<String>,

    #[serde(rename = "@xlink:actuate", alias = "@actuate", default="default_optstring_on_request")]
    pub actuate: Option<String>,

    #[serde(rename = "@id")]
    pub id: Option<String>,

    /// The start time of the Period relative to the MPD availability start time.
    #[serde(rename = "@start",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub start: Option<Duration>,

    // note: the spec says that this is an xs:duration, not an unsigned int as for other "duration" fields
    #[serde(rename = "@duration",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub duration: Option<Duration>,

    // The default for the bitstreamSwitching attribute is specified to be "false".
    #[serde(rename = "@bitstreamSwitching", default)]
    pub bitstreamSwitching: Option<bool>,

    pub BaseURL: Vec<BaseURL>,

    pub SegmentBase: Option<SegmentBase>,

    pub SegmentList: Option<SegmentList>,

    pub SegmentTemplate: Option<SegmentTemplate>,

    #[serde(rename = "AssetIdentifier")]
    pub asset_identifier: Option<AssetIdentifier>,

    #[serde(rename = "EventStream")]
    pub event_streams: Vec<EventStream>,

    #[serde(rename = "ServiceDescription")]
    pub service_description: Vec<ServiceDescription>,

    pub ContentProtection: Vec<ContentProtection>,

    #[serde(rename = "AdaptationSet")]
    pub adaptations: Vec<AdaptationSet>,

    #[serde(rename = "Subset")]
    pub subsets: Vec<Subset>,

    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,

    #[serde(rename = "EmptyAdaptationSet")]
    pub empty_adaptations: Vec<AdaptationSet>,

    #[serde(rename = "GroupLabel")]
    pub group_label: Vec<Label>,

    #[serde(rename = "Preselection")]
    pub pre_selections: Vec<Preselection>,

    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Reporting {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
    #[serde(rename = "@dvb:reportingUrl", alias = "@reportingUrl")]
    pub reportingUrl: Option<String>,
    #[serde(rename = "@dvb:probability", alias = "@probability")]
    pub probability: Option<u64>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Range {
    #[serde(rename = "@starttime",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub starttime: Option<Duration>,
    #[serde(rename = "@duration",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub duration: Option<Duration>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct Metrics {
    #[serde(rename = "@metrics")]
    pub metrics: String,
    pub Reporting: Vec<Reporting>,
    pub Range: Vec<Range>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Latency {
    #[serde(rename = "@min", serialize_with="serialize_opt_xsd_double")]
    pub min: Option<f64>,
    #[serde(rename = "@max", serialize_with="serialize_opt_xsd_double")]
    pub max: Option<f64>,
    #[serde(rename = "@target", serialize_with="serialize_opt_xsd_double")]
    pub target: Option<f64>,
    #[serde(rename = "@referenceId")]
    pub referenceId: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct PlaybackRate {
    #[serde(rename = "@min", serialize_with="serialize_xsd_double")]
    pub min: f64,
    #[serde(rename = "@max", serialize_with="serialize_xsd_double")]
    pub max: f64,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct ServiceDescription {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    pub Latency: Option<Latency>,
    pub PlaybackRate: Option<PlaybackRate>,
    #[serde(rename = "Scope")]
    pub scopes: Vec<Scope>,
}

/// Used to synchronize the clocks of the DASH client and server, to allow low-latency streaming.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct UTCTiming {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    // prefixed with urn:mpeg:dash:utc, one of http-xsdate:2014, http-iso:2014,
    // http-ntp:2014, ntp:2014, http-head:2014, direct:2014
    #[serde(rename = "@schemeIdUri")]
    pub schemeIdUri: String,
    #[serde(rename = "@value")]
    pub value: Option<String>,
}

/// Specifies wall‐clock times at which media fragments were produced.
///
/// This information helps clients consume the fragments at the same rate at which they were
/// produced. Used by the low-latency streaming extensions to DASH.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct ProducerReferenceTime {
    // This attribute is required according to the specification XSD.
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@inband", default = "default_optbool_false")]
    pub inband: Option<bool>,
    // This attribute is required according to the specification XSD.
    #[serde(rename = "@presentationTime")]
    pub presentationTime: Option<u64>,
    #[serde(rename = "@type", default = "default_optstring_encoder")]
    pub prtType: Option<String>,
    // There are two capitalizations for this attribute in the specification at
    // https://dashif.org/docs/CR-Low-Latency-Live-r8.pdf. The attribute is required according to
    // the specification XSD.
    #[serde(rename = "@wallClockTime",
            alias="@wallclockTime",
            deserialize_with = "deserialize_xs_datetime",
            default)]
    pub wallClockTime: Option<XsDatetime>,
    pub UTCTiming: Option<UTCTiming>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Hash)]
#[serde(default)]
pub struct LeapSecondInformation {
    #[serde(rename = "@availabilityStartLeapOffset")]
    pub availabilityStartLeapOffset: Option<i64>,
    #[serde(rename = "@nextAvailabilityStartLeapOffset")]
    pub nextAvailabilityStartLeapOffset: Option<i64>,
    #[serde(rename = "@nextLeapChangeTime",
            deserialize_with = "deserialize_xs_datetime",
            default)]
    pub nextLeapChangeTime: Option<XsDatetime>,
}

/// The Patch mechanism allows the DASH client to retrieve a set of instructions for replacing
/// certain parts of the MPD manifest with updated information.
///
/// It is a bandwidth-friendly alternative to retrieving a new version of the full MPD manifest. The
/// MPD patch document is guaranteed to be available between MPD@publishTime and MPD@publishTime +
/// PatchLocation@ttl.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct PatchLocation {
    #[serde(rename = "@ttl", serialize_with="serialize_opt_xsd_double")]
    pub ttl: Option<f64>,
    #[serde(rename = "$text")]
    pub content: String,
}

/// The root node of a parsed DASH MPD manifest.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct MPD {
    #[serde(rename = "@xmlns", serialize_with="serialize_xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
    #[serde(rename = "@profiles")]
    pub profiles: Option<String>,
    /// The Presentation Type, either "static" or "dynamic" (a live stream for which segments become
    /// available over time).
    #[serde(rename = "@type")]
    pub mpdtype: Option<String>,
    #[serde(rename = "@availabilityStartTime",
            deserialize_with = "deserialize_xs_datetime",
            default)]
    pub availabilityStartTime: Option<XsDatetime>,
    #[serde(rename = "@availabilityEndTime",
            deserialize_with = "deserialize_xs_datetime",
            default)]
    pub availabilityEndTime: Option<XsDatetime>,
    #[serde(rename = "@publishTime",
            deserialize_with = "deserialize_xs_datetime",
            default)]
    pub publishTime: Option<XsDatetime>,
    #[serde(rename = "@mediaPresentationDuration",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub mediaPresentationDuration: Option<Duration>,
    #[serde(rename = "@minimumUpdatePeriod",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub minimumUpdatePeriod: Option<Duration>,
    // This attribute is actually required by the XSD specification, but we make it optional.
    #[serde(rename = "@minBufferTime",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub minBufferTime: Option<Duration>,
    /// Prescribes how many seconds of buffer a client should keep to avoid stalling when streaming
    /// under ideal network conditions with bandwidth matching the @bandwidth attribute.
    #[serde(rename = "@timeShiftBufferDepth",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub timeShiftBufferDepth: Option<Duration>,
    /// A suggested delay of the presentation compared to the Live edge.
    #[serde(rename = "@suggestedPresentationDelay",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub suggestedPresentationDelay: Option<Duration>,
    #[serde(rename = "@maxSegmentDuration",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub maxSegmentDuration: Option<Duration>,
    #[serde(rename = "@maxSubsegmentDuration",
            serialize_with = "serialize_xs_duration",
            deserialize_with = "deserialize_xs_duration",
            default)]
    pub maxSubsegmentDuration: Option<Duration>,
    /// The XML namespace prefix used by convention for the XML Schema Instance namespace.
    #[serialize_always]
    #[serde(rename="@xmlns:xsi", alias="@xsi", serialize_with="serialize_xsi_ns")]
    pub xsi: Option<String>,
    #[serde(alias = "@ext", rename = "@xmlns:ext")]
    pub ext: Option<String>,
    /// The XML namespace prefix used by convention for the Common Encryption scheme.
    #[serialize_always]
    #[serde(rename="@xmlns:cenc", alias="@cenc", serialize_with="serialize_cenc_ns")]
    pub cenc: Option<String>,
    /// The XML namespace prefix used by convention for the Microsoft PlayReady scheme.
    #[serialize_always]
    #[serde(rename="@xmlns:mspr", alias="@mspr", serialize_with="serialize_mspr_ns")]
    pub mspr: Option<String>,
    /// The XML namespace prefix used by convention for the XML Linking Language.
    #[serialize_always]
    #[serde(rename="@xmlns:xlink", alias="@xlink", serialize_with="serialize_xlink_ns")]
    pub xlink: Option<String>,
    /// The XML namespace prefix used by convention for the “Digital Program Insertion Cueing
    /// Message for Cable” (SCTE 35) signaling standard.
    #[cfg(feature = "scte35")]
    #[serialize_always]
    #[serde(rename="@xmlns:scte35", alias="@scte35", serialize_with="scte35::serialize_scte35_ns")]
    pub scte35: Option<String>,
    /// The XML namespace prefix used by convention for DASH extensions proposed by the Digital
    /// Video Broadcasting Project, as per RFC 5328.
    #[serialize_always]
    #[serde(rename="@xmlns:dvb", alias="@dvb", serialize_with="serialize_dvb_ns")]
    pub dvb: Option<String>,
    #[serde(rename = "@xsi:schemaLocation", alias = "@schemaLocation")]
    pub schemaLocation: Option<String>,
    // scte214 namespace
    #[serde(alias = "@scte214", rename = "@xmlns:scte214")]
    pub scte214: Option<String>,
    pub ProgramInformation: Vec<ProgramInformation>,
    /// There may be several BaseURLs, for redundancy (for example multiple CDNs)
    #[serde(rename = "BaseURL")]
    pub base_url: Vec<BaseURL>,
    #[serde(rename = "Location", default)]
    pub locations: Vec<Location>,
    /// Specifies the location of an MPD “patch document”, a set of instructions for replacing
    /// certain parts of the MPD manifest with updated information.
    pub PatchLocation: Vec<PatchLocation>,
    pub ServiceDescription: Vec<ServiceDescription>,
    // TODO: elements InitializationSet, InitializationGroup, InitializationPresentation
    pub ContentProtection: Vec<ContentProtection>,
    #[serde(rename = "Period", default)]
    pub periods: Vec<Period>,
    pub Metrics: Vec<Metrics>,
    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,
    pub UTCTiming: Vec<UTCTiming>,
    /// Correction for leap seconds, used by the DASH Low Latency specification.
    pub LeapSecondInformation: Option<LeapSecondInformation>,
}

impl std::fmt::Display for MPD {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", quick_xml::se::to_string(self).map_err(|_| std::fmt::Error)?)
    }
}

/// Parse an MPD manifest, provided as an XML string, returning an `MPD` node.
pub fn parse(xml: &str) -> Result<MPD, DashMpdError> {
    #[cfg(feature = "warn_ignored_elements")]
    {
        let xd = &mut quick_xml::de::Deserializer::from_str(xml);
        let _: MPD = serde_ignored::deserialize(xd, |path| {
            warn!("Unused XML element in manifest: {path}");
        }).map_err(|e| DashMpdError::Parsing(e.to_string()))?;
    }
    let xd = &mut quick_xml::de::Deserializer::from_str(xml);
    let mpd: MPD = serde_path_to_error::deserialize(xd)
        .map_err(|e| DashMpdError::Parsing(e.to_string()))?;
    Ok(mpd)
}


// Note that a codec name can be of the form "mp4a" or "mp4a.40.2".
fn is_audio_codec(name: &str) -> bool {
    name.starts_with("mp4a") ||
        name.starts_with("aac") ||
        name.starts_with("vorbis") ||
        name.starts_with("opus") ||
        name.starts_with("flac") ||
        name.starts_with("mp3") ||
        name.starts_with("ec-3") ||
        name.starts_with("ac-4") ||
        name.starts_with("dtsc") ||
        name.starts_with("mha1")       // MPEG-H 3D Audio
}


/// Returns `true` if this AdaptationSet contains audio content.
///
/// It contains audio if the codec attribute corresponds to a known audio codec, or the
/// `contentType` attribute` is `audio`, or the `mimeType` attribute is `audio/*`, or if one of its
/// child `Representation` nodes has an audio `contentType` or `mimeType` attribute.
pub fn is_audio_adaptation(a: &&AdaptationSet) -> bool {
    if let Some(codec) = &a.codecs {
        if is_audio_codec(codec) {
            return true;
        }
    }
    if let Some(ct) = &a.contentType {
        if ct == "audio" {
            return true;
        }
    }
    if let Some(mimetype) = &a.mimeType {
        if mimetype.starts_with("audio/") {
            return true;
        }
    }
    for r in a.representations.iter() {
        if let Some(ct) = &r.contentType {
            if ct == "audio" {
                return true;
            }
        }
        if let Some(mimetype) = &r.mimeType {
            if mimetype.starts_with("audio/") {
                return true;
            }
        }
    }
    false
}

/// Returns `true` if this AdaptationSet contains video content.
///
/// It contains video if the `contentType` attribute` is `video`, or the `mimeType` attribute is
/// `video/*` (but without a codec specifying a subtitle format), or if one of its child
/// `Representation` nodes has an audio `contentType` or `mimeType` attribute.
///
/// Note: if it's an audio adaptation then it's not a video adaptation (an audio adaptation means
/// audio-only), but a video adaptation may contain audio.
pub fn is_video_adaptation(a: &&AdaptationSet) -> bool {
    if is_audio_adaptation(a) {
        return false;
    }
    if let Some(ct) = &a.contentType {
        if ct == "video" {
            return true;
        }
    }
    if let Some(mimetype) = &a.mimeType {
        if mimetype.starts_with("video/") {
            return true;
        }
    }
    for r in a.representations.iter() {
        if let Some(ct) = &r.contentType {
            if ct == "video" {
                return true;
            }
        }
        // We can have a Representation with mimeType="video/mp4" and codecs="wvtt", which means
        // WebVTT in a (possibly fragmented) MP4 container.
        if r.codecs.as_deref().is_some_and(is_subtitle_codec) {
            return false;
        }
        if let Some(mimetype) = &r.mimeType {
            if mimetype.starts_with("video/") {
                return true;
            }
        }
    }
    false
}


fn is_subtitle_mimetype(mt: &str) -> bool {
    mt.eq("text/vtt") ||
    mt.eq("application/ttml+xml") ||
    mt.eq("application/x-sami")

    // Some manifests use a @mimeType of "application/mp4" together with @contentType="text"; we'll
    // classify these only based on their contentType.
}

fn is_subtitle_codec(c: &str) -> bool {
    c == "wvtt" ||
    c == "c608" ||
    c == "stpp" ||
    c == "tx3g" ||
    c.starts_with("stpp.")
}

/// Returns `true` if this AdaptationSet contains subtitle content.
///
/// For now, it contains subtitles if the `@mimeType` attribute is "text/vtt" (WebVTT) or
/// "application/ttml+xml" or "application/x-sami" (SAMI). Further work needed to handle an
/// Adaptation that contains a Representation with @contentType="text" and @codecs="stpp" or a
/// subset like @codecs="stpp.ttml.im1t" (fragmented TTML in an MP4 container) or @codecs="wvtt"
/// (fragmented VTTcue in an MP4 container).
///
/// The DVB-DASH specification also allows for closed captions for hearing impaired viewers in an
/// AdaptationSet with Accessibility node having @SchemeIdUri =
/// "urn:tva:metadata:cs:AudioPurposeCS:2007" and @value=2.
pub fn is_subtitle_adaptation(a: &&AdaptationSet) -> bool {
    if a.mimeType.as_deref().is_some_and(is_subtitle_mimetype) {
        return true;
    }
    if a.contentType.as_deref().is_some_and(|ct| ct.eq("text")) {
        return true;
    }
    if a.codecs.as_deref().is_some_and(is_subtitle_codec) {
        return true;
    }
    for cc in a.ContentComponent.iter() {
        if cc.contentType.as_deref().is_some_and(|ct| ct.eq("text")) {
            return true;
        }
    }
    for r in a.representations.iter() {
        if r.mimeType.as_deref().is_some_and(is_subtitle_mimetype) {
            return true;
        }
        // Often, but now always, the subtitle codec is also accompanied by a contentType of "text".
        if r.codecs.as_deref().is_some_and(is_subtitle_codec) {
            return true;
        }
    }
    false
}


// Incomplete, see https://en.wikipedia.org/wiki/Subtitles#Subtitle_formats
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SubtitleType {
    /// W3C WebVTT, as used in particular for HTML5 media
    Vtt,
    /// SubRip
    Srt,
    /// MPSub
    Sub,
    /// Advanced Substation Alpha
    Ass,
    /// MPEG-4 Timed Text, aka MP4TT aka 3GPP-TT (codec=tx3g)
    Ttxt,
    /// Timed Text Markup Language
    Ttml,
    /// Synchronized Accessible Media Interchange
    Sami,
    /// Binary WebVTT in a wvtt box in fragmented MP4 container, as specified by ISO/IEC
    /// 14496-30:2014. Mostly intended for live streams where it's not possible to provide a
    /// standalone VTT file.
    Wvtt,
    /// XML content (generally TTML) in an stpp box in fragmented MP4 container
    Stpp,
    /// EIA-608 aka CEA-608, a legacy standard for closed captioning for NTSC TV
    Eia608,
    Unknown,
}

fn subtitle_type_for_mimetype(mt: &str) -> Option<SubtitleType> {
    match mt {
        "text/vtt" => Some(SubtitleType::Vtt),
        "application/ttml+xml" => Some(SubtitleType::Ttml),
        "application/x-sami" => Some(SubtitleType::Sami),
        _ => None
    }
}

pub fn subtitle_type(a: &&AdaptationSet) -> SubtitleType {
    if let Some(mimetype) = &a.mimeType {
        if let Some(st) = subtitle_type_for_mimetype(mimetype) {
            return st;
        }
    }
    if let Some(codecs) = &a.codecs {
        if codecs == "wvtt" {
            // can be extracted with https://github.com/xhlove/dash-subtitle-extractor
            return SubtitleType::Wvtt;
        }
        if codecs == "c608" {
            return SubtitleType::Eia608;
        }
        if codecs == "tx3g" {
            return SubtitleType::Ttxt;
        }
        if codecs == "stpp" {
            return SubtitleType::Stpp;
        }
        if codecs.starts_with("stpp.") {
            return SubtitleType::Stpp;
        }
    }
    for r in a.representations.iter() {
        if let Some(mimetype) = &r.mimeType {
            if let Some(st) = subtitle_type_for_mimetype(mimetype) {
                return st;
            }
        }
        if let Some(codecs) = &r.codecs {
            if codecs == "wvtt" {
                return SubtitleType::Wvtt;
            }
            if codecs == "c608" {
                return SubtitleType::Eia608;
            }
            if codecs == "tx3g" {
                return SubtitleType::Ttxt;
            }
            if codecs == "stpp" {
                return SubtitleType::Stpp;
            }
            if codecs.starts_with("stpp.") {
                return SubtitleType::Stpp;
            }
        }
    }
    SubtitleType::Unknown
}


#[allow(dead_code)]
fn content_protection_type(cp: &ContentProtection) -> String {
    if let Some(v) = &cp.value {
        if v.eq("cenc") {
            return String::from("cenc");
        }
        if v.eq("Widevine") {
            return String::from("Widevine");
        }
        if v.eq("MSPR 2.0") {
            return String::from("PlayReady");
        }
    }
    // See list at https://dashif.org/identifiers/content_protection/
    let uri = &cp.schemeIdUri;
    let uri = uri.to_lowercase();
    if uri.eq("urn:mpeg:dash:mp4protection:2011") {
        return String::from("cenc");
    }
    if uri.eq("urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed") {
        return String::from("Widevine");
    }
    if uri.eq("urn:uuid:9a04f079-9840-4286-ab92-e65be0885f95") {
        return String::from("PlayReady");
    }
    if uri.eq("urn:uuid:94ce86fb-07ff-4f43-adb8-93d2fa968ca2") {
        return String::from("FairPlay");
    }
    if uri.eq("urn:uuid:3ea8778f-7742-4bf9-b18b-e834b2acbd47") {
        return String::from("Clear Key AES-128");
    }
    if uri.eq("urn:uuid:be58615b-19c4-4684-88b3-c8c57e99e957") {
        return String::from("Clear Key SAMPLE-AES");
    }
    if uri.eq("urn:uuid:adb41c24-2dbf-4a6d-958b-4457c0d27b95") {
        return String::from("Nagra");
    }
    if uri.eq("urn:uuid:5e629af5-38da-4063-8977-97ffbd9902d4") {
        return String::from("Marlin");
    }
    if uri.eq("urn:uuid:f239e769-efa3-4850-9c16-a903c6932efb") {
        return String::from("Adobe PrimeTime");
    }
    if uri.eq("urn:uuid:1077efec-c0b2-4d02-ace3-3c1e52e2fb4b") {
        return String::from("W3C Common PSSH box");
    }
    if uri.eq("urn:uuid:80a6be7e-1448-4c37-9e70-d5aebe04c8d2") {
        return String::from("Irdeto Content Protection");
    }
    if uri.eq("urn:uuid:3d5e6d35-9b9a-41e8-b843-dd3c6e72c42c") {
        return String::from("WisePlay-ChinaDRM");
    }
    if uri.eq("urn:uuid:616c7469-6361-7374-2d50-726f74656374") {
        return String::from("Alticast");
    }
    if uri.eq("urn:uuid:6dd8b3c3-45f4-4a68-bf3a-64168d01a4a6") {
        return String::from("ABV DRM");
    }
    // Segment encryption
    if uri.eq("urn:mpeg:dash:sea:2012") {
        return String::from("SEA");
    }
    String::from("<unknown>")
}


fn check_segment_template_duration(
    st: &SegmentTemplate,
    max_seg_duration: &Duration,
    outer_timescale: u64) -> Vec<String>
{
    let mut errors = Vec::new();
    if let Some(timeline) = &st.SegmentTimeline {
        for s in &timeline.segments {
            let sd = s.d / st.timescale.unwrap_or(outer_timescale);
            if sd > max_seg_duration.as_secs() {
                errors.push(String::from("SegmentTimeline has segment@d > @maxSegmentDuration"));
            }
        }
    }
    errors
}

fn check_segment_template_conformity(st: &SegmentTemplate) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(md) = &st.media {
        if !valid_url_p(md) {
            errors.push(format!("invalid URL {md}"));
        }
        if md.contains("$Number$") && md.contains("$Time") {
            errors.push(String::from("both $Number$ and $Time$ are used in media template URL"));
        }
    }
    if let Some(init) = &st.initialization {
        if !valid_url_p(init) {
            errors.push(format!("invalid URL {init}"));
        }
        if init.contains("$Number") {
            errors.push(String::from("$Number$ identifier used in initialization segment URL"));
        }
        if init.contains("$Time") {
            errors.push(String::from("$Time$ identifier used in initialization segment URL"));
        }
    }
    if st.duration.is_some() && st.SegmentTimeline.is_some() {
        errors.push(String::from("both SegmentTemplate.duration and SegmentTemplate.SegmentTimeline present"));
    }
    errors
}


// Check the URL or URL path u for conformity. This is a very relaxed check because the Url crate is
// very tolerant, in particular concerning the syntax accepted for the path component of an URL.
fn valid_url_p(u: &str) -> bool {
    use url::ParseError;

    match Url::parse(u) {
        Ok(url) => {
            url.scheme() == "https" ||
                url.scheme() == "http" ||
                url.scheme() == "ftp" ||
                url.scheme() == "file" ||
                url.scheme() == "data"
        },
        Err(ParseError::RelativeUrlWithoutBase) => true,
        Err(_) => false,
    }
}

/// Returns a list of DASH conformity errors in the DASH manifest mpd.
pub fn check_conformity(mpd: &MPD) -> Vec<String> {
    let mut errors = Vec::new();

    // @maxHeight on the AdaptationSet should give the maximum value of the @height values of its
    // Representation elements.
    for p in &mpd.periods {
        if p.adaptations.is_empty() {
            errors.push(format!("Period with @id {} contains no AdaptationSet elements",
                                p.id.clone().unwrap_or(String::from("<unspecified>"))));
        }
        for a in &p.adaptations {
            if let Some(mh) = a.maxHeight {
                if let Some(mr) = a.representations.iter().max_by_key(|r| r.height.unwrap_or(0)) {
                    if mr.height.unwrap_or(0) > mh {
                        errors.push(String::from("invalid @maxHeight on AdaptationSet"));
                    }
                }
            }
        }
    }
    // @maxWidth on the AdaptationSet should give the maximum value of the @width values of its
    // Representation elements.
    for p in &mpd.periods {
        for a in &p.adaptations {
            if let Some(mw) = a.maxWidth {
                if let Some(mr) = a.representations.iter().max_by_key(|r| r.width.unwrap_or(0)) {
                    if mr.width.unwrap_or(0) > mw {
                        errors.push(String::from("invalid @maxWidth on AdaptationSet"));
                    }
                }
            }
        }
    }
    // @maxBandwidth on the AdaptationSet should give the maximum value of the @bandwidth values of its
    // Representation elements.
    for p in &mpd.periods {
        for a in &p.adaptations {
            if let Some(mb) = a.maxBandwidth {
                if let Some(mr) = a.representations.iter().max_by_key(|r| r.bandwidth.unwrap_or(0)) {
                    if mr.bandwidth.unwrap_or(0) > mb {
                        errors.push(String::from("invalid @maxBandwidth on AdaptationSet"));
                    }
                }
            }
        }
    }
    // No @d of a segment should be greater than @maxSegmentDuration.
    if let Some(max_seg_duration) = mpd.maxSegmentDuration {
        for p in &mpd.periods {
            for a in &p.adaptations {
                // We need to keep track of outer_timescale for situations with a nested SegmentTemplate.
                // For an example see test/fixtures/aws.xml.
                // <SegmentTemplate startNumber="1" timescale="90000"/>
                //   <Representation bandwidth="3296000" ...>
                //     <SegmentTemplate initialization="i.mp4" media="m$Number$.mp4">
                //       <SegmentTimeline>
                //         <S d="180000" r="6" t="0"/>
                //       </SegmentTimeline>
                //     </SegmentTemplate>
                // ...
                let mut outer_timescale = 1;
                if let Some(st) = &a.SegmentTemplate {
                    check_segment_template_duration(st, &max_seg_duration, outer_timescale)
                        .into_iter()
                        .for_each(|msg| errors.push(msg));
                    if let Some(ots) = st.timescale {
                        outer_timescale = ots;
                    }
                }
                for r in &a.representations {
                    if let Some(st) = &r.SegmentTemplate {
                        check_segment_template_duration(st, &max_seg_duration, outer_timescale)
                            .into_iter()
                            .for_each(|msg| errors.push(msg));
                    }
                }
            }
        }
    }

    for bu in &mpd.base_url {
        if !valid_url_p(&bu.base) {
            errors.push(format!("invalid URL {}", &bu.base));
        }
    }
    for p in &mpd.periods {
        for bu in &p.BaseURL {
            if !valid_url_p(&bu.base) {
                errors.push(format!("invalid URL {}", &bu.base));
            }
        }
        for a in &p.adaptations {
            for bu in &a.BaseURL {
                if !valid_url_p(&bu.base) {
                    errors.push(format!("invalid URL {}", &bu.base));
                }
            }
            if let Some(st) = &a.SegmentTemplate {
                check_segment_template_conformity(st)
                    .into_iter()
                    .for_each(|msg| errors.push(msg));
            }
            for r in &a.representations {
                for bu in &r.BaseURL {
                    if !valid_url_p(&bu.base) {
                        errors.push(format!("invalid URL {}", &bu.base));
                    }
                }
                if let Some(sb) = &r.SegmentBase {
                    if let Some(init) = &sb.Initialization {
                        if let Some(su) = &init.sourceURL {
                            if !valid_url_p(su) {
                                errors.push(format!("invalid URL {su}"));
                            }
                            if su.contains("$Number") {
                                errors.push(String::from("$Number$ identifier used in initialization segment URL"));
                            }
                            if su.contains("$Time") {
                                errors.push(String::from("$Time$ identifier used in initialization segment URL"));
                            }
                        }
                    }
                    if let Some(ri) = &sb.representation_index {
                        if let Some(su) = &ri.sourceURL {
                            if !valid_url_p(su) {
                                errors.push(format!("invalid URL {su}"));
                            }
                        }
                    }
                }
                if let Some(sl) = &r.SegmentList {
                    if let Some(hr) = &sl.href {
                        if !valid_url_p(hr) {
                            errors.push(format!("invalid URL {hr}"));
                        }
                    }
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            if !valid_url_p(su) {
                                errors.push(format!("invalid URL {su}"));
                            }
                            if su.contains("$Number") {
                                errors.push(String::from("$Number$ identifier used in initialization segment URL"));
                            }
                            if su.contains("$Time") {
                                errors.push(String::from("$Time$ identifier used in initialization segment URL"));
                            }
                        }
                    }
                    for su in &sl.segment_urls {
                        if let Some(md) = &su.media {
                            if !valid_url_p(md) {
                                errors.push(format!("invalid URL {md}"));
                            }
                        }
                        if let Some(ix) = &su.index {
                            if !valid_url_p(ix) {
                                errors.push(format!("invalid URL {ix}"));
                            }
                        }
                    }
                }
                if let Some(st) = &r.SegmentTemplate {
                    check_segment_template_conformity(st)
                        .into_iter()
                        .for_each(|msg| errors.push(msg));
                }
            }
        }
    }
    for pi in &mpd.ProgramInformation {
        if let Some(u) = &pi.moreInformationURL {
            if !valid_url_p(u) {
                errors.push(format!("invalid URL {u}"));
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn doesnt_crash(s in "\\PC*") {
            let _ = super::parse_xs_duration(&s);
            let _ = super::parse_xs_datetime(&s);
        }
    }

    #[test]
    fn test_parse_xs_duration() {
        use std::time::Duration;
        use super::parse_xs_duration;

        assert!(parse_xs_duration("").is_err());
        assert!(parse_xs_duration("foobles").is_err());
        assert!(parse_xs_duration("P").is_err());
        assert!(parse_xs_duration("PW").is_err());
        // assert!(parse_xs_duration("PT-4.5S").is_err());
        assert!(parse_xs_duration("-PT4.5S").is_err());
        assert!(parse_xs_duration("1Y2M3DT4H5M6S").is_err()); // missing initial P
        assert_eq!(parse_xs_duration("PT3H11M53S").ok(), Some(Duration::new(11513, 0)));
        assert_eq!(parse_xs_duration("PT42M30S").ok(), Some(Duration::new(2550, 0)));
        assert_eq!(parse_xs_duration("PT30M38S").ok(), Some(Duration::new(1838, 0)));
        assert_eq!(parse_xs_duration("PT0H10M0.00S").ok(), Some(Duration::new(600, 0)));
        assert_eq!(parse_xs_duration("PT1.5S").ok(), Some(Duration::new(1, 500_000_000)));
        assert_eq!(parse_xs_duration("PT1.500S").ok(), Some(Duration::new(1, 500_000_000)));
        assert_eq!(parse_xs_duration("PT1.500000000S").ok(), Some(Duration::new(1, 500_000_000)));
        assert_eq!(parse_xs_duration("PT0S").ok(), Some(Duration::new(0, 0)));
        assert_eq!(parse_xs_duration("PT0.001S").ok(), Some(Duration::new(0, 1_000_000)));
        assert_eq!(parse_xs_duration("PT0.00100S").ok(), Some(Duration::new(0, 1_000_000)));
        assert_eq!(parse_xs_duration("PT344S").ok(), Some(Duration::new(344, 0)));
        assert_eq!(parse_xs_duration("PT634.566S").ok(), Some(Duration::new(634, 566_000_000)));
        assert_eq!(parse_xs_duration("PT72H").ok(), Some(Duration::new(72*60*60, 0)));
        assert_eq!(parse_xs_duration("PT0H0M30.030S").ok(), Some(Duration::new(30, 30_000_000)));
        assert_eq!(parse_xs_duration("PT1004199059S").ok(), Some(Duration::new(1004199059, 0)));
        assert_eq!(parse_xs_duration("P0Y20M0D").ok(), Some(Duration::new(51840000, 0)));
        assert_eq!(parse_xs_duration("PT1M30.5S").ok(), Some(Duration::new(90, 500_000_000)));
        assert_eq!(parse_xs_duration("PT10M10S").ok(), Some(Duration::new(610, 0)));
        assert_eq!(parse_xs_duration("PT1H0.040S").ok(), Some(Duration::new(3600, 40_000_000)));
        assert_eq!(parse_xs_duration("PT00H03M30SZ").ok(), Some(Duration::new(210, 0)));
        assert_eq!(parse_xs_duration("PT3.14159S").ok(), Some(Duration::new(3, 141_590_000)));
        assert_eq!(parse_xs_duration("PT3.14159265S").ok(), Some(Duration::new(3, 141_592_650)));
        assert_eq!(parse_xs_duration("PT3.141592653S").ok(), Some(Duration::new(3, 141_592_653)));
        // We are truncating rather than rounding the number of nanoseconds
        assert_eq!(parse_xs_duration("PT3.141592653897S").ok(), Some(Duration::new(3, 141_592_653)));
        assert_eq!(parse_xs_duration("P0W").ok(), Some(Duration::new(0, 0)));
        assert_eq!(parse_xs_duration("P26W").ok(), Some(Duration::new(15724800, 0)));
        assert_eq!(parse_xs_duration("P52W").ok(), Some(Duration::new(31449600, 0)));
        assert_eq!(parse_xs_duration("P10D").ok(), Some(Duration::new(864000, 0)));
        assert_eq!(parse_xs_duration("P0Y").ok(), Some(Duration::new(0, 0)));
        assert_eq!(parse_xs_duration("P1Y").ok(), Some(Duration::new(31536000, 0)));
        assert_eq!(parse_xs_duration("P1Y0W0S").ok(), Some(Duration::new(31536000, 0)));
        assert_eq!(parse_xs_duration("PT4H").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("+PT4H").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("PT0004H").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("PT4H0M").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("PT4H0S").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("P23DT23H").ok(), Some(Duration::new(2070000, 0)));
        assert_eq!(parse_xs_duration("P0Y0M0DT0H4M20.880S").ok(), Some(Duration::new(260, 880_000_000)));
        assert_eq!(parse_xs_duration("P1Y2M3DT4H5M6.7S").ok(), Some(Duration::new(36993906, 700_000_000)));
        assert_eq!(parse_xs_duration("P1Y2M3DT4H5M6,7S").ok(), Some(Duration::new(36993906, 700_000_000)));

        // we are not currently handling fractional parts except in the seconds
        // assert_eq!(parse_xs_duration("PT0.5H1S").ok(), Some(Duration::new(30*60+1, 0)));
        // assert_eq!(parse_xs_duration("P0001-02-03T04:05:06").ok(), Some(Duration::new(36993906, 0)));
    }

    #[test]
    fn test_serialize_xs_duration() {
        use std::time::Duration;
        use super::MPD;

        fn serialized_xs_duration(d: Duration) -> String {
            let mpd = MPD {
                minBufferTime: Some(d),
                ..Default::default()
            };
            let xml = mpd.to_string();
            let doc = roxmltree::Document::parse(&xml).unwrap();
            String::from(doc.root_element().attribute("minBufferTime").unwrap())
        }

        assert_eq!("PT0S", serialized_xs_duration(Duration::new(0, 0)));
        assert_eq!("PT0.001S", serialized_xs_duration(Duration::new(0, 1_000_000)));
        assert_eq!("PT42S", serialized_xs_duration(Duration::new(42, 0)));
        assert_eq!("PT1.5S", serialized_xs_duration(Duration::new(1, 500_000_000)));
        assert_eq!("PT30.03S", serialized_xs_duration(Duration::new(30, 30_000_000)));
        assert_eq!("PT1M30.5S", serialized_xs_duration(Duration::new(90, 500_000_000)));
        assert_eq!("PT5M44S", serialized_xs_duration(Duration::new(344, 0)));
        assert_eq!("PT42M30S", serialized_xs_duration(Duration::new(2550, 0)));
        assert_eq!("PT30M38S", serialized_xs_duration(Duration::new(1838, 0)));
        assert_eq!("PT10M10S", serialized_xs_duration(Duration::new(610, 0)));
        assert_eq!("PT1H0M0.04S", serialized_xs_duration(Duration::new(3600, 40_000_000)));
        assert_eq!("PT3H11M53S", serialized_xs_duration(Duration::new(11513, 0)));
        assert_eq!("PT4H0M0S", serialized_xs_duration(Duration::new(14400, 0)));
    }

    #[test]
    fn test_parse_xs_datetime() {
        use chrono::{DateTime, NaiveDate};
        use chrono::offset::Utc;
        use super::parse_xs_datetime;

        let date = NaiveDate::from_ymd_opt(2023, 4, 19)
            .unwrap()
            .and_hms_opt(1, 3, 2)
            .unwrap();
        assert_eq!(parse_xs_datetime("2023-04-19T01:03:02Z").ok(),
                   Some(DateTime::<Utc>::from_naive_utc_and_offset(date, Utc)));
        let date = NaiveDate::from_ymd_opt(2023, 4, 19)
            .unwrap()
            .and_hms_nano_opt(1, 3, 2, 958*1000*1000)
            .unwrap();
        assert_eq!(parse_xs_datetime("2023-04-19T01:03:02.958Z").ok(),
                   Some(DateTime::<Utc>::from_naive_utc_and_offset(date, Utc)));
    }
}
