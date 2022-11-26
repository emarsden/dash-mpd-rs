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
//! format, as formally defined in ISO/IEC standard 23009-1:2019. XML schema files are [available
//! for no cost from
//! ISO](https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/). When
//! MPD files in practical use diverge from the formal standard, this library prefers to
//! interoperate with existing practice.
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
//! - Media containers of types supported by mkvmerge, ffmpeg or VLC (this includes Matroska,
//!   ISO-BMFF / CMAF / MP4, WebM, MPEG-2 TS)
//!
//!
//! ## Limitations / unsupported features
//!
//! - Dynamic MPD manifests, that are used for live streaming/OTT TV
//! - Encrypted content using DRM such as Encrypted Media Extensions (EME) and Media Source Extension (MSE)
//! - Subtitles (eg. WebVTT and TTML streams)
//! - XLink with actuate=onRequest
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


#![allow(non_snake_case)]

/// If library feature `libav` is enabled, muxing support (combining audio and video streams, which
/// are often separated out in DASH streams) is provided by ffmpeg's libav library, via the
/// `ac_ffmpeg` crate. Otherwise, muxing is implemented by calling `mkvmerge`, `ffmpeg` or `vlc` as
/// a subprocess. The muxing support is only compiled when the fetch feature is enabled.
#[cfg(all(feature = "fetch", feature = "libav"))]
mod libav;
#[cfg(all(feature = "fetch", not(feature = "libav")))]
mod ffmpeg;
#[cfg(feature = "fetch")]
pub mod fetch;

#[cfg(all(feature = "fetch", feature = "libav"))]
use crate::libav::mux_audio_video;
#[cfg(all(feature = "fetch", not(feature = "libav")))]
use crate::ffmpeg::mux_audio_video;
use serde::{Serialize, Serializer, Deserialize};
use serde::de;
use serde_with::skip_serializing_none;
use regex::Regex;
use std::time::Duration;
use chrono::DateTime;


/// Type representing an xs:dateTime, as per <https://www.w3.org/TR/xmlschema-2/#dateTime>
// Something like 2021-06-03T13:00:00Z
pub type XsDatetime = DateTime<chrono::offset::Utc>;


