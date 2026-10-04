//! A Rust library for downloading media content from a DASH MPD manifest, as used for on-demand
//! replay of TV content and video streaming services.
//!
//! [DASH](https://en.wikipedia.org/wiki/Dynamic_Adaptive_Streaming_over_HTTP) (dynamic adaptive
//! streaming over HTTP), also called MPEG-DASH, is a technology used for media streaming over the
//! web, commonly used for video on demand (VOD) services. The Media Presentation Description (MPD)
//! is a description of the resources (manifest or “playlist”) forming a streaming service, that a
//! DASH client uses to determine which assets to request in order to perform adaptive streaming of
//! the content. DASH MPD manifests can be used both with content encoded as MPEG and as WebM.
//!
//! The DASH format is formally defined in ISO/IEC standard 23009-1:2022. This version of the
//! standard is [available for free
//! online](https://standards.iso.org/ittf/PubliclyAvailableStandards/c083314_ISO_IEC%2023009-1_2022(en).zip).
//! XML schema files are [available for no cost from
//! ISO](https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/). When
//! MPD files in practical use diverge from the formal standard, this library prefers to
//! interoperate with existing practice.
//!
//! This library provides support for downloading content (audio or video) described by an MPD
//! manifest. This involves selecting the alternative with the most appropriate encoding (in terms
//! of bitrate, codec, etc.), fetching segments of the content using HTTP or HTTPS requests (this
//! functionality depends on the `reqwest` crate) and muxing audio and video segments together
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
//! - Subtitles: support for WebVTT, SRT, STPP, TTML, tx3g and SMIL streams, either provided as a single media
//!   stream or as a fragmented MP4 stream.
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

// TODO: improve handling of dynamic manifests, as per https://livesim.dashif.org/livesim/mup_30/testpic_2s/Manifest.mpd
// TODO: handle indexRange attribute, as per https://dash.akamaized.net/dash264/TestCasesMCA/dolby/2/1/ChID_voices_71_768_ddp.mpd
// TODO: implement MPD Patch support when downloading, with test cases from https://github.com/ab2022/mpddiffs/tree/main


#![allow(non_snake_case)]

/// If library feature `libav` is enabled, muxing support (combining audio and video streams, which
/// are often separated out in DASH streams) is provided by ffmpeg's libav library, via the
/// `ac_ffmpeg` crate. Otherwise, muxing is implemented by calling `mkvmerge`, `ffmpeg` or `vlc` as
/// a subprocess.
pub mod media;
#[cfg(feature = "libav")]
mod libav;
#[cfg(not(feature = "libav"))]
pub mod ffmpeg;
pub mod sidx;
pub mod fetch;
pub mod decryption;
pub mod subtitles;
pub mod stpp;
pub mod vtt;

#[cfg(feature = "libav")]
use crate::libav::{mux_audio_video, copy_video_to_container, copy_audio_to_container};
#[cfg(not(feature = "libav"))]
use crate::ffmpeg::{mux_audio_video, copy_video_to_container, copy_audio_to_container};

#[cfg(all(feature = "sandbox", target_os = "linux"))]
pub mod sandbox;


pub use crate::fetch::{DashDownloader, ProgressObserver, parse_resolving_xlinks};
