# dash-mpd

A Rust library for parsing and downloading media content from a DASH MPD file, as used by video
services such as on-demand replay of TV content and video streaming services like YouTube. 

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
content. DASH MPD manifests can be used both with content encoded as MPEG and as WebM. There is a
good explanation of adaptive bitrate video streaming at
[howvideo.works](https://howvideo.works/#dash).

This library provides a serde-based parser for the DASH MPD format, as formally defined in ISO/IEC
standard 23009-1:2019. XML schema files are
[available for no cost from ISO](https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/).
When MPD files in practical use diverge from the formal standard, this library prefers to
interoperate with existing practice. 


If the library feature `fetch` is enabled (which it is by default), the library also provides
support for downloading content (audio or video) described by an MPD manifest. This involves
selecting the alternative with the most appropriate encoding (in terms of bitrate, codec, etc.),
fetching segments of the content using HTTP or HTTPS requests (this functionality depends on the
`reqwest` crate) and muxing audio and video segments together.

If the library feature `libav` is enabled, muxing support (combining audio and video streams, which
are often separated out in DASH streams) is provided by ffmpeg’s libav library, via the `ac_ffmpeg`
crate. Otherwise, muxing is implemented by calling `mkvmerge`, `ffmpeg` or `vlc` as a subprocess. If
the call to `mkvmerge` fails (for example because it is not installed), then `ffmpeg` is called, and
if that fails `vlc` is called. Note that these commandline applications implement a number of checks
and workarounds to fix invalid input streams that tend to exist in the wild. Some of these
workarounds are implemented here when using libav as a library, but not all of them, so download
support tends to be more robust with the default configuration (using an external application as a
subprocess).


## DASH features supported

- VOD (static) stream manifests
- Multi-period content
- XLink elements (only with actuate=onLoad semantics), including resolve-to-zero
- All forms of segment index info: SegmentBase@indexRange, SegmentTimeline,
  SegmentTemplate@duration, SegmentTemplate@index, SegmentList
- Media containers of types supported by mkvmerge, ffmpeg or VLC (this includes Matroska, ISO-BMFF /
  CMAF / MP4, WebM, MPEG-2 TS)


## Limitations / unsupported features

- Dynamic MPD manifests, that are used for live streaming/OTT TV
- Encrypted content using DRM such as Encrypted Media Extensions (EME) and Media Source Extension (MSE)
- Subtitles (eg. WebVTT and TTML streams)
- XLink with actuate=onRequest


## Usage

```rust
use std::time::Duration;
use dash_mpd::{MPD, parse};

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(10, 0))
        .gzip(true)
        .build()
        .expect("creating reqwest HTTP client");
    let xml = client.get("http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-events.mpd")
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
            println!("Contains Period of duration {:?}", d);
        }
    }
}
```

To download content from an MPD manifest:

```rust
use dash_mpd::fetch::DashDownloader;

let url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
match DashDownloader::new(url)
       .worst_quality()
       .download()
{
   Ok(path) => println!("Downloaded to {:?}", path),
   Err(e) => eprintln!("Download failed: {:?}", e),
}
```

An application that provides a commandline interface for this library is available separately in the
[dash-mpd-cli](https://crates.io/crates/dash-mpd-cli) crate.


## Installation

Add to your `Cargo.toml` file:

```toml
[dependencies]
dash-mpd = "0.4"
```

If you don’t need the download functionality and wish to reduce code size, use:

```toml
[dependencies]
dash-mpd = { version = "0.4", default-features = false }
```



## Platforms

This crate is tested on the following platforms:

- Linux, with default features (mkvmerge or ffmpeg or vlc as a subprocess) and libav support, on
  AMD64 and Aarch64 architectures
- MacOS, only with default features (problems building the ac-ffmpeg crate against current ffmpeg)
- Microsoft Windows 10, only with default features
- Android 11 on Aarch64 via termux (you'll need to install the rust, binutils and ffmpeg packages)
- OpenBSD/AMD64, only with default features (libav support not tested)


## License

This project is licensed under the MIT license. For more information, see the `LICENSE-MIT` file.