#[derive(thiserror::Error, Debug)]
pub enum DashMpdError {
    #[error("parse error {0}")]
    Parsing(String),
    #[error("invalid Duration: {0}")]
    InvalidDuration(String),
    #[error("invalid media stream: {0}")]
    UnhandledMediaStream(String),
    #[error("I/O error {1}")]
    Io(#[source] std::io::Error, String),
    #[error("network error {0}")]
    Network(String),
    #[error("muxing error {0}")]
    Muxing(String),
    #[error("unknown error {0}")]
    Other(String),
}


// Parse an XML duration string, as per https://www.w3.org/TR/xmlschema-2/#duration
//
// The lexical representation for duration is the ISO 8601 extended format PnYn MnDTnH nMnS, where
// nY represents the number of years, nM the number of months, nD the number of days, 'T' is the
// date/time separator, nH the number of hours, nM the number of minutes and nS the number of
// seconds. The number of seconds can include decimal digits to arbitrary precision.
//
// Examples: "PT0H0M30.030S", "PT1.2S", PT1004199059S, PT130S
// P2Y6M5DT12H35M30S	=> 2 years, 6 months, 5 days, 12 hours, 35 minutes, 30 seconds
// P1DT2H => 1 day, 2 hours
// P0Y20M0D => 20 months (0 is permitted as a number, but is not required)
// PT1M30.5S => 1 minute, 30.5 seconds
//
// Limitations: we can't represent negative durations (leading "-" character) due to the choice of a
// std::time::Duration. We only accept fractional parts of seconds, and reject for example "P0.5Y" and "PT2.3H". 
fn parse_xs_duration(s: &str) -> Result<Duration, DashMpdError> {
    let re = Regex::new(concat!(r"^(?P<sign>[+-])?P",
                                r"(?:(?P<years>\d+)Y)?",
                                r"(?:(?P<months>\d+)M)?",
                                r"(?:(?P<weeks>\d+)W)?",
                                r"(?:(?P<days>\d+)D)?",
                                r"(?:(?P<hastime>T)", // time part must begin with a T
                                r"(?:(?P<hours>\d+)H)?",
                                r"(?:(?P<minutes>\d+)M)?",
                                r"(?:(?P<seconds>\d+)(?:(?P<nanoseconds>[.,]\d+)?)S)?",
                                r")?")).unwrap();
    match re.captures(s) {
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
            if let Some(s) = m.name("nanoseconds") {
                let mut s = &s.as_str()[1..]; // drop initial "."
                if s.len() > 9 {
                    s = &s[..9];
                }
                let padded = format!("{s:0<9}");
                nsecs = padded.parse::<u32>().unwrap();
            }
            if let Some(s) = m.name("seconds") {
                let seconds = s.as_str().parse::<u64>().unwrap();
                secs += seconds;
            }
            if let Some(s) = m.name("minutes") {
                let minutes = s.as_str().parse::<u64>().unwrap();
                secs += minutes * 60;
            }
            if let Some(s) = m.name("hours") {
                let hours = s.as_str().parse::<u64>().unwrap();
                secs += hours * 60 * 60;
            }
            if let Some(s) = m.name("days") {
                let days = s.as_str().parse::<u64>().unwrap();
                secs += days * 60 * 60 * 24;
            }
            if let Some(s) = m.name("weeks") {
                let weeks = s.as_str().parse::<u64>().unwrap();
                secs += weeks * 60 * 60 * 24 * 7;
            }
            if let Some(s) = m.name("months") {
                let months = s.as_str().parse::<u64>().unwrap();
                secs += months * 60 * 60 * 24 * 30;
            }
            if let Some(s) = m.name("years") {
                let years = s.as_str().parse::<u64>().unwrap();
                secs += years * 60 * 60 * 24 * 365;
            }
            if let Some(s) = m.name("sign") {
                if s.as_str() == "-" {
                    return Err(DashMpdError::InvalidDuration("can't represent negative durations".to_string()));
                }
            }
            Ok(Duration::new(secs, nsecs))
        },
        None => Err(DashMpdError::InvalidDuration("couldn't parse XS duration".to_string())),
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

fn serialize_xs_duration<S>(oxs: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // this is a very simple-minded way of converting to an ISO 8601 duration
    if let Some(xs) = oxs {
        let secs = xs.as_secs();
        let ms = xs.subsec_millis();
        serializer.serialize_str(&format!("PT{secs}.{ms:03}S"))
    } else {
        // in fact this won't be called because of the #[skip_serializing_none] annotation
        serializer.serialize_none()
    }
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
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Title {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// The original source of the media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Source {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// Copyright information concerning the media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Copyright {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// Metainformation concerning the media stream (title, language, etc.)
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ProgramInformation {
    pub Title: Option<Title>,
    pub Source: Option<Source>,
    pub Copyright: Option<Copyright>,
    /// Language in RFC 5646 format
    pub lang: Option<String>,
    pub moreInformationURL: Option<String>,
}

/// Describes a sequence of contiguous Segments with identical duration.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct S {
    /// time
    pub t: Option<i64>,
    /// the duration (shall not exceed the value of MPD@maxSegmentDuration)
    pub d: i64,
    /// the repeat count (number of contiguous Segments with identical MPD duration minus one),
    /// defaulting to zero if not present
    pub r: Option<i64>,
}

/// Contains a sequence of `S` elements, each of which describes a sequence of contiguous segments of
/// identical duration.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SegmentTimeline {
    #[serde(rename = "S")]
    pub segments: Vec<S>,
}

/// The first media segment in a sequence of Segments. Subsequent segments can be concatenated to this
/// segment to produce a media stream.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Initialization {
    pub sourceURL: Option<String>,
    pub range: Option<String>,
}

/// Allows template-based `SegmentURL` construction. Specifies various substitution rules using
/// dynamic values such as `$Time$` and `$Number$` that map to a sequence of Segments.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SegmentTemplate {
    pub initialization: Option<String>,
    pub media: Option<String>,
    pub index: Option<String>,
    pub SegmentTimeline: Option<SegmentTimeline>,
    pub startNumber: Option<u64>,
    // note: the spec says this is an unsigned int, not an xs:duration. In practice, some manifests
    // use a floating point value (eg.
    // https://dash.akamaized.net/akamai/bbb_30fps/bbb_with_multiple_tiled_thumbnails.mpd)
    pub duration: Option<f64>,
    pub timescale: Option<u64>,
    /// Indicates a possible offset between media segment start/end points and period start/end points.
    pub eptDelta: Option<i64>,
    pub presentationTimeOffset: Option<u64>,
    pub bitstreamSwitching: Option<bool>,
}

