# dash-mpd

A Rust library for parsing and downloading media content from a DASH MPD file, as used by video
services such as on-demand replay of TV content and video streaming services like YouTube. 

[Documentation](https://docs.rs/dash-mpd/)
[![LICENSE](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-MIT)
[![Crates.io](https://img.shields.io/crates/v/dash-mpd)](https://crates.io/crates/dash-mpd)
[![dependency status](https://deps.rs/repo/github/emarsden/dash-mpd-rs/status.svg)](https://deps.rs/repo/github/emarsden/dash-mpd-rs)

[DASH](https://en.wikipedia.org/wiki/Dynamic_Adaptive_Streaming_over_HTTP) (dynamic adaptive
streaming over HTTP), also called MPEG-DASH, is a technology used for media streaming over the web,
commonly used for video on demand (VOD) services. The Media Presentation Description (MPD) is a
description of the resources (manifest or “playlist”) forming a streaming service, that a DASH
client uses to determine which assets to request in order to perform adaptive streaming of the
content. DASH MPD manifests can be used both with content encoded as MPEG and as WebM.

This library provides a serde-based parser for the DASH MPD format, as formally defined in ISO/IEC
standard 23009-1:2019. XML schema files are
[available for no cost from ISO](https://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/).
When MPD files in practical use diverge from the formal standard, this library prefers to
interoperate with existing practice. 


The library also provides experimental support for downloading content (audio or video) described by
an MPD manifest. This involves selecting the alternative with the most appropriate encoding (in
terms of bitrate, codec, etc.), fetching segments of the content using HTTP or HTTPS requests (this
functionality depends on the `reqwest` crate) and muxing audio and video segments together.

If the library feature `libav` is enabled, muxing support (combining audio and video streams, which
are often separated out in DASH streams) is provided by ffmpeg’s libav library, via the `ac_ffmpeg`
crate. Otherwise, muxing is implemented by calling `ffmpeg` as a subprocess. The ffmpeg commandline
application implements a number of checks and workarounds to fix invalid input streams that tend to
exist in the wild. Some of these workarounds, but not all, are implemented here when using libav as
a library, so download support tends to be more robust with the default configuration (using ffmpeg
as a subprocess).


## Limitations

- This crate does not support content encrypted with DRM such as Encrypted Media Extensions (EME) and
  Media Source Extension (MSE)
- Currently no download support for dynamic MPD manifests, that are used for live streaming/OTT TV
- No support for subtitles (eg. WebVTT streams)


## Usage

```rust
use std::time::Duration;
use dash_mpd::{MPD, parse};

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let xml = client.get("http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-events.mpd")
        .header("Accept", "application/dash+xml")
        .send()
        .expect("Requesting MPD content")
        .text()
        .expect("Fetching MPD content");
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

The experimental support for downloading content from an MPD manifest:

```rust
use std::time::Duration;
use dash_mpd::fetch_mpd;

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let url = "http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-ctv-events.mpd";
    if let Err(e) = fetch_mpd(&client, url, "/tmp/BBC-MPD-test.mp4") {
        eprintln!("Error downloading DASH MPD file: {:?}", e);
    }
}
```



## Installation

Add to your `Cargo.toml` file:

```toml
[dependencies]
dash-mpd = "0.2"
```



## License

This project is licensed under the MIT license. For more information, see the `LICENSE-MIT` file.


