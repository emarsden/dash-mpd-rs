# Changelog

## [0.16.6] - Unreleased

- Progress reporters will be called (and the progress bar updated) more frequently, and more
  reliably when segment sizes are small and network speeds are high (suggestion from @filiptibell).


## [0.16.5] - 2024-06-22

- The `scte214:supplementalProfiles` and `supplementalCodecs` attributes can live at the level of an
  `AdaptationSet` element, in addition to `Representation` element (from @sbuzzard).


## [0.16.4] - 2024-06-08

- Downloading: fix a bug in concatenation for multiperiod manifests that occurs when one of the
  Periods does not contain any audio.

- Downloading: add support for concatenating streams in multi-period manifests using mkvmerge, as an
  alternative to the existing support for concatenation using ffmpeg. The preference ordering for
  concatenation helpers is specified by method `with_concat_helper` on `DashDownloader`.
  Concatenation using mkvmerge works at least with MP4 and with Matroska (.mkv) containers. It tends
  to be much faster than using ffmpeg but also less robust (less tolerant of the variety of media
  encoding specificities found in the wild). You can specify multiple concatenation helpers, in
  which case they will be called sequentially until one of them returns a success code.

- Downloading: accomodate manifests which say a Representation has `mimeType="video/mp4"` and
  `codecs="mp4a"`. These are now detected as audio streams rather than as video streams.

- Allow the user to specify a preference for streams based on the value of the `Role` element in an
  `AdaptationSet`. Streaming services sometimes publish various additional streams marked with roles
  such as `alternate` or `supplementary` or `description`, in addition to the main stream which is
  generalled labelled with a role of `main`. The user can specify a preference order for these role
  annotations, which is applied after the language preference and before the width/height/quality
  preference.

- Add scte214 attributes `@supplementalProfiles` and `@supplementalCodecs` to `Representation` nodes
  (from @sbuzzard).


## [0.16.3] - 2024-05-21

- Downloading: new method `minimum_period_duration()` on `DashDownloader`. Periods whose duration is
  less than this value will not be downloaded.

- Downloading: fix a bug in the concatenation of multiperiod manifests. When per-Period files
  contained both audio and video content, the content was being included twice in the concatenated
  file. Add an integration test for concatenation using ffmpeg.

- `AssetIdentifier` and `SupplementalProperty` elements may contain `Scte214ContentIdentifier`
  elements, as per the SCTE 214-1 specification (from @sbuzzard).


## [0.16.2] - 2024-05-09

- Fix bug in filename handling when using the ffmpeg concatenation filter. Filenames were not
  properly escaped when passed as arguments to the `filter_complex` commandline argument.

- Add support for subtitles that use SegmentBase addressing.

- Subtitles in STPP format (a data stream in MP4 fragments) are now converted to TTML format. The
  XML data stream is extracted using ffmpeg. If the conversion is successful it will be saved to a
  file with the same name as the output file, but with a `.ttml` extension.


## [0.16.1] - 2024-04-15

- Network requests for media fragments that fail are retried a certain number of times. The number
  of retries for each fragment request can be set using the `fragment_retry_count` method on
  `DashDownloader` (default is 10). Network errors that are identified as being transient (for
  example, network timeouts) do not count towards this retry count. Network requests were previously
  retried only if they were identified as transient, but anecdotally it seems that the internet and
  CDN servers are not set up in a way that allows transient errors reliably to be distinguished from
  non-transient errors. Non-transient retries still count towards the `max_error_count`, whose default
  value is increased to 30.


## [0.16.0] - 2024-03-30

- New error types `NetworkTimeout` and `NetworkConnect` in `DashMpdError`. These error types would
  previously have been reported as the `Network` error type. This is an API incompatible change.

- Error messages include additional contextual information (they are formatted using `Debug` instead
  of `Display`). For example, a network connection error caused by a TLS configuration error will
  include information on the underlying issue.

- The `ContentProtection.clearkey:Laurl` element, containing information on the license acquisition
  URL, is superseded by the `ContentProtection.dashif:laurl` element. We parse the former to an
  field (re)named `clearkey_laurl` and the latter to the field `laurl` in `ContentProtection`
  elements. Some manifests in the wild use both of these names, and we can't parse both into a Vec
  due to a parser limitation when using aliases. This is an API incompatible change. Reported by
  @pando-emil.