/// A URI string to which a new request for an updated manifest should be made. This feature is
/// intended for servers and clients that can't use sticky HTTP redirects.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Location {
    #[serde(rename = "$value")]
    pub url: String,
}

/// A URI string that specifies one or more common locations for Segments and other resources.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct BaseURL {
    #[serde(rename = "$value")]
    pub base: String,
    /// Elements with the same `@serviceLocation` value are likely to have their URLs resolve to
    /// services at a common network location, for example the same CDN.
    pub serviceLocation: Option<String>,
}

/// Specifies some common information concerning media segments.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SegmentBase {
    #[serde(rename = "Initialization")]
    pub initialization: Option<Initialization>,
    pub timescale: Option<u64>,
    pub presentationTimeOffset: Option<u64>,
    pub indexRange: Option<String>,
    pub indexRangeExact: Option<bool>,
    pub availabilityTimeOffset: Option<f64>,
    pub availabilityTimeComplete: Option<bool>,
}

/// The URL of a media segment.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SegmentURL {
    pub media: Option<String>, // actually an URI
    pub mediaRange: Option<String>,
    pub index: Option<String>, // actually an URI
    pub indexRange: Option<String>,
}

/// Contains a sequence of SegmentURL elements.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SegmentList {
    // note: the spec says this is an unsigned int, not an xs:duration
    pub duration: Option<u64>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
    #[serde(rename = "xlink:actuate")]
    pub actuate: Option<String>,
    #[serde(rename = "xlink:type")]
    pub sltype: Option<String>,
    #[serde(rename = "xlink:show")]
    pub show: Option<String>,
    pub Initialization: Option<Initialization>,
    #[serde(rename = "SegmentURL")]
    pub segment_urls: Vec<SegmentURL>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Resync {
    pub dT: Option<u64>,
    pub dImax: Option<u64>,
    pub dImin: Option<u64>,
    #[serde(rename = "type")]
    pub rtype: Option<String>,
}

/// Specifies information concerning the audio channel (eg. stereo, multichannel).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AudioChannelConfiguration {
    pub id: Option<String>,
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

/// Specifies the accessibility scheme used by the media content.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Accessibility {
    pub id: Option<String>,
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

/// A representation describes a version of the content, using a specific encoding and bitrate.
/// Streams often have multiple representations with different bitrates, to allow the client to
/// select that most suitable to its network conditions.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Representation {
    // no id for a linked Representation (with xlink:href)
    pub id: Option<String>,
    // The specification says that @mimeType is mandatory, but it's not always present on
    // akamaized.net MPDs
    pub mimeType: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381>
    pub codecs: Option<String>,
    pub contentType: Option<String>,
    /// If present, this attribute is expected to be set to "progressive".
    pub scanType: Option<String>,
    pub frameRate: Option<String>, // can be something like "15/2"
    pub sar: Option<String>,
    pub bandwidth: Option<u64>,
    pub audioSamplingRate: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub startWithSAP: Option<u64>,
    pub BaseURL: Vec<BaseURL>,
    pub AudioChannelConfiguration: Option<AudioChannelConfiguration>,
    pub mediaStreamStructureId: Option<String>,
    pub SegmentTemplate: Option<SegmentTemplate>,
    pub SegmentBase: Option<SegmentBase>,
    pub SegmentList: Option<SegmentList>,
    pub Resync: Option<Resync>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
}

