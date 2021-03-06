//! A Rust library for parsing and downloading media content from a DASH MPD manifest, as used for
//! on-demand replay of TV content and video streaming services.
//!
//! [DASH](https://en.wikipedia.org/wiki/Dynamic_Adaptive_Streaming_over_HTTP) (dynamic adaptive
//! streaming over HTTP), also called MPEG-DASH, is a technology used for media streaming over the
//! web, commonly used for video on demand (VOD) services. The Media Presentation Description (MPD)
//! is a description of the resources (manifest or “playlist”) forming a streaming service, that a
//! DASH client uses to determine which assets to request in order to perform adaptive streaming of
//! the content. DASH MPD manifests can be used both with content encoded as MPEG and as WebM.
//!
//! This library provides a serde-based parser for the DASH MPD format, as formally defined in
//! ISO/IEC standard 23009-1:2019. XML schema files are [available for no cost from
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
//! - Media containers of types supported by ffmpeg or VLC (this includes ISO-BMFF / CMAF / MP4, WebM, MPEG-2 TS)
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
/// `ac_ffmpeg` crate. Otherwise, muxing is implemented by calling `ffmpeg` or `vlc` as a subprocess.
#[cfg(feature = "libav")]
mod libav;
#[cfg(not(feature = "libav"))]
mod ffmpeg;
pub mod fetch;

use anyhow::{Result, Context, anyhow};
use serde::Deserialize;
use serde::de;
use regex::Regex;
use std::time::Duration;
use tempfile::NamedTempFile;
#[cfg(feature = "libav")]
use crate::libav::mux_audio_video;
#[cfg(not(feature = "libav"))]
use crate::ffmpeg::mux_audio_video;




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
fn parse_xs_duration(s: &str) -> Result<Duration> {
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
                  return Err(anyhow!("Couldn't parse empty XS duration"));
            }
            let mut secs: u64 = 0;
            let mut nsecs: u32 = 0;
            if let Some(s) = m.name("nanoseconds") {
                let mut s = &s.as_str()[1..]; // drop initial "."
                if s.len() > 9 {
                    s = &s[..9];
                }
                let padded = format!("{:0<9}", s);
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
                secs += hours as u64 * 60 * 60;
            }
            if let Some(s) = m.name("days") {
                let days = s.as_str().parse::<u64>().unwrap();
                secs += days as u64 * 60 * 60 * 24;
            }
            if let Some(s) = m.name("weeks") {
                let weeks = s.as_str().parse::<u64>().unwrap();
                secs += weeks as u64 * 60 * 60 * 24 * 7;
            }
            if let Some(s) = m.name("months") {
                let months = s.as_str().parse::<u64>().unwrap();
                secs += months as u64 * 60 * 60 * 24 * 30;
            }
            if let Some(s) = m.name("years") {
                let years = s.as_str().parse::<u64>().unwrap();
                secs += years as u64 * 60 * 60 * 24 * 365;
            }
            if let Some(s) = m.name("sign") {
                if s.as_str() == "-" {
                    return Err(anyhow!("Can't represent negative durations"));
                }
            }
            Ok(Duration::new(secs, nsecs))
        },
        None => Err(anyhow!("Couldn't parse XS duration")),
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


// The MPD format is documented by ISO using an XML Schema at
// https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/DASH-MPD-edition2.xsd
// Historical spec: https://ptabdata.blob.core.windows.net/files/2020/IPR2020-01688/v67_EXHIBIT%201067%20-%20ISO-IEC%2023009-1%202019(E)%20-%20Info.%20Tech.%20-%20Dynamic%20Adaptive%20Streaming%20Over%20HTTP%20(DASH).pdf
// We occasionally diverge from the standard when in-the-wild implementations do.
// Some reference code for DASH is at https://github.com/bitmovin/libdash
//
// We are using the quick_xml + serde crates to deserialize the XML content to Rust structs. Note
// that serde will ignore unknown fields when deserializing, so we don't need to cover every single
// possible field.