- The `trust-dns` build feature has been renamed to `hickory-dns` following the same rename in the
  reqwest crate, which was triggered by the change in name of the Hickory DNS resolver. The old name
  is still supported, but is deprecated.


## [0.15.0] - 2024-02-24

- Type of `S.@t` and `S.@d` attributes changed from `Option<i64>` to `Option<u64>`, to conform to the
  DASH specification. This is an API-breaking change which requires a semver bump.

- Add support for `MPD.ContentProtection` elements, as per the fifth edition of the DASH specification
  (ISO/IEC 23009-1:2021).
  
- Add support for `Period.Subset` elements.

- Add support for a `FailoverContent` element in a `SegmentTemplate` element. The XSD included in
  the DASH specification only includes a `FailoverContent` element on the `SegmentBase` element, but
  also includes it on a `SegmentTemplate` element in one of the examples. Even if examples are
  not normative, we choose to be tolerant in parsing.

- Add support for `@n` and `@k` attributes on `SegmentTimeline.S` elements.

- Downloading: fix the support for specifying referer using the `with_referer` method on
  `DashDownloader` (bug reported by @yoyo890121).


## [0.14.9] - 2024-02-18

- The tokio crate is now an optional feature, only needed when the `fetch` feature is enabled. This
  oversight was pointed out by @pando-fredrik. 

- Add `SegmentTimeline` element to `SegmentList` elements (from @erik-moqvist). 

- Add definition for `BitstreamSwitching` elements to `SegmentTemplate` and `SegmentList` nodes.

- Fix type of `@bitstreamSwitching` attribute on `SegmentTemplate` elements (xs:string rather than
  xs:bool as for all other uses of the `@bitstreamSwitching` attribute). 

- Fix type of `@audioSamplingRate` attributes on various elements (xs:string rather than u64).

- Downloading: add support for specifying the Referer HTTP header explicitly, through the
  `with_referer` method on `DashDownloader`. It was previously possible to specify the Referer by
  adding it to the default headers in the user-provided reqwest `Client`. However, we were not able
  to distinguish whether this had been specified by the caller or not, so were not able to add a
  relevant Referer header only in the absence of a user-provided Referer. Referer headers are
  included in all HTTP requests, for the MPD manifest, for audio and video segments, and for
  subtitle content.

- Downloading: default to enabling cookie support in the reqwest Client used for network requests.
  Cookies set while retrieving the MPD manifest will be included in requests for media segments.
  (In practice, media servers rarely check cookies, as doing so is expensive on a CDN infrastructure.)

- Downloading: fix handling of XLinked elements when remote XML fragment contains multiple elements.


## [0.14.8] - 2024-02-04

