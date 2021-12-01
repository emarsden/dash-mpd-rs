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
//! ## Limitations
//! 
//! - This crate does not support content encrypted with DRM such as Encrypted Media Extensions (EME) and
//!   Media Source Extension (MSE)
//! - Currently no download support for dynamic MPD manifests, that are used for live streaming/OTT TV
//! - No support for subtitles (eg. WebVTT streams)
//
// 
//
// Reference libdash library: https://github.com/bitmovin/libdash
//   https://github.com/bitmovin/libdash/blob/master/libdash/libdash/source/xml/Node.cpp
// The DASH code in VLC: https://code.videolan.org/videolan/vlc/-/tree/master/modules/demux/dash
// Streamlink source code: https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/dash_manifest.py

// TODO: better retry handling (distinguish HTTP 404 from 301)
// TODO: allow user to specify preference for selecting representation (highest quality, lowest quality, etc.)
// TODO: handle dynamic MPD as per https://livesim.dashif.org/livesim/mup_30/testpic_2s/Manifest.mpd
// TODO: handle indexRange attribute, as per https://dash.akamaized.net/dash264/TestCasesMCA/dolby/2/1/ChID_voices_71_768_ddp.mpd



#![allow(non_snake_case)]

/// If library feature `libav` is enabled, muxing support (combining audio and video streams, which
/// are often separated out in DASH streams) is provided by ffmpeg's libav library, via the
/// `ac_ffmpeg` crate. Otherwise, muxing is implemented by calling `ffmpeg` as a subprocess.
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
use std::time::Duration;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use tempfile::NamedTempFile;
use url::Url;
use regex::Regex;
use backoff::{retry, retry_notify, ExponentialBackoff, Error};
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

/// The first media segment in a sequence of Segments. Subsequent segments can be concatenate to this
/// segment to produce a media stream. 
#[derive(Debug, Deserialize, Clone)]
pub struct Initialization {
    pub sourceURL: Option<String>,
    pub range: Option<String>,
}

