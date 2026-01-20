//! Support for downloading content from DASH MPD media streams.

use std::env;
use tokio::io;
use tokio::fs;
use tokio::fs::File;
use tokio::io::{BufReader, BufWriter, AsyncWriteExt};
use std::io::{Read, Write, Seek};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::time::Instant;
use chrono::Utc;
use std::sync::Arc;
use std::borrow::Cow;
use std::collections::HashMap;
use std::cmp::min;
use std::ffi::OsStr;
use std::num::NonZeroU32;
use futures_util::TryFutureExt;
use tracing::{trace, info, warn, error};
use regex::Regex;
use url::Url;
use bytes::Bytes;
use data_url::DataUrl;
use reqwest::header::{RANGE, CONTENT_TYPE};
use backon::{ExponentialBuilder, Retryable};
use governor::{Quota, RateLimiter};
use lazy_static::lazy_static;
use xot::{xmlname, Xot};
use crate::{MPD, Period, Representation, AdaptationSet, SegmentBase, DashMpdError};
use crate::{parse, mux_audio_video, copy_video_to_container, copy_audio_to_container};
use crate::{is_audio_adaptation, is_video_adaptation, is_subtitle_adaptation};
use crate::{subtitle_type, content_protection_type, SubtitleType};
use crate::check_conformity;
#[cfg(not(feature = "libav"))]
use crate::ffmpeg::concat_output_files;
use crate::media::{temporary_outpath, AudioTrack};
use crate::decryption::{
    decrypt_mp4decrypt,
    decrypt_shaka,
    decrypt_shaka_container,
    decrypt_mp4box,
    decrypt_mp4box_container
};
#[allow(unused_imports)]
use crate::media::video_containers_concatable;

#[cfg(all(feature = "sandbox", target_os = "linux"))]
use crate::sandbox::{restrict_thread};


/// A `Client` from the `reqwest` crate, that we use to download content over HTTP.
pub type HttpClient = reqwest::Client;
type DirectRateLimiter = RateLimiter<governor::state::direct::NotKeyed,
                                     governor::state::InMemoryState,
                                     governor::clock::DefaultClock,
                                     governor::middleware::NoOpMiddleware>;


// When reading stdout or stderr from an external commandline application to display for the user,
// this is the maximum number of octets read.
pub fn partial_process_output(output: &[u8]) -> Cow<'_, str> {
    let len = min(output.len(), 4096);
    #[allow(clippy::indexing_slicing)]
    String::from_utf8_lossy(&output[0..len])
}


// This doesn't work correctly on modern Android, where there is no global location for temporary
// files (fix needed in the tempfile crate)
pub fn tmp_file_path(prefix: &str, extension: &OsStr) -> Result<PathBuf, DashMpdError> {
    if let Some(ext) = extension.to_str() {
        // suffix should include the "." separator
        let fmt = format!(".{}", extension.to_string_lossy());
        let suffix = if ext.starts_with('.') {
            extension
        } else {
            OsStr::new(&fmt)
        };
        let file = tempfile::Builder::new()
            .prefix(prefix)
            .suffix(suffix)
            .rand_bytes(7)
            .disable_cleanup(env::var("DASHMPD_PERSIST_FILES").is_ok())
            .tempfile()
            .map_err(|e| DashMpdError::Io(e, String::from("creating temporary file")))?;
        Ok(file.path().to_path_buf())
    } else {
        Err(DashMpdError::Other(String::from("converting filename extension")))
    }
}


// This version avoids calling set_readonly(false), which results in a world-writable file on Unix
// platforms.
// https://rust-lang.github.io/rust-clippy/master/index.html#permissions_set_readonly_false
#[cfg(unix)]
async fn ensure_permissions_readable(path: &Path) -> Result<(), DashMpdError> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let perms = Permissions::from_mode(0o644);
    fs::set_permissions(path, perms)
        .map_err(|e| DashMpdError::Io(e, String::from("setting file permissions"))).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn ensure_permissions_readable(path: &Path) -> Result<(), DashMpdError> {
    let mut perms = fs::metadata(path).await
        .map_err(|e| DashMpdError::Io(e, String::from("reading file permissions")))?
        .permissions();
    perms.set_readonly(false);
    fs::set_permissions(path, perms)
        .map_err(|e| DashMpdError::Io(e, String::from("setting file permissions"))).await?;
    Ok(())
}


/// Receives updates concerning the progression of the download, and can display this information to
/// the user, for example using a progress bar.
pub trait ProgressObserver: Send + Sync {
    fn update(&self, percent: u32, message: &str);
}


/// Preference for retrieving media representation with highest quality (and highest file size) or
/// lowest quality (and lowest file size).
#[derive(PartialEq, Eq, Clone, Copy, Default)]
pub enum QualityPreference { #[default] Lowest, Intermediate, Highest }


/// The `DashDownloader` allows the download of streaming media content from a DASH MPD manifest.
///
/// This involves:
///    - fetching the manifest file
///    - parsing its XML contents
///    - identifying the different Periods, potentially filtering out Periods that contain undesired
///      content such as advertising
///    - selecting for each Period the desired audio and video representations, according to user
///      preferences concerning the audio language, video dimensions and quality settings, and other
///      attributes such as label and role
///    - downloading all the audio and video segments for each Representation
///    - concatenating the audio segments and video segments into a stream
///    - potentially decrypting the audio and video content, if DRM is present
///    - muxing the audio and video streams to produce a single video file including audio
///    - concatenating the streams from each Period into a single media container.
///
/// This should work with both MPEG-DASH MPD manifests (where the media segments are typically
/// placed in fragmented MP4 or MPEG-2 TS containers) and for
/// [WebM-DASH](http://wiki.webmproject.org/adaptive-streaming/webm-dash-specification).
pub struct DashDownloader {
    pub mpd_url: String,
    pub redirected_url: Url,
    base_url: Option<String>,
    referer: Option<String>,
    auth_username: Option<String>,
    auth_password: Option<String>,
    auth_bearer_token: Option<String>,
    pub output_path: Option<PathBuf>,
    http_client: Option<HttpClient>,
    quality_preference: QualityPreference,
    language_preference: Option<String>,
    role_preference: Vec<String>,
    video_width_preference: Option<u64>,
    video_height_preference: Option<u64>,
    fetch_video: bool,
    fetch_audio: bool,
    fetch_subtitles: bool,
    keep_video: Option<PathBuf>,
    keep_audio: Option<PathBuf>,
    concatenate_periods: bool,
    fragment_path: Option<PathBuf>,
    pub decryption_keys: HashMap<String, String>,
    xslt_stylesheets: Vec<PathBuf>,
    minimum_period_duration: Option<Duration>,
    content_type_checks: bool,
    conformity_checks: bool,
    use_index_range: bool,
    fragment_retry_count: u32,
    max_error_count: u32,
    progress_observers: Vec<Arc<dyn ProgressObserver>>,
    sleep_between_requests: u8,
    allow_live_streams: bool,
    force_duration: Option<f64>,
    rate_limit: u64,
    bw_limiter: Option<DirectRateLimiter>,
    bw_estimator_started: Instant,
    bw_estimator_bytes: usize,
    pub sandbox: bool,
    pub verbosity: u8,
    record_metainformation: bool,
    pub muxer_preference: HashMap<String, String>,
    pub concat_preference: HashMap<String, String>,
    pub decryptor_preference: String,
    pub ffmpeg_location: String,
    pub vlc_location: String,
    pub mkvmerge_location: String,
    pub mp4box_location: String,
    pub mp4decrypt_location: String,
    pub shaka_packager_location: String,
}


// We don't want to test this code example on the CI infrastructure as it's too expensive
// and requires network access.
#[cfg(not(doctest))]
/// The DashDownloader follows the builder pattern to allow various optional arguments concerning
/// the download of DASH media content (preferences concerning bitrate/quality, specifying an HTTP
/// proxy, etc.).
///
/// # Example
///
/// ```rust
/// use dash_mpd::fetch::DashDownloader;
///
/// let url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
/// match DashDownloader::new(url)
///        .worst_quality()
///        .download().await
/// {
///    Ok(path) => println!("Downloaded to {path:?}"),
///    Err(e) => eprintln!("Download failed: {e}"),
/// }
/// ```
impl DashDownloader {
    /// Create a `DashDownloader` for the specified DASH manifest URL `mpd_url`.
    ///
    /// # Panics
    ///
    /// Will panic if `mpd_url` cannot be parsed as an URL.
    pub fn new(mpd_url: &str) -> DashDownloader {
        DashDownloader {
            mpd_url: String::from(mpd_url),
            redirected_url: Url::parse(mpd_url).unwrap(),
            base_url: None,
            referer: None,
            auth_username: None,
            auth_password: None,
            auth_bearer_token: None,
            output_path: None,
            http_client: None,
            quality_preference: QualityPreference::Lowest,
            language_preference: None,
            role_preference: vec!["main".to_string(), "alternate".to_string()],
            video_width_preference: None,
            video_height_preference: None,
            fetch_video: true,
            fetch_audio: true,
            fetch_subtitles: false,
            keep_video: None,
            keep_audio: None,
            concatenate_periods: true,
            fragment_path: None,
            decryption_keys: HashMap::new(),
            xslt_stylesheets: Vec::new(),
            minimum_period_duration: None,
            content_type_checks: true,
            conformity_checks: true,
            use_index_range: true,
            fragment_retry_count: 10,
            max_error_count: 30,
            progress_observers: Vec::new(),
            sleep_between_requests: 0,
            allow_live_streams: false,
            force_duration: None,
            rate_limit: 0,
            bw_limiter: None,
            bw_estimator_started: Instant::now(),
            bw_estimator_bytes: 0,
            sandbox: false,
            verbosity: 0,
            record_metainformation: true,
            muxer_preference: HashMap::new(),
            concat_preference: HashMap::new(),
            decryptor_preference: String::from("mp4decrypt"),
            ffmpeg_location: String::from("ffmpeg"),
	    vlc_location: if cfg!(target_os = "windows") {
                // The official VideoLan Windows installer doesn't seem to place its installation
                // directory in the PATH, so we try with the default full path.
                String::from("c:/Program Files/VideoLAN/VLC/vlc.exe")
            } else {
                String::from("vlc")
            },
	    mkvmerge_location: String::from("mkvmerge"),
	    mp4box_location: if cfg!(target_os = "windows") {
                String::from("MP4Box.exe")
            } else if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
                String::from("MP4Box")
            } else {
                String::from("mp4box")
            },
            mp4decrypt_location: String::from("mp4decrypt"),
            shaka_packager_location: String::from("shaka-packager"),
        }
    }

