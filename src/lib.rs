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

// TODO: allow user to specify preference for selecting representation (highest quality, lowest quality, etc.)
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

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde::de;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::io::BufWriter;
use std::time::Duration;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use tempfile::NamedTempFile;
use url::Url;
use regex::Regex;
use backoff::{retry, retry_notify, ExponentialBackoff};
#[cfg(feature = "libav")]
use crate::libav::mux_audio_video;
#[cfg(not(feature = "libav"))]
use crate::ffmpeg::mux_audio_video;


/// A blocking `Client` from the `reqwest` crate, that we use to download content over HTTP.
pub type HttpClient = reqwest::blocking::Client;



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
// Note bug in current version of the iso8601 crate which incorrectly parses strings like "PT344S"
// (seen in a real MPD) as a zero duration. However, ISO 8601 standard as adopted by Indian Bureau
// of Standards includes p29 an example "PT72H"
// https://archive.org/details/gov.in.is.7900.2007/
fn parse_xs_duration(s: &str) -> Result<Duration> {
    match iso8601::duration(s) {
        Ok(iso_duration) => {
            match iso_duration {
                iso8601::Duration::Weeks(w) => Ok(Duration::new(w as u64*60 * 60 * 24 * 7, 0)),
                iso8601::Duration::YMDHMS {year, month, day, hour, minute, second, millisecond } => {
                    // note that if year and month are specified, we are not going to do a very
                    // good conversion here
                    let mut secs: u64 = second.into();
                    secs += minute as u64 * 60;
                    secs += hour   as u64 * 60 * 60;
                    secs += day    as u64 * 60 * 60 * 24;
                    secs += month  as u64 * 60 * 60 * 24 * 31;
                    secs += year   as u64 * 60 * 60 * 24 * 31 * 365;
                    Ok(Duration::new(secs, millisecond * 1000))
                },
            }
        },
        Err(e) => Err(anyhow!("Couldn't parse XS duration {}: {:?}", s, e)),
    }
}


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
    // note: the spec says this is an unsigned int, not an xs:duration
    pub duration: Option<u64>,
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
    /// Language in RFC 5646 format
    pub lang: Option<String>,
    pub contentType: Option<String>,
    pub par: Option<String>,
    pub tag: Option<String>,
    pub Accessibility: Option<Accessibility>,
}

/// Contains information on DRM (rights management / encryption) mechanisms used in the stream, such
/// as Widevine and Playready. Note that this library is not able to download content with DRM. If
/// this node is not present, no content protection is applied.
#[derive(Debug, Deserialize, Clone)]
pub struct ContentProtection {
    pub robustness: Option<String>,
    pub refId: Option<String>,
    #[serde(rename = "ref")]
    pub cpref: Option<String>,
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
    pub contentType: Option<String>,
    /// Content language, in RFC 5646 format
    pub lang: Option<String>,
    pub par: Option<String>,
    pub segmentAlignment: Option<bool>,
    pub subsegmentAlignment: Option<bool>,
    pub bitstreamSwitching: Option<bool>,
    pub audioSamplingRate: Option<u64>,
    pub mimeType: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381>
    pub codecs: Option<String>,
    pub minBandwidth: Option<u64>,
    pub maxBandwidth: Option<u64>,
    pub minWidth: Option<u64>,
    pub maxWidth: Option<u64>,
    pub minHeight: Option<u64>,
    pub maxHeight: Option<u64>,
    pub frameRate: Option<String>, // it can be something like "15/2"
    pub SegmentTemplate: Option<SegmentTemplate>,
    pub ContentComponent: Option<ContentComponent>,
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
    let mpd: MPD = quick_xml::de::from_str(xml)?;
    Ok(mpd)
}


fn is_absolute_url(s: &str) -> bool {
    s.starts_with("http://") ||
        s.starts_with("https://") ||
        s.starts_with("file://")
}