/// Describes a media content component.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ContentComponent {
    pub id: Option<String>,
    /// Language in RFC 5646 format (eg. "fr-FR", "en-AU")
    pub lang: Option<String>,
    pub contentType: Option<String>,
    pub par: Option<String>,
    pub tag: Option<String>,
    pub Accessibility: Option<Accessibility>,
}

/// A Common Encryption "Protection System Specific Header" box. Content is typically base64 encoded.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct CencPssh {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// Contains information on DRM (rights management / encryption) mechanisms used in the stream, such
/// as Widevine and Playready. Note that this library is not able to download content with DRM. If
/// this node is not present, no content protection is applied by the source.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ContentProtection {
    pub robustness: Option<String>,
    pub refId: Option<String>,
    #[serde(rename = "ref")]
    pub cpref: Option<String>,
    pub schemeIdUri: Option<String>,
    // In fact will be cenc:pssh, where cenc is the urn:mpeg:cenc:2013 XML namespace, but the serde
    // crate doesn't support XML namespaces
    #[serde(rename = "pssh")]
    pub cenc_pssh: Option<CencPssh>,
    // the DRM key identifier
    #[serde(rename = "cenc:default_KID")]
    pub default_KID: Option<String>,
    pub value: Option<String>,
}

/// The purpose of this media stream, such as captions, subtitle, main, alternate, supplementary,
/// commentary, and dub.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Role {
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Viewpoint {
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Binary {
    #[serde(rename = "$value")]
    pub content: Vec<u8>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Signal {
    #[serde(rename = "Binary")]
    pub content: Vec<Binary>,
}

/// A DASH event.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Event {
    pub id: Option<String>,
    pub presentationTime: Option<u64>,
    pub duration: Option<u64>,
    pub timescale: Option<u64>,
    #[serde(rename = "Signal")]
    pub signal: Vec<Signal>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EventStream {
    pub timescale: Option<u64>,
    pub schemeIdUri: Option<String>,
    #[serde(rename = "Event")]
    pub event: Vec<Event>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EssentialProperty {
    pub id: Option<String>,
    pub schemeIdUri: String,
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct SupplementalProperty {
    pub id: Option<String>,
    pub schemeIdUri: String,
    pub value: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Label {
    #[serde(rename = "$value")]
    pub content: String,
}

/// Contains a set of Representations. For example, if multiple language streams are available for
/// the audio content, each one can be in its own AdaptationSet.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AdaptationSet {
    pub id: Option<i64>,
    pub label: Option<Label>,
    pub BaseURL: Vec<BaseURL>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
    #[serde(rename = "xlink:actuate")]
    pub actuate: Option<String>,
    pub group: Option<i64>,
    pub selectionPriority: Option<u64>,
    // eg "audio", "video", "text"
    pub contentType: Option<String>,
    /// Content language, in RFC 5646 format
    pub lang: Option<String>,
    pub par: Option<String>,
    pub segmentAlignment: Option<bool>,
    pub subsegmentAlignment: Option<bool>,
    pub subsegmentStartsWithSAP: Option<u64>,
    pub bitstreamSwitching: Option<bool>,
    pub audioSamplingRate: Option<u64>,
    // eg "video/mp4"
    pub mimeType: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381> (eg. "avc1.4D400C")
    pub codecs: Option<String>,
    pub minBandwidth: Option<u64>,
    pub maxBandwidth: Option<u64>,
    pub minWidth: Option<u64>,
    pub maxWidth: Option<u64>,
    pub minHeight: Option<u64>,
    pub maxHeight: Option<u64>,
    pub frameRate: Option<String>, // it can be something like "15/2"
    pub SegmentTemplate: Option<SegmentTemplate>,
    pub ContentComponent: Vec<ContentComponent>,
    pub ContentProtection: Vec<ContentProtection>,
    pub Accessibility: Option<Accessibility>,
    pub AudioChannelConfiguration: Option<AudioChannelConfiguration>,
    #[serde(rename = "Representation")]
    pub representations: Vec<Representation>,
}

/// Identifies the asset to which a given Period belongs. Can be used to implement
/// client functionality that depends on distinguishing between ads and main content.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AssetIdentifier {
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

/// Describes a chunk of the content with a start time and a duration. Content can be split up into
/// multiple periods (such as chapters, advertising segments).
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Period {
    pub id: Option<String>,
    pub start: Option<String>,
    // note: the spec says that this is an xs:duration, not an unsigned int as for other "duration" fields
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub duration: Option<Duration>,
    pub bitstreamSwitching: Option<bool>,
    pub BaseURL: Vec<BaseURL>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
    #[serde(rename = "xlink:actuate")]
    pub actuate: Option<String>,
    pub SegmentTemplate: Option<SegmentTemplate>,
    #[serde(rename = "AdaptationSet")]
    pub adaptations: Vec<AdaptationSet>,
    pub asset_identifier: Option<AssetIdentifier>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Reporting {
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
    #[serde(rename = "dvb:reportingUrl")]
    pub reportingUrl: Option<String>,
    #[serde(rename = "dvb:probability")]
    pub probability: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Range {
    pub starttime: Option<Duration>,
    pub duration: Option<Duration>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Metrics {
    pub metrics: String,
    pub reporting: Vec<Reporting>,
    pub range: Vec<Range>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Latency {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub target: Option<f64>,
    pub referenceId: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PlaybackRate {
    pub min: f64,
    pub max: f64,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ServiceDescription {
    pub id: Option<String>,
    pub Latency: Option<Latency>,
    pub PlaybackRate: Option<PlaybackRate>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct UTCTiming {
    // prefixed with urn:mpeg:dash:utc, one of http-xsdate:2014, http-iso:2014,
    // http-ntp:2014, ntp:2014, http-head:2014, direct:2014
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct LeapSecondInformation {
    pub availabilityStartLeapOffset: Option<i64>,
    pub nextAvailabilityStartLeapOffset: Option<i64>,
    pub nextLeapChangeTime: Option<XsDatetime>,
}

/// The root node of a parsed DASH MPD manifest.
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct MPD {
    /// The Presentation Type, either "static" or "dynamic" (a live stream for which segments become
    /// available over time).
    #[serde(rename = "type")]
    pub mpdtype: Option<String>,
    pub xmlns: Option<String>,
    #[serde(rename = "xsi:schemaLocation")]
    pub schemaLocation: Option<String>,
    pub profiles: Option<String>,
    /// Prescribes how many seconds of buffer a client should keep to avoid stalling when streaming
    /// under ideal network conditions with bandwidth matching the @bandwidth attribute.
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub minBufferTime: Option<Duration>,
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub minimumUpdatePeriod: Option<Duration>,
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub timeShiftBufferDepth: Option<Duration>,
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub mediaPresentationDuration: Option<Duration>,
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub maxSegmentDuration: Option<Duration>,
    /// A suggested delay of the presentation compared to the Live edge.
    #[serde(deserialize_with = "deserialize_xs_duration", default)]
    #[serde(serialize_with = "serialize_xs_duration")]
    pub suggestedPresentationDelay: Option<Duration>,
    pub publishTime: Option<XsDatetime>,
    pub availabilityStartTime: Option<XsDatetime>,
    pub availabilityEndTime: Option<XsDatetime>,
    #[serde(rename = "Period", default)]
    pub periods: Vec<Period>,
    /// There may be several BaseURLs, for redundancy (for example multiple CDNs)
    #[serde(rename = "BaseURL")]
    pub base_url: Vec<BaseURL>,
    pub locations: Vec<Location>,
    pub ServiceDescription: Option<ServiceDescription>,
    pub ProgramInformation: Option<ProgramInformation>,
    pub Metrics: Vec<Metrics>,
    pub UTCTiming: Vec<UTCTiming>,
    /// Correction for leap seconds, used by the DASH Low Latency specification. 
    pub LeapSecondInformation: Option<LeapSecondInformation>,
    #[serde(rename = "EssentialProperty")]
    pub essential_property: Vec<EssentialProperty>,
    #[serde(rename = "SupplementalProperty")]
    pub supplemental_property: Vec<SupplementalProperty>,
}


/// Parse an MPD manifest, provided as an XML string, returning an `MPD` node.
pub fn parse(xml: &str) -> Result<MPD, DashMpdError> {
    let mpd: Result<MPD, quick_xml::DeError> = quick_xml::de::from_str(xml);
    match mpd {
        Ok(mpd) => Ok(mpd),
        Err(e) => Err(DashMpdError::Parsing(e.to_string())),
    }
}


/// Returns `true` if this AdaptationSet contains audio content.
///
/// It contains audio if the `contentType` attribute` is `audio`, or the `mimeType` attribute is
/// `audio/*`, or if one of its child `Representation` nodes has an audio `contentType` or
/// `mimeType` attribute.
pub fn is_audio_adaptation(a: &&AdaptationSet) -> bool {
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
/// `video/*`, or if one of its child `Representation` nodes has an audio `contentType` or
/// `mimeType` attribute.
pub fn is_video_adaptation(a: &&AdaptationSet) -> bool {
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
        if let Some(mimetype) = &r.mimeType {
            if mimetype.starts_with("video/") {
                return true;
            }
        }
    }
    false
}


#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_xs_duration() {
        use std::time::Duration;
        use super::parse_xs_duration;

        assert!(parse_xs_duration("").is_err());
        assert!(parse_xs_duration("foobles").is_err());
        assert!(parse_xs_duration("P").is_err());
        assert!(parse_xs_duration("1Y2M3DT4H5M6S").is_err()); // missing initial P
        assert_eq!(parse_xs_duration("PT3H11M53S").ok(), Some(Duration::new(11513, 0)));
        assert_eq!(parse_xs_duration("PT42M30S").ok(), Some(Duration::new(2550, 0)));
        assert_eq!(parse_xs_duration("PT30M38S").ok(), Some(Duration::new(1838, 0)));
        assert_eq!(parse_xs_duration("PT0H10M0.00S").ok(), Some(Duration::new(600, 0)));
        assert_eq!(parse_xs_duration("PT1.5S").ok(), Some(Duration::new(1, 500_000_000)));
        assert_eq!(parse_xs_duration("PT0S").ok(), Some(Duration::new(0, 0)));
        assert_eq!(parse_xs_duration("PT0.001S").ok(), Some(Duration::new(0, 1_000_000)));
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
        assert!(parse_xs_duration("PW").is_err());
        assert_eq!(parse_xs_duration("P0W").ok(), Some(Duration::new(0, 0)));
        assert_eq!(parse_xs_duration("P26W").ok(), Some(Duration::new(15724800, 0)));
        assert_eq!(parse_xs_duration("P52W").ok(), Some(Duration::new(31449600, 0)));
        assert_eq!(parse_xs_duration("P0Y").ok(), Some(Duration::new(0, 0)));
        assert_eq!(parse_xs_duration("P1Y").ok(), Some(Duration::new(31536000, 0)));
        assert_eq!(parse_xs_duration("PT4H").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("+PT4H").ok(), Some(Duration::new(14400, 0)));
        assert_eq!(parse_xs_duration("P23DT23H").ok(), Some(Duration::new(2070000, 0)));
        assert_eq!(parse_xs_duration("P0Y0M0DT0H4M20.880S").ok(), Some(Duration::new(260, 880_000_000)));
        assert_eq!(parse_xs_duration("P1Y2M3DT4H5M6.7S").ok(), Some(Duration::new(36993906, 700_000_000)));
        assert_eq!(parse_xs_duration("P1Y2M3DT4H5M6,7S").ok(), Some(Duration::new(36993906, 700_000_000)));

        // we are not currently handling fractional parts except in the seconds
        // assert_eq!(parse_xs_duration("PT0.5H1S").ok(), Some(Duration::new(30*60+1, 0)));
        // assert_eq!(parse_xs_duration("P0001-02-03T04:05:06").ok(), Some(Duration::new(36993906, 0)));
    }
}
