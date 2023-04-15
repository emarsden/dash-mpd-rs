# Changelog

## [0.7.3] - 2023-04-15
### New
- Add definition for `SubRepresentation` nodes.
- Add definition for `Rating` nodes.
- Add `@presentationTimeOffset`, `@contentEncoding` and `@messageData` attributes to Event nodes.
  The node content is available via the `content` attribute.
- Add `@availabilityTimeOffset` and `@availabilityTimeComplete` to SegmentTemplate nodes (from
  @sbuzzard).
- Add `@weight` attribute to BaseURL nodes.
- Add `Role`, `Rating` and `Viewpoint` content to ContentComponent and AdaptationSet nodes.
- Add `Label` content to SubRepresentation and AdaptationSet nodes.
- Add `SupplementalProperty` to Period nodes.
- Add `@id` attribute to MPD nodes.

### Changed
- Downloading: New option `max_error_count` on DashDownloader to specify the maximum number of
  non-transient network errors that should be ignored before a download is aborted. This is useful
  on some manifests using Time-based or Number-based SegmentLists for which the packager calculates
  a number of segments which is different to our calculation (in which case the last segment can
  generate an HTTP 404 error).
- Serializing: the formatting of xs:duration attributes in generated XML has been improved to use
  nanosecond instead of millisecond precision, and to use minute and hour markers if relevant.


## [0.7.2] - 2023-03-19
### New
- Downloading: improve support for subtitles by making use of Representation.mimeType attributes
- New crate feature `native-tls` (enabled by default) which is passed through to the `reqwest`
  crate. This change allows users to select between TLS support using the platform-native stack
  (`native-tls`) and using the `rustls-tls` stack.
- New crate feature `socks` (enabled by default) which is passed through to the `reqwest` crate. It
  enables SOCKS5 proxy support for HTTP/HTTPS requests.
- Add `@segmentProfiles` attributes to Representation and AdaptationSet nodes.
- Add `@dependencyId` attribute to Representation nodes.
- Add `@qualityRanking` attribute to Representation nodes.
- Add `@indexRange` and `@indexRangeExact` attributes to SegmentTemplate and SegmentList nodes.
- Add definition for `Representation.FramePacking` nodes.
- Add definition for `MPD.PatchLocation` nodes, that allow a DASH client to retrieve a patch from
  the server that contains a set of instructions for replacing certain parts of the MPD manifest
  with updated information.
- Add definition (with correct capitalization) for `Metrics.Reporting` nodes.

### Changed
- Downloading: use `Representation.qualityRanking` attribute, if present, to select representations
  according to user's quality preference.
- Update dependency quick-xml to v0.28.
- Update dependency xattr to v1.0.
- `AudioChannelConfiguration` nodes in Representation and AdaptationSet changed from an Option to a
  Vec (they may be present multiple times).


## [0.7.1] - 2023-03-12
### New
- Add `EventStream` node to Period nodes (thanks to @noguxun).
- Add `@maxPlayoutRate`, `@profiles` and `@codingDependency` attributes on Representation and
  AdaptationSet nodes.
- New crate features `compression`, `rustls-tls` and `trust-dns` to enable (pass through) the
  corresponding features on the `reqwest` dependency. Otherwise, we use reqwest with its default
  feature set. Suggestion from @HoLLy.

### Changed
- Fix download of media streams with `$Time`-based `SegmentTimeline` when initial `@t` is non-zero.
- Update dependency iso8601 to v0.6.
- The tokio crate is a dev-dependency rather than a full dependency (from @HoLLy).


## [0.7.0] - 2023-01-28
### Changed
- Downloading: switched to an asynchronous API. This will require code changes for clients.
  Functions `download` and `download_to` are now `async`, and you will need to call them from an
  async context and use `.await` (see the example `download_bbc.rs` for some sample code). If you
  are passing a reqwest client to `DashDownloader` (using `with_http_client`), you should now use a
  standard `Client` built using `reqwest::Client::builder()`, instead of using
  `reqwest::blocking::Client::builder()` as previously (see the example `download_proxy.rs` for some
  sample code). Clients will need to add an explicit dependency on the tokio crate (which was
  already pulled in via the reqwest crate, but implicitly).


## [0.6.4] - 2023-01-14
### New
- Preliminary support for fetching subtitles (see function `fetch_subtitles` on `DashDownloader`).
  There is support for subtitles in WebVTT format (an AdaptationSet node with a `@mimeType`
  attribute of "text/vtt"), TTML (`@mimeType` of "application/ttml+xml") and SAMI (`@mimeType` of
  "application/x-sami"). There is also some support for WVTT (binary WebVTT in a wvtt box in
  fragmented MP4 container, as specified by ISO/IEC 14496-30:2014) and for STPP format (TTML in a
  fragmented MP4 container). WVTT subtitles will be converted to SRT format using the MP4Box
  commandline application, if it is available in the PATH.