- Fix the serialization of various attributes that are declared as being of type `xsd:double` in the
  DASH XML Schema definition. These are represented as Rust f64 values, but need to be serialized
  slightly differently (in particular for values +INF, -INF, NaN that were previously serialized
  incorrectly). Bug reported by @ypo (#49).

- Add some missing ContentProtection uids and rename ChinaDRM as WisePlay (they share the same
  system ID, and WisePlay seems to be more prevalent than ChinaDRM).

- Downloading: fix the handling of XLinked elements. The Shaka heliocentrism test case now works
  correctly.

- Downloading: Widevine and PlayReady initialization data will now be decoded and pretty printed,
  alongside their Base64 representation (uses the new `pssh-box` crate).

- Downloading: fix concatenation for multiperiod manifests in situations where one period has audio
  and another has no audio track.


## [0.14.7] - 2023-12-25

- The library uses the `tracing` crate for all logging purposes, and will no longer print anything to
  stdout or stderr. Users should use a `tracing_subscriber` functionality to obtain logging
  information.

- Downloading: Fix bug in the handling of toplevel `Period.SegmentTemplate` elements (rarely present
  in the wild, but allowed by the DASH specification).

- Regexps used for parsing are statically allocated to avoid ongoing memory allocation overheads.

- Downloading: When deciding whether video files can be concatenated using the ffmpeg concat muxer,
  we allow for missing sar metainformation (not always present in MP4 containers in the wild).


## [0.14.6] - 2023-12-09

- Downloading: include the query component of the MPD URL in requests for media segments, to support
  the token-based authentication used by some streaming services. If the manifest URL is
  `https://example.com/manifest.mpd?token=foo`, requests to segments will look like
  `/segment/42.m4v?token=foo`, unless the manifest includes an explicit query component in the
  segment URLs.

- Muxing to a WebM container using the VLC external muxer should be fixed.


## [0.14.5] - 2023-11-28

- Downloading: Display current download bandwidth in the progress bar, if it is activated.

- Parsing: the path to the unparsable element is now shown, which greatly facilitates debugging!
  Uses the `serde_path_to_error` crate. The output is something like

     "Period[0].EventStream[0].Event[0].Signal[0].Binary.$value[0]: invalid digit found in string"

- Parsing: when the `warn_ignored_elements` build feature is enabled, a warning will be issued when
  an XML element present in the manifest is not deserialized into a Rust struct. The default
  behaviour is to ignore elements for which we have not defined serde deserialization instructions.
  This feature is implemented with the `serde_ignored` crate.

- Parsing: we no longer attempt to decode SCTE-35 Cue messages as Base64. Their format is more
  complicated than Base64 and attempting to decode them naively can generate spurious parse errors.

- Downloading: fix the calculation of audio segments to be downloaded for a live stream for which
  `force_duration` has been specified.


## [0.14.4] - 2023-11-18

- Add possibility to use Shaka packager application for decryption of media with Content Protection,
  as an alternative to mp4decrypt. The shaka-packager application is able to handle more media
  formats (e.g. WebM/Matroska containers) and is better maintained than mp4decrypt. See method
  `with_decryptor_preference` on `DashDownloader`.

- New method `allow_live_streams` on DashDownloader that makes it possible to attempt to download
  from a live (dynamic) manifest. Downloading from a genuinely live stream won't work well, because
  we don't implement the clock-related throttling needed to only download media segments when they
  become available. However, some media sources publish “pseudo-live” streams where all media segments
  are in fact available (they don't update the manifest once the live is complete), which we will be
  able to download. You might also have some success in combination with the
  `sleep_between_requests` method.

- New method `force_duration(secs)` on `DashDownloader` to specify the number of seconds to capture
  from the media stream, overriding the duration specified in the DASH manifest. This is mostly
  useful for live streams, whose duration is often not specified. It can also be used to capture
  only the first part of certain normal (static/on-demand) media streams, though this functionality
  is not currently fully implemented for all segment description types.

- Fix the selection of the desired Representation (according to the user's quality/resolution
  preferences) for DASH manifests that include multiple AdaptationSets. This is the case on some
  manifests that offer media streams using different codecs. We were previously only examining
  Representation elements in the first AdaptationSet present in the manifest.

- Reduce memory usage when downloading media segments by using the reqwest crate's chunk API,
  instead of reading all content using the `bytes()` method. This is important for DASH manifests
  that use indexRange addressing, which we don't download using byte range requests as a normal DASH
  client would do, but rather download using a single network request.


## [0.14.3] - 2023-11-04

- Add `@pdDelta` attribute on `SegmentTemplate` and `SegmentBase` elements.

- Add preliminary support for applying rewrite rules to the MPD manifest before downloading media
  segments. Rewrite rules are expressed as XSLT stylesheets that are applied to the manifest using
  the `xsltproc` commandline tool (which supports XSLT v1.0). This allows complex rewrite rules to
  be expressed using a standard (if a little finicky) stylesheet language. See the
  `with_xslt_stylesheet()` method on `DashDownloader`.
  
  This functionality and API are experimental, and may evolve to use a different XSLT processor, such as
  Saxon-HE (https://github.com/Saxonica/Saxon-HE/) which has support for XSLT v3.0, but is
  implemented in Java. Alternatively, a more general filtering functionality based on WASM bytecode
  might be implemented to allow the implementation of rewrite rules in a range of languages that can
  compile to WebAssembly.

- DASH conformity checks are now optional, controlled by a call to `conformity_checks()` on
  `DashDownloader` (default is to run conformity checks). Conformity checks will print a warning to
  stderr instead of causing a download to fail (a surprising number of manifests, including some
  generated by the most widely used commercial streaming software, include non-conformities such as
  incorrect values of @maxWidth / @maxHeight or inserted advertising segments that don't respect
  @maxSegmentDuration).

- Downloading: change the default ordering of muxer applications when saving media to a .webm
  container to prefer VLC over ffmpeg. With the commandline arguments that we use, ffmpeg does not
  automatically reencode content to a codec that is allowed by the WebM specification, whereas VLC
  does do so.


## [0.14.2] - 2023-10-15
- Add preliminary support for some simple conformity checks on DASH manifests during parsing.

- Implement `std::fmt::Display` on `MPD` structs, which makes it possible to serialize them easily
  using `.to_string()` (thanks to @Yesterday17).

- Add attribute `presentationTimeOffset` to `EventStream` elements (thanks to @sbuzzard).

- Downloading: allow the user to specify the order in which muxer applications are tried, instead of
  using a hard-coded ordering per container type. The ordering is specified per container type
  ("mkv", "mp4", "avi", "ts", etc.). The user specifies an ordering such as "ffmpeg,vlc,mp4box"
  which means that ffmpeg is tried first, and if that fails vlc, and if that fails mp4box. The
  muxers currently available are ffmpeg, vlc, mkvmerge and mp4box. See function
  `with_muxer_preference` on `DashDownloader`.

- Downloading: work around a bug in VLC, which does not correctly report failure to mux via a
  non-zero exit code.


## [0.14.1] - 2023-09-30
- Downloading: enable support for Bearer authentication of network requests to retrieve the manifest
  and the media segments. See function `with_auth_bearer` on `DashDownloader`. This is the
  authentication method specified in RFC 6750, originally designed for OAuth 2.0, but also used in
  other settings such as JSON Web Token (JWT).

- Downloading: enable support for MPEG-4 Part 17 (Timed Text) subtitles (tx3g codec). They will be
  converted to SRT format if the MP4Box commandline application is installed.

- Downloading: when printing the available media streams, print `Role` and `Label` information if
  they are specified on an `AdaptationSet` element.

- Fix naming/parsing of `MPD.Location` field (thanks to @nissy34).


## [0.14.0] - 2023-09-03
### New
- Downloading: add support for selecting the desired video stream based on its resolution. See
  functions `prefer_video_width` and `prefer_video_height` on `DashDownloader`.

- Downloading: new function `intermediate_quality` on `DashDownloader` which controls the choice of
  media stream when multiple Adaptations are specified. This requests the download of the Adaptation
  with an intermediate bitrate (closest to the median value). Similar to `best_quality` and
  `worst_quality` functions.

- Downloading: enable support for authentication of network requests to retrieve the manifest and
  the media segments. See function `with_authentication` on `DashDownloader`. This provides support
  only for the “Basic” HTTP authentication scheme (RFC 7617). Bearer authentication (RFC 6750) is
  not currently supported.

- Downloading: improve support for selecting the output container format based on its filename
  extension. Selecting an output file with an `.mkv` extension will now produce an output file in
  Matroska container format, even in cases where the manifest only contains a video stream or only
  an audio stream (shortcircuiting the muxing functionality). In these cases, the stream will be
  copied if the output container requested is compatible with the downloaded stream format, and
  otherwise a new media container with the requested format will be created and the audio or video
  stream will be inserted (and reencoded if necessary) into the output file. This insertion and
  reencoding is undertaken by the same commandline applications used for muxing: ffmpeg, mkvmerge,
  mp4box (currently not vlc). This support is not currently implemented when building with the libav
  feature.

- Derive `Hash` on those structs for which it can be derived automatically.


## [0.13.1] - 2023-08-14
### New
- Support for certain nodes used with PlayReady ContentProtection: `clearkey:Laurl`, `mspr:pro`,
  `mspr:IsEncrypted`, `mspr:IV_Size`, `mspd:kid`.

### Changed
- Downloading: improve support for multiperiod manifests. When the contents of the different periods
  can be joined into a single output container (because they share the same resolution, frame rate
  and aspect ratio), we concatenate them using ffmpeg (with reencoding in case the codecs in the
  various periods are different). If they cannot be joined, we save the content in output files
  named according to the requested output file (whose name is used for the first period). Names
  ressemble "output-p2.mp4" for the second period, and so on.
- Downloading: new function `concatenate_periods` on `DashDownloader` to specify whether the
  concatenation (which is very CPU-intensive due to the reencoding) of multi-period manifests should
  be attempted. The default behaviour is to concatenate when the media contents allow it.
- Downloading: improved support for certain addressing types on subtitles
  (AdaptationSet>SegmentList, Representation>SegmentList, SegmentTemplate+SegmentTimeline addressing
  modes).
- Significantly improved support for XLink semantics on elements (remote elements). In particular,
  resolve-to-zero semantics are implemented, a remote XLinked element may resolve to multiple
  elements (e.g. a Period with href pointing to a remote MPD fragment may resolve to three final
  Period elements), and a remote XLinked element may contain a remote XLinked element (the number of
  repeated resolutions is limited, to avoid DoS attacks).


## [0.13.0] - 2023-08-05
### Changed
- Change element `Accessibility` of `ContentComponent` and `AdaptationSet` nodes to a `Vec` instead of
  an `Option` (incompatible change).
- Downloading: handling of STPP subtitles distributed as fragmented MP4 segments has been improved.
  They will be merged with the final output container if MP4Box is installed.
- Downloading: more diagnostics information is printed concerning the selected audio/video streams,
  when requested verbosity is higher than 1. In particular, pssh information will be printed for
  streams with ContentProtection whose pssh is embedded in the initialization segments rather than
  in the DASH manifest.


## [0.12.0] - 2023-07-16
### Changed
- Downloading: function `fetch_subtitles` on `DashDownloader` takes a boolean parameter, instead of
  unconditionally requesting retrieval of subtitle content (incompatible change).

### New
- Downloading: new functions `fetch_audio` and `fetch_video` on `DashDownloader`.
- Downloading: improve support for retrieving subtitles that are distributed in fragmented MP4 streams
  (in particular WebVTT/STPP).
- Downloading: more diagnostics information printed concerning the selected audio/video streams.


## [0.11.0] - 2023-07-08
### New
- Downloading: add support for decrypting encrypted media content, using the Bento4 mp4decrypt
  commandline application. Decryption keys can be specified using the `add_decryption_key` function
  on `DashDownloader`. The location of the mp4decrypt binary can be specified using the
  `with_mp4decrypt_location` function on `DashDownloader`, if it is not located in the PATH.

### Changed
- Change element `InbandEventStream` of `Representation` and `AdaptationSet` nodes to a `Vec`
  instead of an `Option` (incompatible change).
- Fix spurious error regarding deletion of temporary file for audio/video segments when using
  `keep_audio` / `keep_video` in conjunction with `fetch_audio` / `fetch_video`.
- Downloading: show download bitrate for audio and video streams in verbose mode.


## [0.10.0] - 2023-06-25
### Changed
- Downloading: incompatible change to the `keep_audio` and `keep_video` attributes on
  DashDownloader, to allow the user to specify the path for the audio and video files.
- Print information on the different media streams available (resolution, bitrate, codec) in a
  manifest when requested verbosity is non-zero.
- Update dependency quick-xml to v0.29 (thanks to @sdroege).


## [0.9.2] - 2023-06-10
### Changed
- Downloading: a connect error is handled as a permanent, rather than a transient, error. In
  particular, TLS certificate verification errors will no longer be treated as transient errors.
- Downloading: fix a bug in the handling of the `Location` element.


## [0.9.1] - 2023-05-28
### New
- Add definition for the `Preselection` element.
- Add attributes `@byteRange`, `@availabilityTimeOffset` and `@availabilityTimeComplete` to BaseURL
  elements (pointed out by @ypo).
### Changed
- Downloading: only download subtitles when `fetch_subtitles()` has been called on DashDownloader
  (from @sleepycatcoding).
- Add derived PartialEq to data structures to allow for comparison.
- Parsing: certain MPDs including "overlapping" elements can now be parsed.


## [0.9.0] - 2023-05-10
### New
- Downloading: add support for saving media fragments to a user-specified directory, using new
  function `save_fragments_to` on `DashDownloader`. This may be useful to help debug issues with
  DASH streams or to extract additional information from fragmented MP4 segments.
- Support for the DASH XML vocabulary associated with the SCTE-35 standard. This standard allows
  dynamic insertion of alternate content (mostly used for advertising). Support is gated by the
  new `scte35` feature, which is enabled by default.
- Parsing of xs:datetime fields attempts to use the rfc3339 crate before falling back to the iso8601
  crate if the datetime is not in RFC 3339 format (for example, if it doesn't include a timezone).
  The rfc3339 crate parses with nanosecond precision, whereas the iso8601 crate only has millisecond
  resolution.
- Downloading: fix an off-by-one error when calculating $Number$-based SegmentTemplate-based
  addressing (the initialization segment is now counted towards the total number of segments).


## [0.8.1] - 2023-04-27
### New
- Downloading: add preliminary support for throttling the network bandwidth, with method
  `with_rate_limit` on DashDownloader.
- Add `@scanType` attribute to AdaptationSet nodes.
- Add `@presentationDuration` to SegmentBase nodes.
- Add `FailoverContent` element to SegmentBase nodes (from @sbuzzard).

### Changed
- Serialization: default values for the XML namespaces for xlink, xsi, cenc, dvb and scte35
  will be provided if they are not supplied explicitly. This should make it easier to generate
  standards-compliant manifests.
- Downloading: limit length of default output pathname (when using method `download`) to avoid
  exceeding filesystem limits.


## [0.8.0] - 2023-04-22
### New
- Downloading: add support for `MP4Box` as an external muxing tool. When built without libav support
  (the default configuration) and downloading to a path with ".mp4" extension, try to use the
  `MP4Box` commandline application (from the GPAC suite) as a subprocess for muxing, if ffmpeg and
  VLC fail. `MP4Box` is available for Linux and other Unixes, Microsoft Windows and MacOS.
- New function `with_mp4box()` on `DashDownloader` to allow the location of the MP4Box commandline
  application to be specified, if in a non-standard location.
- New example `round_trip.rs` which can be used to check round trip from XML to Rust structs to XML.
- Add `@startWithSAP` and `@maximumSAPPeriod` attributes to AdaptationSet nodes.
- Add `@timescale` attribute to SegmentList nodes.
- Add `@numChannels`, `@sampleRate` and `@lang` attributes to Representation nodes.
- Add `@sar`, `@width`, `@height`, `@maximumSAPPeriod` and `@startWithSAP`attributes to
  AdaptationSet nodes.
- Add `EssentialProperty` and `SupplementalProperty` node vectors to Representation and
  AdaptationSet nodes.
- Add definition for `ProducerReferenceTime` nodes, used for low-latency streaming.
- Add definition for `Switching` nodes, used for Adaptation Set switching.
- Add definition for `InbandEventStream` nodes, used to signal presence of DASH event boxes in a
  media stream.
- Add definition for `RepresentationIndex` nodes.
- Add `@schemeIdUri` and `@value` (deprecated) to Event nodes.
- Add `scte214:ContentIdentifier` element to ProgramInformation nodes.
- Add `@maxSubsegmentDuration` attribute to MPD nodes.

### Changed
- `AdaptationSet.@id` changed from u64 to String type (breaking change).
- `Period.@start` changed from a String to an xs:duration type (breaking change).
- `ContentProtect.@cenc_pssh` changed from an Option to a Vec (breaking change).
- `DashMpdError` enum made `#[non_exhaustive]` (breaking change).
- Fixed a bug in the parsing of xs:datetime attributes with fractional seconds.
- Fixed parsing of `@starttime` and `@duration` attributes on Range elements.
- Fixed XML namespace issues for attributes declared in the XLink, XMLSchema-instance, Common
  Encryption, DVB and SCTE-35 namespaces. These attributes should now be serialized correctly when
  generating an MPD.


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
  in RFC 5646 format (e.g. "fr" or "en-AU"). If a preference is not specified and multiple audio
  streams are present, the first one listed in the DASH manifest will be downloaded.


## [0.4.4] - 2022-06-01
### New
- Downloading: support for sleeping between network requests, a primitive mechanism for throttling
  network bandwidth consumption (function `sleep_between_requests` on DashDownloader).

### Fixed
- Fixes to allow download of DASH streams with SegmentList addressing where the `SegmentURL` nodes
  use `BaseURL` instead of `@media` paths (e.g.
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
  be an unsigned int, but some manifests in the wild use a floating point value (e.g.
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