    /// Specify the base URL to use when downloading content from the manifest. This may be useful
    /// when downloading from a file:// URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: String) -> DashDownloader {
        self.base_url = Some(base_url);
        self
    }


    /// Specify the reqwest Client to be used for HTTP requests that download the DASH streaming
    /// media content. Allows you to specify a proxy, the user agent, custom request headers,
    /// request timeouts, additional root certificates to trust, client identity certificates, etc.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dash_mpd::fetch::DashDownloader;
    ///
    /// let client = reqwest::Client::builder()
    ///      .user_agent("Mozilla/5.0")
    ///      .timeout(Duration::new(30, 0))
    ///      .build()
    ///      .expect("creating HTTP client");
    ///  let url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    ///  let out = PathBuf::from(env::temp_dir()).join("cloudflarestream.mp4");
    ///  DashDownloader::new(url)
    ///      .with_http_client(client)
    ///      .download_to(out)
    ///       .await
    /// ```
    #[must_use]
    pub fn with_http_client(mut self, client: HttpClient) -> DashDownloader {
        self.http_client = Some(client);
        self
    }

    /// Specify the value for the Referer HTTP header used in network requests. This value is used
    /// when retrieving the MPD manifest, when retrieving video and audio media segments, and when
    /// retrieving subtitle data.
    #[must_use]
    pub fn with_referer(mut self, referer: String) -> DashDownloader {
        self.referer = Some(referer);
        self
    }

    /// Specify the username and password to use to authenticate network requests for the manifest
    /// and media segments.
    #[must_use]
    pub fn with_authentication(mut self, username: &str, password: &str) -> DashDownloader {
        self.auth_username = Some(username.to_string());
        self.auth_password = Some(password.to_string());
        self
    }

    /// Specify the Bearer token to use to authenticate network requests for the manifest and media
    /// segments.
    #[must_use]
    pub fn with_auth_bearer(mut self, token: &str) -> DashDownloader {
        self.auth_bearer_token = Some(token.to_string());
        self
    }

    /// Add an observer implementing the `ProgressObserver` trait, that will receive updates concerning
    /// the progression of the download (allows implementation of a progress bar, for example).
    #[must_use]
    pub fn add_progress_observer(mut self, observer: Arc<dyn ProgressObserver>) -> DashDownloader {
        self.progress_observers.push(observer);
        self
    }

    /// If the DASH manifest specifies several Adaptations with different bitrates (levels of
    /// quality), prefer the Adaptation with the highest bitrate (largest output file).
    #[must_use]
    pub fn best_quality(mut self) -> DashDownloader {
        self.quality_preference = QualityPreference::Highest;
        self
    }

    /// If the DASH manifest specifies several Adaptations with different bitrates (levels of
    /// quality), prefer the Adaptation with an intermediate bitrate (closest to the median value).
    #[must_use]
    pub fn intermediate_quality(mut self) -> DashDownloader {
        self.quality_preference = QualityPreference::Intermediate;
        self
    }

    /// If the DASH manifest specifies several Adaptations with different bitrates (levels of
    /// quality), prefer the Adaptation with the lowest bitrate (smallest output file).
    #[must_use]
    pub fn worst_quality(mut self) -> DashDownloader {
        self.quality_preference = QualityPreference::Lowest;
        self
    }

    /// Specify the preferred language when multiple audio streams with different languages are
    /// available. Must be in RFC 5646 format (e.g. "fr" or "en-AU"). If a preference is not
    /// specified and multiple audio streams are present, the first one listed in the DASH manifest
    /// will be downloaded.
    //
    // TODO: this could be modified to allow a comma-separated list, or the special value "all"
    #[must_use]
    pub fn prefer_language(mut self, lang: String) -> DashDownloader {
        self.language_preference = Some(lang);
        self
    }

    /// Specify the preference ordering for Role annotations on AdaptationSet elements. Some DASH
    /// streams include multiple AdaptationSets, one annotated "main" and another "alternate", for
    /// example. If `role_preference` is ["main", "alternate"] and one of the AdaptationSets is
    /// annotated "main", then we will only download that AdaptationSet. If no role annotations are
    /// specified, this preference is ignored. This preference selection is applied before the
    /// preferences related to stream quality and video height/width: for example an AdaptationSet
    /// with role=alternate will be ignored when a role=main AdaptationSet is present, even if we
    /// also specify a quality preference for highest and the role=alternate stream has a higher
    /// quality.
    #[must_use]
    pub fn prefer_roles(mut self, role_preference: Vec<String>) -> DashDownloader {
        if role_preference.len() < u8::MAX as usize {
            self.role_preference = role_preference;
        } else {
            warn!("Ignoring role_preference ordering due to excessive length");
        }
        self
    }

    /// If the DASH manifest specifies several video Adaptations with different resolutions, prefer
    /// the Adaptation whose width is closest to the specified `width`.
    #[must_use]
    pub fn prefer_video_width(mut self, width: u64) -> DashDownloader {
        self.video_width_preference = Some(width);
        self
    }

    /// If the DASH manifest specifies several video Adaptations with different resolutions, prefer
    /// the Adaptation whose height is closest to the specified `height`.
    #[must_use]
    pub fn prefer_video_height(mut self, height: u64) -> DashDownloader {
        self.video_height_preference = Some(height);
        self
    }

    /// If the media stream has separate audio and video streams, only download the video stream.
    #[must_use]
    pub fn video_only(mut self) -> DashDownloader {
        self.fetch_audio = false;
        self.fetch_video = true;
        self
    }

    /// If the media stream has separate audio and video streams, only download the audio stream.
    #[must_use]
    pub fn audio_only(mut self) -> DashDownloader {
        self.fetch_audio = true;
        self.fetch_video = false;
        self
    }

    /// Keep the file containing video at the specified path. If the path already exists, file
    /// contents will be overwritten.
    #[must_use]
    pub fn keep_video_as<P: Into<PathBuf>>(mut self, video_path: P) -> DashDownloader {
        self.keep_video = Some(video_path.into());
        self
    }

    /// Keep the file containing audio at the specified path. If the path already exists, file
    /// contents will be overwritten.
    #[must_use]
    pub fn keep_audio_as<P: Into<PathBuf>>(mut self, audio_path: P) -> DashDownloader {
        self.keep_audio = Some(audio_path.into());
        self
    }

    /// Save media fragments to the directory `fragment_path`. The directory will be created if it
    /// does not exist.
    #[must_use]
    pub fn save_fragments_to<P: Into<PathBuf>>(mut self, fragment_path: P) -> DashDownloader {
        self.fragment_path = Some(fragment_path.into());
        self
    }

    /// Add a key to be used to decrypt MPEG media streams that use Common Encryption (cenc). This
    /// function may be called several times to specify multiple kid/key pairs. Decryption uses the
    /// external commandline application specified by `with_decryptor_preference`, run as a
    /// subprocess.
    ///
    /// # Arguments
    ///
    /// * `id` - a track ID in decimal or a 128-bit KID in hexadecimal format (32 hex characters).
    ///   Examples: "1" or "eb676abbcb345e96bbcf616630f1a3da".
    ///
    /// * `key` - a 128-bit key in hexadecimal format.
    #[must_use]
    pub fn add_decryption_key(mut self, id: String, key: String) -> DashDownloader {
        self.decryption_keys.insert(id, key);
        self
    }

    /// Register an XSLT stylesheet that will be applied to the MPD manifest after XLink processing
    /// and before deserialization into Rust structs. The stylesheet will be applied to the manifest
    /// using the xsltproc commandline tool, which supports XSLT 1.0. If multiple stylesheets are
    /// registered, they will be called in sequence in the same order as their registration. If the
    /// application of a stylesheet fails, the download will be aborted.
    ///
    /// This is an experimental API which may change in future versions of the library.
    ///
    /// # Arguments
    ///
    /// * `stylesheet`: the path to an XSLT stylesheet.
    #[must_use]
    pub fn with_xslt_stylesheet<P: Into<PathBuf>>(mut self, stylesheet: P) -> DashDownloader {
        self.xslt_stylesheets.push(stylesheet.into());
        self
    }

    /// Don't download (skip) Periods in the manifest whose duration is less than the specified
    /// value.
    #[must_use]
    pub fn minimum_period_duration(mut self, value: Duration) -> DashDownloader {
        self.minimum_period_duration = Some(value);
        self
    }

    /// Parameter `value` determines whether audio content is downloaded. If disabled, the output
    /// media file will either contain only a video track (if `fetch_video` is true and the manifest
    /// includes a video stream), or will be empty.
    #[must_use]
    pub fn fetch_audio(mut self, value: bool) -> DashDownloader {
        self.fetch_audio = value;
        self
    }

    /// Parameter `value` determines whether video content is downloaded. If disabled, the output
    /// media file will either contain only an audio track (if `fetch_audio` is true and the manifest
    /// includes an audio stream which is separate from the video stream), or will be empty.
    #[must_use]
    pub fn fetch_video(mut self, value: bool) -> DashDownloader {
        self.fetch_video = value;
        self
    }

    /// Specify whether subtitles should be fetched, if they are available. If subtitles are
    /// requested and available, they will be downloaded to a file named with the same name as the
    /// media output and an appropriate extension (".vtt", ".ttml", ".srt", etc.).
    ///
    /// # Arguments
    ///
    /// * `value`: enable or disable the retrieval of subtitles.
    #[must_use]
    pub fn fetch_subtitles(mut self, value: bool) -> DashDownloader {
        self.fetch_subtitles = value;
        self
    }

    /// For multi-Period manifests, parameter `value` determines whether the content of multiple
    /// Periods is concatenated into a single output file where their resolutions, frame rate and
    /// aspect ratios are compatible, or kept in individual files.
    #[must_use]
    pub fn concatenate_periods(mut self, value: bool) -> DashDownloader {
        self.concatenate_periods = value;
        self
    }

    /// Don't check that the content-type of downloaded segments corresponds to audio or video
    /// content (may be necessary with poorly configured HTTP servers).
    #[must_use]
    pub fn without_content_type_checks(mut self) -> DashDownloader {
        self.content_type_checks = false;
        self
    }

    /// Specify whether to check that the content-type of downloaded segments corresponds to audio
    /// or video content (this may need to be set to false with poorly configured HTTP servers).
    #[must_use]
    pub fn content_type_checks(mut self, value: bool) -> DashDownloader {
        self.content_type_checks = value;
        self
    }

    /// Specify whether to run various conformity checks on the content of the DASH manifest before
    /// downloading media segments.
    #[must_use]
    pub fn conformity_checks(mut self, value: bool) -> DashDownloader {
        self.conformity_checks = value;
        self
    }

    /// Specify whether the use the sidx/Cue index for SegmentBase@indexRange addressing.
    ///
    /// If set to true (the default value), downloads of media whose manifest uses
    /// SegmentBase@indexRange addressing will retrieve the index information (currently only sidx
    /// information used in ISOBMFF/MP4 containers; Cue information for WebM containers is currently
    /// not supported) with a byte range request, then retrieve and concatenate the different bytes
    /// ranges indicated in the index. This is the download method used by most DASH players
    /// (set-top box and browser-based). It avoids downloading the content identified by the
    /// BaseURL as a very large chunk, which can fill up RAM and may be banned by certain content
    /// servers.
    ///
    /// If set to false, the BaseURL content will be downloaded as a single large chunk. This may be
    /// more robust on certain content streams that have been encoded in a manner which is not
    /// suitable for byte range retrieval.
    #[must_use]
    pub fn use_index_range(mut self, value: bool) -> DashDownloader {
        self.use_index_range = value;
        self
    }

    /// The upper limit on the number of times to attempt to fetch a media segment, even in the
    /// presence of network errors. Transient network errors (such as timeouts) do not count towards
    /// this limit.
    #[must_use]
    pub fn fragment_retry_count(mut self, count: u32) -> DashDownloader {
        self.fragment_retry_count = count;
        self
    }

    /// The upper limit on the number of non-transient network errors encountered for this download
    /// before we abort the download.
    ///
    /// Transient network errors such as an HTTP 408 “request timeout” are retried automatically
    /// with an exponential backoff mechanism, and do not count towards this upper limit. The
    /// default is to fail after 30 non-transient network errors over the whole download.
    #[must_use]
    pub fn max_error_count(mut self, count: u32) -> DashDownloader {
        self.max_error_count = count;
        self
    }

    /// Specify a number of seconds to sleep between network requests (default 0).
    #[must_use]
    pub fn sleep_between_requests(mut self, seconds: u8) -> DashDownloader {
        self.sleep_between_requests = seconds;
        self
    }

    /// Specify whether to attempt to download from a “live” stream, or dynamic DASH manifest.
    /// Default is false.
    ///
    /// Downloading from a genuinely live stream won’t work well, because this library doesn’t
    /// implement the clock-related throttling needed to only download media segments when they
    /// become available. However, some media sources publish pseudo-live streams where all media
    /// segments are in fact available, which we will be able to download. You might also have some
    /// success in combination with the `sleep_between_requests()` method.
    ///
    /// You may also need to force a duration for the live stream using method
    /// `force_duration()`, because live streams often don’t specify a duration.
    #[must_use]
    pub fn allow_live_streams(mut self, value: bool) -> DashDownloader {
        self.allow_live_streams = value;
        self
    }

    /// Specify the number of seconds to capture from the media stream, overriding the duration
    /// specified in the DASH manifest.
    ///
    /// This is mostly useful for live streams, for which the duration is often not specified. It
    /// can also be used to capture only the first part of a normal (static/on-demand) media stream.
    #[must_use]
    pub fn force_duration(mut self, seconds: f64) -> DashDownloader {
        self.force_duration = Some(seconds);
        self
    }

    /// A maximal limit on the network bandwidth consumed to download media segments, expressed in
    /// octets (bytes) per second. No limit on bandwidth if set to zero (the default value).
    ///
    /// Limiting bandwidth below 50kB/s is not recommended, as the downloader may fail to respect
    /// this limit.
    #[must_use]
    pub fn with_rate_limit(mut self, bps: u64) -> DashDownloader {
        if bps < 10 * 1024 {
            warn!("Limiting bandwidth below 10kB/s is unlikely to be stable");
        }
        if self.verbosity > 1 {
            info!("Limiting bandwidth to {} kB/s", bps/1024);
        }
        self.rate_limit = bps;
        // Our rate_limit is in bytes/second, but the governor::RateLimiter can only handle an u32 rate.
        // We express our cells in the RateLimiter in kB/s instead of bytes/second, to allow for numbing
        // future bandwidth capacities. We need to be careful to allow a quota burst size which
        // corresponds to the size (in kB) of the largest media segments we are going to be retrieving,
        // because that's the number of bucket cells that will be consumed for each downloaded segment.
        let mut kps = 1 + bps / 1024;
        if kps > u64::from(u32::MAX) {
            warn!("Throttling bandwidth limit");
            kps = u32::MAX.into();
        }
        if let Some(bw_limit) = NonZeroU32::new(kps as u32) {
            if let Some(burst) = NonZeroU32::new(10 * 1024) {
                let bw_quota = Quota::per_second(bw_limit)
                    .allow_burst(burst);
                self.bw_limiter = Some(RateLimiter::direct(bw_quota));
            }
        }
        self
    }

    /// Set the verbosity level of the download process.
    ///
    /// # Arguments
    ///
    /// * Level - an integer specifying the verbosity level.
    /// - 0: no information is printed
    /// - 1: basic information on the number of Periods and bandwidth of selected representations
    /// - 2: information above + segment addressing mode
    /// - 3 or larger: information above + size of each downloaded segment
    #[must_use]
    pub fn verbosity(mut self, level: u8) -> DashDownloader {
        self.verbosity = level;
        self
    }

    /// Enable or disable the security sandboxing support.
    ///
    /// Security sandboxing is experimental. It is only available on Linux, when the crate is
    /// compiled with the `sandbox` feature enabled. It uses features of the Landlock LSM.
    ///
    /// # Arguments
    ///
    /// * enable - a boolean specifying whether to enable the sandboxing support. If enabling is
    ///   requested but support is not available, a warning message will be printed.
    #[must_use]
    pub fn sandbox(mut self, enable: bool) -> DashDownloader {
        #[cfg(not(all(feature = "sandbox", target_os = "linux")))]
        if enable {
            warn!("Sandboxing only available on Linux with crate feature sandbox enabled");
        }
        self.sandbox = enable;
        self
    }

    /// Specify whether to record metainformation concerning the media content (origin URL, title,
    /// source and copyright metainformation) as extended attributes in the output file, assuming
    /// this information is present in the DASH manifest.
    #[must_use]
    pub fn record_metainformation(mut self, record: bool) -> DashDownloader {
        self.record_metainformation = record;
        self
    }

    /// When muxing audio and video streams to a container of type `container`, try muxing
    /// applications following the order given by `ordering`.
    ///
    /// This function may be called multiple times to specify the ordering for different container
    /// types. If called more than once for the same container type, the ordering specified in the
    /// last call is retained.
    ///
    /// # Arguments
    ///
    /// * `container`: the container type (e.g. "mp4", "mkv", "avi")
    /// * `ordering`: the comma-separated order of preference for trying muxing applications (e.g.
    ///   "ffmpeg,vlc,mp4box")
    ///
    /// # Example
    ///
    /// ```rust
    /// let out = DashDownloader::new(url)
    ///      .with_muxer_preference("mkv", "ffmpeg")
    ///      .download_to("wonderful.mkv")
    ///      .await?;
    /// ```
    #[must_use]
    pub fn with_muxer_preference(mut self, container: &str, ordering: &str) -> DashDownloader {
        self.muxer_preference.insert(container.to_string(), ordering.to_string());
        self
    }

    /// When concatenating streams from a multi-period manifest to a container of type `container`,
    /// try concat helper applications following the order given by `ordering`.
    ///
    /// This function may be called multiple times to specify the ordering for different container
    /// types. If called more than once for the same container type, the ordering specified in the
    /// last call is retained.
    ///
    /// # Arguments
    ///
    /// * `container`: the container type (e.g. "mp4", "mkv", "avi")
    /// * `ordering`: the comma-separated order of preference for trying concat helper applications.
    ///   Valid possibilities are "ffmpeg" (the ffmpeg concat filter, slow), "ffmpegdemuxer" (the
    ///   ffmpeg concat demuxer, fast but less robust), "mkvmerge" (fast but not robust), and "mp4box".
    ///
    /// # Example
    ///
    /// ```rust
    /// let out = DashDownloader::new(url)
    ///      .with_concat_preference("mkv", "ffmpeg,mkvmerge")
    ///      .download_to("wonderful.mkv")
    ///      .await?;
    /// ```
    #[must_use]
    pub fn with_concat_preference(mut self, container: &str, ordering: &str) -> DashDownloader {
        self.concat_preference.insert(container.to_string(), ordering.to_string());
        self
    }

    /// Specify the commandline application to be used to decrypt media which has been enriched with
    /// ContentProtection (DRM).
    ///
    /// # Arguments
    ///
    /// * `decryption_tool`: one of "mp4decrypt", "shaka", "mp4box", "shaka-container",
    ///   "mp4box-container". The options with `-container` in the name are run via a Docker/Podman
    ///   container.
    #[must_use]
    pub fn with_decryptor_preference(mut self, decryption_tool: &str) -> DashDownloader {
        self.decryptor_preference = decryption_tool.to_string();
        self
    }

    /// Specify the location of the `ffmpeg` application, if not located in PATH.
    ///
    /// # Arguments
    ///
    /// * `ffmpeg_path`: the path to the ffmpeg application. If it does not specify an absolute
    ///   path, the `PATH` environment variable will be searched in a platform-specific way
    ///   (implemented in `std::process::Command`).
    ///
    /// # Example
    ///
    /// ```rust
    /// #[cfg(target_os = "unix")]
    /// let ddl = ddl.with_ffmpeg("/opt/ffmpeg-next/bin/ffmpeg");
    /// ```
    #[must_use]
    pub fn with_ffmpeg(mut self, ffmpeg_path: &str) -> DashDownloader {
        self.ffmpeg_location = ffmpeg_path.to_string();
        self
    }

    /// Specify the location of the VLC application, if not located in PATH.
    ///
    /// # Arguments
    ///
    /// * `vlc_path`: the path to the VLC application. If it does not specify an absolute
    ///   path, the `PATH` environment variable will be searched in a platform-specific way
    ///   (implemented in `std::process::Command`).
    ///
    /// # Example
    ///
    /// ```rust
    /// #[cfg(target_os = "windows")]
    /// let ddl = ddl.with_vlc("C:/Program Files/VideoLAN/VLC/vlc.exe");
    /// ```
    #[must_use]
    pub fn with_vlc(mut self, vlc_path: &str) -> DashDownloader {
        self.vlc_location = vlc_path.to_string();
        self
    }

    /// Specify the location of the mkvmerge application, if not located in PATH.
    ///
    /// # Arguments
    ///
    /// * `path`: the path to the mkvmerge application. If it does not specify an absolute
    ///   path, the `PATH` environment variable will be searched in a platform-specific way
    ///   (implemented in `std::process::Command`).
    #[must_use]
    pub fn with_mkvmerge(mut self, path: &str) -> DashDownloader {
        self.mkvmerge_location = path.to_string();
        self
    }

    /// Specify the location of the MP4Box application, if not located in PATH.
    ///
    /// # Arguments
    ///
    /// * `path`: the path to the MP4Box application. If it does not specify an absolute
    ///   path, the `PATH` environment variable will be searched in a platform-specific way
    ///   (implemented in `std::process::Command`).
    #[must_use]
    pub fn with_mp4box(mut self, path: &str) -> DashDownloader {
        self.mp4box_location = path.to_string();
        self
    }

    /// Specify the location of the Bento4 mp4decrypt application, if not located in PATH.
    ///
    /// # Arguments
    ///
    /// * `path`: the path to the mp4decrypt application. If it does not specify an absolute
    ///   path, the `PATH` environment variable will be searched in a platform-specific way
    ///   (implemented in `std::process::Command`).
    #[must_use]
    pub fn with_mp4decrypt(mut self, path: &str) -> DashDownloader {
        self.mp4decrypt_location = path.to_string();
        self
    }

    /// Specify the location of the shaka-packager application, if not located in PATH.
    ///
    /// # Arguments
    ///
    /// * `path`: the path to the shaka-packager application. If it does not specify an absolute
    ///   path, the `PATH` environment variable will be searched in a platform-specific way
    ///   (implemented in `std::process::Command`).
    #[must_use]
    pub fn with_shaka_packager(mut self, path: &str) -> DashDownloader {
        self.shaka_packager_location = path.to_string();
        self
    }

    /// Download DASH streaming media content to the file named by `out`. If the output file `out`
    /// already exists, its content will be overwritten.
    ///
    /// Note that the media container format used when muxing audio and video streams depends on the
    /// filename extension of the path `out`. If the filename extension is `.mp4`, an MPEG-4
    /// container will be used; if it is `.mkv` a Matroska container will be used, for `.webm` a
    /// WebM container (specific type of Matroska) will be used, and otherwise the heuristics
    /// implemented by the selected muxer (by default ffmpeg) will apply (e.g. an `.avi` extension
    /// will generate an AVI container).
    pub async fn download_to<P: Into<PathBuf>>(mut self, out: P) -> Result<PathBuf, DashMpdError> {
        self.output_path = Some(out.into());
        if self.http_client.is_none() {
            let client = reqwest::Client::builder()
                .timeout(Duration::new(30, 0))
                .cookie_store(true)
                .build()
                .map_err(|_| DashMpdError::Network(String::from("building HTTP client")))?;
            self.http_client = Some(client);
        }
        fetch_mpd(&mut self).await
    }

    /// Download DASH streaming media content to a file in the current working directory and return
    /// the corresponding `PathBuf`.
    ///
    /// The name of the output file is derived from the manifest URL. The output file will be
    /// overwritten if it already exists. The downloaded media will be placed in an MPEG-4
    /// container. To select another media container, see the `download_to` function.
    pub async fn download(mut self) -> Result<PathBuf, DashMpdError> {
        let cwd = env::current_dir()
            .map_err(|e| DashMpdError::Io(e, String::from("obtaining current directory")))?;
        let filename = generate_filename_from_url(&self.mpd_url);
        let outpath = cwd.join(filename);
        self.output_path = Some(outpath);
        if self.http_client.is_none() {
            let client = reqwest::Client::builder()
                .timeout(Duration::new(30, 0))
                .cookie_store(true)
                .build()
                .map_err(|_| DashMpdError::Network(String::from("building HTTP client")))?;
            self.http_client = Some(client);
        }
        fetch_mpd(&mut self).await
    }
}


fn mpd_is_dynamic(mpd: &MPD) -> bool {
    if let Some(mpdtype) = mpd.mpdtype.as_ref() {
        return mpdtype.eq("dynamic");
    }
    false
}

// Parse a range specifier, such as Initialization@range or SegmentBase@indexRange attributes, of
// the form "45-67"
fn parse_range(range: &str) -> Result<(u64, u64), DashMpdError> {
    let v: Vec<&str> = range.split_terminator('-').collect();
    if v.len() != 2 {
        return Err(DashMpdError::Parsing(format!("invalid range specifier: {range}")));
    }
    #[allow(clippy::indexing_slicing)]
    let start: u64 = v[0].parse()
        .map_err(|_| DashMpdError::Parsing(String::from("invalid start for range specifier")))?;
    #[allow(clippy::indexing_slicing)]
    let end: u64 = v[1].parse()
        .map_err(|_| DashMpdError::Parsing(String::from("invalid end for range specifier")))?;
    Ok((start, end))
}

#[derive(Debug)]
struct MediaFragment {
    period: u8,
    url: Url,
    start_byte: Option<u64>,
    end_byte: Option<u64>,
    is_init: bool,
    timeout: Option<Duration>,
}

#[derive(Debug)]
struct MediaFragmentBuilder {
    period: u8,
    url: Url,
    start_byte: Option<u64>,
    end_byte: Option<u64>,
    is_init: bool,
    timeout: Option<Duration>,
}

impl MediaFragmentBuilder {
    pub fn new(period: u8, url: Url) -> MediaFragmentBuilder {
        MediaFragmentBuilder {
            period, url, start_byte: None, end_byte: None, is_init: false, timeout: None
        }
    }

    pub fn with_range(mut self, start_byte: Option<u64>, end_byte: Option<u64>) -> MediaFragmentBuilder {
        self.start_byte = start_byte;
        self.end_byte = end_byte;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> MediaFragmentBuilder {
        self.timeout = Some(timeout);
        self
    }

    pub fn set_init(mut self) -> MediaFragmentBuilder {
        self.is_init = true;
        self
    }

    pub fn build(self) -> MediaFragment {
        MediaFragment {
            period: self.period,
            url: self.url,
            start_byte: self.start_byte,
            end_byte: self.end_byte,
            is_init: self.is_init,
            timeout: self.timeout
        }
    }
}

// This struct is used to share information concerning the media fragments identified while parsing
// a Period as being wanted for download, alongside any diagnostics information that we collected
// while parsing the Period (in particular, any ContentProtection details).
#[derive(Debug, Default)]
struct PeriodOutputs {
    fragments: Vec<MediaFragment>,
    diagnostics: Vec<String>,
    subtitle_formats: Vec<SubtitleType>,
    selected_audio_language: String,
}

#[derive(Debug, Default)]
struct PeriodDownloads {
    audio_fragments: Vec<MediaFragment>,
    video_fragments: Vec<MediaFragment>,
    subtitle_fragments: Vec<MediaFragment>,
    subtitle_formats: Vec<SubtitleType>,
    period_counter: u8,
    id: Option<String>,
    selected_audio_language: String,
}

fn period_fragment_count(pd: &PeriodDownloads) -> usize {
    pd.audio_fragments.len() +
        pd.video_fragments.len() +
        pd.subtitle_fragments.len()
}



async fn throttle_download_rate(downloader: &DashDownloader, size: u32) -> Result<(), DashMpdError> {
    if downloader.rate_limit > 0 {
        if let Some(cells) = NonZeroU32::new(size) {
            if let Some(limiter) = downloader.bw_limiter.as_ref() {
                #[allow(clippy::redundant_pattern_matching)]
                if let Err(_) = limiter.until_n_ready(cells).await {
                    return Err(DashMpdError::Other(
                        "Bandwidth limit is too low".to_string()));
                }
            }
        }
    }
    Ok(())
}


fn generate_filename_from_url(url: &str) -> PathBuf {
    use sanitise_file_name::{sanitise_with_options, Options};

    let mut path = url;
    if let Some(p) = path.strip_prefix("http://") {
        path = p;
    } else if let Some(p) = path.strip_prefix("https://") {
        path = p;
    } else if let Some(p) = path.strip_prefix("file://") {
        path = p;
    }
    if let Some(p) = path.strip_prefix("www.") {
        path = p;
    }
    if let Some(p) = path.strip_prefix("ftp.") {
        path = p;
    }
    if let Some(p) = path.strip_suffix(".mpd") {
        path = p;
    }
    let mut sanitize_opts = Options::DEFAULT;
    sanitize_opts.length_limit = 150;
    // We could also enable sanitize_opts.url_safe here.

    // We currently default to an MP4 container (could default to Matroska which is more flexible,
    // and less patent-encumbered, but perhaps less commonly supported).
    PathBuf::from(sanitise_with_options(path, &sanitize_opts) + ".mp4")
}

// A manifest containing a single Period will be saved to the output name requested by calling
// download_to("outputname.mp4") or to a name determined by generate_filename_from_url() above from
// the MPD URL.
//
// A manifest containing multiple Periods will be saved (in the general case where each period has a
// different resolution) to files whose name is built from the outputname, including the period name
// as a stem suffix (e.g. "outputname-p3.mp4" for the third period). The content of the first Period
// will be saved to a file with the requested outputname ("outputname.mp4" in this example).
//
// In the special case where each period has the same resolution (meaning that it is possible to
// concatenate the Periods into a single media container, re-encoding if the codecs used in each
// period differ), the content will be saved to a single file named as for a single Period.
//
// Illustration for a three-Period manifest with differing resolutions:
//
//    download_to("foo.mkv") => foo.mkv (Period 1), foo-p2.mkv (Period 2), foo-p3.mkv (Period 3)
fn output_path_for_period(base: &Path, period: u8) -> PathBuf {
    assert!(period > 0);
    if period == 1 {
        base.to_path_buf()
    } else {
        if let Some(stem) = base.file_stem() {
            if let Some(ext) = base.extension() {
                let fname = format!("{}-p{period}.{}", stem.to_string_lossy(), ext.to_string_lossy());
                return base.with_file_name(fname);
            }
        }
        let p = format!("dashmpd-p{period}");
        tmp_file_path(&p, base.extension().unwrap_or(OsStr::new("mp4")))
            .unwrap_or_else(|_| p.into())
    }
}

fn is_absolute_url(s: &str) -> bool {
    s.starts_with("http://") ||
        s.starts_with("https://") ||
        s.starts_with("file://") ||
        s.starts_with("ftp://")
}

fn merge_baseurls(current: &Url, new: &str) -> Result<Url, DashMpdError> {
    if is_absolute_url(new) {
        Url::parse(new)
            .map_err(|e| parse_error("parsing BaseURL", e))
    } else {
        // We are careful to merge the query portion of the current URL (which is either the
        // original manifest URL, or the URL that it redirected to, or the value of a BaseURL
        // element in the manifest) with the new URL. But if the new URL already has a query string,
        // it takes precedence.
        //
        // Examples
        //
        // merge_baseurls(https://example.com/manifest.mpd?auth=secret, /video42.mp4) =>
        //   https://example.com/video42.mp4?auth=secret
        //
        // merge_baseurls(https://example.com/manifest.mpd?auth=old, /video42.mp4?auth=new) =>
        //   https://example.com/video42.mp4?auth=new
        let mut merged = current.join(new)
            .map_err(|e| parse_error("joining base with BaseURL", e))?;
        if merged.query().is_none() {
            merged.set_query(current.query());
        }
        Ok(merged)
    }
}

// Return true if the response includes a content-type header corresponding to audio. We need to
// allow "video/" MIME types because some servers return "video/mp4" content-type for audio segments
// in an MP4 container, and we accept application/octet-stream headers because some servers are
// poorly configured.
fn content_type_audio_p(response: &reqwest::Response) -> bool {
    match response.headers().get("content-type") {
        Some(ct) => {
            let ctb = ct.as_bytes();
            ctb.starts_with(b"audio/") ||
                ctb.starts_with(b"video/") ||
                ctb.starts_with(b"application/octet-stream")
        },
        None => false,
    }
}

// Return true if the response includes a content-type header corresponding to video.
fn content_type_video_p(response: &reqwest::Response) -> bool {
    match response.headers().get("content-type") {
        Some(ct) => {
            let ctb = ct.as_bytes();
            ctb.starts_with(b"video/") ||
                ctb.starts_with(b"application/octet-stream")
        },
        None => false,
    }
}


// Return a measure of the distance between this AdaptationSet's lang attribute and the language
// code specified by language_preference. If the AdaptationSet node has no lang attribute, return an
// arbitrary large distance.
fn adaptation_lang_distance(a: &AdaptationSet, language_preference: &str) -> u8 {
    if let Some(lang) = &a.lang {
        if lang.eq(language_preference) {
            return 0;
        }
        if lang[0..2].eq(&language_preference[0..2]) {
            return 5;
        }
        100
    } else {
        100
    }
}

// We can have a <Role value="foobles"> element directly within the AdaptationSet element, or within
// a ContentComponent element in the AdaptationSet.
fn adaptation_roles(a: &AdaptationSet) -> Vec<String> {
    let mut roles = Vec::new();
    for r in &a.Role {
        if let Some(rv) = &r.value {
            roles.push(String::from(rv));
        }
    }
    for cc in &a.ContentComponent {
        for r in &cc.Role {
            if let Some(rv) = &r.value {
                roles.push(String::from(rv));
            }
        }
    }
    roles
}

// Best possible "score" is zero. 
fn adaptation_role_distance(a: &AdaptationSet, role_preference: &[String]) -> u8 {
    adaptation_roles(a).iter()
        .map(|r| role_preference.binary_search(r).unwrap_or(u8::MAX.into()))
        .map(|u| u8::try_from(u).unwrap_or(u8::MAX))
        .min()
        .unwrap_or(u8::MAX)
}


// We select the AdaptationSets that correspond to our language preference, and if there are several
// with our language preference, that with the role according to role_preference, and if no
// role_preference, return all adaptations.
//
// Start by getting a Vec of adaptation_lang_distance
// Take the min and collect all Adaptations where dist = min_distance
// then apply role_preference
fn select_preferred_adaptations<'a>(
    adaptations: Vec<&'a AdaptationSet>,
    downloader: &DashDownloader) -> Vec<&'a AdaptationSet>
{
    let mut preferred: Vec<&'a AdaptationSet>;
    // TODO: modify this algorithm to allow for multiple preferred languages
    if let Some(ref lang) = downloader.language_preference {
        preferred = Vec::new();
        let distance: Vec<u8> = adaptations.iter()
            .map(|a| adaptation_lang_distance(a, lang))
            .collect();
        let min_distance = distance.iter().min().unwrap_or(&0);
        for (i, a) in adaptations.iter().enumerate() {
            if let Some(di) = distance.get(i) {
                if di == min_distance {
                    preferred.push(a);
                }
            }
        }
    } else {
        preferred = adaptations;
    }
    // Apply the role_preference. For example, a role_preference of ["main", "alternate",
    // "supplementary", "commentary"] means we should prefer an AdaptationSet with role=main, and
    // return only that AdaptationSet. If there are no role annotations on the AdaptationSets, or
    // the specified roles don't match anything in our role_preference ordering, then all
    // AdaptationSets will receive the maximum distance and they will all be returned.
    let role_distance: Vec<u8> = preferred.iter()
        .map(|a| adaptation_role_distance(a, &downloader.role_preference))
        .collect();
    let role_distance_min = role_distance.iter().min().unwrap_or(&0);
    let mut best = Vec::new();
    for (i, a) in preferred.into_iter().enumerate() {
        if let Some(rdi) = role_distance.get(i) {
            if rdi == role_distance_min {
                best.push(a);
            }
        }
    }
    best
}