### Changed
- Update dependency quick-xml to v0.27
- Simplify serialization example using new version of the quick-xml crate.


## [0.6.3] - 2022-12-10
### Changed
- Fix: xs:datetime fields such as `MPD@publishTime` and `MPD@availabilityStartTime` without a
  timezone are now parsed correctly instead of triggering an error. Issue seen with YouTube DASH
  manifests, reported by @erg43hergeg.


## [0.6.2] - 2022-11-27
### Changed
- Downloading: implement support for `SegmentURL@mediaRange` and `Initialization@range` using HTTP
  byte range requests. This allows us to download crazy DASH manifests that misuse Twitter's CDN by
  prepending dummy PNG headers to media segments
  (https://twitter.com/David3141593/status/1587978423120666625).
- Fixed default value for `SegmentTemplate@startNumber` when downloading (1 instead of 0).
- Fix: an AdaptationSet may contain a SegmentList.
### New
- We now check that the HTTP content-type of downloaded segments corresponds to audio or video content.
  New function `without_content_type_checks` on `DashDownloader` to disable these checks (may be
  necessary with poorly configured HTTP servers). 
- Added functions `keep_video` and `keep_audio` on `DashDownloader` to retain video and audio
  streams on disk after muxing.
- Added attribute `Representation@mediaStreamStructureId`.
- Added attribute `SegmentTemplate@eptDelta`.


## [0.6.1] - 2022-11-12
### New
- Support for data URLs in initialization segments (per RFC 2397).

### Changed
- API change: rationalize struct field types: fields that were of type Option<Vec<>> (such as
  MPD.Periods and Period.Representations) become Vec<> in the serialized representation. If none
  present, the vector is empty. This simplifies iteration over their contents. Some items such as
  BaseURL that can appear multiple times changed to Vec<> instead of Option<>.
- Add missing `Event@presentationTime` attribute.
- Add missing `AdaptationSet > Label` node type.
- Add missing `AdaptationSet@selectionPriority` attribute.


## [0.6.0] - 2022-10-02
### New
- Serialization support to allow programmatic generation of an MPD manifest (in XML format) from Rust
  structs. See `examples/serialize.rs` for some example code. 


## [0.5.1] - 2022-09-10
### New
- New functions `with_vlc()` and `with_mkvmerge()` on `DashDownloader` to allow the location of VLC
  and mkvmerge applications to be specified, if in a non-standard location. Aligns with the existing
  functionality to specify the location of the ffmpeg binary.

### Changed
- The default path for the external muxing applications now depends on the platform (for instance
  "ffmpeg.exe" on Windows and "ffmpeg" elsewhere). 
- The `download_to()` function returns the path that the media was downloaded to, instead of `()`.


## [0.5.0] - 2022-09-03
### Changed
- API change: reworked the error handling using an error enumeration DashMpdError and the
  `thiserror` crate, instead of the `anyhow` crate. This allows clients of the library to handle
  errors depending on their type (I/O, network, parsing, muxing, etc.).
- Update required version of chrono crate to resolve security vulnerability RUSTSEC-2020-0159.

## [0.4.6] - 2022-08-27
### Changed
- Download support is conditional on the `fetch` crate feature being enabled (which is the default
  configuration). Disabling it reduces code size and the number of dependencies pulled in.

### New
- Downloading: add support for `mkvmerge` as an external muxing tool. When built without libav support
  (the default configuration) and downloading to a path with ".mkv" extension, try to use `mkvmerge`
  (from the MkvToolnix suite) as a subprocess for muxing, before falling back to `ffmpeg`.
  `mkvmerge` will generate files in a Matroska container, which allows more codec flexibility than
  MPEG-4. `mkvmerge` is available for Linux and other Unixes, Microsoft Windows and MacOS.
- Add support for manifests containing a `Location` node. This allows the server to specify a new
  URL from which the client should request an updated manifest (similar to an HTTP redirect).
- Change type of some attributes specified to be of type `xs:dateTime` to an `XsDatetime` instead of
  an unserialized String, using serde support in the chrono crate (@publishTime,
  @availabilityStartTime, @availabilityEndTime).
- Change type of some attributes specified to be of type `xs:duration` to a `Duration` instead of an
  unserialized String (@minBufferTime, @minimumUpdatePeriod, @timeShiftBufferDepth,
  @mediaPresentationDuration, @suggestedPresentationDelay).


## [0.4.5] - 2022-07-02
### New
- Downloading: functions `audio_only` and `video_only` on DashDownloader allow the user to fetch
  only the audio stream, or only the video stream (for streams in which audio and video content are
  available separately).
- Downloading: function `prefer_language` on DashDownloader allows the user to specify the preferred
  language when multiple audio streams with different languages are available. The argument must be
  in RFC 5646 format (eg. "fr" or "en-AU"). If a preference is not specified and multiple audio
  streams are present, the first one listed in the DASH manifest will be downloaded.


## [0.4.4] - 2022-06-01
### New
- Downloading: support for sleeping between network requests, a primitive mechanism for throttling
  network bandwidth consumption (function `sleep_between_requests` on DashDownloader).

### Fixed
- Fixes to allow download of DASH streams with SegmentList addressing where the `SegmentURL` nodes
  use `BaseURL` instead of `@media` paths (eg.
  http://download.tsi.telecom-paristech.fr/gpac/DASH_CONFORMANCE/TelecomParisTech/mp4-main-single/mp4-main-single-mpd-AV-BS.mpd) 
- Downloading: muxing using VLC should now work correctly.
- Downloading: improve handling of transient and permanent HTTP errors.

## [0.4.3] - 2022-05-16
### Changed
- An `AdaptationSet` node may contain multiple `ContentComponent` nodes.
- Optional fields `frameRate` and `sar` added to `Representation` nodes.
- Implement our own parser for ISO 8601 durations to avoid bugs in the iso8601 crate. This fixes
  download support for several video publishers (content previously handled as if it had zero
  length, due to this parsing bug).
- Optional `SegmentTemplate@duration` field changed from u64 to f64 type. It is specified to
  be an unsigned int, but some manifests in the wild use a floating point value (eg.
  https://dash.akamaized.net/akamai/bbb_30fps/bbb_with_multiple_tiled_thumbnails.mpd). 

## [0.4.2] - 2022-03-19
### New
- Function `with_ffmpeg` on DashDownloader allows the user to specify the location of the ffmpeg
  binary to use for muxing (useful if it's not in the PATH).
- Optional field `ContentProtection` added to the `AdaptationSet` node type.
- Add optional field `cenc_pssh` to `ContentProtection` nodes.

## [0.4.1] - 2022-01-24
### New
- Function `add_progress_observer` on DashDownloader provides support for implementing progress bar,
  using the observer pattern.
- Function `verbosity` on DashDownloader provides support for setting the level of verbose messages
  concerning the progression of the download.
- Function `record_metainformation` controls whether metainformation such as the origin URL are
  recorded in the output file as extended attributes.

### Changed
- fetch_mpd() function now takes only a single DashDownloader argument.

## [0.4.0] - 2022-01-13
### Changed
- Downloading: move to a builder pattern with DashDownloader API. The function `fetch_mpd` should
  now be considered internal.
- Downloading: preference for quality/bitrate can be specified.

## [0.3.1] - 2022-01-08
### Fixed
- Downloading: fix use of SegmentTemplate `@startNumber` attribute.
- Downloading: fix regression concerning streams that use a SegmentTimeline.
- Path fixes to allow tests and examples to run on Windows.

## [0.3.0] - 2021-12-28
### Changed
- Downloading: support multi-period MPD manifests. 
- Downloading: support remote resources using XLink (`xlink:href` attributes).
- The `id` and `bandwidth` attributes of a `Representation` node are now optional (for XLink
support).

### Fixed
- Downloading: fix handling of manifests with negative `@r` attribute on `S` nodes.
- Downloading: fix handling of manifests with segment templates that use `$Bandwidth$`.

## [0.2.0] - 2021-12-11
### Changed
- Add support for using ffmpeg or vlc as a subprocess for muxing, rather than ffmpeg's libav
  library. This is more robust on certain invalid media streams and may be easier to build on
  certain platforms. Support is gated by the "libav" feature.
- When using libav (ffmpeg as a library), errors and informational messages from ffmpeg will be
  logged at info level.
- The `serviceLocation` attribute on `BaseURL` nodes is now public.
- The `media` attribute on `SegmentTemplate` nodes is now an optional field (it was previously a
  required field).
- The `actuate` (xlink:actuate) attribute on `Period` nodes is now of type `Option<String>`
  (previously `Option<bool>`).
- On platforms that support extended filesystem attributes, write the origin MPD URL to attribute
  `user.xdg.origin.url` on the output media file, using the `xattr` crate, unless the URL contains
  sensitive information such as a password. Likewise, write any MPD.ProgramInformation.{Title,
  Source, Copyright} information from the MPD manifest to attributes user.dublincore.{title, source,
  rights}.
- Downloading: improve handling of transient HTTP errors.
- Downloading: improve support for certain stream types.

## [0.1.0] - 2021-12-01

- Initial release.