// From the DASH-IF-IOP-v4.0 specification, "If the value of the @xlink:href attribute is
// urn:mpeg:dash:resolve-to-zero:2013, HTTP GET request is not issued, and the in-MPD element shall
// be removed from the MPD."
fn fetchable_xlink_href(href: &str) -> bool {
    (!href.is_empty()) && href.ne("urn:mpeg:dash:resolve-to-zero:2013")
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


// From https://dashif.org/docs/DASH-IF-IOP-v4.3.pdf:
// "For the avoidance of doubt, only %0[width]d is permitted and no other identifiers. The reason
// is that such a string replacement can be easily implemented without requiring a specific library."
//
// Instead of pulling in C printf() or a reimplementation such as the printf_compat crate, we reimplement
// this functionality directly.
//
// Example template: "$RepresentationID$/$Number%06d$.m4s"
fn resolve_url_template(template: &str, params: &HashMap<&str, String>) -> String {
    let mut result = template.to_string();
    for k in ["RepresentationID", "Number", "Time", "Bandwidth"] {
        // first check for simple case eg $Number$
        let ident = format!("${}$", k);
        if result.contains(&ident) {
            if let Some(value) = params.get(k as &str) {
                result = result.replace(&ident, value);
            }
        }
        // now check for complex case eg $Number%06d$
        let re = format!("\\${}%0([\\d])d\\$", k);
        let ident_re = Regex::new(&re).unwrap();
        if let Some(cap) = ident_re.captures(&result) {
            if let Some(value) = params.get(k as &str) {
                let width: usize = cap[1].parse::<usize>().unwrap();
                let count = format!("{:0>width$}", value, width=width);
                let m = ident_re.find(&result).unwrap();
                result = result[..m.start()].to_owned() + &count + &result[m.end()..];
            }
        }
    }
    result
}


fn reqwest_error_transient_p(e: &reqwest::Error) -> bool {
    if e.is_timeout() || e.is_connect() {
        return true;
    }
    if let Some(s) = e.status() {
        if s == reqwest::StatusCode::REQUEST_TIMEOUT ||
            s == reqwest::StatusCode::TOO_MANY_REQUESTS ||
            s == reqwest::StatusCode::SERVICE_UNAVAILABLE ||
            s == reqwest::StatusCode::GATEWAY_TIMEOUT {
                return true;
            }
    }
    false
}

fn categorize_reqwest_error(e: reqwest::Error) -> backoff::Error<reqwest::Error> {
    if reqwest_error_transient_p(&e) {
        backoff::Error::retry_after(e, Duration::new(5, 0))
    } else {
        backoff::Error::permanent(e)
    }
}

fn notify_transient<E: std::fmt::Debug>(err: E, dur: Duration) {
    log::info!("Transient error at {:?}: {:?}", dur, err);
}



/// Download the media stream content from a DASH MPD manifest.
///
/// This involves fetching the manifest file, parsing it, identifying the relevant audio and video
/// representations, downloading all the segments, concatenating them and muxing them together to
/// produce a single video file including audio. Currently, the "relevant" representations are those
/// with the lowest bandwidth. This should work with both MPEG-DASH MPD manifests (where the media
/// segments are typically placed in MPEG-2 TS containers) and for
/// [WebM-DASH](http://wiki.webmproject.org/adaptive-streaming/webm-dash-specification).
///
/// The `client` argument is a blocking Client from the `reqwest` crate.
/// The `mpd_url` argument is the URL of the DASH manifest.
/// The `path` argument names a local file that the media content will be saved to.
///
/// Example
/// ```rust
/// use dash_mpd::fetch_mpd;
///
/// let client = reqwest::blocking::Client::builder().build().unwrap();
/// let url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
/// if let Err(e) = fetch_mpd(&client, url, "/tmp/MPD-test.mp4") {
///    eprintln!("Error downloading DASH MPD file: {:?}", e);
/// }
/// ```
pub fn fetch_mpd(client: &HttpClient,
                 mpd_url: &str,
                 path: &str) -> Result<()> {
    let fetch = || {
        client.get(mpd_url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .header("Accept-language", "en-US,en")
            .send()
            .map_err(categorize_reqwest_error)
    };
    // could also try crate https://lib.rs/crates/reqwest-retry for a "middleware" solution to retries
    // or https://docs.rs/again/latest/again/ with async support
    let backoff = ExponentialBackoff::default();
    let response = retry(backoff, fetch)
        .context("Requesting DASH manifest")?;
    let redirected_url = response.url().clone();
    let xml = response.text()
        .context("Fetching DASH manifest")?;
    let mpd: MPD = parse(&xml)?;
    if let Some(mpdtype) = mpd.mpdtype {
        if mpdtype.eq("dynamic") {
            // An example https://cmafref.akamaized.net/cmaf/live-ull/2006350/akambr/out.mpd
            // we have no period.duration but we have some Resync XML packets, perhaps indicating
            // use of DASH Low Latency streaming
            // https://dashif.org/docs/CR-Low-Latency-Live-r8.pdf
            //
            // TODO: look at algorithm used in function segment_numbers at
            // https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/dash_manifest.py
            return Err(anyhow!("Don't know how to download dynamic MPD"));
        }
    }
    let mut toplevel_base_url = redirected_url.clone();
    // There may be several BaseURL tags in the MPD, but we don't currently implement failover
    if let Some(bases) = &mpd.base_urls {
        if is_absolute_url(&bases[0].base) {
            toplevel_base_url = Url::parse(&bases[0].base)?;
        } else {
            toplevel_base_url = redirected_url.join(&bases[0].base)?;
        }
    }
    let mut video_segment_urls = Vec::new();
    let mut audio_segment_urls = Vec::new();
    let tmppath_video = tmp_file_path("dashmpd-video-track");
    let tmppath_audio = tmp_file_path("dashmpd-audio-track");
    let mut tmpfile_video = BufWriter::new(File::create(tmppath_video.clone())
                                           .context("Creating video tmpfile")?);
    let mut tmpfile_audio = BufWriter::new(File::create(tmppath_audio.clone())
                                           .context("Creating audio tmpfile")?);
    let mut have_audio = false;
    let mut have_video = false;
    for mpd_period in &mpd.periods {
        let mut period = mpd_period.clone();
        // Resolve a possible xlink:href (though this seems in practice mostly to be used for ad
        // insertion, so perhaps we should implement an option to ignore these).
        if let Some(href) = &period.href {
            if fetchable_xlink_href(href) {
                let xlink_url;
                if is_absolute_url(href) {
                    xlink_url = Url::parse(href)?;
                } else {
                    // Note that we are joining against the original/redirected URL for the MPD, and
                    // not against the currently scoped BaseURL
                    xlink_url = redirected_url.join(href)?;
                }
                let xml = client.get(xlink_url)
                    .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                    .header("Accept-language", "en-US,en")
                    .send()?
                    .text()
                    .context("Resolving XLink on Period element")?;
                let linked_period: Period = quick_xml::de::from_str(&xml)?;
                period.clone_from(&linked_period);
            }
        }
        // The period_duration is specified either by the <Period> duration attribute, or by the
        // mediaPresentationDuration of the top-level MPD node.
        let mut period_duration_secs: f64 = 0.0;
        if let Some(d) = mpd.mediaPresentationDuration {
            period_duration_secs = d.as_secs_f64();
        }
        if let Some(d) = &period.duration {
            period_duration_secs = d.as_secs_f64();
        }
        let mut base_url = toplevel_base_url.clone();
        // A BaseURL could be specified for each Period
        if let Some(bu) = &period.BaseURL {
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)?;
            } else {
                base_url = base_url.join(&bu.base)?;
            }
        }
        // Handle the AdaptationSet with audio content. Note that some streams don't separate out
        // audio and video streams.
        let maybe_audio_adaptation = match &period.adaptations {
            Some(a) => a.iter().find(is_audio_adaptation),
            None => None,
        };
        // TODO: we could perhaps factor out the treatment of the audio adaptation and video
        // adaptation into a common handle_adaptation() function
        if let Some(period_audio) = maybe_audio_adaptation {
            let mut audio = period_audio.clone();
            // Resolve a possible xlink:href on the AdaptationSet
            if let Some(href) = &audio.href {
                if fetchable_xlink_href(href) {
                    let xlink_url;
                    if is_absolute_url(href) {
                        xlink_url = Url::parse(href)?;
                    } else {
                        // Note that we are joining against the original/redirected URL for the MPD, and
                        // not against the currently scoped BaseURL
                        xlink_url = redirected_url.join(href)?;
                    }
                    let xml = client.get(xlink_url)
                        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                        .header("Accept-language", "en-US,en")
                        .send()?
                        .text()
                        .context("Resolving XLink on AdaptationSet element")?;
                    let linked_adaptation: AdaptationSet = quick_xml::de::from_str(&xml)?;
                    audio.clone_from(&linked_adaptation);
                }
            }
            // The AdaptationSet may have a BaseURL (eg the test BBC streams). We use a local variable
            // to make sure we don't "corrupt" the base_url for the video segments.
            let mut base_url = base_url.clone();
            if let Some(bu) = &audio.BaseURL {
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base)?;
                } else {
                    base_url = base_url.join(&bu.base)?;
                }
            }
            // Start by resolving any xlink:href elements on Representation nodes, which we need to
            // do before the selection based on the @bandwidth attribute below.
            let mut representations = Vec::<Representation>::new();
            if let Some(reps) = audio.representations {
                for r in reps.iter() {
                    if let Some(href) = &r.href {
                        if fetchable_xlink_href(href) {
                            let xlink_url;
                            if is_absolute_url(href) {
                                xlink_url = Url::parse(href)?;
                            } else {
                                xlink_url = redirected_url.join(href)?;
                            }
                            let xml = client.get(xlink_url)
                                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                                .header("Accept-language", "en-US,en")
                                .send()?
                                .text()
                                .context("Resolving XLink on Representation element")?;
                            let linked_representation: Representation = quick_xml::de::from_str(&xml)?;
                            representations.push(linked_representation);
                        }
                    } else {
                        representations.push(r.clone());
                    }
                }
            }
            if let Ok(audio_repr) = representations.iter()
                .min_by_key(|x| x.bandwidth.unwrap_or(1_000_000_000))
                .context("Finding min bandwidth audio representation")
            {
                // the Representation may have a BaseURL
                let mut base_url = base_url;
                if let Some(bu) = &audio_repr.BaseURL {
                    if is_absolute_url(&bu.base) {
                        base_url = Url::parse(&bu.base)?;
                    } else {
                        base_url = base_url.join(&bu.base)?;
                    }
                }
                let mut opt_init: Option<String> = None;
                let mut opt_media: Option<String> = None;
                let mut opt_duration: Option<f64> = None;
                let mut timescale = 1;
                let mut start_number = 0;
                // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
                // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
                // nodes in an enclosed Representation node. Don't download media segments here, only
                // download for SegmentTemplate nodes that are children of a Representation node.
                if let Some(st) = &audio.SegmentTemplate {
                    if let Some(i) = &st.initialization {
                        opt_init = Some(i.to_string());
                    }
                    if let Some(m) = &st.media {
                        opt_media = Some(m.to_string());
                    }
                    if let Some(d) = st.duration {
                        opt_duration = Some(d as f64);
                    }
                    if let Some(ts) = st.timescale {
                        timescale = ts;
                    }
                    if let Some(s) = st.startNumber {
                        start_number = s;
                    }
                }
                let rid = match &audio_repr.id {
                    Some(id) => id,
                    None => return Err(anyhow!("Missing @id on Representation node")),
                };
                let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
                if let Some(b) = &audio_repr.bandwidth {
                    dict.insert("Bandwidth", b.to_string());
                }
                // Now the 6 possible addressing modes: SegmentBase@indexRange, SegmentList, SegmentTimeline,
                // SegmentTemplate@duration, SegmentTemplate@index
                if let Some(sb) = &audio_repr.SegmentBase {
                    // (1) SegmentBase@indexRange addressing mode
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path)?;
                            } else {
                                init_url = base_url.join(&path)?;
                            }
                            audio_segment_urls.push(init_url);
                        }
                    }
                    // TODO: properly handle indexRange attribute
                    audio_segment_urls.push(base_url.clone());
                } else if let Some(sl) = &audio_repr.SegmentList {
                    // (2) SegmentList addressing mode
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path)?;
                            } else {
                                init_url = base_url.join(&path)?;
                            }
                            audio_segment_urls.push(init_url);
                        } else {
                            audio_segment_urls.push(base_url.clone());
                        }
                    }
                    for su in sl.segment_urls.iter() {
                        if let Some(m) = &su.media {
                            let segment = base_url.join(m)?;
                            audio_segment_urls.push(segment);
                        }
                    }
                } else if audio_repr.SegmentTemplate.is_some() || audio.SegmentTemplate.is_some() {
                    // Here we are either looking at a Representation.SegmentTemplate, or a
                    // higher-level AdaptationSet.SegmentTemplate
                    let st;
                    if let Some(it) = &audio_repr.SegmentTemplate {
                        st = it;
                    } else if let Some(it) = &audio.SegmentTemplate {
                        st = it;
                    } else {
                        panic!("impossible");
                    }
                    if let Some(i) = &st.initialization {
                        opt_init = Some(i.to_string());
                    }
                    if let Some(m) = &st.media {
                        opt_media = Some(m.to_string());
                    }
                    if let Some(ts) = st.timescale {
                        timescale = ts;
                    }
                    if let Some(stl) = &st.SegmentTimeline {
                        // (3) SegmentTemplate with SegmentTimeline addressing mode
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            audio_segment_urls.push(base_url.join(&path)?);
                        }
                        if let Some(media) = opt_media {
                            let audio_path = resolve_url_template(&media, &dict);
                            let mut segment_time = 0;
                            let mut segment_duration;
                            let mut number = start_number;
                            for s in &stl.segments {
                                // the URLTemplate may be based on $Time$, or on $Number$
                                let dict = HashMap::from([("Time", segment_time.to_string()),
                                                          ("Number", number.to_string())]);
                                let path = resolve_url_template(&audio_path, &dict);
                                audio_segment_urls.push(base_url.join(&path)?);
                                number += 1;
                                if let Some(t) = s.t {
                                    segment_time = t;
                                }
                                segment_duration = s.d;
                                if let Some(r) = s.r {
                                    let mut count = 0i64;
                                    // FIXME perhaps we also need to account for startTime?
                                    let end_time = period_duration_secs * timescale as f64;
                                    loop {
                                        count += 1;
                                        // Exit from the loop after @r iterations (if @r is
                                        // positive). A negative value of the @r attribute indicates
                                        // that the duration indicated in @d attribute repeats until
                                        // the start of the next S element, the end of the Period or
                                        // until the next MPD update.
                                        if r >= 0 {
                                            if count > r {
                                                break;
                                            }
                                        } else if segment_time as f64 > end_time {
                                            break;
                                        }
                                        segment_time += segment_duration;
                                        let dict = HashMap::from([("Time", segment_time.to_string()),
                                                                  ("Number", number.to_string())]);
                                        let path = resolve_url_template(&audio_path, &dict);
                                        audio_segment_urls.push(base_url.join(&path)?);
                                        number += 1;
                                    }
                                }
                                segment_time += segment_duration;
                            }
                        } else {
                            return Err(anyhow!("SegmentTimeline without a media attribute"));
                        }
                    } else { // no SegmentTimeline element
                        // (4) SegmentTemplate@duration addressing mode or (5) SegmentTemplate@index addressing mode
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            audio_segment_urls.push(base_url.join(&path)?);
                        }
                        if let Some(media) = opt_media {
                            let audio_path = resolve_url_template(&media, &dict);
                            let timescale = st.timescale.unwrap_or(timescale);
                            let mut segment_duration: f64 = -1.0;
                            if let Some(d) = opt_duration {
                                // it was set on the Period.SegmentTemplate node
                                segment_duration = d;
                            }
                            if let Some(std) = st.duration {
                                segment_duration = std as f64 / timescale as f64;
                            }
                            if segment_duration < 0.0 {
                                return Err(anyhow!("Audio representation is missing SegmentTemplate @duration attribute"));
                            }
                            let total_number: u64 = (period_duration_secs / segment_duration).ceil() as u64;
                            let mut number = start_number;
                            for _ in 1..total_number {
                                let dict = HashMap::from([("Number", number.to_string())]);
                                let path = resolve_url_template(&audio_path, &dict);
                                let segment_uri = base_url.join(&path)?;
                                audio_segment_urls.push(segment_uri);
                                number += 1;
                            }
                        }
                    }
                } else {
                    return Err(anyhow!("Need either a SegmentBase or a SegmentTemplate node"));
                }
                // Concatenate the audio segments to a file on disk.
                // In DASH, the first segment contains necessary headers to generate a valid MP4 file,
                // so we should always abort if the first segment cannot be fetched. However, we could
                // tolerate loss of subsequent segments.
                let mut seen_urls: HashMap<Url, bool> = HashMap::new();
                for url in &audio_segment_urls {
                    // Don't download repeated URLs multiple times: they may be caused by a MediaRange parameter
                    // on the SegmentURL, which we are currently not handling correctly
                    // Example here
                    // http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd
                    if let Entry::Vacant(e) = seen_urls.entry(url.clone()) {
                        e.insert(true);
                        if url.scheme() == "data" {
                            return Err(anyhow!("data URLs currently unsupported"));
                        } else {
                            // We could download these segments in parallel using reqwest in async mode,
                            // though that might upset some servers.
                            let backoff = ExponentialBackoff::default();
                            let fetch = || {
                                client.get(url.clone())
                                // Don't use only "audio/*" in Accept header because some web servers
                                // (eg. media.axprod.net) are misconfigured and reject requests for
                                // valid audio content (eg .m4s)
                                    .header("Accept", "audio/*;q=0.9,*/*;q=0.5")
                                    .header("Referer", redirected_url.to_string())
                                    .send()?
                                    .bytes()
                                    .map_err(categorize_reqwest_error)
                            };
                            let dash_bytes = retry(backoff, fetch)
                                .context("Fetching DASH audio segment")?;
                            // eprintln!("Audio segment {} -> {} octets", url, dash_bytes.len());
                            if let Err(e) = tmpfile_audio.write_all(&dash_bytes) {
                                log::error!("Unable to write DASH audio data: {:?}", e);
                                return Err(anyhow!("Unable to write DASH audio data: {:?}", e));
                            }
                            have_audio = true;
                        }
                    }
                }
                tmpfile_audio.flush().map_err(|e| {
                    log::error!("Couldn't flush DASH audio file to disk: {:?}", e);
                    e
                })?;
                if let Ok(metadata) = fs::metadata(tmppath_audio.clone()) {
                    log::info!("Wrote {:.1}MB to DASH audio stream", metadata.len() as f64 / (1024.0 * 1024.0));
                }
            }
        }

        // Handle the AdaptationSet which contains video content
        let maybe_video_adaptation = period.adaptations.as_ref()
            .and_then(|a| a.iter().find(is_video_adaptation));
        if let Some(period_video) = maybe_video_adaptation {
            let mut video = period_video.clone();
            // Resolve a possible xlink:href.
            if let Some(href) = &video.href {
                if fetchable_xlink_href(href) {
                    let xlink_url;
                    if is_absolute_url(href) {
                        xlink_url = Url::parse(href)?;
                    } else {
                        // Note that we are joining against the original/redirected URL for the MPD, and
                        // not against the currently scoped BaseURL
                        xlink_url = redirected_url.join(href)?;
                    }
                    let xml = client.get(xlink_url)
                        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                        .header("Accept-language", "en-US,en")
                        .send()?
                        .text()
                        .context("Resolving XLink on AdaptationSet element")?;
                    let linked_adaptation: AdaptationSet = quick_xml::de::from_str(&xml)?;
                    video.clone_from(&linked_adaptation);
                }
            }
            // the AdaptationSet may have a BaseURL (eg the test BBC streams)
            if let Some(bu) = &video.BaseURL {
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base)?;
                } else {
                    base_url = base_url.join(&bu.base)?;
                }
            }
            // Start by resolving any xlink:href elements on Representation nodes, which we need to
            // do before the selection based on the @bandwidth attribute below.
            let mut representations = Vec::<Representation>::new();
            if let Some(reps) = video.representations {
                for r in reps.iter() {
                    if let Some(href) = &r.href {
                        if fetchable_xlink_href(href) {
                            let xlink_url;
                            if is_absolute_url(href) {
                                xlink_url = Url::parse(href)?;
                            } else {
                                xlink_url = redirected_url.join(href)?;
                            }
                            let xml = client.get(xlink_url)
                                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                                .header("Accept-language", "en-US,en")
                                .send()?
                                .text()
                                .context("Resolving XLink on Representation element")?;
                            let linked_representation: Representation = quick_xml::de::from_str(&xml)?;
                            representations.push(linked_representation);
                        }
                    } else {
                        representations.push(r.clone());
                    }
                }
            }
            if let Ok(video_repr) = representations.iter()
                .min_by_key(|x| x.width)
                .context("Finding video representation with lowest bandwith")
            {
                if let Some(bu) = &video_repr.BaseURL {
                    if is_absolute_url(&bu.base) {
                        base_url = Url::parse(&bu.base)?;
                    } else {
                        base_url = base_url.join(&bu.base)?;
                    }
                }
                let rid = match &video_repr.id {
                    Some(id) => id,
                    None => return Err(anyhow!("Missing @id on Representation node")),
                };
                let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
                if let Some(b) = &video_repr.bandwidth {
                    dict.insert("Bandwidth", b.to_string());
                }
                let mut opt_init: Option<String> = None;
                let mut opt_media: Option<String> = None;
                let mut opt_duration: Option<f64> = None;
                let mut timescale = 1;
                let mut start_number = 0;
                // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
                // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
                // nodes in an enclosed Representation node. Don't download media segments here, only
                // download for SegmentTemplate nodes that are children of a Representation node.
                if let Some(st) = &video.SegmentTemplate {
                    if let Some(i) = &st.initialization {
                        opt_init = Some(i.to_string());
                    }
                    if let Some(m) = &st.media {
                        opt_media = Some(m.to_string());
                    }
                    if let Some(d) = st.duration {
                        opt_duration = Some(d as f64);
                    }
                    if let Some(ts) = st.timescale {
                        timescale = ts;
                    }
                    if let Some(s) = st.startNumber {
                        start_number = s;
                    }
                }
                // Now the 6 possible addressing modes: SegmentBase@indexRange, SegmentList,
                // SegmentTemplate+SegmentTimeline, SegmentTemplate@duration, SegmentTemplate@index
                if let Some(sb) = &video_repr.SegmentBase {
                    // (1) SegmentBase@indexRange addressing mode
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path)?;
                            } else {
                                init_url = base_url.join(&path)?;
                            }
                            video_segment_urls.push(init_url);
                        }
                    }
                    // TODO: properly handle indexRange attribute
                    video_segment_urls.push(base_url.clone());
                } else if let Some(sl) = &video_repr.SegmentList {
                    // (2) SegmentList addressing mode
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path)?;
                            } else {
                                init_url = base_url.join(&path)?;
                            }
                            video_segment_urls.push(init_url);
                        } else {
                            video_segment_urls.push(base_url.clone());
                        }
                    }
                    for su in sl.segment_urls.iter() {
                        if let Some(m) = &su.media {
                            let segment = base_url.join(m)?;
                            video_segment_urls.push(segment);
                        }
                    }
                } else if video_repr.SegmentTemplate.is_some() || video.SegmentTemplate.is_some() {
                    // Here we are either looking at a Representation.SegmentTemplate, or a
                    // higher-level AdaptationSet.SegmentTemplate
                    let st;
                    if let Some(it) = &video_repr.SegmentTemplate {
                        st = it;
                    } else if let Some(it) = &video.SegmentTemplate {
                        st = it;
                    } else {
                        panic!("impossible");
                    }
                    if let Some(i) = &st.initialization {
                        opt_init = Some(i.to_string());
                    }
                    if let Some(m) = &st.media {
                        opt_media = Some(m.to_string());
                    }
                    if let Some(ts) = st.timescale {
                        timescale = ts;
                    }
                    if let Some(stl) = &st.SegmentTimeline {
                        // (3) SegmentTemplate with SegmentTimeline addressing mode
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            video_segment_urls.push(base_url.join(&path)?);
                        }
                        if let Some(media) = opt_media {
                            let video_path = resolve_url_template(&media, &dict);
                            let mut segment_time = 0;
                            let mut segment_duration;
                            let mut number = start_number;
                            for s in &stl.segments {
                                // the URLTemplate may be based on $Time$, or on $Number$
                                let dict = HashMap::from([("Time", segment_time.to_string()),
                                                          ("Number", number.to_string())]);
                                let path = resolve_url_template(&video_path, &dict);
                                video_segment_urls.push(base_url.join(&path)?);
                                number += 1;
                                if let Some(t) = s.t {
                                    segment_time = t;
                                }
                                segment_duration = s.d;
                                if let Some(r) = s.r {
                                    let mut count = 0i64;
                                    // FIXME perhaps we also need to account for startTime?
                                    let end_time = period_duration_secs * timescale as f64;
                                    loop {
                                        count += 1;
                                        // Exit from the loop after @r iterations (if @r is
                                        // positive). A negative value of the @r attribute indicates
                                        // that the duration indicated in @d attribute repeats until
                                        // the start of the next S element, the end of the Period or
                                        // until the next MPD update.
                                        if r >= 0 {
                                            if count > r {
                                                break;
                                            }
                                        } else if segment_time as f64 > end_time {
                                            break;
                                        }
                                        segment_time += segment_duration;
                                        let dict = HashMap::from([("Time", segment_time.to_string()),
                                                                  ("Number", number.to_string())]);
                                        let path = resolve_url_template(&video_path, &dict);
                                        video_segment_urls.push(base_url.join(&path)?);
                                        number += 1;
                                    }
                                }
                                segment_time += segment_duration;
                            }
                        } else {
                            return Err(anyhow!("SegmentTimeline without a media attribute"));
                        }
                    } else { // no SegmentTimeline element
                        // (4) SegmentTemplate@duration addressing mode or (5) SegmentTemplate@index addressing mode
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            video_segment_urls.push(base_url.join(&path)?);
                        }
                        if let Some(media) = opt_media {
                            let video_path = resolve_url_template(&media, &dict);
                            let timescale = st.timescale.unwrap_or(timescale);
                            let mut segment_duration: f64 = -1.0;
                            if let Some(d) = opt_duration {
                                // it was set on the Period.SegmentTemplate node
                                segment_duration = d;
                            }
                            if let Some(std) = st.duration {
                                segment_duration = std as f64 / timescale as f64;
                            }
                            if segment_duration < 0.0 {
                                return Err(anyhow!("Video representation is missing SegmentTemplate @duration attribute"));
                            }
                            let total_number: u64 = (period_duration_secs / segment_duration).ceil() as u64;
                            let mut number = start_number;
                            for _ in 1..total_number {
                                let dict = HashMap::from([("Number", number.to_string())]);
                                let path = resolve_url_template(&video_path, &dict);
                                let segment_uri = base_url.join(&path)?;
                                video_segment_urls.push(segment_uri);
                                number += 1;
                            }
                        }
                    }
                } else {
                    return Err(anyhow!("Need either a SegmentBase or a SegmentTemplate node"));
                }
                // Now fetch the video segments and write them to the requested file path
                let mut seen_urls: HashMap<Url, bool> = HashMap::new();
                for url in &video_segment_urls {
                    // Don't download repeated URLs multiple times: they may be caused by a MediaRange parameter
                    // on the SegmentURL, which we are currently not handling correctly
                    // Example here
                    // http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd
                    if let Entry::Vacant(e) = seen_urls.entry(url.clone()) {
                        e.insert(true);
                        let backoff = ExponentialBackoff::default();
                        let fetch = || {
                            client.get(url.clone())
                                .header("Accept", "video/*")
                                .header("Referer", redirected_url.to_string())
                                .send()?
                                .bytes()
                                .map_err(categorize_reqwest_error)
                        };
                        let dash_bytes = retry_notify(backoff, fetch, notify_transient)
                            .context("Fetching DASH video segment")?;
                        // eprintln!("Video segment {} -> {} octets", url, dash_bytes.len());
                        if let Err(e) = tmpfile_video.write_all(&dash_bytes) {
                            return Err(anyhow!("Unable to write video data: {:?}", e));
                        }
                        have_video = true;
                    }
                }
                tmpfile_video.flush().map_err(|e| {
                    log::error!("Couldn't flush video file to disk: {:?}", e);
                    e
                })?;
                if let Ok(metadata) = fs::metadata(tmppath_video.clone()) {
                    log::info!("Wrote {:.1}MB to DASH video file", metadata.len() as f64 / (1024.0 * 1024.0));
                }
            } else {
                return Err(anyhow!("Couldn't find lowest bandwidth video stream in DASH manifest"));
            }
        }
    }
    // Our final output file is either a mux of the audio and video streams, if both are present, or just
    // the audio stream, or just the video stream.
    if have_audio && have_video {
        mux_audio_video(&tmppath_audio, &tmppath_video, path)?;
        fs::remove_file(tmppath_audio)?;
        fs::remove_file(tmppath_video)?;
    } else if have_audio {
        fs::rename(&tmppath_audio, &path)?;
    } else if have_video {
        fs::rename(&tmppath_video, &path)?;
    } else {
        return Err(anyhow!("no audio or video streams found"));
    }
    // As per https://www.freedesktop.org/wiki/CommonExtendedAttributes/, set extended filesystem
    // attributes indicating metadata such as the origin URL, title, source and copyright, if
    // specified in the MPD manifest. This functionality is only active on platforms where the xattr
    // crate supports extended attributes (currently Linux, MacOS, FreeBSD, and NetBSD); on
    // unsupported platforms it's a no-op.
    let origin_url = Url::parse(mpd_url)
        .context("Can't parse MPD URL")?;
    // Don't record the origin URL if it contains sensitive information such as passwords
    #[allow(clippy::collapsible_if)]
    if origin_url.username().is_empty() && origin_url.password().is_none() {
        #[cfg(target_family = "unix")]
        if xattr::set(&path, "user.xdg.origin.url", mpd_url.as_bytes()).is_err() {
            log::info!("Failed to set user.xdg.origin.url xattr on output file");
        }
    }
    if let Some(pi) = mpd.ProgramInformation {
        if let Some(t) = pi.Title {
            if let Some(tc) = t.content {
                #[cfg(target_family = "unix")]
                if xattr::set(&path, "user.dublincore.title", tc.as_bytes()).is_err() {
                    log::info!("Failed to set user.dublincore.title xattr on output file");
                }
            }
        }
        if let Some(source) = pi.Source {
            if let Some(sc) = source.content {
                #[cfg(target_family = "unix")]
                if xattr::set(&path, "user.dublincore.source", sc.as_bytes()).is_err() {
                    log::info!("Failed to set user.dublincore.source xattr on output file");
                }
            }
        }
        if let Some(copyright) = pi.Copyright {
            if let Some(cc) = copyright.content {
                #[cfg(target_family = "unix")]
                if xattr::set(&path, "user.dublincore.rights", cc.as_bytes()).is_err() {
                    log::info!("Failed to set user.dublincore.rights xattr on output file");
                }
            }
        }
    }
    Ok(())
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
    fn test_resolve_url_template() {
        use std::collections::HashMap;
        use crate::resolve_url_template;

        assert_eq!(resolve_url_template("AA$Time$BB", &HashMap::from([("Time", "ZZZ".to_string())])),
                   "AAZZZBB");
        assert_eq!(resolve_url_template("AA$Number%06d$BB", &HashMap::from([("Number", "42".to_string())])),
                   "AA000042BB");
        let dict = HashMap::from([("RepresentationID", "640x480".to_string()),
                                  ("Number", "42".to_string()),
                                  ("Time", "ZZZ".to_string())]);
        assert_eq!(resolve_url_template("AA/$RepresentationID$/segment-$Number%05d$.mp4", &dict),
                   "AA/640x480/segment-00042.mp4");
    }


    #[test]
    fn test_parse_xs_duration() {
        use std::time::Duration;
        use crate::parse_xs_duration;

        assert!(parse_xs_duration("").is_err());
        assert!(parse_xs_duration("foobles").is_err());
        assert_eq!(parse_xs_duration("PT3H11M53S").ok(), Some(Duration::new(11513, 0)));
        assert_eq!(parse_xs_duration("PT30M38S").ok(), Some(Duration::new(1838, 0)));
        assert_eq!(parse_xs_duration("PT0H10M0.00S").ok(), Some(Duration::new(600, 0)));
        assert_eq!(parse_xs_duration("PT1.5S").ok(), Some(Duration::new(1, 500_000)));
        assert_eq!(parse_xs_duration("PT0S").ok(), Some(Duration::new(0, 0)));
        // This test currently fails due to a bug in the iso8601 crate
        // assert_eq!(parse_xs_duration("PT344S").ok(), Some(Duration::new(344, 0)));
        assert_eq!(parse_xs_duration("PT1H0.040S").ok(), Some(Duration::new(3600, 40_000)));
        assert_eq!(parse_xs_duration("PT00H03M30SZ").ok(), Some(Duration::new(210, 0)));
        assert_eq!(parse_xs_duration("P0Y0M0DT0H4M20.880S").ok(), Some(Duration::new(260, 880_000)));
    }
}