// A manifest often contains multiple video Representations with different bandwidths and video
// resolutions. We select the Representation to download by ranking following the user's specified
// quality preference. We first rank following the @qualityRanking attribute if it is present, and
// otherwise by the bandwidth specified. Note that quality ranking may be different from bandwidth
// ranking when different codecs are used.
fn select_preferred_representation<'a>(
    representations: &[&'a Representation],
    downloader: &DashDownloader) -> Option<&'a Representation>
{
    if representations.iter().all(|x| x.qualityRanking.is_some()) {
        // rank according to the @qualityRanking attribute (lower values represent
        // higher quality content)
        match downloader.quality_preference {
            QualityPreference::Lowest =>
                representations.iter()
                .max_by_key(|r| r.qualityRanking.unwrap_or(u8::MAX))
                .copied(),
            QualityPreference::Highest =>
                representations.iter().min_by_key(|r| r.qualityRanking.unwrap_or(0))
                    .copied(),
            QualityPreference::Intermediate => {
                let count = representations.len();
                match count {
                    0 => None,
                    1 => Some(representations[0]),
                    _ => {
                        let mut ranking: Vec<u8> = representations.iter()
                            .map(|r| r.qualityRanking.unwrap_or(u8::MAX))
                            .collect();
                        ranking.sort_unstable();
                        if let Some(want_ranking) = ranking.get(count / 2) {
                            representations.iter()
                                .find(|r| r.qualityRanking.unwrap_or(u8::MAX) == *want_ranking)
                                .copied()
                        } else {
                            representations.first().copied()
                        }
                    },
                }
            },
        }
    } else {
        // rank according to the bandwidth attribute (lower values imply lower quality)
        match downloader.quality_preference {
            QualityPreference::Lowest => representations.iter()
                .min_by_key(|r| r.bandwidth.unwrap_or(1_000_000_000))
                .copied(),
            QualityPreference::Highest => representations.iter()
                .max_by_key(|r| r.bandwidth.unwrap_or(0))
                .copied(),
            QualityPreference::Intermediate => {
                let count = representations.len();
                match count {
                    0 => None,
                    1 => Some(representations[0]),
                    _ => {
                        let mut ranking: Vec<u64> = representations.iter()
                            .map(|r| r.bandwidth.unwrap_or(100_000_000))
                            .collect();
                        ranking.sort_unstable();
                        if let Some(want_ranking) = ranking.get(count / 2) {
                            representations.iter()
                                .find(|r| r.bandwidth.unwrap_or(100_000_000) == *want_ranking)
                                .copied()
                        } else {
                            representations.first().copied()
                        }
                    },
                }
            },
        }
    }
}


// The AdaptationSet a is the parent of the Representation r.
fn print_available_subtitles_representation(r: &Representation, a: &AdaptationSet) {
    let unspecified = "<unspecified>".to_string();
    let empty = "".to_string();
    let lang = r.lang.as_ref().unwrap_or(a.lang.as_ref().unwrap_or(&unspecified));
    let codecs = r.codecs.as_ref().unwrap_or(a.codecs.as_ref().unwrap_or(&empty));
    let typ = subtitle_type(&a);
    let stype = if !codecs.is_empty() {
        format!("{typ:?}/{codecs}")
    } else {
        format!("{typ:?}")
    };
    let role = a.Role.first()
        .map_or_else(|| String::from(""),
                     |r| r.value.as_ref().map_or_else(|| String::from(""), |v| format!(" role={v}")));
    let label = a.Label.first()
        .map_or_else(|| String::from(""), |l| format!(" label={}", l.clone().content));
    info!("  subs {stype:>18} | {lang:>10} |{role}{label}");
}

fn print_available_subtitles_adaptation(a: &AdaptationSet) {
    a.representations.iter()
        .for_each(|r| print_available_subtitles_representation(r, a));
}

// The AdaptationSet a is the parent of the Representation r.
fn print_available_streams_representation(r: &Representation, a: &AdaptationSet, typ: &str) {
    // for now, we ignore the Vec representation.SubRepresentation which could contain width, height, bw etc.
    let unspecified = "<unspecified>".to_string();
    let w = r.width.unwrap_or(a.width.unwrap_or(0));
    let h = r.height.unwrap_or(a.height.unwrap_or(0));
    let codec = r.codecs.as_ref().unwrap_or(a.codecs.as_ref().unwrap_or(&unspecified));
    let bw = r.bandwidth.unwrap_or(a.maxBandwidth.unwrap_or(0));
    let fmt = if typ.eq("audio") {
        let unknown = String::from("?");
        format!("lang={}", r.lang.as_ref().unwrap_or(a.lang.as_ref().unwrap_or(&unknown)))
    } else if w == 0 || h == 0 {
        // Some MPDs do not specify width and height, such as
        // https://dash.akamaized.net/fokus/adinsertion-samples/scte/dash.mpd
        String::from("")
    } else {
        format!("{w}x{h}")
    };
    let role = a.Role.first()
        .map_or_else(|| String::from(""),
                     |r| r.value.as_ref().map_or_else(|| String::from(""), |v| format!(" role={v}")));
    let label = a.Label.first()
        .map_or_else(|| String::from(""), |l| format!(" label={}", l.clone().content));
    info!("  {typ} {codec:17} | {:5} Kbps | {fmt:>9}{role}{label}", bw / 1024);
}

fn print_available_streams_adaptation(a: &AdaptationSet, typ: &str) {
    a.representations.iter()
        .for_each(|r| print_available_streams_representation(r, a, typ));
}

fn print_available_streams_period(p: &Period) {
    p.adaptations.iter()
        .filter(is_audio_adaptation)
        .for_each(|a| print_available_streams_adaptation(a, "audio"));
    p.adaptations.iter()
        .filter(is_video_adaptation)
        .for_each(|a| print_available_streams_adaptation(a, "video"));
    p.adaptations.iter()
        .filter(is_subtitle_adaptation)
        .for_each(print_available_subtitles_adaptation);
}

#[tracing::instrument(level="trace", skip_all)]
fn print_available_streams(mpd: &MPD) {
    use humantime::format_duration;

    let mut counter = 0;
    for p in &mpd.periods {
        let mut period_duration_secs: f64 = -1.0;
        if let Some(d) = mpd.mediaPresentationDuration {
            period_duration_secs = d.as_secs_f64();
        }
        if let Some(d) = &p.duration {
            period_duration_secs = d.as_secs_f64();
        }
        counter += 1;
        let duration = if period_duration_secs > 0.0 {
            format_duration(Duration::from_secs_f64(period_duration_secs)).to_string()
        } else {
            String::from("unknown")
        };
        if let Some(id) = p.id.as_ref() {
            info!("Streams in period {id} (#{counter}), duration {duration}:");
        } else {
            info!("Streams in period #{counter}, duration {duration}:");
        }
        print_available_streams_period(p);
    }
}

async fn extract_init_pssh(downloader: &DashDownloader, init_url: Url) -> Option<Vec<u8>> {
    use bstr::ByteSlice;
    use hex_literal::hex;

    if let Some(client) = downloader.http_client.as_ref() {
        let mut req = client.get(init_url);
        if let Some(referer) = &downloader.referer {
            req = req.header("Referer", referer);
        }
        if let Some(username) = &downloader.auth_username {
            if let Some(password) = &downloader.auth_password {
                req = req.basic_auth(username, Some(password));
            }
        }
        if let Some(token) = &downloader.auth_bearer_token {
            req = req.bearer_auth(token);
        }
        if let Ok(mut resp) = req.send().await {
            // We only download the first bytes of the init segment, because it may be very large in the
            // case of indexRange adressing, and we don't want to fill up RAM.
            let mut chunk_counter = 0;
            let mut segment_first_bytes = Vec::<u8>::new();
            while let Ok(Some(chunk)) = resp.chunk().await {
                let size = min((chunk.len()/1024+1) as u32, u32::MAX);
                #[allow(clippy::redundant_pattern_matching)]
                if let Err(_) = throttle_download_rate(downloader, size).await {
                    return None;
                }
                segment_first_bytes.append(&mut chunk.to_vec());
                chunk_counter += 1;
                if chunk_counter > 20 {
                    break;
                }
            }
            let needle = b"pssh";
            for offset in segment_first_bytes.find_iter(needle) {
                #[allow(clippy::needless_range_loop)]
                for i in offset-4..offset+2 {
                    if let Some(b) = segment_first_bytes.get(i) {
                        if *b != 0 {
                            continue;
                        }
                    }
                }
                #[allow(clippy::needless_range_loop)]
                for i in offset+4..offset+8 {
                    if let Some(b) = segment_first_bytes.get(i) {
                        if *b != 0 {
                            continue;
                        }
                    }
                }
                if offset+24 > segment_first_bytes.len() {
                    continue;
                }
                // const PLAYREADY_SYSID: [u8; 16] = hex!("9a04f07998404286ab92e65be0885f95");
                const WIDEVINE_SYSID: [u8; 16] = hex!("edef8ba979d64acea3c827dcd51d21ed");
                if let Some(sysid) = segment_first_bytes.get((offset+8)..(offset+24)) {
                    if !sysid.eq(&WIDEVINE_SYSID) {
                        continue;
                    }
                }
                if let Some(length) = segment_first_bytes.get(offset-1) {
                    let start = offset - 4;
                    let end = start + *length as usize;
                    if let Some(pssh) = &segment_first_bytes.get(start..end) {
                        return Some(pssh.to_vec());
                    }
                }
            }
        }
        None
    } else {
        None
    }
}


// From https://dashif.org/docs/DASH-IF-IOP-v4.3.pdf:
// "For the avoidance of doubt, only %0[width]d is permitted and no other identifiers. The reason
// is that such a string replacement can be easily implemented without requiring a specific library."
//
// Instead of pulling in C printf() or a reimplementation such as the printf_compat crate, we reimplement
// this functionality directly.
//
// Example template: "$RepresentationID$/$Number%06d$.m4s"
lazy_static! {
    static ref URL_TEMPLATE_IDS: Vec<(&'static str, String, Regex)> = {
        vec!["RepresentationID", "Number", "Time", "Bandwidth"].into_iter()
            .map(|k| (k, format!("${k}$"), Regex::new(&format!("\\${k}%0([\\d])d\\$")).unwrap()))
            .collect()
    };
}

fn resolve_url_template(template: &str, params: &HashMap<&str, String>) -> String {
    let mut result = template.to_string();
    for (k, ident, rx) in URL_TEMPLATE_IDS.iter() {
        // first check for simple cases such as $Number$
        if result.contains(ident) {
            if let Some(value) = params.get(k as &str) {
                result = result.replace(ident, value);
            }
        }
        // now check for complex cases such as $Number%06d$
        if let Some(cap) = rx.captures(&result) {
            if let Some(value) = params.get(k as &str) {
                if let Ok(width) = cap[1].parse::<usize>() {
                    if let Some(m) = rx.find(&result) {
                        let count = format!("{value:0>width$}");
                        result = result[..m.start()].to_owned() + &count + &result[m.end()..];
                    }
                }
            }
        }
    }
    result
}


fn reqwest_error_transient_p(e: &reqwest::Error) -> bool {
    if e.is_timeout() {
        return true;
    }
    if let Some(s) = e.status() {
        if s == reqwest::StatusCode::REQUEST_TIMEOUT ||
            s == reqwest::StatusCode::TOO_MANY_REQUESTS ||
            s == reqwest::StatusCode::SERVICE_UNAVAILABLE ||
            s == reqwest::StatusCode::GATEWAY_TIMEOUT {
                return true;
            }
    }
    false
}

fn notify_transient<E: std::fmt::Debug>(err: &E, dur: Duration) {
    warn!("Transient error after {dur:?}: {err:?}");
}

fn network_error(why: &str, e: &reqwest::Error) -> DashMpdError {
    if e.is_timeout() {
        DashMpdError::NetworkTimeout(format!("{why}: {e:?}"))
    } else if e.is_connect() {
        DashMpdError::NetworkConnect(format!("{why}: {e:?}"))
    } else {
        DashMpdError::Network(format!("{why}: {e:?}"))
    }
}

fn parse_error(why: &str, e: impl std::error::Error) -> DashMpdError {
    DashMpdError::Parsing(format!("{why}: {e:#?}"))
}


// This would be easier with middleware such as https://lib.rs/crates/tower-reqwest or
// https://lib.rs/crates/reqwest-retry or https://docs.rs/again/latest/again/
// or https://github.com/naomijub/tokio-retry
async fn reqwest_bytes_with_retries(
    client: &reqwest::Client,
    req: reqwest::Request,
    retry_count: u32) -> Result<Bytes, reqwest::Error>
{
    let mut last_error = None;
    for _ in 0..retry_count {
        if let Some(rqw) = req.try_clone() {
            match client.execute(rqw).await {
                Ok(response) => {
                    match response.error_for_status() {
                        Ok(resp) => {
                            match resp.bytes().await {
                                Ok(bytes) => return Ok(bytes),
                                Err(e) => {
                                    info!("Retrying after HTTP error {e:?}");
                                    last_error = Some(e);
                                },
                            }
                        },
                        Err(e) => {
                            info!("Retrying after HTTP error {e:?}");
                            last_error = Some(e);
                        },
                    }
                },
                Err(e) => {
                    info!("Retrying after HTTP error {e:?}");
                    last_error = Some(e);
                },
            }
        }
    }
    Err(last_error.unwrap())
}

// As per https://www.freedesktop.org/wiki/CommonExtendedAttributes/, set extended filesystem
// attributes indicating metadata such as the origin URL, title, source and copyright, if
// specified in the MPD manifest. This functionality is only active on platforms where the xattr
// crate supports extended attributes (currently Android, Linux, MacOS, FreeBSD, and NetBSD); on
// unsupported Unix platforms it's a no-op. On other non-Unix platforms the crate doesn't build.
//
// TODO: on Windows, could use NTFS Alternate Data Streams
// https://en.wikipedia.org/wiki/NTFS#Alternate_data_stream_(ADS)
//
// We could also include a certain amount of metainformation (title, copyright) in the video
// container metadata, though this would have to be implemented separately by each muxing helper and
// each concat helper application in the ffmpeg module.
#[allow(unused_variables)]
fn maybe_record_metainformation(path: &Path, downloader: &DashDownloader, mpd: &MPD) {
    #[cfg(target_family = "unix")]
    if downloader.record_metainformation && (downloader.fetch_audio || downloader.fetch_video) {
        if let Ok(origin_url) = Url::parse(&downloader.mpd_url) {
            // Don't record the origin URL if it contains sensitive information such as passwords
            #[allow(clippy::collapsible_if)]
            if origin_url.username().is_empty() && origin_url.password().is_none() {
                #[cfg(target_family = "unix")]
                if xattr::set(path, "user.xdg.origin.url", downloader.mpd_url.as_bytes()).is_err() {
                    info!("Failed to set user.xdg.origin.url xattr on output file");
                }
            }
            for pi in &mpd.ProgramInformation {
                if let Some(t) = &pi.Title {
                    if let Some(tc) = &t.content {
                        if xattr::set(path, "user.dublincore.title", tc.as_bytes()).is_err() {
                            info!("Failed to set user.dublincore.title xattr on output file");
                        }
                    }
                }
                if let Some(source) = &pi.Source {
                    if let Some(sc) = &source.content {
                        if xattr::set(path, "user.dublincore.source", sc.as_bytes()).is_err() {
                            info!("Failed to set user.dublincore.source xattr on output file");
                        }
                    }
                }
                if let Some(copyright) = &pi.Copyright {
                    if let Some(cc) = &copyright.content {
                        if xattr::set(path, "user.dublincore.rights", cc.as_bytes()).is_err() {
                            info!("Failed to set user.dublincore.rights xattr on output file");
                        }
                    }
                }
            }
        }
    }
}

// From the DASH-IF-IOP-v4.0 specification, "If the value of the @xlink:href attribute is
// urn:mpeg:dash:resolve-to-zero:2013, HTTP GET request is not issued, and the in-MPD element shall
// be removed from the MPD."
fn fetchable_xlink_href(href: &str) -> bool {
    (!href.is_empty()) && href.ne("urn:mpeg:dash:resolve-to-zero:2013")
}

fn element_resolves_to_zero(xot: &mut Xot, element: xot::Node) -> bool {
    let xlink_ns = xmlname::CreateNamespace::new(xot, "xlink", "http://www.w3.org/1999/xlink");
    let xlink_href_name = xmlname::CreateName::namespaced(xot, "href", &xlink_ns);
    if let Some(href) = xot.get_attribute(element, xlink_href_name.into()) {
        return href.eq("urn:mpeg:dash:resolve-to-zero:2013");
    }
    false
}

fn skip_xml_preamble(input: &str) -> &str {
    if input.starts_with("<?xml") {
        if let Some(end_pos) = input.find("?>") {
            // Return the part of the string after the XML declaration
            return &input[end_pos + 2..]; // Skip past "?>"
        }
    }
    // If no XML preamble, return the original string
    input
}

// Run user-specified XSLT stylesheets on the manifest, using xsltproc (a component of libxslt)
// as a commandline filter application. Existing XSLT implementations in Rust are incomplete
// (but improving; hopefully we will one day be able to use the xrust crate).
async fn apply_xslt_stylesheets_xsltproc(
    downloader: &DashDownloader,
    xot: &mut Xot,
    doc: xot::Node) -> Result<String, DashMpdError> {
    let mut buf = Vec::new();
    xot.write(doc, &mut buf)
        .map_err(|e| parse_error("serializing rewritten manifest", e))?;
    for ss in &downloader.xslt_stylesheets {
        if downloader.verbosity > 0 {
            info!("Applying XSLT stylesheet {} with xsltproc", ss.display());
        }
        let tmpmpd = tmp_file_path("dashxslt", OsStr::new("xslt"))?;
        fs::write(&tmpmpd, &buf).await
            .map_err(|e| DashMpdError::Io(e, String::from("writing MPD")))?;
        let xsltproc = Command::new("xsltproc")
            .args([ss, &tmpmpd])
            .output()
            .map_err(|e| DashMpdError::Io(e, String::from("spawning xsltproc")))?;
        if !xsltproc.status.success() {
            let msg = format!("xsltproc returned {}", xsltproc.status);
            let out = partial_process_output(&xsltproc.stderr).to_string();
            return Err(DashMpdError::Io(std::io::Error::other(msg), out));
        }
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
            if let Err(e) = fs::remove_file(&tmpmpd).await {
                warn!("Error removing temporary MPD after XSLT processing: {e:?}");
            }
        }
        buf.clone_from(&xsltproc.stdout);
        if downloader.verbosity > 2 {
            println!("Rewritten XSLT: {}", String::from_utf8_lossy(&buf));
        }
    }
    String::from_utf8(buf)
        .map_err(|e| parse_error("parsing UTF-8", e))
}

// Try to use the xee crate functionality for XSLT processing. We need an alternative utility
// function to evaluate that accepts a full XSLT stylehseet, rather than only the XML for a
// transform.
/*
fn apply_xslt_stylesheets_xee(
    downloader: &DashDownloader,
    xot: &mut Xot,
    doc: xot::Node) -> Result<String, DashMpdError> {
    use xee_xslt_compiler::evaluate;
    use std::fmt::Write;

    let mut xml = xot.to_string(doc)
        .map_err(|e| parse_error("serializing rewritten manifest", e))?;
    for ss in &downloader.xslt_stylesheets {
        if downloader.verbosity > 0 {
            info!("  Applying XSLT stylesheet {} with xee", ss.display());
        }
        let xslt = fs::read_to_string(ss)
            .map_err(|_| DashMpdError::Other(String::from("reading XSLT stylesheet")))?;
        let seq = evaluate(xot, &xml, &xslt).unwrap();
        let mut f = String::new();
        for item in seq.iter() {
            f.write_str(&xot.to_string(item.to_node().unwrap()).unwrap())
                .unwrap();
        }
        xml = f;
    }
    Ok(xml)
}
*/

