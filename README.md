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

Muxing (merging audio and video streams, which are often published separately in DASH media streams)
is implemented by calling an external commandline application, either mkvmerge (from the
[MkvToolnix](https://mkvtoolnix.download/) suite), [ffmpeg](https://ffmpeg.org/),
[vlc](https://www.videolan.org/vlc/) or [MP4Box](https://github.com/gpac/gpac/wiki/MP4Box). The
choice of external muxer depends on the filename extension of the path supplied to `download_to()`
(will be `.mp4` if you call `download()`):

- `.mkv`: call mkvmerge first, then if that isn't installed or fails call ffmpeg, then try MP4Box
- `.mp4`: call ffmpeg first, then if that fails call vlc, then try MP4Box
- `.webm`: call vlc, then if that fails ffmpeg
- other: try ffmpeg, which supports many container formats, then try MP4Box

You can specify a different order of preference for muxing applications using the
`with_muxer_preference` method on DashDownloader. For example, `with_muxer_preference("avi",
"vlc,ffmpeg")` means that for an AVI media container the external muxer vlc will be tried first,
then ffmpeg in case of failure. This method option can be used multiple times to specify options for
different container types.

If the library feature `libav` is enabled, muxing is implemented using ffmpeg’s libav library, via
the `ac_ffmpeg` crate. This allows the library to work with fewer runtime dependencies. However,
these commandline applications implement a number of checks and workarounds to fix invalid input
streams that tend to exist in the wild. Some of these workarounds are implemented here when using
libav as a library, but not all of them, so download support tends to be more robust with the
default configuration (using an external application as a subprocess).


## DASH features supported

- **Multi-period** content. The media in the different streams will be saved in a single media container
  if the formats are compatible (same resolution, codecs, bitrate and so on) and
  `concatenate_periods(false)` has not been called on DashDownloader, and otherwise in separate
  media containers.

- WebVTT/wvtt, TTML, STPP, SRT, tx3g and SMIL **subtitles**, either provided as a single media
  stream or as a fragmented MP4 stream. Subtitles that are distributed as a single media stream will
  be saved to a file with the same base name as the requested output file, but with an extension
  corresponding to the subtitle type (e.g. `.srt`, `.vtt`). Subtitles distributed in WebVTT/wvtt
  format (either as a single media stream or a fragmented MP4 stream) will be converted to the more
  standard SRT format using the MP4Box commandline utility (from the [GPAC](https://gpac.wp.imt.fr/)
  project), if it is installed. STPP subtitles (which according to the DASH specifications should be
  formatted as EBU-TT) will be muxed into the output media container as a `subt:stpp` stream using
  MP4Box. Note that common media players such as mplayer and VLC don't currently support this
  subtitle type; you can try using the GPAC media player (available with `gpac -gui`).

- Support for **decrypting** media streams that use MPEG Common Encryption (cenc) ContentProtection.
  This requires either the `mp4decrypt` commandline application from the [Bento4
  suite](https://github.com/axiomatic-systems/Bento4/) to be installed ([binaries are
  available](https://www.bento4.com/downloads/) for common platforms), or the [Shaka
  packager](https://github.com/shaka-project/shaka-packager) application (binaries for common
  platforms are available as GitHub releases). See the `add_decryption_key` function on
  `DashDownloader`, the `with_decryptor_preference` function on `DashDownloader`, and the
  [decrypt.rs](https://github.com/emarsden/dash-mpd-rs/blob/main/examples/decrypt.rs) example.

- XLink elements (only with actuate=onLoad semantics), including resolve-to-zero.

- All forms of segment index info: SegmentBase@indexRange, SegmentTimeline,
  SegmentTemplate@duration, SegmentTemplate@index, SegmentList.

- Media containers of types supported by mkvmerge, ffmpeg, VLC or MP4Box (this includes Matroska,
  ISO-BMFF / CMAF / MP4, WebM, MPEG-2 TS), and all codecs supported by these applications.



## Limitations / unsupported features

- We can’t really download content from **dynamic MPD manifests**, that are used for live
  streaming/OTT TV. This is because we don't implement the clock functionality needed to know when
  new media segments become available nor the bandwidth management functionality that allows
  adaptive streaming. Note however that some OTT providers public dynamic manifests for content that
  is not live (i.e. all media segments are already available), and which we can download in dumb
  “fast-as-possible” mode. You can use the method `allow_live_streams()` on `DashDownloader` to
  attempt to download from these “**pseudo-live**” streams. It may also be useful to specify
  `force_duration(secs)` and to use `sleep_between_requests()` to ensure downloading is not faster
  than real time.

  An alternative technique is to use the XSLT stylesheet `tests/fixtures/rewrite-drop-dynamic.xslt`
  to change the `dynamic` attribute to `static` before downloading, which should allow you to
  download this type of content.

- No support for XLink with actuate=onRequest semantics.


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

   let xml = mpd.to_string();
}
```

See example [serialize.rs](https://github.com/emarsden/dash-mpd-rs/blob/main/examples/serialize.rs) for more detail.



To **download content** from an MPD manifest:

```rust
use dash_mpd::fetch::DashDownloader;

let url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
match DashDownloader::new(url)
       .worst_quality()
       .download().await
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
dash-mpd = "0.15.0"
```

If you don’t need the download functionality and wish to reduce code size, use:

```toml
[dependencies]
dash-mpd = { version = "0.15.0", default-features = false }
```

We endeavour to use **semantic versioning** for this crate despite its 0.x version number: a major
change which requires users of the library to change their code (such as a change in an attribute
name or type) will be published in a major release. For a version number `0.y.z`, a major release
implies a change to `y`.


## Optional features

The following additive [Cargo
features](https://doc.rust-lang.org/stable/cargo/reference/features.html#the-features-section) can
be enabled:

- `fetch` *(enabled by default)*: enables support for downloading stream content. This accounts for
  most of the code size of the library, so disable it if you only need the struct definitions for
  serializing and deserializing MPDs.

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
  mkvmerge, ffmpeg or vlc as a subprocess), via the `ac-ffmpeg` crate.

- `trust-dns`: enable the `trust-dns` feature on our `reqwest` dependency, to use the trust-dns DNS
  resolver library instead of the system resolver.

- `scte35` *(enabled by default)*: enable support for XML elements corresponding to the SCTE-35
  standard for insertion of alternate content (mostly used for dynamic insertion of advertising).

- `warn_ignored_elements`: if this feature is enabled, a warning will be issued when an XML element
  present in the DASH manifest is not deserialized into a Rust struct, while parsing the manifest.
  The default behaviour is to ignore elements for which we have not defined serde deserialization
  instructions. This feature is implemented with the `serde_ignored` crate.


## Platforms

This crate is tested on the following platforms:

- Linux, with default features (muxing using mkvmerge, ffmpeg, vlc or MP4Box as a subprocess) and
  libav support, on AMD64 and Aarch64 architectures

- MacOS/Aarch64, without the libav feature (problems building the ac-ffmpeg crate against current ffmpeg)

- Microsoft Windows 10 and Windows 11, without the libav feature

- Android 12 on Aarch64 via [termux](https://termux.dev/), without the libav feature. You'll need to
  install the `rust`, `binutils`, `ffmpeg` and `protobuf` packages.

- FreeBSD/AMD64 and OpenBSD/AMD64, without the libav feature. Note however that some of the external
  utility applications we use for muxing or decrypting media content are poorly supported on
  these platforms.


## Why?

This library was developed to allow the author to watch a news programme produced by a public media
broadcaster whilst at the gym. The programme is published as a DASH stream on the broadcaster’s
“replay” service, but network service at the gym is sometimes poor. First world problems!

The author is not the morality police nor a lawyer, but please note that redistributing media
content that you have not produced may, depending on the publication licence, be a breach of
intellectual property laws. Also, circumventing DRM may be prohibited in some countries.



## License

This project is licensed under the MIT license. For more information, see the `LICENSE-MIT` file.

