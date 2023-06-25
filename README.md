# dash-mpd

A Rust library for parsing, serializing and downloading media content from a DASH MPD file, as used
by video services such as on-demand replay of TV content and video streaming services like YouTube.
Allows both parsing of a DASH manifest (XML format) to Rust structs (deserialization) and
programmatic generation of an MPD manifest (serialization). The library also allows you to download
media content from a streaming server.

[![Crates.io](https://img.shields.io/crates/v/dash-mpd)](https://crates.io/crates/dash-mpd)
[![Released API docs](https://docs.rs/dash-mpd/badge.svg)](https://docs.rs/dash-mpd/)
[![CI](https://github.com/emarsden/dash-mpd-rs/workflows/build/badge.svg)](https://github.com/emarsden/dash-mpd-rs/workflows/build/badge.svg)
[![Dependency status](https://deps.rs/repo/github/emarsden/dash-mpd-rs/status.svg)](https://deps.rs/repo/github/emarsden/dash-mpd-rs)
[![LICENSE](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-MIT)

[DASH](https://en.wikipedia.org/wiki/Dynamic_Adaptive_Streaming_over_HTTP) (dynamic adaptive
streaming over HTTP), also called MPEG-DASH, is a technology used for media streaming over the web,
commonly used for video on demand (VOD) services. The Media Presentation Description (MPD) is a
description of the resources (manifest or “playlist”) forming a streaming service, that a DASH
client uses to determine which assets to request in order to perform adaptive streaming of the
content. DASH MPD manifests can be used both with content encoded as H.264/MPEG and as WebM, and
with file segments using either MPEG-2 Transport Stream (M2TS) container format or fragmented MPEG-4
(also called CFF). There is a good explanation of adaptive bitrate video streaming at
[howvideo.works](https://howvideo.works/#dash).

This library provides a serde-based parser (deserializer) and serializer for the DASH MPD format, as
formally defined in ISO/IEC standard 23009-1:2019. XML schema files are [available for no cost from
ISO](https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/). The library
also provides non-exhaustive support for certain DASH extensions such as the DVB-DASH and HbbTV
(Hybrid Broadcast Broadband TV) profiles. When MPD files in practical use diverge from the formal
standard(s), this library prefers to interoperate with existing practice.

If the library feature `fetch` is enabled (which it is by default), the library also provides
support for downloading content (audio or video) described by an MPD manifest. This involves
selecting the alternative with the most appropriate encoding (in terms of bitrate, codec, etc.),
fetching segments of the content using HTTP or HTTPS requests (this functionality depends on the
`reqwest` crate) and muxing audio and video segments together.

If the library feature `libav` is enabled, muxing support (combining audio and video streams, which
are often separated out in DASH streams) is provided by ffmpeg’s libav library, via the `ac_ffmpeg`
crate. Otherwise, muxing is implemented by calling an external muxer, mkvmerge (from the
[MkvToolnix](https://mkvtoolnix.download/) suite), [ffmpeg](https://ffmpeg.org/),
[vlc](https://www.videolan.org/vlc/) or [MP4Box](https://github.com/gpac/gpac/wiki/MP4Box) as a
subprocess. Note that these commandline applications implement a number of checks and workarounds to
fix invalid input streams that tend to exist in the wild. Some of these workarounds are implemented
here when using libav as a library, but not all of them, so download support tends to be more robust
with the default configuration (using an external application as a subprocess).

The choice of external muxer depends on the filename extension of the path supplied to `download_to()`
(will be `.mp4` if you call `download()`):

- `.mkv`: call mkvmerge first, then if that fails call ffmpeg, then try MP4Box
- `.mp4`: call ffmpeg first, then if that fails call vlc, then try MP4Box
- other: try ffmpeg, which supports many container formats, then try MP4Box


## DASH features supported

- VOD (static) stream manifests
- Multi-period content
- XLink elements (only with actuate=onLoad semantics), including resolve-to-zero
- All forms of segment index info: SegmentBase@indexRange, SegmentTimeline,
  SegmentTemplate@duration, SegmentTemplate@index, SegmentList.
- Media containers of types supported by mkvmerge, ffmpeg, VLC or MP4Box (this includes Matroska,
  ISO-BMFF / CMAF / MP4, WebM, MPEG-2 TS).
- WebVTT, TTML and SMIL subtitles (preliminary support). There is some support for subtitles that
  are made available in wvtt format, that will be converted to SRT format using the MP4Box
  commandline utility (from the [GPAC](https://gpac.wp.imt.fr/) project), if it is installed.


## Limitations / unsupported features

- We can't download content from dynamic MPD manifests, that are used for live streaming/OTT TV
- Content encrypted with ContentProtection DRM mechanisms such as Clear Key, FairPlay, PlayReady, Widevine
- No support for XLink with actuate=onRequest semantics


## Usage

To **parse** (deserialize) the contents of an MPD manifest into Rust structs:

```rust
use std::time::Duration;
use dash_mpd::{MPD, parse};

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .build()
        .expect("creating HTTP client");
    let xml = client.get("https://rdmedia.bbc.co.uk/testcard/vod/manifests/avc-ctv-stereo-en.mpd")
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send()
        .expect("requesting MPD content")
        .text()
        .expect("fetching MPD content");
    let mpd: MPD = parse(&xml)
        .expect("parsing MPD");
    if let Some(pi) = mpd.ProgramInformation {
        if let Some(title) = pi.Title {
            println!("Title: {:?}", title.content);
        }
        if let Some(source) = pi.Source {
            println!("Source: {:?}", source.content);
        }
    }
    for p in mpd.periods {
        if let Some(d) = p.duration {
            println!("Contains Period of duration {d:?}");
        }
    }
}
```

See example
[dash_stream_info.rs](https://github.com/emarsden/dash-mpd-rs/blob/main/examples/dash_stream_info.rs)
for more information.


To **generate an MPD manifest programmatically**:

```rust
use dash_mpd::{MPD, ProgramInformation, Title};

fn main() {
   let pi = ProgramInformation {
       Title: Some(Title { content: Some("My serialization example".into()) }),
       lang: Some("eng".into()),
       moreInformationURL: Some("https://github.com/emarsden/dash-mpd-rs".into()),
       ..Default::default()
   };
   let mpd = MPD {
       mpdtype: Some("static".into()),
       xmlns: Some("urn:mpeg:dash:schema:mpd:2011".into()),
       ProgramInformation: Some(pi),
       ..Default::default()
   };

   let xml = quick_xml::se::to_string(&mpd)
        .expect("serializing MPD struct");
}
```

See example [serialize.rs](https://github.com/emarsden/dash-mpd-rs/blob/main/examples/serialize.rs) for more detail.



To **download content** from an MPD manifest:

```rust
use dash_mpd::fetch::DashDownloader;

let url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
match DashDownloader::new(url)
       .worst_quality()
       .download()
       .await
{
   Ok(path) => println!("Downloaded to {path:?}"),
   Err(e) => eprintln!("Download failed: {e:?}"),
}
```

See example
[download_bbc.rs](https://github.com/emarsden/dash-mpd-rs/blob/main/examples/download_bbc.rs) for a
little more detail.

An application that provides a convenient commandline interface for the download functionality is
available separately in the [dash-mpd-cli](https://crates.io/crates/dash-mpd-cli) crate.

## Installation

Add to your `Cargo.toml` file:

```toml
[dependencies]
dash-mpd = "0.10"
```

If you don’t need the download functionality and wish to reduce code size, use:

```toml
[dependencies]
dash-mpd = { version = "0.10", default-features = false }
```

We endeavour to use **semantic versioning** for this crate despite its 0.x version number: a major
change which requires users of the library to change their code (such as a change in an attribute
name or type) will be published in a major release. For a version number `0.y.z`, a major release
implies a change to `y`.


## Optional features

The following additive [Cargo
features](https://doc.rust-lang.org/stable/cargo/reference/features.html#the-features-section) can
be enabled:

- `fetch` *(enabled by default)*: enables support for downloading stream content

- `socks` *(enabled by default)*: enables the `socks` feature on our `reqwest` dependency, which
  provides SOCKS5 proxy support for HTTP/HTTPS requests.

- `compression` *(enabled by default)*: enables the `gzip` feature on our `reqwest` dependency, to
  enable gzip compression and decompression of HTTP/HTTPS requests.

- `native-tls` *(enabled by default)*: enables the native-tls feature on our `reqwest` dependency,
  to enable HTTPS requests using the platform's default TLS implementation.

- `rustls-tls`: enable the `rustls-tls` feature on our `reqwest` dependency (use `rustls` instead of
  system-native TLS). You may need to enable this (and build without `native-tls`) for static linking
  with the musl-libc target on Linux.

- `libav`: enables linking to ffmpeg as a library for muxing support (instead of calling out to
  mkvmerge, ffmpeg or vlc as a subprocess), via the `ac-ffmpeg` crate

- `trust-dns`: enable the `trust-dns` feature on our `reqwest` dependency, to use the trust-dns DNS
  resolver library instead of the system resolver.

- `scte35` *(enabled by default)*: enable support for XML elements corresponding to the SCTE-35
  standard for insertion of alternate content (mostly used for dynamic insertion of advertising).


## Platforms

This crate is tested on the following platforms:

- Linux, with default features (muxing using mkvmerge, ffmpeg, vlc or MP4Box as a subprocess) and
  libav support, on AMD64 and Aarch64 architectures

- MacOS/Aarch64, without the libav feature (problems building the ac-ffmpeg crate against current ffmpeg)

- Microsoft Windows 10 and Windows 11, without the libav feature

- Android 12 on Aarch64 via [termux](https://termux.dev/), without the libav feature (you'll need to
  install the rust, binutils and ffmpeg packages)

- OpenBSD/AMD64, without the libav feature


## License

This project is licensed under the MIT license. For more information, see the `LICENSE-MIT` file.