// Walk all descendents of the root node, looking for target nodes with an xlink:href and collect
// into a Vec. For each of these, retrieve the remote content, insert_after() the target node, then
// delete the target node.
async fn resolve_xlink_references(
    downloader: &DashDownloader,
    xot: &mut Xot,
    node: xot::Node) -> Result<(), DashMpdError>
{
    let xlink_ns = xmlname::CreateNamespace::new(xot, "xlink", "http://www.w3.org/1999/xlink");
    let xlink_href_name = xmlname::CreateName::namespaced(xot, "href", &xlink_ns);
    let xlinked = xot.descendants(node)
        .filter(|d| xot.get_attribute(*d, xlink_href_name.into()).is_some())
        .collect::<Vec<_>>();
    for xl in xlinked {
        if element_resolves_to_zero(xot, xl) {
            trace!("Removing node with resolve-to-zero xlink:href {xl:?}");
            if let Err(e) = xot.remove(xl) {
                return Err(parse_error("Failed to remove resolve-to-zero XML node", e));
            }
        } else if let Some(href) = xot.get_attribute(xl, xlink_href_name.into()) {
            if fetchable_xlink_href(href) {
                let xlink_url = if is_absolute_url(href) {
                    Url::parse(href)
                        .map_err(|e|
                            if let Ok(ns) = xot.to_string(node) {
                                parse_error(&format!("parsing XLink on {ns}"), e)
                            } else {
                                parse_error("parsing XLink", e)
                            }
                        )?
                } else {
                    // Note that we are joining against the original/redirected URL for the MPD, and
                    // not against the currently scoped BaseURL
                    let mut merged = downloader.redirected_url.join(href)
                        .map_err(|e|
                            if let Ok(ns) = xot.to_string(node) {
                                parse_error(&format!("parsing XLink on {ns}"), e)
                            } else {
                                parse_error("parsing XLink", e)
                            }
                        )?;
                    merged.set_query(downloader.redirected_url.query());
                    merged
                };
                let client = downloader.http_client.as_ref().unwrap();
                trace!("Fetching XLinked element {}", xlink_url.clone());
                let mut req = client.get(xlink_url.clone())
                    .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                    .header("Accept-Language", "en-US,en")
                    .header("Sec-Fetch-Mode", "navigate");
                if let Some(referer) = &downloader.referer {
                    req = req.header("Referer", referer);
                } else {
                    req = req.header("Referer", downloader.redirected_url.to_string());
                }
                if let Some(username) = &downloader.auth_username {
                    if let Some(password) = &downloader.auth_password {
                        req = req.basic_auth(username, Some(password));
                    }
                }
                if let Some(token) = &downloader.auth_bearer_token {
                    req = req.bearer_auth(token);
                }
                let xml = req.send().await
                    .map_err(|e|
                             if let Ok(ns) = xot.to_string(node) {
                                 network_error(&format!("fetching XLink for {ns}"), &e)
                             } else {
                                 network_error("fetching XLink", &e)
                             }
                        )?
                    .error_for_status()
                    .map_err(|e|
                             if let Ok(ns) = xot.to_string(node) {
                                 network_error(&format!("fetching XLink for {ns}"), &e)
                             } else {
                                 network_error("fetching XLink", &e)
                             }
                        )?
                    .text().await
                    .map_err(|e|
                             if let Ok(ns) = xot.to_string(node) {
                                 network_error(&format!("resolving XLink for {ns}"), &e)
                             } else {
                                 network_error("resolving XLink", &e)
                             }
                        )?;
                if downloader.verbosity > 2 {
                    if let Ok(ns) = xot.to_string(node) {
                        info!("  Resolved onLoad XLink {xlink_url} on {ns} -> {} octets", xml.len());
                    } else {
                        info!("  Resolved onLoad XLink {xlink_url} -> {} octets", xml.len());
                    }
                }
                // The difficulty here is that the XML fragment received may contain multiple elements,
                // for example a Period with xlink resolves to two Period elements. For a single
                // resolved element we can simply replace the original element by its resolved
                // counterpart. When the xlink resolves to multiple elements, we can't insert them back
                // into the parent node directly, but need to return them to the caller for later insertion.
                let wrapped_xml = r#"<?xml version="1.0" encoding="utf-8"?>"#.to_owned() +
                    r#"<wrapper xmlns="urn:mpeg:dash:schema:mpd:2011" "# +
                    r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" "# +
                    r#"xmlns:cenc="urn:mpeg:cenc:2013" "# +
                    r#"xmlns:mspr="urn:microsoft:playready" "# +
                    r#"xmlns:xlink="http://www.w3.org/1999/xlink">"# +
                    skip_xml_preamble(&xml) +
                    r"</wrapper>";
                let wrapper_doc = xot.parse(&wrapped_xml)
                    .map_err(|e| parse_error("parsing xlinked content", e))?;
                let wrapper_doc_el = xot.document_element(wrapper_doc)
                    .map_err(|e| parse_error("extracting XML document element", e))?;
                for needs_insertion in xot.children(wrapper_doc_el).collect::<Vec<_>>() {
                    // FIXME we are inserting nodes that serialize to nothing (namespace nodes?)
                    xot.insert_after(xl, needs_insertion)
                        .map_err(|e| parse_error("inserting XLinked content", e))?;
                }
                xot.remove(xl)
                    .map_err(|e| parse_error("removing XLink node", e))?;
            }
        }
    }
    Ok(())
}

#[tracing::instrument(level="trace", skip_all)]
pub async fn parse_resolving_xlinks(
    downloader: &DashDownloader,
    xml: &[u8]) -> Result<MPD, DashMpdError>
{
    use xot::xmlname::NameStrInfo;

    let mut xot = Xot::new();
    let doc = xot.parse_bytes(xml)
        .map_err(|e| parse_error("XML parsing", e))?;
    let doc_el = xot.document_element(doc)
        .map_err(|e| parse_error("extracting XML document element", e))?;
    let doc_name = match xot.node_name(doc_el) {
        Some(n) => n,
        None => return Err(DashMpdError::Parsing(String::from("missing root node name"))),
    };
    let root_name = xot.name_ref(doc_name, doc_el)
        .map_err(|e| parse_error("extracting root node name", e))?;
    let root_local_name = root_name.local_name();
    if !root_local_name.eq("MPD") {
        return Err(DashMpdError::Parsing(format!("root element is {root_local_name}, expecting <MPD>")));
    }
    // The remote XLink fragments may contain further XLink references. However, we only repeat the
    // resolution 5 times to avoid potential infloop DoS attacks.
    for _ in 1..5 {
        resolve_xlink_references(downloader, &mut xot, doc).await?;
    }
    let rewritten = apply_xslt_stylesheets_xsltproc(downloader, &mut xot, doc).await?;
    // Here using the quick-xml serde support to deserialize into Rust structs.
    let mpd = parse(&rewritten)?;
    if downloader.conformity_checks {
        for emsg in check_conformity(&mpd) {
            warn!("DASH conformity error in manifest: {emsg}");
        }
    }
    Ok(mpd)
}

async fn do_segmentbase_indexrange(
    downloader: &DashDownloader,
    period_counter: u8,
    base_url: Url,
    sb: &SegmentBase,
    dict: &HashMap<&str, String>
) -> Result<Vec<MediaFragment>, DashMpdError>
{
    // Something like the following
    //
    // <SegmentBase indexRange="839-3534" timescale="12288">
    //   <Initialization range="0-838"/>
    // </SegmentBase>
    //
    // The SegmentBase@indexRange attribute points to a byte range in the media file
    // that contains index information (an sidx box for MPEG files, or a Cues entry for
    // a DASH-WebM stream). There are two possible strategies to implement when downloading this content:
    //
    //   - Simply download the full content specified by the BaseURL element for this
    //     segment (ignoring the indexRange attribute).
    //
    //   - Download the sidx box using a Range request, parse the segment references it
    //     contains, and download each one using a different Range request, and
    //     concatenate the full contents.
    //
    // The first option is what a browser-based player does. It avoids making a huge
    // segment download that will fill up our RAM if chunked download is not offered by
    // the server. It works with web servers that prevent direct access to the full
    // MP4/WebM file by blocking requests without a limited byte range. Its more
    // correct, because in theory the content at BaseURL might contain lots of
    // irrelevant information which is not pointed to by any of the sidx byte ranges.
    // However, it is a little more fragile because some MP4 elements that are necessary
    // to create a valid MP4 file (e.g. trex, trun, tfhd boxes) might not be included in
    // the sidx-referenced byte ranges.
    //
    // In practice, it seems that the indexRange information is mostly provided by DASH
    // encoders to allow clients to rewind and fast-forward a stream, and both
    // strategies work. We default to using the indexRange information, but include the
    // option parse_index_range to allow fallback to the simpler "download-it-all"
    // strategy.
    let mut fragments = Vec::new();
    let mut start_byte: Option<u64> = None;
    let mut end_byte: Option<u64> = None;
    let mut indexable_segments = false;
    if downloader.use_index_range {
        if let Some(ir) = &sb.indexRange {
            // Fetch the octet slice corresponding to the (sidx) index.
            let (s, e) = parse_range(ir)?;
            trace!("Fetching sidx for {}", base_url.clone());
            let mut req = downloader.http_client.as_ref()
                .unwrap()
                .get(base_url.clone())
                .header(RANGE, format!("bytes={s}-{e}"))
                .header("Referer", downloader.redirected_url.to_string())
                .header("Sec-Fetch-Mode", "navigate");
            if let Some(username) = &downloader.auth_username {
                if let Some(password) = &downloader.auth_password {
                    req = req.basic_auth(username, Some(password));
                }
            }
            if let Some(token) = &downloader.auth_bearer_token {
                req = req.bearer_auth(token);
            }
            let mut resp = req.send().await
                .map_err(|e| network_error("fetching index data", &e))?
                .error_for_status()
                .map_err(|e| network_error("fetching index data", &e))?;
            let headers = std::mem::take(resp.headers_mut());
            if let Some(content_type) = headers.get(CONTENT_TYPE) {
                let idx = resp.bytes().await
                    .map_err(|e| network_error("fetching index data", &e))?;
                if idx.len() as u64 != e - s + 1 {
                    warn!("  HTTP server does not support Range requests; can't use indexRange addressing");
                } else {
                    #[allow(clippy::collapsible_else_if)]
                    if content_type.eq("video/mp4") ||
                        content_type.eq("audio/mp4") {
                            // Handle as ISOBMFF. First prepare to save the index data itself
                            // and any leading bytes (from byte positions 0 to s) to the output
                            // container, because it may contain other types of MP4 boxes than
                            // only sidx boxes (eg. trex, trun tfhd boxes), which are necessary
                            // to play the media content. Then prepare to save each referenced
                            // segment chunk to the output container.
                            let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                                .with_range(Some(0), Some(e))
                                .build();
                            fragments.push(mf);
                            let mut max_chunk_pos = 0;
                            if let Ok(segment_chunks) = crate::sidx::from_isobmff_sidx(&idx, e+1) {
                                trace!("Have {} segment chunks in sidx data", segment_chunks.len());
                                for chunk in segment_chunks {
                                    let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                                        .with_range(Some(chunk.start), Some(chunk.end))
                                        .build();
                                    fragments.push(mf);
                                    if chunk.end > max_chunk_pos {
                                        max_chunk_pos = chunk.end;
                                    }
                                }
                                indexable_segments = true;
                            }
                        }
                    // In theory we should also be able to handle Cue data in a WebM media
                    // stream similarly to chunks specified by an sidx box in an ISOBMFF/MP4
                    // container. However, simply appending the content pointed to by the
                    // different Cue elements in the WebM file leads to an invalid media
                    // file. We need to implement more complicated logic to reconstruct a
                    // valid WebM file from chunks of content.
                }
            }
        }
    }
    if indexable_segments {
        if let Some(init) = &sb.Initialization {
            if let Some(range) = &init.range {
                let (s, e) = parse_range(range)?;
                start_byte = Some(s);
                end_byte = Some(e);
            }
            if let Some(su) = &init.sourceURL {
                let path = resolve_url_template(su, dict);
                let u = merge_baseurls(&base_url, &path)?;
                let mf = MediaFragmentBuilder::new(period_counter, u)
                    .with_range(start_byte, end_byte)
                    .set_init()
                    .build();
                fragments.push(mf);
            } else {
                // Use the current BaseURL
                let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                    .with_range(start_byte, end_byte)
                    .set_init()
                    .build();
                fragments.push(mf);
            }
        }
    } else {
        // If anything prevented us from handling this SegmentBase@indexRange element using
        // HTTP Range requests, just download the whole segment as a single chunk. This is
        // likely to be a large HTTP request (for instance, the full video content as a
        // single MP4 file), so we increase our network request timeout.
        trace!("Falling back to retrieving full SegmentBase for {}", base_url.clone());
        let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
            .with_timeout(Duration::new(10_000, 0))
            .build();
        fragments.push(mf);
    }
    Ok(fragments)
}


