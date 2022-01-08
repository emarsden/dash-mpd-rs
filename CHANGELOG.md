# Changelog

## [0.3.1] - 2022-01-08
### Changed
- Downloading: fix use of SegmentTemplate `@startNumber` attribute.
- Downloading: fix regression concerning streams that use a SegmentTimeline.
- Path fixes to allow tests and examples to run on Windows.

## [0.3.0] - 2021-12-28
### Changed
- Downloading: support multi-period MPD manifests. 
- Downloading: support remote resources using XLink (`xlink:href` attributes).
- The `id` and `bandwidth` attributes of a `Representation` node are now optional (for XLink
  support).
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
