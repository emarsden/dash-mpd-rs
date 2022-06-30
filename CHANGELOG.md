# Changelog


## [0.4.5] - 2022-07
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