/// The title of the media stream.
#[derive(Debug, Deserialize, Clone)]
pub struct Title {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// The original source of the media stream.
#[derive(Debug, Deserialize, Clone)]
pub struct Source {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// Copyright information concerning the media stream.
#[derive(Debug, Deserialize, Clone)]
pub struct Copyright {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// Metainformation concerning the media stream (title, language, etc.)
#[derive(Debug, Deserialize, Clone)]
pub struct ProgramInformation {
    pub Title: Option<Title>,
    pub Source: Option<Source>,
    pub Copyright: Option<Copyright>,
    /// Language in RFC 5646 format
    pub lang: Option<String>,
    pub moreInformationURL: Option<String>,
}

/// Describes a sequence of contiguous Segments with identical duration.
#[derive(Debug, Deserialize, Clone)]
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
#[derive(Debug, Deserialize, Clone)]
pub struct SegmentTimeline {
    #[serde(rename = "S")]
    pub segments: Vec<S>,
}

/// The first media segment in a sequence of Segments. Subsequent segments can be concatenated to this
/// segment to produce a media stream.
#[derive(Debug, Deserialize, Clone)]
pub struct Initialization {
    pub sourceURL: Option<String>,
    pub range: Option<String>,
}

/// Allows template-based `SegmentURL` construction. Specifies various substitution rules using
/// dynamic values such as `$Time$` and `$Number$` that map to a sequence of Segments.
#[derive(Debug, Deserialize, Clone)]
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
    pub presentationTimeOffset: Option<u64>,
    pub bitstreamSwitching: Option<String>,  // bool?
}

/// A URI string that specifies one or more common locations for Segments and other resources.
#[derive(Debug, Deserialize, Clone)]
pub struct BaseURL {
    #[serde(rename = "$value")]
    pub base: String,
    /// Elements with the same `@serviceLocation` value are likely to have their URLs resolve to
    /// services at a common network location, for example the same CDN.
    pub serviceLocation: Option<String>,
}

/// Specifies some common information concerning media segments.
#[derive(Debug, Deserialize, Clone)]
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
#[derive(Debug, Deserialize, Clone)]
pub struct SegmentURL {
    pub media: Option<String>, // actually an URI
    pub mediaRange: Option<String>,
    pub index: Option<String>, // actually an URI
    pub indexRange: Option<String>,
}

/// Contains a sequence of SegmentURL elements.
#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Deserialize, Clone)]
pub struct Resync {
    pub dT: Option<u64>,
    pub dImax: Option<u64>,
    pub dImin: Option<u64>,
    #[serde(rename = "type")]
    pub rtype: Option<String>,
}

/// Specifies information concerning the audio channel (eg. stereo, multichannel).
#[derive(Debug, Deserialize, Clone)]
pub struct AudioChannelConfiguration {
    pub id: Option<String>,
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

/// Specifies the accessibility scheme used by the media content.
#[derive(Debug, Deserialize, Clone)]
pub struct Accessibility {
    pub id: Option<String>,
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

/// A representation describes a version of the content, using a specific encoding and bitrate.
/// Streams often have multiple representations with different bitrates, to allow the client to
/// select that most suitable to its network conditions.
#[derive(Debug, Deserialize, Clone)]
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
    pub startsWithSAP: Option<u64>,
    pub BaseURL: Option<BaseURL>,
    pub AudioChannelConfiguration: Option<AudioChannelConfiguration>,
    pub SegmentTemplate: Option<SegmentTemplate>,
    pub SegmentBase: Option<SegmentBase>,
    pub SegmentList: Option<SegmentList>,
    pub Resync: Option<Resync>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
}

/// Describes a media content component.
#[derive(Debug, Deserialize, Clone)]
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
#[derive(Debug, Deserialize, Clone)]
pub struct CencPssh {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

/// Contains information on DRM (rights management / encryption) mechanisms used in the stream, such
/// as Widevine and Playready. Note that this library is not able to download content with DRM. If
/// this node is not present, no content protection is applied by the source.
#[derive(Debug, Deserialize, Clone)]
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
#[derive(Debug, Deserialize, Clone)]
pub struct Role {
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Viewpoint {
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Binary {
    #[serde(rename = "$value")]
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Signal {
    #[serde(rename = "Binary")]
    pub contents: Option<Vec<Binary>>,
}

/// A DASH event.
#[derive(Debug, Deserialize, Clone)]
pub struct Event {
    pub id: Option<String>,
    pub duration: Option<u64>,
    #[serde(rename = "Signal")]
    pub signals: Option<Vec<Signal>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EventStream {
    pub timescale: Option<u64>,
    pub schemeIdUri: Option<String>,
    #[serde(rename = "Event")]
    pub events: Option<Vec<Event>>,
}

/// Contains a set of Representations. For example, if multiple language streams are available for
/// the audio content, each one can be in its own AdaptationSet.
#[derive(Debug, Deserialize, Clone)]
pub struct AdaptationSet {
    pub id: Option<i64>,
    pub BaseURL: Option<BaseURL>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
    #[serde(rename = "xlink:actuate")]
    pub actuate: Option<String>,
    pub group: Option<i64>,
    // eg "audio", "video", "text"
    pub contentType: Option<String>,
    /// Content language, in RFC 5646 format
    pub lang: Option<String>,
    pub par: Option<String>,
    pub segmentAlignment: Option<bool>,
    pub subsegmentAlignment: Option<bool>,
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
    pub ContentComponent: Option<Vec<ContentComponent>>,
    pub ContentProtection: Option<Vec<ContentProtection>>,
    pub Accessibility: Option<Accessibility>,
    pub AudioChannelConfiguration: Option<AudioChannelConfiguration>,
    #[serde(rename = "Representation")]
    pub representations: Option<Vec<Representation>>,
}

/// Describes a chunk of the content with a start time and a duration. Content can be split up into
/// multiple periods (such as chapters, advertising segments).
#[derive(Debug, Deserialize, Clone)]
pub struct Period {
    pub id: Option<String>,
    pub start: Option<String>,
    // note: the spec says that this is an xs:duration, not an unsigned int as for other "duration" fields
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_xs_duration")]
    pub duration: Option<Duration>,
    pub bitstreamSwitching: Option<bool>,
    pub BaseURL: Option<BaseURL>,
    /// A "remote resource", following the XML Linking Language (XLink) specification.
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
    #[serde(rename = "xlink:actuate")]
    pub actuate: Option<String>,
    pub SegmentTemplate: Option<SegmentTemplate>,
    #[serde(rename = "AdaptationSet")]
    pub adaptations: Option<Vec<AdaptationSet>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Latency {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub target: Option<f64>,
    pub referenceId: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PlaybackRate {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServiceDescription {
    pub id: Option<String>,
    pub Latency: Option<Latency>,
    pub PlaybackRate: Option<PlaybackRate>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UTCTiming {
    // prefixed with urn:mpeg:dash:utc, one of http-xsdate:2014, http-iso:2014,
    // http-ntp:2014, ntp:2014, http-head:2014, direct:2014
    pub schemeIdUri: Option<String>,
    pub value: Option<String>,
}

/// The root node of a parsed DASH MPD manifest.
#[derive(Debug, Deserialize, Clone)]
pub struct MPD {
    #[serde(rename = "type")]
    pub mpdtype: Option<String>,
    pub xmlns: Option<String>,
    #[serde(rename = "xsi:schemaLocation")]
    pub schemaLocation: Option<String>,
    pub profiles: Option<String>,
    pub minBufferTime: Option<String>,
    pub minimumUpdatePeriod: Option<String>,
    pub timeShiftBufferDepth: Option<String>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_xs_duration")]
    pub mediaPresentationDuration: Option<Duration>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_xs_duration")]
    pub maxSegmentDuration: Option<Duration>,
    pub suggestedPresentationDelay: Option<String>,
    pub publishTime: Option<String>,
    pub availabilityStartTime: Option<String>,
    #[serde(rename = "Period")]
    pub periods: Vec<Period>,
    /// There may be several BaseURLs, for redundancy (for example multiple CDNs)
    #[serde(rename = "BaseURL")]
    pub base_urls: Option<Vec<BaseURL>>,
    pub ServiceDescription: Option<ServiceDescription>,
    pub ProgramInformation: Option<ProgramInformation>,
    pub UTCTiming: Option<UTCTiming>,
}


/// Parse an MPD manifest, provided as an XML string, returning an `MPD` node.
pub fn parse(xml: &str) -> Result<MPD> {
    let mpd: MPD = quick_xml::de::from_str(xml)
        .context("parsing MPD XML")?;
    Ok(mpd)
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
    if let Some(reps) = &a.representations {
        for r in reps.iter() {
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
    if let Some(reps) = &a.representations {
        for r in reps.iter() {
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
    }
    false
}


// This doesn't work correctly on Android (fix needed in the tempfile crate)
fn tmp_file_path(_prefix: &str) -> String {
    let file = NamedTempFile::new()
        .expect("Creating named temp file");
    let path = file.path().to_str()
        .expect("Creating named temp file");
    path.to_string()
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