#[tracing::instrument(level="trace", skip_all)]
async fn do_period_audio(
    downloader: &DashDownloader,
    mpd: &MPD,
    period: &Period,
    period_counter: u8,
    base_url: Url
) -> Result<PeriodOutputs, DashMpdError>
{
    let mut fragments = Vec::new();
    let mut diagnostics = Vec::new();
    let mut opt_init: Option<String> = None;
    let mut opt_media: Option<String> = None;
    let mut opt_duration: Option<f64> = None;
    let mut timescale = 1;
    let mut start_number = 1;
    // The period_duration is specified either by the <Period> duration attribute, or by the
    // mediaPresentationDuration of the top-level MPD node.
    let mut period_duration_secs: f64 = -1.0;
    if let Some(d) = mpd.mediaPresentationDuration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(d) = period.duration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(s) = downloader.force_duration {
        period_duration_secs = s;
    }
    // SegmentTemplate as a direct child of a Period element. This can specify some common attribute
    // values (media, timescale, duration, startNumber) for child SegmentTemplate nodes in an
    // enclosed AdaptationSet or Representation node.
    if let Some(st) = &period.SegmentTemplate {
        if let Some(i) = &st.initialization {
            opt_init = Some(i.clone());
        }
        if let Some(m) = &st.media {
            opt_media = Some(m.clone());
        }
        if let Some(d) = st.duration {
            opt_duration = Some(d);
        }
        if let Some(ts) = st.timescale {
            timescale = ts;
        }
        if let Some(s) = st.startNumber {
            start_number = s;
        }
    }
    let mut selected_audio_language = "unk";
    // Handle the AdaptationSet with audio content. Note that some streams don't separate out
    // audio and video streams, so this might be None.
    let audio_adaptations: Vec<&AdaptationSet> = period.adaptations.iter()
        .filter(is_audio_adaptation)
        .collect();
    let representations: Vec<&Representation> = select_preferred_adaptations(audio_adaptations, downloader)
        .iter()
        .flat_map(|a| a.representations.iter())
        .collect();
    if let Some(audio_repr) = select_preferred_representation(&representations, downloader) {
        // Find the AdaptationSet that is the parent of the selected Representation. This may be
        // needed for certain Representation attributes whose value can be located higher in the XML
        // tree.
        let audio_adaptation = period.adaptations.iter()
            .find(|a| a.representations.iter().any(|r| r.eq(audio_repr)))
            .unwrap();
        if let Some(lang) = audio_repr.lang.as_ref().or(audio_adaptation.lang.as_ref()) {
            selected_audio_language = lang;
        }
        // The AdaptationSet may have a BaseURL (e.g. the test BBC streams). We use a local variable
        // to make sure we don't "corrupt" the base_url for the video segments.
        let mut base_url = base_url.clone();
        if let Some(bu) = &audio_adaptation.BaseURL.first() {
            base_url = merge_baseurls(&base_url, &bu.base)?;
        }
        if let Some(bu) = audio_repr.BaseURL.first() {
            base_url = merge_baseurls(&base_url, &bu.base)?;
        }
        if downloader.verbosity > 0 {
            let bw = if let Some(bw) = audio_repr.bandwidth {
                format!("bw={} Kbps ", bw / 1024)
            } else {
                String::from("")
            };
            let unknown = String::from("?");
            let lang = audio_repr.lang.as_ref()
                .unwrap_or(audio_adaptation.lang.as_ref()
                           .unwrap_or(&unknown));
            let codec = audio_repr.codecs.as_ref()
                .unwrap_or(audio_adaptation.codecs.as_ref()
                           .unwrap_or(&unknown));
            diagnostics.push(format!("  Audio stream selected: {bw}lang={lang} codec={codec}"));
            // Check for ContentProtection on the selected Representation/Adaptation
            for cp in audio_repr.ContentProtection.iter()
                .chain(audio_adaptation.ContentProtection.iter())
            {
                diagnostics.push(format!("  ContentProtection: {}", content_protection_type(cp)));
                if let Some(kid) = &cp.default_KID {
                    diagnostics.push(format!("    KID: {}", kid.replace('-', "")));
                }
                for pssh_element in &cp.cenc_pssh {
                    if let Some(pssh_b64) = &pssh_element.content {
                        diagnostics.push(format!("    PSSH (from manifest): {pssh_b64}"));
                        if let Ok(pssh) = pssh_box::from_base64(pssh_b64) {
                            diagnostics.push(format!("    {pssh}"));
                        }
                    }
                }
            }
        }
        // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
        // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
        // nodes in an enclosed Representation node. Don't download media segments here, only
        // download for SegmentTemplate nodes that are children of a Representation node.
        if let Some(st) = &audio_adaptation.SegmentTemplate {
            if let Some(i) = &st.initialization {
                opt_init = Some(i.clone());
            }
            if let Some(m) = &st.media {
                opt_media = Some(m.clone());
            }
            if let Some(d) = st.duration {
                opt_duration = Some(d);
            }
            if let Some(ts) = st.timescale {
                timescale = ts;
            }
            if let Some(s) = st.startNumber {
                start_number = s;
            }
        }
        let mut dict = HashMap::new();
        if let Some(rid) = &audio_repr.id {
            dict.insert("RepresentationID", rid.clone());
        }
        if let Some(b) = &audio_repr.bandwidth {
            dict.insert("Bandwidth", b.to_string());
        }
        // Now the 6 possible addressing modes: (1) SegmentList,
        // (2) SegmentTemplate+SegmentTimeline, (3) SegmentTemplate@duration,
        // (4) SegmentTemplate@index, (5) SegmentBase@indexRange, (6) plain BaseURL
        
        // Though SegmentBase and SegmentList addressing modes are supposed to be
        // mutually exclusive, some manifests in the wild use both. So we try to work
        // around the brokenness.
        // Example: http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd
        if let Some(sl) = &audio_adaptation.SegmentList {
            // (1) AdaptationSet>SegmentList addressing mode (can be used in conjunction
            // with Representation>SegmentList addressing mode)
            if downloader.verbosity > 1 {
                info!("  Using AdaptationSet>SegmentList addressing mode for audio representation");
            }
            let mut start_byte: Option<u64> = None;
            let mut end_byte: Option<u64> = None;
            if let Some(init) = &sl.Initialization {
                if let Some(range) = &init.range {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(su) = &init.sourceURL {
                    let path = resolve_url_template(su, &dict);
                    let init_url = merge_baseurls(&base_url, &path)?;
                    let mf = MediaFragmentBuilder::new(period_counter, init_url)
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                } else {
                    let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                }
            }
            for su in &sl.segment_urls {
                start_byte = None;
                end_byte = None;
                // we are ignoring SegmentURL@indexRange
                if let Some(range) = &su.mediaRange {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(m) = &su.media {
                    let u = merge_baseurls(&base_url, m)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                } else if let Some(bu) = audio_adaptation.BaseURL.first() {
                    let u = merge_baseurls(&base_url, &bu.base)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                }
            }
        }
        if let Some(sl) = &audio_repr.SegmentList {
            // (1) Representation>SegmentList addressing mode
            if downloader.verbosity > 1 {
                info!("  Using Representation>SegmentList addressing mode for audio representation");
            }
            let mut start_byte: Option<u64> = None;
            let mut end_byte: Option<u64> = None;
            if let Some(init) = &sl.Initialization {
                if let Some(range) = &init.range {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(su) = &init.sourceURL {
                    let path = resolve_url_template(su, &dict);
                    let init_url = merge_baseurls(&base_url, &path)?;
                    let mf = MediaFragmentBuilder::new(period_counter, init_url)
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                } else {
                    let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                }
            }
            for su in &sl.segment_urls {
                start_byte = None;
                end_byte = None;
                // we are ignoring SegmentURL@indexRange
                if let Some(range) = &su.mediaRange {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(m) = &su.media {
                    let u = merge_baseurls(&base_url, m)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                } else if let Some(bu) = audio_repr.BaseURL.first() {
                    let u = merge_baseurls(&base_url, &bu.base)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                }
            }
        } else if audio_repr.SegmentTemplate.is_some() ||
            audio_adaptation.SegmentTemplate.is_some()
        {
            // Here we are either looking at a Representation.SegmentTemplate, or a
            // higher-level AdaptationSet.SegmentTemplate
            let st;
            if let Some(it) = &audio_repr.SegmentTemplate {
                st = it;
            } else if let Some(it) = &audio_adaptation.SegmentTemplate {
                st = it;
            } else {
                panic!("unreachable");
            }
            if let Some(i) = &st.initialization {
                opt_init = Some(i.clone());
            }
            if let Some(m) = &st.media {
                opt_media = Some(m.clone());
            }
            if let Some(ts) = st.timescale {
                timescale = ts;
            }
            if let Some(sn) = st.startNumber {
                start_number = sn;
            }
            if let Some(stl) = &audio_repr.SegmentTemplate.as_ref().and_then(|st| st.SegmentTimeline.clone())
                .or(audio_adaptation.SegmentTemplate.as_ref().and_then(|st| st.SegmentTimeline.clone()))
            {
                // (2) SegmentTemplate with SegmentTimeline addressing mode (also called
                // "explicit addressing" in certain DASH-IF documents)
                if downloader.verbosity > 1 {
                    info!("  Using SegmentTemplate+SegmentTimeline addressing mode for audio representation");
                }
                if let Some(init) = opt_init {
                    let path = resolve_url_template(&init, &dict);
                    let u = merge_baseurls(&base_url, &path)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .set_init()
                        .build();
                    fragments.push(mf);
                }
                if let Some(media) = opt_media {
                    let audio_path = resolve_url_template(&media, &dict);
                    let mut segment_time = 0;
                    let mut segment_duration;
                    let mut number = start_number;
                    for s in &stl.segments {
                        if let Some(t) = s.t {
                            segment_time = t;
                        }
                        segment_duration = s.d;
                        // the URLTemplate may be based on $Time$, or on $Number$
                        let dict = HashMap::from([("Time", segment_time.to_string()),
                                                  ("Number", number.to_string())]);
                        let path = resolve_url_template(&audio_path, &dict);
                        let u = merge_baseurls(&base_url, &path)?;
                        fragments.push(MediaFragmentBuilder::new(period_counter, u).build());
                        number += 1;
                        if let Some(r) = s.r {
                            let mut count = 0i64;
                            // FIXME perhaps we also need to account for startTime?
                            let end_time = period_duration_secs * timescale as f64;
                            loop {
                                count += 1;
                                // Exit from the loop after @r iterations (if @r is
                                // positive). A negative value of the @r attribute indicates
                                // that the duration indicated in @d attribute repeats until
                                // the start of the next S element, the end of the Period or
                                // until the next MPD update.
                                if r >= 0 {
                                    if count > r {
                                        break;
                                    }
                                    if downloader.force_duration.is_some() && segment_time as f64 > end_time {
                                        break;
                                    }
                                } else if segment_time as f64 > end_time {
                                    break;
                                }
                                segment_time += segment_duration;
                                let dict = HashMap::from([("Time", segment_time.to_string()),
                                                          ("Number", number.to_string())]);
                                let path = resolve_url_template(&audio_path, &dict);
                                let u = merge_baseurls(&base_url, &path)?;
                                fragments.push(MediaFragmentBuilder::new(period_counter, u).build());
                                number += 1;
                            }
                        }
                        segment_time += segment_duration;
                    }
                } else {
                    return Err(DashMpdError::UnhandledMediaStream(
                        "SegmentTimeline without a media attribute".to_string()));
                }
            } else { // no SegmentTimeline element
                // (3) SegmentTemplate@duration addressing mode or (4) SegmentTemplate@index
                // addressing mode (also called "simple addressing" in certain DASH-IF
                // documents)
                if downloader.verbosity > 1 {
                    info!("  Using SegmentTemplate addressing mode for audio representation");
                }
                let mut total_number = 0i64;
                if let Some(init) = opt_init {
                    let path = resolve_url_template(&init, &dict);
                    let u = merge_baseurls(&base_url, &path)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .set_init()
                        .build();
                    fragments.push(mf);
                }
                if let Some(media) = opt_media {
                    let audio_path = resolve_url_template(&media, &dict);
                    let timescale = st.timescale.unwrap_or(timescale);
                    let mut segment_duration: f64 = -1.0;
                    if let Some(d) = opt_duration {
                        // it was set on the Period.SegmentTemplate node
                        segment_duration = d;
                    }
                    if let Some(std) = st.duration {
                        if timescale == 0 {
                            return Err(DashMpdError::UnhandledMediaStream(
                                "SegmentTemplate@duration attribute cannot be zero".to_string()));
                        }
                        segment_duration = std / timescale as f64;
                    }
                    if segment_duration < 0.0 {
                        return Err(DashMpdError::UnhandledMediaStream(
                            "Audio representation is missing SegmentTemplate@duration attribute".to_string()));
                    }
                    total_number += (period_duration_secs / segment_duration).round() as i64;
                    let mut number = start_number;
                    // For dynamic MPDs the latest available segment is numbered
                    //    LSN = floor((now - (availabilityStartTime+PST))/segmentDuration + startNumber - 1)
                    if mpd_is_dynamic(mpd) {
                        if let Some(start_time) = mpd.availabilityStartTime {
                            let elapsed = Utc::now().signed_duration_since(start_time).as_seconds_f64() / segment_duration;
                            number = (elapsed + number as f64 - 1f64).floor() as u64;
                        } else {
                            return Err(DashMpdError::UnhandledMediaStream(
                                "dynamic manifest is missing @availabilityStartTime".to_string()));
                        }
                    }
                    for _ in 1..=total_number {
                        let dict = HashMap::from([("Number", number.to_string())]);
                        let path = resolve_url_template(&audio_path, &dict);
                        let u = merge_baseurls(&base_url, &path)?;
                        fragments.push(MediaFragmentBuilder::new(period_counter, u).build());
                        number += 1;
                    }
                }
            }
        } else if let Some(sb) = &audio_repr.SegmentBase {
            // (5) SegmentBase@indexRange addressing mode
            if downloader.verbosity > 1 {
                info!("  Using SegmentBase@indexRange addressing mode for audio representation");
            }
            let mf = do_segmentbase_indexrange(downloader, period_counter, base_url, sb, &dict).await?;
            fragments.extend(mf);
         } else if fragments.is_empty() {
            if let Some(bu) = audio_repr.BaseURL.first() {
                // (6) plain BaseURL addressing mode
                if downloader.verbosity > 1 {
                    info!("  Using BaseURL addressing mode for audio representation");
                }
                let u = merge_baseurls(&base_url, &bu.base)?;
                fragments.push(MediaFragmentBuilder::new(period_counter, u).build());
            }
        }
        if fragments.is_empty() {
            return Err(DashMpdError::UnhandledMediaStream(
                "no usable addressing mode identified for audio representation".to_string()));
        }
    }
    Ok(PeriodOutputs {
        fragments, diagnostics, subtitle_formats: Vec::new(),
        selected_audio_language: String::from(selected_audio_language)
    })
}


#[tracing::instrument(level="trace", skip_all)]
async fn do_period_video(
    downloader: &DashDownloader,
    mpd: &MPD,
    period: &Period,
    period_counter: u8,
    base_url: Url
    ) -> Result<PeriodOutputs, DashMpdError>
{
    let mut fragments = Vec::new();
    let mut diagnostics = Vec::new();
    let mut period_duration_secs: f64 = 0.0;
    let mut opt_init: Option<String> = None;
    let mut opt_media: Option<String> = None;
    let mut opt_duration: Option<f64> = None;
    let mut timescale = 1;
    let mut start_number = 1;
    if let Some(d) = mpd.mediaPresentationDuration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(d) = period.duration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(s) = downloader.force_duration {
        period_duration_secs = s;
    }
    // SegmentTemplate as a direct child of a Period element. This can specify some common attribute
    // values (media, timescale, duration, startNumber) for child SegmentTemplate nodes in an
    // enclosed AdaptationSet or Representation node.
    if let Some(st) = &period.SegmentTemplate {
        if let Some(i) = &st.initialization {
            opt_init = Some(i.clone());
        }
        if let Some(m) = &st.media {
            opt_media = Some(m.clone());
        }
        if let Some(d) = st.duration {
            opt_duration = Some(d);
        }
        if let Some(ts) = st.timescale {
            timescale = ts;
        }
        if let Some(s) = st.startNumber {
            start_number = s;
        }
    }
    // A manifest may contain multiple AdaptationSets with video content (in particular, when
    // different codecs are offered). Each AdaptationSet often contains multiple video
    // Representations with different bandwidths and video resolutions. We select the Representation
    // to download by ranking the available streams according to the preferred width specified by
    // the user, or by the preferred height specified by the user, or by the user's specified
    // quality preference.
    let video_adaptations: Vec<&AdaptationSet> = period.adaptations.iter()
        .filter(is_video_adaptation)
        .collect();
    let representations: Vec<&Representation> = select_preferred_adaptations(video_adaptations, downloader)
        .iter()
        .flat_map(|a| a.representations.iter())
        .collect();
    let maybe_video_repr = if let Some(want) = downloader.video_width_preference {
        representations.iter()
            .min_by_key(|x| if let Some(w) = x.width { want.abs_diff(w) } else { u64::MAX })
            .copied()
    }  else if let Some(want) = downloader.video_height_preference {
        representations.iter()
            .min_by_key(|x| if let Some(h) = x.height { want.abs_diff(h) } else { u64::MAX })
            .copied()
    } else {
        select_preferred_representation(&representations, downloader)
    };
    if let Some(video_repr) = maybe_video_repr {
        // Find the AdaptationSet that is the parent of the selected Representation. This may be
        // needed for certain Representation attributes whose value can be located higher in the XML
        // tree.
        let video_adaptation = period.adaptations.iter()
            .find(|a| a.representations.iter().any(|r| r.eq(video_repr)))
            .unwrap();
        // The AdaptationSet may have a BaseURL. We use a local variable to make sure we
        // don't "corrupt" the base_url for the subtitle segments.
        let mut base_url = base_url.clone();
        if let Some(bu) = &video_adaptation.BaseURL.first() {
            base_url = merge_baseurls(&base_url, &bu.base)?;
        }
        if let Some(bu) = &video_repr.BaseURL.first() {
            base_url = merge_baseurls(&base_url, &bu.base)?;
        }
        if downloader.verbosity > 0 {
            let bw = if let Some(bw) = video_repr.bandwidth.or(video_adaptation.maxBandwidth) {
                format!("bw={} Kbps ", bw / 1024)
            } else {
                String::from("")
            };
            let unknown = String::from("?");
            let w = video_repr.width.unwrap_or(video_adaptation.width.unwrap_or(0));
            let h = video_repr.height.unwrap_or(video_adaptation.height.unwrap_or(0));
            let fmt = if w == 0 || h == 0 {
                String::from("")
            } else {
                format!("resolution={w}x{h} ")
            };
            let codec = video_repr.codecs.as_ref()
                .unwrap_or(video_adaptation.codecs.as_ref().unwrap_or(&unknown));
            diagnostics.push(format!("  Video stream selected: {bw}{fmt}codec={codec}"));
            // Check for ContentProtection on the selected Representation/Adaptation
            for cp in video_repr.ContentProtection.iter()
                .chain(video_adaptation.ContentProtection.iter())
            {
                diagnostics.push(format!("  ContentProtection: {}", content_protection_type(cp)));
                if let Some(kid) = &cp.default_KID {
                    diagnostics.push(format!("    KID: {}", kid.replace('-', "")));
                }
                for pssh_element in &cp.cenc_pssh {
                    if let Some(pssh_b64) = &pssh_element.content {
                        diagnostics.push(format!("    PSSH (from manifest): {pssh_b64}"));
                        if let Ok(pssh) = pssh_box::from_base64(pssh_b64) {
                            diagnostics.push(format!("    {pssh}"));
                        }
                    }
                }
            }
        }
        let mut dict = HashMap::new();
        if let Some(rid) = &video_repr.id {
            dict.insert("RepresentationID", rid.clone());
        }
        if let Some(b) = &video_repr.bandwidth {
            dict.insert("Bandwidth", b.to_string());
        }
        // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
        // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
        // nodes in an enclosed Representation node. Don't download media segments here, only
        // download for SegmentTemplate nodes that are children of a Representation node.
        if let Some(st) = &video_adaptation.SegmentTemplate {
            if let Some(i) = &st.initialization {
                opt_init = Some(i.clone());
            }
            if let Some(m) = &st.media {
                opt_media = Some(m.clone());
            }
            if let Some(d) = st.duration {
                opt_duration = Some(d);
            }
            if let Some(ts) = st.timescale {
                timescale = ts;
            }
            if let Some(s) = st.startNumber {
                start_number = s;
            }
        }
        // Now the 6 possible addressing modes: (1) SegmentList,
        // (2) SegmentTemplate+SegmentTimeline, (3) SegmentTemplate@duration,
        // (4) SegmentTemplate@index, (5) SegmentBase@indexRange, (6) plain BaseURL
        if let Some(sl) = &video_adaptation.SegmentList {
            // (1) AdaptationSet>SegmentList addressing mode
            if downloader.verbosity > 1 {
                info!("  Using AdaptationSet>SegmentList addressing mode for video representation");
            }
            let mut start_byte: Option<u64> = None;
            let mut end_byte: Option<u64> = None;
            if let Some(init) = &sl.Initialization {
                if let Some(range) = &init.range {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(su) = &init.sourceURL {
                    let path = resolve_url_template(su, &dict);
                    let u = merge_baseurls(&base_url, &path)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                }
            } else {
                let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                    .with_range(start_byte, end_byte)
                    .set_init()
                    .build();
                fragments.push(mf);
            }
            for su in &sl.segment_urls {
                start_byte = None;
                end_byte = None;
                // we are ignoring @indexRange
                if let Some(range) = &su.mediaRange {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(m) = &su.media {
                    let u = merge_baseurls(&base_url, m)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                } else if let Some(bu) = video_adaptation.BaseURL.first() {
                    let u = merge_baseurls(&base_url, &bu.base)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                }
            }
        }
        if let Some(sl) = &video_repr.SegmentList {
            // (1) Representation>SegmentList addressing mode
            if downloader.verbosity > 1 {
                info!("  Using Representation>SegmentList addressing mode for video representation");
            }
            let mut start_byte: Option<u64> = None;
            let mut end_byte: Option<u64> = None;
            if let Some(init) = &sl.Initialization {
                if let Some(range) = &init.range {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(su) = &init.sourceURL {
                    let path = resolve_url_template(su, &dict);
                    let u = merge_baseurls(&base_url, &path)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                } else {
                    let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                        .with_range(start_byte, end_byte)
                        .set_init()
                        .build();
                    fragments.push(mf);
                }
            }
            for su in sl.segment_urls.iter() {
                start_byte = None;
                end_byte = None;
                // we are ignoring @indexRange
                if let Some(range) = &su.mediaRange {
                    let (s, e) = parse_range(range)?;
                    start_byte = Some(s);
                    end_byte = Some(e);
                }
                if let Some(m) = &su.media {
                    let u = merge_baseurls(&base_url, m)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                } else if let Some(bu) = video_repr.BaseURL.first() {
                    let u = merge_baseurls(&base_url, &bu.base)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_range(start_byte, end_byte)
                        .build();
                    fragments.push(mf);
                }
            }
        } else if video_repr.SegmentTemplate.is_some() ||
            video_adaptation.SegmentTemplate.is_some() {
                // Here we are either looking at a Representation.SegmentTemplate, or a
                // higher-level AdaptationSet.SegmentTemplate
                let st;
                if let Some(it) = &video_repr.SegmentTemplate {
                    st = it;
                } else if let Some(it) = &video_adaptation.SegmentTemplate {
                    st = it;
                } else {
                    panic!("impossible");
                }
                if let Some(i) = &st.initialization {
                    opt_init = Some(i.clone());
                }
                if let Some(m) = &st.media {
                    opt_media = Some(m.clone());
                }
                if let Some(ts) = st.timescale {
                    timescale = ts;
                }
                if let Some(sn) = st.startNumber {
                    start_number = sn;
                }
                if let Some(stl) = &video_repr.SegmentTemplate.as_ref().and_then(|st| st.SegmentTimeline.clone())
                    .or(video_adaptation.SegmentTemplate.as_ref().and_then(|st| st.SegmentTimeline.clone()))
                {
                    // (2) SegmentTemplate with SegmentTimeline addressing mode
                    if downloader.verbosity > 1 {
                        info!("  Using SegmentTemplate+SegmentTimeline addressing mode for video representation");
                    }
                    if let Some(init) = opt_init {
                        let path = resolve_url_template(&init, &dict);
                        let u = merge_baseurls(&base_url, &path)?;
                        let mf = MediaFragmentBuilder::new(period_counter, u)
                            .set_init()
                            .build();
                        fragments.push(mf);
                    }
                    if let Some(media) = opt_media {
                        let video_path = resolve_url_template(&media, &dict);
                        let mut segment_time = 0;
                        let mut segment_duration;
                        let mut number = start_number;
                        for s in &stl.segments {
                            if let Some(t) = s.t {
                                segment_time = t;
                            }
                            segment_duration = s.d;
                            // the URLTemplate may be based on $Time$, or on $Number$
                            let dict = HashMap::from([("Time", segment_time.to_string()),
                                                      ("Number", number.to_string())]);
                            let path = resolve_url_template(&video_path, &dict);
                            let u = merge_baseurls(&base_url, &path)?;
                            let mf = MediaFragmentBuilder::new(period_counter, u).build();
                            fragments.push(mf);
                            number += 1;
                            if let Some(r) = s.r {
                                let mut count = 0i64;
                                // FIXME perhaps we also need to account for startTime?
                                let end_time = period_duration_secs * timescale as f64;
                                loop {
                                    count += 1;
                                    // Exit from the loop after @r iterations (if @r is
                                    // positive). A negative value of the @r attribute indicates
                                    // that the duration indicated in @d attribute repeats until
                                    // the start of the next S element, the end of the Period or
                                    // until the next MPD update.
                                    if r >= 0 {
                                        if count > r {
                                            break;
                                        }
                                        if downloader.force_duration.is_some() && segment_time as f64 > end_time {
                                            break;
                                        }
                                    } else if segment_time as f64 > end_time {
                                        break;
                                    }
                                    segment_time += segment_duration;
                                    let dict = HashMap::from([("Time", segment_time.to_string()),
                                                              ("Number", number.to_string())]);
                                    let path = resolve_url_template(&video_path, &dict);
                                    let u = merge_baseurls(&base_url, &path)?;
                                    let mf = MediaFragmentBuilder::new(period_counter, u).build();
                                    fragments.push(mf);
                                    number += 1;
                                }
                            }
                            segment_time += segment_duration;
                        }
                    } else {
                        return Err(DashMpdError::UnhandledMediaStream(
                            "SegmentTimeline without a media attribute".to_string()));
                    }
                } else { // no SegmentTimeline element
                    // (3) SegmentTemplate@duration addressing mode or (4) SegmentTemplate@index addressing mode
                    if downloader.verbosity > 1 {
                        info!("  Using SegmentTemplate addressing mode for video representation");
                    }
                    let mut total_number = 0i64;
                    if let Some(init) = opt_init {
                        let path = resolve_url_template(&init, &dict);
                        let u = merge_baseurls(&base_url, &path)?;
                        let mf = MediaFragmentBuilder::new(period_counter, u)
                            .set_init()
                            .build();
                        fragments.push(mf);
                    }
                    if let Some(media) = opt_media {
                        let video_path = resolve_url_template(&media, &dict);
                        let timescale = st.timescale.unwrap_or(timescale);
                        let mut segment_duration: f64 = -1.0;
                        if let Some(d) = opt_duration {
                            // it was set on the Period.SegmentTemplate node
                            segment_duration = d;
                        }
                        if let Some(std) = st.duration {
                            if timescale == 0 {
                                return Err(DashMpdError::UnhandledMediaStream(
                                    "SegmentTemplate@duration attribute cannot be zero".to_string()));
                            }
                            segment_duration = std / timescale as f64;
                        }
                        if segment_duration < 0.0 {
                            return Err(DashMpdError::UnhandledMediaStream(
                                "Video representation is missing SegmentTemplate@duration attribute".to_string()));
                        }
                        total_number += (period_duration_secs / segment_duration).round() as i64;
                        let mut number = start_number;
                        // For a live manifest (dynamic MPD), we look at the time elapsed since now
                        // and the mpd.availabilityStartTime to determine the correct value for
                        // startNumber, based on duration and timescale. The latest available
                        // segment is numbered
                        //
                        //    LSN = floor((now - (availabilityStartTime+PST))/segmentDuration + startNumber - 1)

                        // https://dashif.org/Guidelines-TimingModel/Timing-Model.pdf
                        // To be more precise, any LeapSecondInformation should be added to the availabilityStartTime.
                        if mpd_is_dynamic(mpd) {
                            if let Some(start_time) = mpd.availabilityStartTime {
                                let elapsed = Utc::now().signed_duration_since(start_time).as_seconds_f64() / segment_duration;
                                number = (elapsed + number as f64 - 1f64).floor() as u64;
                            } else {
                                return Err(DashMpdError::UnhandledMediaStream(
                                    "dynamic manifest is missing @availabilityStartTime".to_string()));
                            }
                        }
                        for _ in 1..=total_number {
                            let dict = HashMap::from([("Number", number.to_string())]);
                            let path = resolve_url_template(&video_path, &dict);
                            let u = merge_baseurls(&base_url, &path)?;
                            let mf = MediaFragmentBuilder::new(period_counter, u).build();
                            fragments.push(mf);
                            number += 1;
                        }
                    }
                }
            } else if let Some(sb) = &video_repr.SegmentBase {
                // (5) SegmentBase@indexRange addressing mode
                if downloader.verbosity > 1 {
                    info!("  Using SegmentBase@indexRange addressing mode for video representation");
                }
                let mf = do_segmentbase_indexrange(downloader, period_counter, base_url, sb, &dict).await?;
                fragments.extend(mf);
            } else if fragments.is_empty()  {
                if let Some(bu) = video_repr.BaseURL.first() {
                    // (6) BaseURL addressing mode
                    if downloader.verbosity > 1 {
                        info!("  Using BaseURL addressing mode for video representation");
                    }
                    let u = merge_baseurls(&base_url, &bu.base)?;
                    let mf = MediaFragmentBuilder::new(period_counter, u)
                        .with_timeout(Duration::new(10000, 0))
                        .build();
                    fragments.push(mf);
                }
            }
        if fragments.is_empty() {
            return Err(DashMpdError::UnhandledMediaStream(
                "no usable addressing mode identified for video representation".to_string()));
        }
    }
    // FIXME we aren't correctly handling manifests without a Representation node
    // eg https://raw.githubusercontent.com/zencoder/go-dash/master/mpd/fixtures/newperiod.mpd
    Ok(PeriodOutputs {
        fragments,
        diagnostics,
        subtitle_formats: Vec::new(),
        selected_audio_language: String::from("unk")
    })
}

#[tracing::instrument(level="trace", skip_all)]
async fn do_period_subtitles(
    downloader: &DashDownloader,
    mpd: &MPD,
    period: &Period,
    period_counter: u8,
    base_url: Url
    ) -> Result<PeriodOutputs, DashMpdError>
{
    let client = downloader.http_client.as_ref().unwrap();
    let output_path = &downloader.output_path.as_ref().unwrap().clone();
    let period_output_path = output_path_for_period(output_path, period_counter);
    let mut fragments = Vec::new();
    let mut subtitle_formats = Vec::new();
    let mut period_duration_secs: f64 = 0.0;
    if let Some(d) = mpd.mediaPresentationDuration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(d) = period.duration {
        period_duration_secs = d.as_secs_f64();
    }
    let maybe_subtitle_adaptation = if let Some(ref lang) = downloader.language_preference {
        period.adaptations.iter().filter(is_subtitle_adaptation)
            .min_by_key(|a| adaptation_lang_distance(a, lang))
    } else {
        // returns the first subtitle adaptation found
        period.adaptations.iter().find(is_subtitle_adaptation)
    };
    if downloader.fetch_subtitles {
        if let Some(subtitle_adaptation) = maybe_subtitle_adaptation {
            let subtitle_format = subtitle_type(&subtitle_adaptation);
            subtitle_formats.push(subtitle_format);
            if downloader.verbosity > 1 && downloader.fetch_subtitles {
                info!("  Retrieving subtitles in format {subtitle_format:?}");
            }
            // The AdaptationSet may have a BaseURL. We use a local variable to make sure we
            // don't "corrupt" the base_url for the subtitle segments.
            let mut base_url = base_url.clone();
            if let Some(bu) = &subtitle_adaptation.BaseURL.first() {
                base_url = merge_baseurls(&base_url, &bu.base)?;
            }
            // We don't do any ranking on subtitle Representations, because there is probably only a
            // single one for our selected Adaptation.
            if let Some(rep) = subtitle_adaptation.representations.first() {
                if !rep.BaseURL.is_empty() {
                    for st_bu in &rep.BaseURL {
                        let st_url = merge_baseurls(&base_url, &st_bu.base)?;
                        let mut req = client.get(st_url.clone());
                        if let Some(referer) = &downloader.referer {
                            req = req.header("Referer", referer);
                        } else {
                            req = req.header("Referer", base_url.to_string());
                        }
                        let rqw = req.build()
                            .map_err(|e| network_error("building request", &e))?;
                        let subs = reqwest_bytes_with_retries(client, rqw, 5).await
                            .map_err(|e| network_error("fetching subtitles", &e))?;
                        let mut subs_path = period_output_path.clone();
                        let subtitle_format = subtitle_type(&subtitle_adaptation);
                        match subtitle_format {
                            SubtitleType::Vtt => subs_path.set_extension("vtt"),
                            SubtitleType::Srt => subs_path.set_extension("srt"),
                            SubtitleType::Ttml => subs_path.set_extension("ttml"),
                            SubtitleType::Sami => subs_path.set_extension("sami"),
                            SubtitleType::Wvtt => subs_path.set_extension("wvtt"),
                            SubtitleType::Stpp => subs_path.set_extension("stpp"),
                            _ => subs_path.set_extension("sub"),
                        };
                        subtitle_formats.push(subtitle_format);
                        let mut subs_file = File::create(subs_path.clone()).await
                            .map_err(|e| DashMpdError::Io(e, String::from("creating subtitle file")))?;
                        if downloader.verbosity > 2 {
                            info!("  Subtitle {st_url} -> {} octets", subs.len());
                        }
                        match subs_file.write_all(&subs).await {
                            Ok(()) => {
                                if downloader.verbosity > 0 {
                                    info!("  Downloaded subtitles ({subtitle_format:?}) to {}",
                                             subs_path.display());
                                }
                            },
                            Err(e) => {
                                error!("Unable to write subtitle file: {e:?}");
                                return Err(DashMpdError::Io(e, String::from("writing subtitle data")));
                            },
                        }
                        if subtitle_formats.contains(&SubtitleType::Wvtt) ||
                            subtitle_formats.contains(&SubtitleType::Ttxt)
                        {
                            if downloader.verbosity > 0 {
                                info!("   Converting subtitles to SRT format with MP4Box ");
                            }
                            let out = subs_path.with_extension("srt");
                            // We try to convert this to SRT format, which is more widely supported,
                            // using MP4Box. However, it's not a fatal error if MP4Box is not
                            // installed or the conversion fails.
                            //
                            // Could also try to convert to WebVTT with
                            //   MP4Box -raw "0:output=output.vtt" input.mp4
                            let out_str = out.to_string_lossy();
                            let subs_str = subs_path.to_string_lossy();
                            let args = vec![
                                "-srt", "1",
                                "-out", &out_str,
                                &subs_str];
                            if downloader.verbosity > 0 {
                                info!("  Running MPBox {}", args.join(" "));
                            }
                            if let Ok(mp4box) = Command::new(downloader.mp4box_location.clone())
                                .args(args)
                                .output()
                            {
                                let msg = partial_process_output(&mp4box.stdout);
                                if !msg.is_empty() {
                                    info!("MP4Box stdout: {msg}");
                                }
                                let msg = partial_process_output(&mp4box.stderr);
                                if !msg.is_empty() {
                                    info!("MP4Box stderr: {msg}");
                                }
                                if mp4box.status.success() {
                                    info!("   Converted subtitles to SRT");
                                } else {
                                    warn!("Error running MP4Box to convert subtitles");
                                }
                            }
                        }
                    }
                } else if rep.SegmentTemplate.is_some() || subtitle_adaptation.SegmentTemplate.is_some() {
                    let mut opt_init: Option<String> = None;
                    let mut opt_media: Option<String> = None;
                    let mut opt_duration: Option<f64> = None;
                    let mut timescale = 1;
                    let mut start_number = 1;
                    // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
                    // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
                    // nodes in an enclosed Representation node. Don't download media segments here, only
                    // download for SegmentTemplate nodes that are children of a Representation node.
                    if let Some(st) = &rep.SegmentTemplate {
                        if let Some(i) = &st.initialization {
                            opt_init = Some(i.clone());
                        }
                        if let Some(m) = &st.media {
                            opt_media = Some(m.clone());
                        }
                        if let Some(d) = st.duration {
                            opt_duration = Some(d);
                        }
                        if let Some(ts) = st.timescale {
                            timescale = ts;
                        }
                        if let Some(s) = st.startNumber {
                            start_number = s;
                        }
                    }
                    let rid = match &rep.id {
                        Some(id) => id,
                        None => return Err(
                            DashMpdError::UnhandledMediaStream(
                                "Missing @id on Representation node".to_string())),
                    };
                    let mut dict = HashMap::from([("RepresentationID", rid.clone())]);
                    if let Some(b) = &rep.bandwidth {
                        dict.insert("Bandwidth", b.to_string());
                    }
                    // Now the 6 possible addressing modes: (1) SegmentList,
                    // (2) SegmentTemplate+SegmentTimeline, (3) SegmentTemplate@duration,
                    // (4) SegmentTemplate@index, (5) SegmentBase@indexRange, (6) plain BaseURL
                    if let Some(sl) = &rep.SegmentList {
                        // (1) AdaptationSet>SegmentList addressing mode (can be used in conjunction
                        // with Representation>SegmentList addressing mode)
                        if downloader.verbosity > 1 {
                            info!("  Using AdaptationSet>SegmentList addressing mode for subtitle representation");
                        }
                        let mut start_byte: Option<u64> = None;
                        let mut end_byte: Option<u64> = None;
                        if let Some(init) = &sl.Initialization {
                            if let Some(range) = &init.range {
                                let (s, e) = parse_range(range)?;
                                start_byte = Some(s);
                                end_byte = Some(e);
                            }
                            if let Some(su) = &init.sourceURL {
                                let path = resolve_url_template(su, &dict);
                                let u = merge_baseurls(&base_url, &path)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            } else {
                                let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                                    .with_range(start_byte, end_byte)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            }
                        }
                        for su in &sl.segment_urls {
                            start_byte = None;
                            end_byte = None;
                            // we are ignoring SegmentURL@indexRange
                            if let Some(range) = &su.mediaRange {
                                let (s, e) = parse_range(range)?;
                                start_byte = Some(s);
                                end_byte = Some(e);
                            }
                            if let Some(m) = &su.media {
                                let u = merge_baseurls(&base_url, m)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .build();
                                fragments.push(mf);
                            } else if let Some(bu) = subtitle_adaptation.BaseURL.first() {
                                let u = merge_baseurls(&base_url, &bu.base)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .build();
                                fragments.push(mf);
                            }
                        }
                    }
                    if let Some(sl) = &rep.SegmentList {
                        // (1) Representation>SegmentList addressing mode
                        if downloader.verbosity > 1 {
                            info!("  Using Representation>SegmentList addressing mode for subtitle representation");
                        }
                        let mut start_byte: Option<u64> = None;
                        let mut end_byte: Option<u64> = None;
                        if let Some(init) = &sl.Initialization {
                            if let Some(range) = &init.range {
                                let (s, e) = parse_range(range)?;
                                start_byte = Some(s);
                                end_byte = Some(e);
                            }
                            if let Some(su) = &init.sourceURL {
                                let path = resolve_url_template(su, &dict);
                                let u = merge_baseurls(&base_url, &path)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            } else {
                                let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                                    .with_range(start_byte, end_byte)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            }
                        }
                        for su in sl.segment_urls.iter() {
                            start_byte = None;
                            end_byte = None;
                            // we are ignoring SegmentURL@indexRange
                            if let Some(range) = &su.mediaRange {
                                let (s, e) = parse_range(range)?;
                                start_byte = Some(s);
                                end_byte = Some(e);
                            }
                            if let Some(m) = &su.media {
                                let u = merge_baseurls(&base_url, m)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .build();
                                fragments.push(mf);
                            } else if let Some(bu) = &rep.BaseURL.first() {
                                let u = merge_baseurls(&base_url, &bu.base)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .build();
                                fragments.push(mf);
                            };
                        }
                    } else if rep.SegmentTemplate.is_some() ||
                        subtitle_adaptation.SegmentTemplate.is_some()
                    {
                        // Here we are either looking at a Representation.SegmentTemplate, or a
                        // higher-level AdaptationSet.SegmentTemplate
                        let st;
                        if let Some(it) = &rep.SegmentTemplate {
                            st = it;
                        } else if let Some(it) = &subtitle_adaptation.SegmentTemplate {
                            st = it;
                        } else {
                            panic!("unreachable");
                        }
                        if let Some(i) = &st.initialization {
                            opt_init = Some(i.clone());
                        }
                        if let Some(m) = &st.media {
                            opt_media = Some(m.clone());
                        }
                        if let Some(ts) = st.timescale {
                            timescale = ts;
                        }
                        if let Some(sn) = st.startNumber {
                            start_number = sn;
                        }
                        if let Some(stl) = &rep.SegmentTemplate.as_ref().and_then(|st| st.SegmentTimeline.clone())
                            .or(subtitle_adaptation.SegmentTemplate.as_ref().and_then(|st| st.SegmentTimeline.clone()))
                        {
                            // (2) SegmentTemplate with SegmentTimeline addressing mode (also called
                            // "explicit addressing" in certain DASH-IF documents)
                            if downloader.verbosity > 1 {
                                info!("  Using SegmentTemplate+SegmentTimeline addressing mode for subtitle representation");
                            }
                            if let Some(init) = opt_init {
                                let path = resolve_url_template(&init, &dict);
                                let u = merge_baseurls(&base_url, &path)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            }
                            if let Some(media) = opt_media {
                                let sub_path = resolve_url_template(&media, &dict);
                                let mut segment_time = 0;
                                let mut segment_duration;
                                let mut number = start_number;
                                for s in &stl.segments {
                                    if let Some(t) = s.t {
                                        segment_time = t;
                                    }
                                    segment_duration = s.d;
                                    // the URLTemplate may be based on $Time$, or on $Number$
                                    let dict = HashMap::from([("Time", segment_time.to_string()),
                                                              ("Number", number.to_string())]);
                                    let path = resolve_url_template(&sub_path, &dict);
                                    let u = merge_baseurls(&base_url, &path)?;
                                    let mf = MediaFragmentBuilder::new(period_counter, u).build();
                                    fragments.push(mf);
                                    number += 1;
                                    if let Some(r) = s.r {
                                        let mut count = 0i64;
                                        // FIXME perhaps we also need to account for startTime?
                                        let end_time = period_duration_secs * timescale as f64;
                                        loop {
                                            count += 1;
                                            // Exit from the loop after @r iterations (if @r is
                                            // positive). A negative value of the @r attribute indicates
                                            // that the duration indicated in @d attribute repeats until
                                            // the start of the next S element, the end of the Period or
                                            // until the next MPD update.
                                            if r >= 0 {
                                                if count > r {
                                                    break;
                                                }
                                                if downloader.force_duration.is_some() &&
                                                    segment_time as f64 > end_time
                                                {
                                                    break;
                                                }
                                            } else if segment_time as f64 > end_time {
                                                break;
                                            }
                                            segment_time += segment_duration;
                                            let dict = HashMap::from([("Time", segment_time.to_string()),
                                                                      ("Number", number.to_string())]);
                                            let path = resolve_url_template(&sub_path, &dict);
                                            let u = merge_baseurls(&base_url, &path)?;
                                            let mf = MediaFragmentBuilder::new(period_counter, u).build();
                                            fragments.push(mf);
                                            number += 1;
                                        }
                                    }
                                    segment_time += segment_duration;
                                }
                            } else {
                                return Err(DashMpdError::UnhandledMediaStream(
                                    "SegmentTimeline without a media attribute".to_string()));
                            }
                        } else { // no SegmentTimeline element
                            // (3) SegmentTemplate@duration addressing mode or (4) SegmentTemplate@index
                            // addressing mode (also called "simple addressing" in certain DASH-IF
                            // documents)
                            if downloader.verbosity > 0 {
                                info!("  Using SegmentTemplate addressing mode for stpp subtitles");
                            }
                            if let Some(i) = &st.initialization {
                                opt_init = Some(i.to_string());
                            }
                            if let Some(m) = &st.media {
                                opt_media = Some(m.to_string());
                            }
                            if let Some(d) = st.duration {
                                opt_duration = Some(d);
                            }
                            if let Some(ts) = st.timescale {
                                timescale = ts;
                            }
                            if let Some(s) = st.startNumber {
                                start_number = s;
                            }
                            let rid = match &rep.id {
                                Some(id) => id,
                                None => return Err(
                                    DashMpdError::UnhandledMediaStream(
                                        "Missing @id on Representation node".to_string())),
                            };
                            let mut dict = HashMap::from([("RepresentationID", rid.clone())]);
                            if let Some(b) = &rep.bandwidth {
                                dict.insert("Bandwidth", b.to_string());
                            }
                            let mut total_number = 0i64;
                            if let Some(init) = opt_init {
                                let path = resolve_url_template(&init, &dict);
                                let u = merge_baseurls(&base_url, &path)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            }
                            if let Some(media) = opt_media {
                                let sub_path = resolve_url_template(&media, &dict);
                                let mut segment_duration: f64 = -1.0;
                                if let Some(d) = opt_duration {
                                    // it was set on the Period.SegmentTemplate node
                                    segment_duration = d;
                                }
                                if let Some(std) = st.duration {
                                    if timescale == 0 {
                                        return Err(DashMpdError::UnhandledMediaStream(
                                            "SegmentTemplate@duration attribute cannot be zero".to_string()));
                                    }
                                    segment_duration = std / timescale as f64;
                                }
                                if segment_duration < 0.0 {
                                    return Err(DashMpdError::UnhandledMediaStream(
                                        "Subtitle representation is missing SegmentTemplate@duration".to_string()));
                                }
                                total_number += (period_duration_secs / segment_duration).ceil() as i64;
                                let mut number = start_number;
                                for _ in 1..=total_number {
                                    let dict = HashMap::from([("Number", number.to_string())]);
                                    let path = resolve_url_template(&sub_path, &dict);
                                    let u = merge_baseurls(&base_url, &path)?;
                                    let mf = MediaFragmentBuilder::new(period_counter, u).build();
                                    fragments.push(mf);
                                    number += 1;
                                }
                            }
                        }
                    } else if let Some(sb) = &rep.SegmentBase {
                        // SegmentBase@indexRange addressing mode
                        info!("  Using SegmentBase@indexRange for subs");
                        if downloader.verbosity > 1 {
                            info!("  Using SegmentBase@indexRange addressing mode for subtitle representation");
                        }
                        let mut start_byte: Option<u64> = None;
                        let mut end_byte: Option<u64> = None;
                        if let Some(init) = &sb.Initialization {
                            if let Some(range) = &init.range {
                                let (s, e) = parse_range(range)?;
                                start_byte = Some(s);
                                end_byte = Some(e);
                            }
                            if let Some(su) = &init.sourceURL {
                                let path = resolve_url_template(su, &dict);
                                let u = merge_baseurls(&base_url, &path)?;
                                let mf = MediaFragmentBuilder::new(period_counter, u)
                                    .with_range(start_byte, end_byte)
                                    .set_init()
                                    .build();
                                fragments.push(mf);
                            }
                        }
                        let mf = MediaFragmentBuilder::new(period_counter, base_url.clone())
                            .set_init()
                            .build();
                        fragments.push(mf);
                        // TODO also implement SegmentBase addressing mode for subtitles
                        // (sample MPD: https://usp-cmaf-test.s3.eu-central-1.amazonaws.com/tears-of-steel-ttml.mpd)
                    }
                }
            }
        }
    }
    Ok(PeriodOutputs {
        fragments,
        diagnostics: Vec::new(),
        subtitle_formats,
        selected_audio_language: String::from("unk")
    })
}


// This is a complement to the DashDownloader struct, intended to contain the mutable state
// associated with a download. We have chosen an API where the DashDownloader is not mutable.
struct DownloadState {
    period_counter: u8,
    segment_count: usize,
    segment_counter: usize,
    download_errors: u32
}

// Fetch a media fragment at URL frag.url, using the reqwest client in downloader.http_client.
// Network bandwidth is throttled according to downloader.rate_limit. Transient network failures are
// retried.
//
// Note: We return a File instead of a Bytes buffer, because some streams using SegmentBase indexing
// have huge segments that can fill up RAM.
#[tracing::instrument(level="trace", skip_all)]
async fn fetch_fragment(
    downloader: &mut DashDownloader,
    frag: &MediaFragment,
    fragment_type: &str,
    progress_percent: u32) -> Result<std::fs::File, DashMpdError>
{
    let send_request = || async {
        trace!("send_request {}", frag.url.clone());
        // Don't use only "audio/*" or "video/*" in Accept header because some web servers (eg.
        // media.axprod.net) are misconfigured and reject requests for valid audio content (eg .m4s)
        let mut req = downloader.http_client.as_ref().unwrap()
            .get(frag.url.clone())
            .header("Accept", format!("{fragment_type}/*;q=0.9,*/*;q=0.5"))
            .header("Sec-Fetch-Mode", "navigate");
        if let Some(sb) = &frag.start_byte {
            if let Some(eb) = &frag.end_byte {
                req = req.header(RANGE, format!("bytes={sb}-{eb}"));
            }
        }
        if let Some(ts) = &frag.timeout {
            req = req.timeout(*ts);
        }
        if let Some(referer) = &downloader.referer {
            req = req.header("Referer", referer);
        } else {
            req = req.header("Referer", downloader.redirected_url.to_string());
        }
        if let Some(username) = &downloader.auth_username {
            if let Some(password) = &downloader.auth_password {
                req = req.basic_auth(username, Some(password));
            }
        }
        if let Some(token) = &downloader.auth_bearer_token {
            req = req.bearer_auth(token);
        }
        req.send().await?
            .error_for_status()
    };
    match send_request
        .retry(ExponentialBuilder::default())
        .when(reqwest_error_transient_p)
        .notify(notify_transient)
        .await
    {
        Ok(response) => {
            match response.error_for_status() {
                Ok(mut resp) => {
                    let mut tmp_out = tempfile::tempfile()
                        .map_err(|e| DashMpdError::Io(e, String::from("creating tmpfile for fragment")))?;
                      let content_type_checker = if fragment_type.eq("audio") {
                        content_type_audio_p
                    } else if fragment_type.eq("video") {
                        content_type_video_p
                    } else {
                        panic!("fragment_type not audio or video");
                    };
                    if !downloader.content_type_checks || content_type_checker(&resp) {
                        let mut fragment_out: Option<File> = None;
                        if let Some(ref fragment_path) = downloader.fragment_path {
                            if let Some(path) = frag.url.path_segments()
                                .unwrap_or_else(|| "".split(' '))
                                .next_back()
                            {
                                let vf_file = fragment_path.clone().join(fragment_type).join(path);
                                if let Ok(f) = File::create(vf_file).await {
                                    fragment_out = Some(f)
                                }
                            }
                        }
                        let mut segment_size = 0;
                        // Download in chunked format instead of using reqwest's .bytes() API, in
                        // order to avoid saturating RAM with a large media segment. This is
                        // important for DASH manifests that use indexRange addressing, which we
                        // don't download using byte range requests as a normal DASH client would
                        // do, but rather download using a single network request.
                        while let Some(chunk) = resp.chunk().await
                            .map_err(|e| network_error(&format!("fetching DASH {fragment_type} segment"), &e))?
                        {
                            segment_size += chunk.len();
                            downloader.bw_estimator_bytes += chunk.len();
                            let size = min((chunk.len()/1024+1) as u32, u32::MAX);
                            throttle_download_rate(downloader, size).await?;
                            if let Err(e) = tmp_out.write_all(&chunk) {
                                return Err(DashMpdError::Io(e, format!("writing DASH {fragment_type} data")));
                            }
                            if let Some(ref mut fout) = fragment_out {
                                fout.write_all(&chunk)
                                    .map_err(|e| DashMpdError::Io(e, format!("writing {fragment_type} fragment")))
                                    .await?;
                            }
                            let elapsed = downloader.bw_estimator_started.elapsed().as_secs_f64();
                            if (elapsed > 1.5) || (downloader.bw_estimator_bytes > 100_000) {
                                let bw = downloader.bw_estimator_bytes as f64 / (1e6 * elapsed);
                                let msg = if bw > 0.5 {
                                    format!("Fetching {fragment_type} segments ({bw:.1} MB/s)")
                                } else {
                                    let kbs = (bw * 1000.0).round() as u64;
                                    format!("Fetching {fragment_type} segments ({kbs:3} kB/s)")
                                };
                                for observer in &downloader.progress_observers {
                                    observer.update(progress_percent, &msg);
                                }
                                downloader.bw_estimator_started = Instant::now();
                                downloader.bw_estimator_bytes = 0;
                            }
                        }
                        if downloader.verbosity > 2 {
                            if let Some(sb) = &frag.start_byte {
                                if let Some(eb) = &frag.end_byte {
                                    info!("  {fragment_type} segment {} range {sb}-{eb} -> {} octets",
                                          frag.url, segment_size);
                                }
                            } else {
                                info!("  {fragment_type} segment {} -> {segment_size} octets", &frag.url);
                            }
                        }
                    } else {
                        warn!("Ignoring segment {} with non-{fragment_type} content-type", frag.url);
                    };
                    tmp_out.sync_all()
                        .map_err(|e| DashMpdError::Io(e, format!("syncing {fragment_type} fragment")))?;
                    Ok(tmp_out)
                },
                Err(e) => Err(network_error("HTTP error", &e)),
            }
        },
        Err(e) => Err(network_error(&format!("{e:?}"), &e)),
    }
}


// Retrieve the audio segments for period `period_counter` and concatenate them to a file at tmppath.
#[tracing::instrument(level="trace", skip_all)]
async fn fetch_period_audio(
    downloader: &mut DashDownloader,
    tmppath: &Path,
    audio_fragments: &[MediaFragment],
    ds: &mut DownloadState) -> Result<bool, DashMpdError>
{
    let start_download = Instant::now();
    let mut have_audio = false;
    {
        // We need a local scope for our temporary File, so that the file is closed when we later
        // optionally call the decryption application (which requires exclusive access to its input
        // file on Windows).
        let tmpfile_audio = File::create(tmppath).await
            .map_err(|e| DashMpdError::Io(e, String::from("creating audio tmpfile")))?;
        ensure_permissions_readable(tmppath).await?;
        let mut tmpfile_audio = BufWriter::new(tmpfile_audio);
        // Optionally create the directory to which we will save the audio fragments.
        if let Some(ref fragment_path) = downloader.fragment_path {
            let audio_fragment_dir = fragment_path.join("audio");
            if !audio_fragment_dir.exists() {
                fs::create_dir_all(audio_fragment_dir).await
                    .map_err(|e| DashMpdError::Io(e, String::from("creating audio fragment dir")))?;
            }
        }
        // TODO: in DASH, the init segment contains headers that are necessary to generate a valid MP4
        // file, so we should always abort if the first segment cannot be fetched. However, we could
        // tolerate loss of subsequent segments.
        for frag in audio_fragments.iter().filter(|f| f.period == ds.period_counter) {
            ds.segment_counter += 1;
            let progress_percent = (100.0 * ds.segment_counter as f32 / (2.0 + ds.segment_count as f32)).ceil() as u32;
            let url = &frag.url;
            // A manifest may use a data URL (RFC 2397) to embed media content such as the
            // initialization segment directly in the manifest (recommended by YouTube for live
            // streaming, but uncommon in practice).
            if url.scheme() == "data" {
                let us = &url.to_string();
                let du = DataUrl::process(us)
                    .map_err(|_| DashMpdError::Parsing(String::from("parsing data URL")))?;
                if du.mime_type().type_ != "audio" {
                    return Err(DashMpdError::UnhandledMediaStream(
                        String::from("expecting audio content in data URL")));
                }
                let (body, _fragment) = du.decode_to_vec()
                    .map_err(|_| DashMpdError::Parsing(String::from("decoding data URL")))?;
                if downloader.verbosity > 2 {
                    info!("  Audio segment data URL -> {} octets", body.len());
                }
                tmpfile_audio.write_all(&body)
                    .map_err(|e| DashMpdError::Io(e, String::from("writing DASH audio data")))
                    .await?;
                have_audio = true;
            } else {
                // We could download these segments in parallel, but that might upset some servers.
                'done: for _ in 0..downloader.fragment_retry_count {
                    match fetch_fragment(downloader, frag, "audio", progress_percent).await {
                        Ok(mut frag_file) => {
                            frag_file.rewind()
                                .map_err(|e| DashMpdError::Io(e, String::from("rewinding fragment tempfile")))?;
                            let mut buf = Vec::new();
                            frag_file.read_to_end(&mut buf)
                                .map_err(|e| DashMpdError::Io(e, String::from("reading fragment tempfile")))?;
                            tmpfile_audio.write_all(&buf)
                                .map_err(|e| DashMpdError::Io(e, String::from("writing DASH audio data")))
                                .await?;
                            have_audio = true;
                            break 'done;
                        },
                        Err(e) => {
                            if downloader.verbosity > 0 {
                                error!("Error fetching audio segment {url}: {e:?}");
                            }
                            ds.download_errors += 1;
                            if ds.download_errors > downloader.max_error_count {
                                error!("max_error_count network errors encountered");
                                return Err(DashMpdError::Network(
                                    String::from("more than max_error_count network errors")));
                            }
                        },
                    }
                    info!("  Retrying audio segment {url}");
                    if downloader.sleep_between_requests > 0 {
                        tokio::time::sleep(Duration::new(downloader.sleep_between_requests.into(), 0)).await;
                    }
                }
            }
        }
        tmpfile_audio.flush().map_err(|e| {
            error!("Couldn't flush DASH audio file: {e}");
            DashMpdError::Io(e, String::from("flushing DASH audio file"))
        }).await?;
    } // end local scope for the FileHandle
    if !downloader.decryption_keys.is_empty() {
        if downloader.verbosity > 0 {
            let metadata = fs::metadata(tmppath).await
                .map_err(|e| DashMpdError::Io(e, String::from("reading encrypted audio metadata")))?;
            info!("  Attempting to decrypt audio stream ({} kB) with {}",
                  metadata.len() / 1024,
                  downloader.decryptor_preference);
        }
        let out_ext = downloader.output_path.as_ref().unwrap()
            .extension()
            .unwrap_or(OsStr::new("mp4"));
        let decrypted = tmp_file_path("dashmpd-decrypted-audio", out_ext)?;
        if downloader.decryptor_preference.eq("mp4decrypt") {
            decrypt_mp4decrypt(downloader, tmppath, &decrypted, "audio").await?;
        } else if downloader.decryptor_preference.eq("shaka") {
            decrypt_shaka(downloader, tmppath, &decrypted, "audio").await?;
        } else if downloader.decryptor_preference.eq("shaka-container") {
            decrypt_shaka_container(downloader, tmppath, &decrypted, "audio").await?;
        } else if downloader.decryptor_preference.eq("mp4box") {
            decrypt_mp4box(downloader, tmppath, &decrypted, "audio").await?;
        } else if downloader.decryptor_preference.eq("mp4box-container") {
            decrypt_mp4box_container(downloader, tmppath, &decrypted, "audio").await?;
        } else {
            return Err(DashMpdError::Decrypting(String::from("unknown decryption application")));
        }
        if let Err(e) = fs::metadata(&decrypted).await {
            return Err(DashMpdError::Decrypting(format!("missing decrypted audio file: {e:?}")));
        }
        fs::remove_file(&tmppath).await
            .map_err(|e| DashMpdError::Io(e, String::from("deleting encrypted audio tmpfile")))?;
        fs::rename(&decrypted, &tmppath).await
            .map_err(|e| {
                let dbg = Command::new("bash")
                    .args(["-c", &format!("id;ls -l {}", decrypted.display())])
                    .output()
                    .unwrap();
                warn!("debugging ls: {}", String::from_utf8_lossy(&dbg.stdout));
                DashMpdError::Io(e, format!("renaming decrypted audio {}->{}", decrypted.display(), tmppath.display()))
            })?;
    }
    if let Ok(metadata) = fs::metadata(&tmppath).await {
        if downloader.verbosity > 1 {
            let mbytes = metadata.len() as f64 / (1024.0 * 1024.0);
            let elapsed = start_download.elapsed();
            info!("  Wrote {mbytes:.1}MB to DASH audio file ({:.1} MB/s)",
                     mbytes / elapsed.as_secs_f64());
        }
    }
    Ok(have_audio)
}


// Retrieve the video segments for period `period_counter` and concatenate them to a file at tmppath.
#[tracing::instrument(level="trace", skip_all)]
async fn fetch_period_video(
    downloader: &mut DashDownloader,
    tmppath: &Path,
    video_fragments: &[MediaFragment],
    ds: &mut DownloadState) -> Result<bool, DashMpdError>
{
    let start_download = Instant::now();
    let mut have_video = false;
    {
        // We need a local scope for our tmpfile_video File, so that the file is closed when we
        // later call the decryption helper application. Certain helper configurations like
        // mp4decrypt on Windows require exclusive access to its input file.
        let tmpfile_video = File::create(tmppath).await
            .map_err(|e| DashMpdError::Io(e, String::from("creating video tmpfile")))?;
        ensure_permissions_readable(tmppath).await?;
        let mut tmpfile_video = BufWriter::new(tmpfile_video);
        // Optionally create the directory to which we will save the video fragments.
        if let Some(ref fragment_path) = downloader.fragment_path {
            let video_fragment_dir = fragment_path.join("video");
            if !video_fragment_dir.exists() {
                fs::create_dir_all(video_fragment_dir).await
                    .map_err(|e| DashMpdError::Io(e, String::from("creating video fragment dir")))?;
            }
        }
        for frag in video_fragments.iter().filter(|f| f.period == ds.period_counter) {
            ds.segment_counter += 1;
            let progress_percent = (100.0 * ds.segment_counter as f32 / ds.segment_count as f32).ceil() as u32;
            if frag.url.scheme() == "data" {
                let us = &frag.url.to_string();
                let du = DataUrl::process(us)
                    .map_err(|_| DashMpdError::Parsing(String::from("parsing data URL")))?;
                if du.mime_type().type_ != "video" {
                    return Err(DashMpdError::UnhandledMediaStream(
                        String::from("expecting video content in data URL")));
                }
                let (body, _fragment) = du.decode_to_vec()
                    .map_err(|_| DashMpdError::Parsing(String::from("decoding data URL")))?;
                if downloader.verbosity > 2 {
                    info!("  Video segment data URL -> {} octets", body.len());
                }
                tmpfile_video.write_all(&body)
                    .map_err(|e| DashMpdError::Io(e, String::from("writing DASH video data")))
                    .await?;
                have_video = true;
            } else {
                'done: for _ in 0..downloader.fragment_retry_count {
                    match fetch_fragment(downloader, frag, "video", progress_percent).await {
                        Ok(mut frag_file) => {
                            frag_file.rewind()
                                .map_err(|e| DashMpdError::Io(e, String::from("rewinding fragment tempfile")))?;
                            let mut buf = Vec::new();
                            frag_file.read_to_end(&mut buf)
                                .map_err(|e| DashMpdError::Io(e, String::from("reading fragment tempfile")))?;
                            tmpfile_video.write_all(&buf)
                                .map_err(|e| DashMpdError::Io(e, String::from("writing DASH video data")))
                                .await?;
                            have_video = true;
                            break 'done;
                        },
                        Err(e) => {
                            if downloader.verbosity > 0 {
                                error!("  Error fetching video segment {}: {e:?}", frag.url);
                            }
                            ds.download_errors += 1;
                            if ds.download_errors > downloader.max_error_count {
                                return Err(DashMpdError::Network(
                                    String::from("more than max_error_count network errors")));
                            }
                        },
                    }
                    info!("  Retrying video segment {}", frag.url);
                    if downloader.sleep_between_requests > 0 {
                        tokio::time::sleep(Duration::new(downloader.sleep_between_requests.into(), 0)).await;
                    }
                }
            }
        }
        tmpfile_video.flush().map_err(|e| {
            error!("  Couldn't flush video file: {e}");
            DashMpdError::Io(e, String::from("flushing video file"))
        }).await?;
    } // end local scope for tmpfile_video File
    if !downloader.decryption_keys.is_empty() {
        if downloader.verbosity > 0 {
            let metadata = fs::metadata(tmppath).await
                .map_err(|e| DashMpdError::Io(e, String::from("reading encrypted video metadata")))?;
            info!("  Attempting to decrypt video stream ({} kB) with {}",
                   metadata.len() / 1024,
                   downloader.decryptor_preference);
        }
        let out_ext = downloader.output_path.as_ref().unwrap()
            .extension()
            .unwrap_or(OsStr::new("mp4"));
        let decrypted = tmp_file_path("dashmpd-decrypted-video", out_ext)?;
        if downloader.decryptor_preference.eq("mp4decrypt") {
            decrypt_mp4decrypt(downloader, tmppath, &decrypted, "video").await?;
        } else if downloader.decryptor_preference.eq("shaka") {
            decrypt_shaka(downloader, tmppath, &decrypted, "video").await?;
        } else if downloader.decryptor_preference.eq("shaka-container") {
            decrypt_shaka_container(downloader, tmppath, &decrypted, "video").await?;
        } else if downloader.decryptor_preference.eq("mp4box") {
            decrypt_mp4box(downloader, tmppath, &decrypted, "video").await?;
        } else if downloader.decryptor_preference.eq("mp4box-container") {
            decrypt_mp4box_container(downloader, tmppath, &decrypted, "video").await?;
        } else {
            return Err(DashMpdError::Decrypting(String::from("unknown decryption application")));
        }
        if let Err(e) = fs::metadata(&decrypted).await {
            return Err(DashMpdError::Decrypting(format!("missing decrypted video file: {e:?}")));
        }
        fs::remove_file(&tmppath).await
            .map_err(|e| DashMpdError::Io(e, String::from("deleting encrypted video tmpfile")))?;
        fs::rename(&decrypted, &tmppath).await
            .map_err(|e| DashMpdError::Io(e, String::from("renaming decrypted video")))?;
    }
    if let Ok(metadata) = fs::metadata(&tmppath).await {
        if downloader.verbosity > 1 {
            let mbytes = metadata.len() as f64 / (1024.0 * 1024.0);
            let elapsed = start_download.elapsed();
            info!("  Wrote {mbytes:.1}MB to DASH video file ({:.1} MB/s)",
                     mbytes / elapsed.as_secs_f64());
        }
    }
    Ok(have_video)
}


// Retrieve the video segments for period `ds.period_counter` and concatenate them to a file at `tmppath`.
#[tracing::instrument(level="trace", skip_all)]
async fn fetch_period_subtitles(
    downloader: &DashDownloader,
    tmppath: &Path,
    subtitle_fragments: &[MediaFragment],
    subtitle_formats: &[SubtitleType],
    ds: &mut DownloadState) -> Result<bool, DashMpdError>
{
    let client = downloader.http_client.clone().unwrap();
    let start_download = Instant::now();
    let mut have_subtitles = false;
    {
        let tmpfile_subs = File::create(tmppath).await
            .map_err(|e| DashMpdError::Io(e, String::from("creating subs tmpfile")))?;
        ensure_permissions_readable(tmppath).await?;
        let mut tmpfile_subs = BufWriter::new(tmpfile_subs);
        for frag in subtitle_fragments {
            // Update any ProgressObservers
            ds.segment_counter += 1;
            let progress_percent = (100.0 * ds.segment_counter as f32 / ds.segment_count as f32).ceil() as u32;
            for observer in &downloader.progress_observers {
                observer.update(progress_percent, "Fetching subtitle segments");
            }
            if frag.url.scheme() == "data" {
                let us = &frag.url.to_string();
                let du = DataUrl::process(us)
                    .map_err(|_| DashMpdError::Parsing(String::from("parsing data URL")))?;
                if du.mime_type().type_ != "video" {
                    return Err(DashMpdError::UnhandledMediaStream(
                        String::from("expecting video content in data URL")));
                }
                let (body, _fragment) = du.decode_to_vec()
                    .map_err(|_| DashMpdError::Parsing(String::from("decoding data URL")))?;
                if downloader.verbosity > 2 {
                    info!("  Subtitle segment data URL -> {} octets", body.len());
                }
                tmpfile_subs.write_all(&body)
                    .map_err(|e| DashMpdError::Io(e, String::from("writing DASH subtitle data")))
                    .await?;
                have_subtitles = true;
            } else {
                let fetch = || async {
                    let mut req = client.get(frag.url.clone())
                        .header("Sec-Fetch-Mode", "navigate");
                    if let Some(sb) = &frag.start_byte {
                        if let Some(eb) = &frag.end_byte {
                            req = req.header(RANGE, format!("bytes={sb}-{eb}"));
                        }
                    }
                    if let Some(referer) = &downloader.referer {
                        req = req.header("Referer", referer);
                    } else {
                        req = req.header("Referer", downloader.redirected_url.to_string());
                    }
                    if let Some(username) = &downloader.auth_username {
                        if let Some(password) = &downloader.auth_password {
                            req = req.basic_auth(username, Some(password));
                        }
                    }
                    if let Some(token) = &downloader.auth_bearer_token {
                        req = req.bearer_auth(token);
                    }
                    req.send().await?
                        .error_for_status()
                };
                let mut failure = None;
                match fetch
                    .retry(ExponentialBuilder::default())
                    .when(reqwest_error_transient_p)
                    .notify(notify_transient)
                    .await
                {
                    Ok(response) => {
                        if response.status().is_success() {
                            let dash_bytes = response.bytes().await
                                .map_err(|e| network_error("fetching DASH subtitle segment", &e))?;
                            if downloader.verbosity > 2 {
                                if let Some(sb) = &frag.start_byte {
                                    if let Some(eb) = &frag.end_byte {
                                        info!("  Subtitle segment {} range {sb}-{eb} -> {} octets",
                                                 &frag.url, dash_bytes.len());
                                    }
                                } else {
                                    info!("  Subtitle segment {} -> {} octets", &frag.url, dash_bytes.len());
                                }
                            }
                            let size = min((dash_bytes.len()/1024 + 1) as u32, u32::MAX);
                            throttle_download_rate(downloader, size).await?;
                            tmpfile_subs.write_all(&dash_bytes)
                                .map_err(|e| DashMpdError::Io(e, String::from("writing DASH subtitle data")))
                                .await?;
                            have_subtitles = true;
                        } else {
                            failure = Some(format!("HTTP error {}", response.status().as_str()));
                        }
                    },
                    Err(e) => failure = Some(format!("{e}")),
                }
                if let Some(f) = failure {
                    if downloader.verbosity > 0 {
                        error!("{f} fetching subtitle segment {}", &frag.url);
                    }
                    ds.download_errors += 1;
                    if ds.download_errors > downloader.max_error_count {
                        return Err(DashMpdError::Network(
                            String::from("more than max_error_count network errors")));
                    }
                }
            }
            if downloader.sleep_between_requests > 0 {
                tokio::time::sleep(Duration::new(downloader.sleep_between_requests.into(), 0)).await;
            }
        }
        tmpfile_subs.flush().map_err(|e| {
            error!("Couldn't flush subs file: {e}");
            DashMpdError::Io(e, String::from("flushing subtitle file"))
        }).await?;
    } // end local scope for tmpfile_subs File
    if have_subtitles {
        if let Ok(metadata) = fs::metadata(tmppath).await {
            if downloader.verbosity > 1 {
                let mbytes = metadata.len() as f64 / (1024.0 * 1024.0);
                let elapsed = start_download.elapsed();
                info!("  Wrote {mbytes:.1}MB to DASH subtitle file ({:.1} MB/s)",
                      mbytes / elapsed.as_secs_f64());
            }
        }
        // TODO: for subtitle_formats sub and srt we could also try to embed them in the output
        // file, for example using MP4Box or mkvmerge
        if subtitle_formats.contains(&SubtitleType::Wvtt) ||
           subtitle_formats.contains(&SubtitleType::Ttxt)
        {
            // We can extract these from the MP4 container in .srt format, using MP4Box.
            if downloader.verbosity > 0 {
                if let Some(fmt) = subtitle_formats.first() {
                    info!("  Downloaded media contains subtitles in {fmt:?} format");
                }
                info!("  Running MP4Box to extract subtitles");
            }
            let out = downloader.output_path.as_ref().unwrap()
                .with_extension("srt");
            let out_str = out.to_string_lossy();
            let tmp_str = tmppath.to_string_lossy();
            let args = vec![
                "-srt", "1",
                "-out", &out_str,
                &tmp_str];
            if downloader.verbosity > 0 {
                info!("  Running MP4Box {}", args.join(" "));
            }
            if let Ok(mp4box) = Command::new(downloader.mp4box_location.clone())
                .args(args)
                .output()
            {
                let msg = partial_process_output(&mp4box.stdout);
                if !msg.is_empty() {
                    info!("  MP4Box stdout: {msg}");
                }
                let msg = partial_process_output(&mp4box.stderr);
                if !msg.is_empty() {
                    info!("  MP4Box stderr: {msg}");
                }
                if mp4box.status.success() {
                    info!("  Extracted subtitles as SRT");
                } else {
                    warn!("  Error running MP4Box to extract subtitles");
                }
            } else {
                warn!("  Failed to spawn MP4Box to extract subtitles");
            }
        }
        if subtitle_formats.contains(&SubtitleType::Stpp) {
            if downloader.verbosity > 0 {
                info!("  Converting STPP subtitles to TTML format with ffmpeg");
            }
            let out = downloader.output_path.as_ref().unwrap()
                .with_extension("ttml");
            let tmppath_arg = tmppath.to_string_lossy();
            let out_arg = &out.to_string_lossy();
            let ffmpeg_args = vec![
                "-hide_banner",
                "-nostats",
                "-loglevel", "error",
                "-y",  // overwrite output file if it exists
                "-nostdin",
                "-i", &tmppath_arg,
                "-f", "data",
                "-map", "0",
                "-c", "copy",
                out_arg];
            if downloader.verbosity > 0 {
                info!("  Running ffmpeg {}", ffmpeg_args.join(" "));
            }
            if let Ok(ffmpeg) = Command::new(downloader.ffmpeg_location.clone())
                .args(ffmpeg_args)
                .output()
            {
                let msg = partial_process_output(&ffmpeg.stdout);
                if !msg.is_empty() {
                    info!("  ffmpeg stdout: {msg}");
                }
                let msg = partial_process_output(&ffmpeg.stderr);
                if !msg.is_empty() {
                    info!("  ffmpeg stderr: {msg}");
                }
                if ffmpeg.status.success() {
                    info!("  Converted STPP subtitles to TTML format");
                } else {
                    warn!("  Error running ffmpeg to convert subtitles");
                }
            }
            // TODO: it would be useful to also convert the subtitles to SRT/WebVTT format, as they tend
            // to be better supported. However, ffmpeg does not seem able to convert from SPTT to
            // these formats. We could perhaps use the Python ttconv package, or below with MP4Box.
        }

    }
    Ok(have_subtitles)
}


// Fetch XML content of manifest from an HTTP/HTTPS URL
async fn fetch_mpd_http(downloader: &mut DashDownloader) -> Result<Bytes, DashMpdError> {
    let client = &downloader.http_client.clone().unwrap();
    let send_request = || async {
        let mut req = client.get(&downloader.mpd_url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .header("Accept-Language", "en-US,en")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Sec-Fetch-Mode", "navigate");
        if let Some(referer) = &downloader.referer {
            req = req.header("Referer", referer);
        }
        if let Some(username) = &downloader.auth_username {
            if let Some(password) = &downloader.auth_password {
                req = req.basic_auth(username, Some(password));
            }
        }
        if let Some(token) = &downloader.auth_bearer_token {
            req = req.bearer_auth(token);
        }
        req.send().await?
            .error_for_status()
    };
    for observer in &downloader.progress_observers {
        observer.update(1, "Fetching DASH manifest");
    }
    if downloader.verbosity > 0 {
        if !downloader.fetch_audio && !downloader.fetch_video && !downloader.fetch_subtitles {
            info!("Only simulating media downloads");
        }
        info!("Fetching the DASH manifest");
    }
    let response = send_request
        .retry(ExponentialBuilder::default())
        .when(reqwest_error_transient_p)
        .notify(notify_transient)
        .await
        .map_err(|e| network_error("requesting DASH manifest", &e))?;
    if !response.status().is_success() {
        let msg = format!("fetching DASH manifest (HTTP {})", response.status().as_str());
        return Err(DashMpdError::Network(msg));
    }
    downloader.redirected_url = response.url().clone();
    response.bytes().await
        .map_err(|e| network_error("fetching DASH manifest", &e))
}

// Fetch XML content of manifest from a file:// URL. The reqwest library is not able to download
// from this URL type.
async fn fetch_mpd_file(downloader: &mut DashDownloader) -> Result<Bytes, DashMpdError> {
    if ! &downloader.mpd_url.starts_with("file://") {
        return Err(DashMpdError::Other(String::from("expecting file:// URL scheme")));
    }
    let url = Url::parse(&downloader.mpd_url)
        .map_err(|_| DashMpdError::Other(String::from("parsing MPD URL")))?;
    let path = url.to_file_path()
        .map_err(|_| DashMpdError::Other(String::from("extracting path from file:// URL")))?;
    let octets = fs::read(path).await
               .map_err(|_| DashMpdError::Other(String::from("reading from file:// URL")))?;
    Ok(Bytes::from(octets))
}


#[tracing::instrument(level="trace", skip_all)]
async fn fetch_mpd(downloader: &mut DashDownloader) -> Result<PathBuf, DashMpdError> {
    #[cfg(all(feature = "sandbox", target_os = "linux"))]
    if downloader.sandbox {
        if let Err(e) = restrict_thread(downloader) {
            warn!("Sandboxing failed: {e:?}");
        }
    }
    let xml = if downloader.mpd_url.starts_with("file://") {
        fetch_mpd_file(downloader).await?
    } else {
        fetch_mpd_http(downloader).await?
    };
    let mut mpd: MPD = parse_resolving_xlinks(downloader, &xml).await
        .map_err(|e| parse_error("parsing DASH XML", e))?;
    // From the DASH specification: "If at least one MPD.Location element is present, the value of
    // any MPD.Location element is used as the MPD request". We make a new request to the URI and reparse.
    let client = &downloader.http_client.clone().unwrap();
    if let Some(new_location) = &mpd.locations.first() {
        let new_url = &new_location.url;
        if downloader.verbosity > 0 {
            info!("Redirecting to new manifest <Location> {new_url}");
        }
        let send_request = || async {
            let mut req = client.get(new_url)
                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                .header("Accept-Language", "en-US,en")
                .header("Sec-Fetch-Mode", "navigate");
            if let Some(referer) = &downloader.referer {
                req = req.header("Referer", referer);
            } else {
                req = req.header("Referer", downloader.redirected_url.to_string());
            }
            if let Some(username) = &downloader.auth_username {
                if let Some(password) = &downloader.auth_password {
                    req = req.basic_auth(username, Some(password));
                }
            }
            if let Some(token) = &downloader.auth_bearer_token {
                req = req.bearer_auth(token);
            }
            req.send().await?
                .error_for_status()
        };
        let response = send_request
            .retry(ExponentialBuilder::default())
            .when(reqwest_error_transient_p)
            .notify(notify_transient)
            .await
            .map_err(|e| network_error("requesting relocated DASH manifest", &e))?;
        if !response.status().is_success() {
            let msg = format!("fetching DASH manifest (HTTP {})", response.status().as_str());
            return Err(DashMpdError::Network(msg));
        }
        downloader.redirected_url = response.url().clone();
        let xml = response.bytes().await
            .map_err(|e| network_error("fetching relocated DASH manifest", &e))?;
        mpd = parse_resolving_xlinks(downloader, &xml).await
            .map_err(|e| parse_error("parsing relocated DASH XML", e))?;
    }
    if mpd_is_dynamic(&mpd) {
        // TODO: look at algorithm used in function segment_numbers at
        // https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/dash_manifest.py
        if downloader.allow_live_streams {
            if downloader.verbosity > 0 {
                warn!("Attempting to download from live stream (this may not work).");
            }
        } else {
            return Err(DashMpdError::UnhandledMediaStream("Don't know how to download dynamic MPD".to_string()));
        }
    }
    let mut toplevel_base_url = downloader.redirected_url.clone();
    // There may be several BaseURL tags in the MPD, but we don't currently implement failover
    if let Some(bu) = &mpd.base_url.first() {
        toplevel_base_url = merge_baseurls(&downloader.redirected_url, &bu.base)?;
    }
    // A BaseURL specified explicitly when instantiating the DashDownloader overrides the BaseURL
    // specified in the manifest.
    if let Some(base) = &downloader.base_url {
        toplevel_base_url = merge_baseurls(&downloader.redirected_url, base)?;
    }
    if downloader.verbosity > 0 {
        let pcount = mpd.periods.len();
        info!("DASH manifest has {pcount} period{}", if pcount > 1 { "s" }  else { "" });
        print_available_streams(&mpd);
    }
    // Analyse the content of each Period in the manifest. We need to ensure that we associate media
    // segments with the correct period, because segments in each Period may use different codecs,
    // so they can't be concatenated together directly without reencoding. The main purpose for this
    // iteration of Periods (which is then followed by an iteration over Periods where we retrieve
    // the media segments and concatenate them) is to obtain a count of the total number of media
    // fragments that we are going to retrieve, so that the ProgressBar shows information relevant
    // to the total download (we don't want a per-Period ProgressBar).
    let mut pds: Vec<PeriodDownloads> = Vec::new();
    let mut period_counter = 0;
    for mpd_period in &mpd.periods {
        let period = mpd_period.clone();
        period_counter += 1;
        if let Some(min) = downloader.minimum_period_duration {
            if let Some(duration) = period.duration {
                if duration < min {
                    if let Some(id) = period.id.as_ref() {
                        info!("Skipping period {id} (#{period_counter}): duration is less than requested minimum");
                    } else {
                        info!("Skipping period #{period_counter}: duration is less than requested minimum");
                    }
                    continue;
                }
            }
        }
        let mut pd = PeriodDownloads { period_counter, ..Default::default() };
        if let Some(id) = period.id.as_ref() {
            pd.id = Some(id.clone());
        }
        if downloader.verbosity > 0 {
            if let Some(id) = period.id.as_ref() {
                info!("Preparing download for period {id} (#{period_counter})");
            } else {
                info!("Preparing download for period #{period_counter}");
            }
        }
        let mut base_url = toplevel_base_url.clone();
        // A BaseURL could be specified for each Period
        if let Some(bu) = period.BaseURL.first() {
            base_url = merge_baseurls(&base_url, &bu.base)?;
        }
        let mut audio_outputs = PeriodOutputs::default();
        if downloader.fetch_audio {
            audio_outputs = do_period_audio(downloader, &mpd, &period, period_counter, base_url.clone()).await?;
            for f in audio_outputs.fragments {
                pd.audio_fragments.push(f);
            }
            pd.selected_audio_language = audio_outputs.selected_audio_language;
        }
        let mut video_outputs = PeriodOutputs::default();
        if downloader.fetch_video {
            video_outputs = do_period_video(downloader, &mpd, &period, period_counter, base_url.clone()).await?;
            for f in video_outputs.fragments {
                pd.video_fragments.push(f);
            }
        }
        match do_period_subtitles(downloader, &mpd, &period, period_counter, base_url.clone()).await {
            Ok(subtitle_outputs) => {
                for f in subtitle_outputs.fragments {
                    pd.subtitle_fragments.push(f);
                }
                for f in subtitle_outputs.subtitle_formats {
                    pd.subtitle_formats.push(f);
                }
            },
            Err(e) => warn!("  Ignoring error triggered while processing subtitles: {e}"),
        }
        // Print some diagnostics information on the selected streams
        if downloader.verbosity > 0 {
            use base64::prelude::{Engine as _, BASE64_STANDARD};

            audio_outputs.diagnostics.iter().for_each(|msg| info!("{}", msg));
            for f in pd.audio_fragments.iter().filter(|f| f.is_init) {
                if let Some(pssh_bytes) = extract_init_pssh(downloader, f.url.clone()).await {
                    info!("    PSSH (from init segment): {}", BASE64_STANDARD.encode(&pssh_bytes));
                    if let Ok(pssh) = pssh_box::from_bytes(&pssh_bytes) {
                        info!("    {}", pssh.to_string());
                    }
                }
            }
            video_outputs.diagnostics.iter().for_each(|msg| info!("{}", msg));
            for f in pd.video_fragments.iter().filter(|f| f.is_init) {
                if let Some(pssh_bytes) = extract_init_pssh(downloader, f.url.clone()).await {
                    info!("    PSSH (from init segment): {}", BASE64_STANDARD.encode(&pssh_bytes));
                    if let Ok(pssh) = pssh_box::from_bytes(&pssh_bytes) {
                        info!("    {}", pssh.to_string());
                    }
                }
            }
        }
        pds.push(pd);
    } // loop over Periods

    // To collect the muxed audio and video segments for each Period in the MPD, before their
    // final concatenation-with-reencoding.
    let output_path = &downloader.output_path.as_ref().unwrap().clone();
    let mut period_output_pathbufs: Vec<PathBuf> = Vec::new();
    let mut ds = DownloadState {
        period_counter: 0,
        // The additional +2 is for our initial .mpd fetch action and final muxing action
        segment_count: pds.iter().map(period_fragment_count).sum(),
        segment_counter: 0,
        download_errors: 0
    };
    for pd in pds {
        let mut have_audio = false;
        let mut have_video = false;
        let mut have_subtitles = false;
        ds.period_counter = pd.period_counter;
        let period_output_path = output_path_for_period(output_path, pd.period_counter);
        #[allow(clippy::collapsible_if)]
        if downloader.verbosity > 0 {
            if downloader.fetch_audio || downloader.fetch_video || downloader.fetch_subtitles {
                let idnum = if let Some(id) = pd.id {
                    format!("id={} (#{})", id, pd.period_counter)
                } else {
                    format!("#{}", pd.period_counter)
                };
                info!("Period {idnum}: fetching {} audio, {} video and {} subtitle segments",
                      pd.audio_fragments.len(),
                      pd.video_fragments.len(),
                      pd.subtitle_fragments.len());
            }
        }
        let output_ext = downloader.output_path.as_ref().unwrap()
            .extension()
            .unwrap_or(OsStr::new("mp4"));
        let tmppath_audio = if let Some(ref path) = downloader.keep_audio {
            path.clone()
        } else {
            tmp_file_path("dashmpd-audio", output_ext)?
        };
        let tmppath_video = if let Some(ref path) = downloader.keep_video {
            path.clone()
        } else {
            tmp_file_path("dashmpd-video", output_ext)?
        };
        let tmppath_subs = tmp_file_path("dashmpd-subs", OsStr::new("sub"))?;
        if downloader.fetch_audio && !pd.audio_fragments.is_empty() {
            // TODO: to allow the download of multiple audio tracks (with multiple languages), we
            // need to call fetch_period_audio multiple times with a different file path each time,
            // and with the audio_fragments only relevant for that language.
            have_audio = fetch_period_audio(downloader,
                                            &tmppath_audio, &pd.audio_fragments,
                                            &mut ds).await?;
        }
        if downloader.fetch_video && !pd.video_fragments.is_empty() {
            have_video = fetch_period_video(downloader,
                                            &tmppath_video, &pd.video_fragments,
                                            &mut ds).await?;
        }
        // Here we handle subtitles that are distributed in fragmented MP4 segments, rather than as a
        // single .srt or .vtt file file. This is the case for WVTT (WebVTT) and STPP (which should be
        // formatted as EBU-TT for DASH media) formats.
        if downloader.fetch_subtitles && !pd.subtitle_fragments.is_empty() {
            have_subtitles = fetch_period_subtitles(downloader,
                                                    &tmppath_subs,
                                                    &pd.subtitle_fragments,
                                                    &pd.subtitle_formats,
                                                    &mut ds).await?;
        }

        // The output file for this Period is either a mux of the audio and video streams, if both
        // are present, or just the audio stream, or just the video stream.
        if have_audio && have_video {
            for observer in &downloader.progress_observers {
                observer.update(99, "Muxing audio and video");
            }
            if downloader.verbosity > 1 {
                info!("  Muxing audio and video streams");
            }
            let audio_tracks = vec![
                AudioTrack {
                    language: pd.selected_audio_language,
                    path: tmppath_audio.clone()
                }];
            mux_audio_video(downloader, &period_output_path, &audio_tracks, &tmppath_video)?;
            if pd.subtitle_formats.contains(&SubtitleType::Stpp) {
                let container = match &period_output_path.extension() {
                    Some(ext) => ext.to_str().unwrap_or("mp4"),
                    None => "mp4",
                };
                if container.eq("mp4") {
                    if downloader.verbosity > 1 {
                        if let Some(fmt) = &pd.subtitle_formats.first() {
                            info!("  Downloaded media contains subtitles in {fmt:?} format");
                        }
                        info!("  Running MP4Box to merge subtitles with output MP4 container");
                    }
                    // We can try to add the subtitles to the MP4 container, using MP4Box. Only
                    // works with MP4 containers.
                    let tmp_str = tmppath_subs.to_string_lossy();
                    let period_output_str = period_output_path.to_string_lossy();
                    let args = vec!["-add", &tmp_str, &period_output_str];
                    if downloader.verbosity > 0 {
                        info!("  Running MP4Box {}", args.join(" "));
                    }
                    if let Ok(mp4box) = Command::new(downloader.mp4box_location.clone())
                        .args(args)
                        .output()
                    {
                        let msg = partial_process_output(&mp4box.stdout);
                        if !msg.is_empty() {
                            info!("  MP4Box stdout: {msg}");
                        }
                        let msg = partial_process_output(&mp4box.stderr);
                        if !msg.is_empty() {
                            info!("  MP4Box stderr: {msg}");
                        }
                        if mp4box.status.success() {
                            info!("  Merged subtitles with MP4 container");
                        } else {
                            warn!("  Error running MP4Box to merge subtitles");
                        }
                    } else {
                        warn!("  Failed to spawn MP4Box to merge subtitles");
                    }
                } else if container.eq("mkv") || container.eq("webm") {
                    // Try using mkvmerge to add a subtitle track. mkvmerge does not seem to be able
                    // to merge STPP subtitles, but can merge SRT if we have managed to convert
                    // them.
                    //
                    // We mkvmerge to a temporary output file, and if the command succeeds we copy
                    // that to the original output path. Note that mkvmerge on Windows is compiled
                    // using MinGW and isn't able to handle native pathnames (for instance files
                    // created with tempfile::Builder), so we use temporary_outpath() which will create a
                    // temporary file in the current directory on Windows.
                    //
                    //    mkvmerge -o output.mkv input.mkv subs.srt
                    let srt = period_output_path.with_extension("srt");
                    if srt.exists() {
                        if downloader.verbosity > 0 {
                            info!("  Running mkvmerge to merge subtitles with output Matroska container");
                        }
                        let tmppath = temporary_outpath(".mkv")?;
                        let pop_arg = &period_output_path.to_string_lossy();
                        let srt_arg = &srt.to_string_lossy();
                        let mkvmerge_args = vec!["-o", &tmppath, pop_arg, srt_arg];
                        if downloader.verbosity > 0 {
                            info!("  Running mkvmerge {}", mkvmerge_args.join(" "));
                        }
                        if let Ok(mkvmerge) = Command::new(downloader.mkvmerge_location.clone())
                            .args(mkvmerge_args)
                            .output()
                        {
                            let msg = partial_process_output(&mkvmerge.stdout);
                            if !msg.is_empty() {
                                info!("  mkvmerge stdout: {msg}");
                            }
                            let msg = partial_process_output(&mkvmerge.stderr);
                            if !msg.is_empty() {
                                info!("  mkvmerge stderr: {msg}");
                            }
                            if mkvmerge.status.success() {
                                info!("  Merged subtitles with Matroska container");
                                // Copy the output file from mkvmerge to the period_output_path
                                // local scope so that tmppath is not busy on Windows and can be deleted
                                {
                                    let tmpfile = File::open(tmppath.clone()).await
                                        .map_err(|e| DashMpdError::Io(
                                            e, String::from("opening mkvmerge output")))?;
                                    let mut merged = BufReader::new(tmpfile);
                                    // This will truncate the period_output_path
                                    let outfile = File::create(period_output_path.clone()).await
                                        .map_err(|e| DashMpdError::Io(
                                            e, String::from("creating output file")))?;
                                    let mut sink = BufWriter::new(outfile);
                                    io::copy(&mut merged, &mut sink).await
                                        .map_err(|e| DashMpdError::Io(
                                            e, String::from("copying mkvmerge output to output file")))?;
                                }
                                if env::var("DASHMPD_PERSIST_FILES").is_err() {
	                            if let Err(e) = fs::remove_file(tmppath).await {
                                        warn!("  Error deleting temporary mkvmerge output: {e}");
                                    }
                                }
                            } else {
                                warn!("  Error running mkvmerge to merge subtitles");
                            }
                        }
                    }
                }
            }
        } else if have_audio {
            copy_audio_to_container(downloader, &period_output_path, &tmppath_audio)?;
        } else if have_video {
            copy_video_to_container(downloader, &period_output_path, &tmppath_video)?;
        } else if downloader.fetch_video && downloader.fetch_audio {
            return Err(DashMpdError::UnhandledMediaStream("no audio or video streams found".to_string()));
        } else if downloader.fetch_video {
            return Err(DashMpdError::UnhandledMediaStream("no video streams found".to_string()));
        } else if downloader.fetch_audio {
            return Err(DashMpdError::UnhandledMediaStream("no audio streams found".to_string()));
        }
        #[allow(clippy::collapsible_if)]
        if downloader.keep_audio.is_none() && downloader.fetch_audio {
            if env::var("DASHMPD_PERSIST_FILES").is_err() {
                if tmppath_audio.exists() && fs::remove_file(tmppath_audio).await.is_err() {
                    info!("  Failed to delete temporary file for audio stream");
                }
            }
        }
        #[allow(clippy::collapsible_if)]
        if downloader.keep_video.is_none() && downloader.fetch_video {
            if env::var("DASHMPD_PERSIST_FILES").is_err() {
                if tmppath_video.exists() && fs::remove_file(tmppath_video).await.is_err() {
                    info!("  Failed to delete temporary file for video stream");
                }
            }
        }
        #[allow(clippy::collapsible_if)]
        if env::var("DASHMPD_PERSIST_FILES").is_err() {
            if downloader.fetch_subtitles && tmppath_subs.exists() &&
                fs::remove_file(tmppath_subs).await.is_err() {
                info!("  Failed to delete temporary file for subtitles");
            }
        }
        if downloader.verbosity > 1 && (downloader.fetch_audio || downloader.fetch_video || have_subtitles) {
            if let Ok(metadata) = fs::metadata(&period_output_path).await {
                info!("  Wrote {:.1}MB to media file", metadata.len() as f64 / (1024.0 * 1024.0));
            }
        }
        if have_audio || have_video {
            period_output_pathbufs.push(period_output_path);
        }
    } // Period iterator
    let period_output_paths: Vec<&Path> = period_output_pathbufs
        .iter()
        .map(PathBuf::as_path)
        .collect();
    #[allow(clippy::comparison_chain)]
    if period_output_paths.len() == 1 {
        // We already arranged to write directly to the requested output_path.
        maybe_record_metainformation(output_path, downloader, &mpd);
    } else if period_output_paths.len() > 1 {
        // If the streams for the different periods are all of the same resolution, we can
        // concatenate them (with reencoding) into a single media file. Otherwise, we can't
        // concatenate without rescaling and loss of quality, so we leave them in separate files.
        // This feature isn't implemented using libav instead of ffmpeg as a subprocess.
        #[allow(unused_mut)]
        let mut concatenated = false;
        #[cfg(not(feature = "libav"))]
        // if downloader.concatenate_periods && video_containers_concatable(downloader, &period_output_paths) {
        if downloader.concatenate_periods && video_containers_concatable(downloader, &period_output_paths) {
            info!("Preparing to concatenate multiple Periods into one output file");
            concat_output_files(downloader, &period_output_paths)?;
            for p in &period_output_paths[1..] {
                if fs::remove_file(p).await.is_err() {
                    warn!("  Failed to delete temporary file {}", p.display());
                }
            }
            concatenated = true;
            if let Some(pop) = period_output_paths.first() {
                maybe_record_metainformation(pop, downloader, &mpd);
            }
        }
        if !concatenated {
            info!("Media content has been saved in a separate file for each period:");
            // FIXME this is not the original period number if we have dropped periods
            period_counter = 0;
            for p in period_output_paths {
                period_counter += 1;
                info!("  Period #{period_counter}: {}", p.display());
                maybe_record_metainformation(p, downloader, &mpd);
            }
        }
    }
    let have_content_protection = mpd.periods.iter().any(
        |p| p.adaptations.iter().any(
            |a| (!a.ContentProtection.is_empty()) ||
                a.representations.iter().any(
                    |r| !r.ContentProtection.is_empty())));
    if have_content_protection && downloader.decryption_keys.is_empty() {
        warn!("Manifest seems to use ContentProtection (DRM), but you didn't provide decryption keys.");
    }
    for observer in &downloader.progress_observers {
        observer.update(100, "Done");
    }
    Ok(PathBuf::from(output_path))
}


#[cfg(test)]
mod tests {
    #[test]
    fn test_resolve_url_template() {
        use std::collections::HashMap;
        use super::resolve_url_template;

        assert_eq!(resolve_url_template("AA$Time$BB", &HashMap::from([("Time", "ZZZ".to_string())])),
                   "AAZZZBB");
        assert_eq!(resolve_url_template("AA$Number%06d$BB", &HashMap::from([("Number", "42".to_string())])),
                   "AA000042BB");
        let dict = HashMap::from([("RepresentationID", "640x480".to_string()),
                                  ("Number", "42".to_string()),
                                  ("Time", "ZZZ".to_string())]);
        assert_eq!(resolve_url_template("AA/$RepresentationID$/segment-$Number%05d$.mp4", &dict),
                   "AA/640x480/segment-00042.mp4");
    }
}