/// A URI string that specifies one or more common locations for Segments and other resources.
#[derive(Debug, Deserialize, Clone)]
pub struct BaseURL {
    #[serde(rename = "$value")]
    pub base: String,
    /// Elements with the same `@serviceLocation` value are likely to have their URLs resolve to
    /// services at a common network location, for example the same CDN. 
    serviceLocation: Option<String>,
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
    pub id: String,
    // The specification says that @mimeType is mandatory, but it's not always present on
    // akamaized.net MPDs
    pub mimeType: Option<String>,
    /// An RFC6381 string, <https://tools.ietf.org/html/rfc6381>
    pub codecs: Option<String>,
    pub contentType: Option<String>,
    /// If present, this attribute is expected to be set to "progressive".
    pub scanType: Option<String>,
    pub bandwidth: u64,
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
    pub representations: Vec<Representation>,
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
    #[serde(rename = "xlink:href")]
    pub href: Option<String>,
    #[serde(rename = "xlink:actuate")]
    pub actuate: Option<bool>,
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

// From https://dashif.org/docs/DASH-IF-IOP-v4.3.pdf:
// "For the avoidance of doubt, only %0[width]d is permitted and no other identifiers. The reason 
// is that such a string replacement can be easily implemented without requiring a specific library."
//
// Instead of pulling in C printf() or a reimplementation such as the printf_compat crate, we reimplement
// this functionality directly.
//
// Example template: "$RepresentationID$/$Number%06d$.m4s"
fn resolve_url_template(template: &str, params: &HashMap<&str, &String>) -> String {
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


fn notify_transient<E: std::fmt::Debug>(err: E, dur: Duration) {
    eprintln!("Transient error at {:?}: {:?}", dur, err);
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
/// let client = reqwest::blocking::Client::builder().build().unwrap();
/// let url = "http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-ctv-events.mpd";
/// if let Err(e) = fetch_mpd(&client, url, "/tmp/BBC-MPD-test.mp4") {
///    eprintln!("Error downloading DASH MPD file: {:?}", e);
/// }
/// ```
pub fn fetch_mpd(client: &HttpClient,
                 mpd_url: &str,
                 path: &str) -> Result<()> {
    let fetch = || {
        client.get(mpd_url)
            .header("Accept", "application/dash+xml")
            .send()
            .map_err(Error::Transient)
    };
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
    let mut base_url = redirected_url.clone();
    // We may have several BaseURL tags in the MPD, but we don't currently implement failover
    if let Some(bases) = &mpd.base_urls {
        if is_absolute_url(&bases[0].base) {
            base_url = Url::parse(&bases[0].base)?;
        } else {
            base_url = redirected_url.join(&bases[0].base)?;
        }
    }
    let period = &mpd.periods[0];
    // A BaseURL could be specified for each Period
    if let Some(bu) = &period.BaseURL {
        if is_absolute_url(&bu.base) {
            base_url = Url::parse(&bu.base)?;
        } else {
            base_url = base_url.join(&bu.base)?;
        }
    }
    let mut video_segment_urls = Vec::new();
    let mut audio_segment_urls = Vec::new();
    let tmppath_video = tmp_file_path("dashmpd-video-track");
    let tmppath_audio = tmp_file_path("dashmpd-audio-track");
    
    // Handle the AdaptationSet with contentType="audio". Note that some streams don't separate out
    // audio and video streams, they have segments of .mp4 that can directly be appended
    let maybe_audio_adaptation = match &period.adaptations {
        Some(a) => a.iter().find(is_audio_adaptation),
        None => None,
    };
    if let Some(audio) = maybe_audio_adaptation {
        // the AdaptationSet may have a BaseURL (eg the test BBC streams). We use a scoped local variable
        // to make sure we don't "corrupt" the base_url for the video segments. 
        let mut base_url = base_url.clone();
        if let Some(bu) = &audio.BaseURL {
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)?;
            } else {
                base_url = base_url.join(&bu.base)?;
            }
        }
        if let Ok(audio_repr) = audio.representations.iter()
            .min_by_key(|x| x.bandwidth)
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
            let maybe_st = audio.SegmentTemplate.as_ref().or_else(|| audio_repr.SegmentTemplate.as_ref());
            if let Some(st) = maybe_st {
                let dict = HashMap::from([("RepresentationID", &audio_repr.id)]);
                if let Some(init) = &st.initialization {
                    let audio_init = resolve_url_template(init, &dict);
                    audio_segment_urls.push(base_url.join(&audio_init)?);
                }
                if let Some(media) = &st.media {
                    let audio_path = resolve_url_template(media, &dict);
                    if let Some(stl) = &st.SegmentTimeline {
                        let mut segment_time = 0;
                        let mut segment_duration;
                        let mut number = st.startNumber.unwrap_or(0);
                        for s in &stl.segments {
                            let time_str = format!("{}", segment_time);
                            let number_str = format!("{}", number);
                            // the URLTemplate may be based on $Time$, or on $Number$
                            let dict = HashMap::from([("Time", &time_str),
                                                      ("Number", &number_str)]);
                            let path = resolve_url_template(&audio_path, &dict);
                            audio_segment_urls.push(base_url.join(&path)?);
                            number += 1;
                            if let Some(t) = s.t {
                                segment_time = t;
                            }
                            segment_duration = s.d;
                            if let Some(r) = s.r {
                                for _ in 0..r {
                                    segment_time += segment_duration;
                                    let time_str = format!("{}", segment_time);
                                    let number_str = format!("{}", number);
                                    let dict = HashMap::from([("Time", &time_str),
                                                              ("Number", &number_str)]);
                                    let path = resolve_url_template(&audio_path, &dict);
                                    audio_segment_urls.push(base_url.join(&path)?);
                                }
                            }
                            segment_time += segment_duration;
                        }
                    } else {
                        // Segments are named using $Number$.
                        // The period_duration is specified either by the <Period> duration attribute, or by the
                        // mediaPresentationDuration of the top-level MPD node. 
                        let mut period_duration: f64 = 0.0;
                        if let Some(d) = mpd.mediaPresentationDuration {
                            period_duration = d.as_secs_f64();
                        }
                        if let Some(d) = &period.duration {
                            period_duration = d.as_secs_f64();
                        }
                        // the SegmentTemplate duration is encoded as an u64
                        let timescale = st.timescale.unwrap_or(1);
                        let segment_duration: f64;
                        if let Some(std) = st.duration {
                            segment_duration = std as f64 / timescale as f64;
                        } else {
                            return Err(anyhow!("Missing SegmentTemplate duration attribute"));
                        }
                        let total_number: u64 = (period_duration / segment_duration).ceil() as u64;
                        let mut number = st.startNumber.unwrap_or(0);
                        for _ in 1..total_number {
                            let path = resolve_url_template(&audio_path, &HashMap::from([("Number", &format!("{}", number))]));
                            let segment_uri = base_url.join(&path)?;
                            audio_segment_urls.push(segment_uri);
                            number += 1;
                        }
                    }
                }
            } else {
                // We don't have a SegmentTemplate, so are using a Option<SegmentBase> plus perhaps
                // a SegmentList of SegmentURL
                if let Some(sb) = audio_repr.SegmentBase.as_ref() {
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let init_url;
                            if is_absolute_url(su) {
                                init_url = Url::parse(su)?;
                            } else {
                                init_url = base_url.join(su)?;
                            }
                            audio_segment_urls.push(init_url);
                        } else {
                            // TODO: need to properly handle indexRange attribute
                            audio_segment_urls.push(base_url.clone());
                        }
                    }
                }
                if let Some(sl) = &audio_repr.SegmentList {
                    // look for optional initialization segment
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            let init_url;
                            if is_absolute_url(su) {
                                init_url = Url::parse(su)?;
                            } else {
                                init_url = base_url.join(su)?;
                            }
                            audio_segment_urls.push(init_url);
                        }
                    }
                    for su in sl.segment_urls.iter() {
                        if let Some(m) = &su.media {
                            let segment = base_url.join(m)?;
                            audio_segment_urls.push(segment);
                        }
                    }
                }
            }
            // Concatenate the audio segments to a file on disk
            let mut tmpfile_audio = File::create(tmppath_audio.clone())
                .context("Creating audio tmpfile")?;
            // In DASH, the first segment contains necessary headers to generate a valid MP4 file,
            // so we should always abort if the first segment cannot be fetched. However, we can
            // tolerate loss of subsequent segments.
            let mut seen_urls: HashMap<Url, bool> = HashMap::new();
            for url in audio_segment_urls {
                // Don't download repeated URLs multiple times: they may be caused by a MediaRange parameter
                // on the SegmentURL, which we are currently not handling correctly
                // Example here
                // http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd
                if let Entry::Vacant(e) = seen_urls.entry(url.clone()) {
                    e.insert(true);
                    let dash_bytes;
                    if url.scheme() == "data" {
                        panic!("data URLs currently unsupported");
                    } else {
                        // We could download these segments in parallel using reqwest in async mode,
                        // though that might upset some servers.
                        let backoff = ExponentialBackoff::default();
                        let fetch = || {
                            client.get(url.clone())
                            // Don't use only "audio/*" in Accept header because some web servers
                            // (eg. media.axprod.net) are misconfigured and reject requests for .m4s
                            // content
                                .header("Accept", "audio/*;q=0.9,*/*;q=0.5")
                                .header("Referer", redirected_url.to_string())
                                .send()?
                                .bytes()
                                .map_err(Error::Transient)
                        };
                        dash_bytes = retry(backoff, fetch)
                            .context("Fetching DASH audio segment")?;
                        // eprintln!("Audio segment {} -> {} octets", url, dash_bytes.len());
                        if let Err(e) = tmpfile_audio.write_all(&dash_bytes) {
                            log::error!("Unable to write DASH audio data: {:?}", e);
                            return Err(anyhow!("Unable to write DASH audio data: {:?}", e));
                        }
                    }
                }
            }
            tmpfile_audio.flush().map_err(|e| {
                log::error!("Couldn't flush DASH audio file to disk: {:?}", e);
                e
            })?;
            if let Ok(metadata) = fs::metadata(tmppath_audio.clone()) {
                log::info!("Wrote {}MB to DASH audio stream", metadata.len() / (1024 * 1024));
            }
        }
    }
        
    // Handle the AdaptationSet with contentType="video", or which includes a member representation
    // that has contentType="video" or mimeType="video"
    let maybe_video_adaptation = period.adaptations.as_ref().and_then(|a| a.iter().find(is_video_adaptation));
    if let Some(video) = maybe_video_adaptation {
        // the AdaptationSet may have a BaseURL (eg the test BBC streams)
        if let Some(bu) = &video.BaseURL {
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)?;
            } else {
                base_url = base_url.join(&bu.base)?;
            }
        }
        if let Ok(video_repr) = video.representations.iter().min_by_key(|x| x.width)
            .context("Finding video representation with lowest bandwith")
        {
            if let Some(bu) = &video_repr.BaseURL {
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base)?;
                } else {
                    base_url = base_url.join(&bu.base)?;
                }
            }
            let maybe_st = video.SegmentTemplate.as_ref().or_else(|| video_repr.SegmentTemplate.as_ref());
            // We can either have a SegmentTemplate with SegmentTimeline, or use of SegmentBase + SegmentURL seq
            if let Some(st) = maybe_st.as_ref() {
                if let Some(init) = &st.initialization {
                    let init = resolve_url_template(init, &HashMap::from([("RepresentationID", &video_repr.id)]));
                    let init_uri = base_url.join(&init)?;
                    video_segment_urls.push(init_uri);
                }
                
                // https://www.unified-streaming.com/blog/stop-numbering-underappreciated-power-dashs-segmenttimeline
                // The ​‘t’-element represents the timestamp of the first segment that has an exact
                // duration specified by the ​‘d’-element, whereas the ​‘r’-element tells the player how
                // many subsequent segments with the same duration are available.
                //         <SegmentTimeline>
                //           <S t="0" d="50" r="1933" />
                //           <S d="49" />
                //         </SegmentTimeline>
                if let Some(media) = &st.media {
                    let media_path = resolve_url_template(media, &HashMap::from([("RepresentationID", &video_repr.id)]));
                    if let Some(stl) = &st.SegmentTimeline {
                        let mut segment_time = 0;
                        let mut segment_duration;
                        let mut number = st.startNumber.unwrap_or(0);
                        for s in &stl.segments {
                            let time_str = format!("{}", segment_time);
                            let number_str = format!("{}", number);
                            // the URLTemplate may be based on $Time$, or on $Number$
                            let dict = HashMap::from([("Time", &time_str),
                                                      ("Number", &number_str)]);
                            let path = resolve_url_template(&media_path, &dict);
                            let segment_uri = base_url.join(&path)?;
                            video_segment_urls.push(segment_uri);
                            number += 1;
                            if let Some(t) = s.t {
                                segment_time = t;
                            }
                            segment_duration = s.d;
                            if let Some(r) = s.r {
                                for _ in 0..r {
                                    segment_time += segment_duration;
                                    let time_str = format!("{}", segment_time);
                                    let number_str = format!("{}", number);
                                    let dict = HashMap::from([("Time", &time_str),
                                                              ("Number", &number_str)]);
                                    let path = resolve_url_template(&media_path, &dict);
                                    video_segment_urls.push(base_url.join(&path)?);
                                    number += 1;
                                }
                            }
                            segment_time += segment_duration;
                        }
                    } else {
                        // This is the case when we don't have a SegmentTimeline, see for example the BBC test case
                        // http://rdmedia.bbc.co.uk/dash/ondemand/bbb/2/client_manifest-common_init.mpd
                        let mut period_duration: f64 = 0.0;
                        if let Some(d) = mpd.mediaPresentationDuration {
                            period_duration = d.as_secs_f64();
                        }
                        if let Some(d) = &period.duration {
                            period_duration = d.as_secs_f64();
                        }
                        // FIXME the duration of a period may also be determined implicitly by the start time
                        // of the following period (see section 8 Period timing in
                        // https://dashif-documents.azurewebsites.net/Guidelines-TimingModel/master/Guidelines-TimingModel.html)
                        let timescale = st.timescale.unwrap_or(1);
                        let segment_duration: f64;
                        if let Some(std) = st.duration {
                            segment_duration = std as f64 / timescale as f64;
                        } else {
                            return Err(anyhow!("Missing SegmentTemplate duration attribute"));
                        }
                        let total_number: u64 = (period_duration / segment_duration).ceil() as u64;
                        let mut number = st.startNumber.unwrap_or(0);
                        for _ in 1..total_number {
                            let path = resolve_url_template(&media_path, &HashMap::from([("Number", &format!("{}", number))]));
                            let segment_uri = base_url.join(&path)?;
                            video_segment_urls.push(segment_uri);
                            number += 1;
                        }
                    }
                }
            } else {
                // We don't have a SegmentTemplate, so are using a Option<SegmentBase> plus perhaps
                // a SegmentList of SegmentURL
                if let Some(sb) = video_repr.SegmentBase.as_ref() {
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let init_url;
                            if is_absolute_url(su) {
                                init_url = Url::parse(su)?;
                            } else {
                                init_url = base_url.join(su)?;
                            }
                            video_segment_urls.push(init_url);
                        }
                    }
                }
                if let Some(sl) = &video_repr.SegmentList {
                    // look for optional initialization segment
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            let init_url;
                            if is_absolute_url(su) {
                                init_url = Url::parse(su)?;
                            } else {
                                init_url = base_url.join(su)?;
                            }
                            video_segment_urls.push(init_url);
                        }
                    }
                    for su in sl.segment_urls.iter() {
                        if let Some(m) = &su.media {
                            let segment = base_url.join(m)?;
                            video_segment_urls.push(segment);
                        }
                    }
                } else if let Some(sb) = &video_repr.SegmentBase {
                    // We don't have a SegmentTemplate, but have a SegmentBase
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let init_url;
                            if is_absolute_url(su) {
                                init_url = Url::parse(su)?;
                            } else {
                                init_url = base_url.join(su)?;
                            }
                            video_segment_urls.push(init_url);
                        } else {
                            // TODO: need to properly handle indexRange attribute
                            video_segment_urls.push(base_url);
                        }
                    }
                }
            }
            // Now fetch the segments and write them to the requested file path
            let mut tmpfile_video = File::create(tmppath_video.clone())
                .context("Creating video tmpfile")?;
            let mut seen_urls: HashMap<Url, bool> = HashMap::new();
            for url in video_segment_urls {
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
                            .map_err(Error::Transient)
                    };
                    let dash_bytes = retry_notify(backoff, fetch, notify_transient)
                        .context("Fetching DASH video segment")?;
                    if let Err(e) = tmpfile_video.write_all(&dash_bytes) {
                        return Err(anyhow!("Unable to write video data: {:?}", e));
                    }
                }
            }
            tmpfile_video.flush().map_err(|e| {
                log::error!("Couldn't flush video file to disk: {:?}", e);
                e
            })?;
            if let Ok(metadata) = fs::metadata(tmppath_video.clone()) {
                log::info!("Wrote {}MB to DASH video file", metadata.len() / (1024 * 1024));
            }
        } else {
            // FIXME look for SegmentTemplates here directly (not enclosed in an AdaptationSet or Representation)
            return Err(anyhow!("Couldn't find lowest bandwidth video stream in DASH manifest"));
        }
    }

    // Our final output file is either a mux of the audio and video streams, if both are present, or just
    // the audio stream, or just the video stream. 
    if maybe_audio_adaptation.is_some() && maybe_video_adaptation.is_some() {
        mux_audio_video(&tmppath_audio, &tmppath_video, path)?;
        fs::remove_file(tmppath_audio)?;
        fs::remove_file(tmppath_video)?;
    } else if maybe_audio_adaptation.is_some() {
        fs::rename(&tmppath_audio, &path)?;
    } else if maybe_video_adaptation.is_some() {
        fs::rename(&tmppath_video, &path)?; 
    } else {
        return Err(anyhow!("no audio or video streams found"));
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
        
        assert_eq!(resolve_url_template("AA$Time$BB", &HashMap::from([("Time", &"ZZZ".to_owned())])),
                   "AAZZZBB");
        assert_eq!(resolve_url_template("AA$Number%06d$BB", &HashMap::from([("Number", &"42".to_owned())])),
                   "AA000042BB");
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
        assert_eq!(parse_xs_duration("PT1H0.040S").ok(), Some(Duration::new(3600, 40_000))); 
        assert_eq!(parse_xs_duration("PT00H03M30SZ").ok(), Some(Duration::new(210, 0)));
        assert_eq!(parse_xs_duration("P0Y0M0DT0H4M20.880S").ok(), Some(Duration::new(260, 880_000)));
    }
}
