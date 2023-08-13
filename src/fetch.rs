//! Support for downloading content from DASH MPD media streams.

use std::env;
use fs_err as fs;
use fs::File;
use std::io;
use std::io::{Write, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::collections::HashMap;
use std::cmp::min;
use std::num::NonZeroU32;
use log::{info, warn, error};
use regex::Regex;
use url::Url;
use data_url::DataUrl;
use reqwest::header::RANGE;
use backoff::{future::retry_notify, ExponentialBackoff};
use governor::{Quota, RateLimiter};
use async_recursion::async_recursion;
use crate::{MPD, Period, Representation, AdaptationSet, DashMpdError};
use crate::{parse, mux_audio_video};
use crate::{is_audio_adaptation, is_video_adaptation, is_subtitle_adaptation};
use crate::{subtitle_type, content_protection_type, SubtitleType};
#[cfg(not(feature = "libav"))]
use crate::ffmpeg::{video_containers_concatable, concat_output_files};

/// A `Client` from the `reqwest` crate, that we use to download content over HTTP.
pub type HttpClient = reqwest::Client;


// This doesn't work correctly on modern Android, where there is no global location for temporary
// files (fix needed in the tempfile crate)
fn tmp_file_path(prefix: &str) -> Result<PathBuf, DashMpdError> {
    let file = tempfile::Builder::new()
        .prefix(prefix)
        .rand_bytes(5)
        .tempfile()
        .map_err(|e| DashMpdError::Io(e, String::from("creating temporary file")))?;
    let s = file.path().to_path_buf();

    Ok(s)
}



/// Receives updates concerning the progression of the download, and can display this information to
/// the user, for example using a progress bar.
pub trait ProgressObserver: Send + Sync {
    fn update(&self, percent: u32, message: &str);
}


/// Preference for retrieving media representation with highest quality (and highest file size) or
/// lowest quality (and lowest file size).
#[derive(PartialEq, Eq, Default)]
pub enum QualityPreference { #[default] Lowest, Highest }


/// The DashDownloader allows the download of streaming media content from a DASH MPD manifest. This
/// involves fetching the manifest file, parsing it, identifying the relevant audio and video
/// representations, downloading all the segments, concatenating them then muxing the audio and
/// video streams to produce a single video file including audio. This should work with both
/// MPEG-DASH MPD manifests (where the media segments are typically placed in MPEG-2 TS containers)
/// and for [WebM-DASH](http://wiki.webmproject.org/adaptive-streaming/webm-dash-specification).
pub struct DashDownloader {
    pub mpd_url: String,
    pub output_path: Option<PathBuf>,
    http_client: Option<HttpClient>,
    quality_preference: QualityPreference,
    language_preference: Option<String>,
    fetch_video: bool,
    fetch_audio: bool,
    fetch_subtitles: bool,
    keep_video: Option<PathBuf>,
    keep_audio: Option<PathBuf>,
    concatenate_periods: bool,
    fragment_path: Option<PathBuf>,
    decryption_keys: HashMap<String, String>,
    content_type_checks: bool,
    max_error_count: u32,
    progress_observers: Vec<Arc<dyn ProgressObserver>>,
    sleep_between_requests: u8,
    rate_limit: u64,
    verbosity: u8,
    record_metainformation: bool,
    pub ffmpeg_location: String,
    pub vlc_location: String,
    pub mkvmerge_location: String,
    pub mp4box_location: String,
    pub mp4decrypt_location: String,
}


// Parse a range specifier, such as Initialization@range or SegmentBase@indexRange attributes, of
// the form "45-67"
fn parse_range(range: &str) -> Result<(u64, u64), DashMpdError> {
    let v: Vec<&str> = range.split_terminator('-').collect();
    if v.len() != 2 {
        return Err(DashMpdError::Parsing(format!("invalid range specifier: {range}")));
    }
    let start: u64 = v[0].parse()
        .map_err(|_| DashMpdError::Parsing(String::from("invalid start for range specifier")))?;
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
}

fn make_fragment(period: u8, url: Url, start_byte: Option<u64>, end_byte: Option<u64>) -> MediaFragment {
    MediaFragment{ period, url, start_byte, end_byte, is_init: false }
}

// This struct is used to share information concerning the media fragments identified while parsing
// a Period as being wanted for download, alongside any diagnostics information that we collected
// while parsing the Period (in particular, any ContentProtection details).
#[derive(Debug)]
struct PeriodOutputs {
    fragments: Vec<MediaFragment>,
    diagnostics: String,
    subtitle_formats: Vec<SubtitleType>,
}


// We don't want to test this code example on the CI infrastructure as it's too expensive
// and requires network access.
#[cfg(not(doctest))]
/// The DashDownloader follows the builder pattern to allow various optional arguments concerning
/// the download of DASH media content (preferences concerning bitrate/quality, specifying an HTTP
/// proxy, etc.).
///
/// Example
/// ```rust
/// use dash_mpd::fetch::DashDownloader;
///
/// let url = "https://storage.googleapis.com/shaka-demo-assets/heliocentrism/heliocentrism.mpd";
/// match DashDownloader::new(url)
///        .worst_quality()
///        .download()
///        .await
/// {
///    Ok(path) => println!("Downloaded to {path:?}"),
///    Err(e) => eprintln!("Download failed: {e}"),
/// }
/// ```
impl DashDownloader {
    /// Create a `DashDownloader` for the specified DASH manifest URL `mpd_url`.
    pub fn new(mpd_url: &str) -> DashDownloader {
        DashDownloader {
            mpd_url: String::from(mpd_url),
            output_path: None,
            http_client: None,
            quality_preference: QualityPreference::Lowest,
            language_preference: None,
            fetch_video: true,
            fetch_audio: true,
            fetch_subtitles: false,
            keep_video: None,
            keep_audio: None,
            concatenate_periods: true,
            fragment_path: None,
            decryption_keys: HashMap::new(),
            content_type_checks: true,
            max_error_count: 10,
            progress_observers: vec![],
            sleep_between_requests: 0,
            rate_limit: 0,
            verbosity: 0,
            record_metainformation: true,
            ffmpeg_location: if cfg!(target_os = "windows") {
                String::from("ffmpeg.exe")
            } else {
                String::from("ffmpeg")
            },
	    vlc_location: if cfg!(target_os = "windows") {
                String::from("vlc.exe")
            } else {
                String::from("vlc")
            },
	    mkvmerge_location: if cfg!(target_os = "windows") {
                String::from("mkvmerge.exe")
            } else {
                String::from("mkvmerge")
            },
	    mp4box_location: if cfg!(target_os = "windows") {
                String::from("MP4Box.exe")
            } else if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
                String::from("MP4Box")
            } else {
                String::from("mp4box")
            },
            mp4decrypt_location: if cfg!(target_os = "windows") {
                String::from("mp4decrypt.exe")
            } else {
                String::from("mp4decrypt")
            },
        }
    }

    /// Specify the reqwest Client to be used for HTTP requests that download the DASH streaming
    /// media content. Allows you to specify a proxy, the user agent, custom request headers,
    /// request timeouts, additional root certificates to trust, client identity certificates, etc.
    ///
    /// Example
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
    //       .await
    /// ```
    pub fn with_http_client(mut self, client: HttpClient) -> DashDownloader {
        self.http_client = Some(client);
        self
    }

    /// Add a observer implementing the ProgressObserver trait, that will receive updates concerning
    /// the progression of the download (allows implementation of a progress bar, for example).
    pub fn add_progress_observer(mut self, observer: Arc<dyn ProgressObserver>) -> DashDownloader {
        self.progress_observers.push(observer);
        self
    }

    /// If the DASH manifest specifies several Adaptations with different bitrates (levels of
    /// quality), prefer the Adaptation with the highest bitrate (largest output file).
    pub fn best_quality(mut self) -> DashDownloader {
        self.quality_preference = QualityPreference::Highest;
        self
    }

    /// If the DASH manifest specifies several Adaptations with different bitrates (levels of
    /// quality), prefer the Adaptation with the lowest bitrate (smallest output file).
    pub fn worst_quality(mut self) -> DashDownloader {
        self.quality_preference = QualityPreference::Lowest;
        self
    }

    /// Preferred language when multiple audio streams with different languages are available. Must
    /// be in RFC 5646 format (e.g. "fr" or "en-AU"). If a preference is not specified and multiple
    /// audio streams are present, the first one listed in the DASH manifest will be downloaded.
    pub fn prefer_language(mut self, lang: String) -> DashDownloader {
        self.language_preference = Some(lang);
        self
    }

    /// If the media stream has separate audio and video streams, only download the video stream.
    pub fn video_only(mut self) -> DashDownloader {
        self.fetch_audio = false;
        self.fetch_video = true;
        self
    }

    /// If the media stream has separate audio and video streams, only download the audio stream.
    pub fn audio_only(mut self) -> DashDownloader {
        self.fetch_audio = true;
        self.fetch_video = false;
        self
    }

    /// Keep the file containing video at the specified path. If the path already exists, file
    /// contents will be overwritten.
    pub fn keep_video_as<P: Into<PathBuf>>(mut self, video_path: P) -> DashDownloader {
        self.keep_video = Some(video_path.into());
        self
    }

    /// Keep the file containing audio at the specified path. If the path already exists, file
    /// contents will be overwritten.
    pub fn keep_audio_as<P: Into<PathBuf>>(mut self, audio_path: P) -> DashDownloader {
        self.keep_audio = Some(audio_path.into());
        self
    }

    /// Save media fragments to the directory `fragment_path`. The directory will be created if it
    /// does not exist.
    pub fn save_fragments_to<P: Into<PathBuf>>(mut self, fragment_path: P) -> DashDownloader {
        self.fragment_path = Some(fragment_path.into());
        self
    }

    /// Add a key to be used to decrypt MPEG media streams that use Common Encryption (cenc). This
    /// function may be called several times to specify multiple kid/key pairs. Decryption uses the
    /// Bento4 commandline application mp4decrypt, run as a subprocess.
    ///
    /// # Arguments
    ///
    /// * `id` - a track ID in decimal or, a 128-bit KID in hexadecimal format (32 hex characters).
    ///    Examples: "1" or "eb676abbcb345e96bbcf616630f1a3da".
    ///
    /// * `key` - a 128-bit key in hexadecimal format.
    pub fn add_decryption_key(mut self, id: String, key: String) -> DashDownloader {
        self.decryption_keys.insert(id, key);
        self
    }

    /// Parameter `value` determines whether audio content is downloaded.
    pub fn fetch_audio(mut self, value: bool) -> DashDownloader {
        self.fetch_audio = value;
        self
    }

    /// Parameter `value` determines whether video content is downloaded.
    pub fn fetch_video(mut self, value: bool) -> DashDownloader {
        self.fetch_video = value;
        self
    }

    /// Fetch subtitles if they are available. If subtitles are available, download them to a file
    /// named with the same name as the media output and an appropriate extension (".vtt", ".ttml",
    /// ".srt", etc.).
    pub fn fetch_subtitles(mut self, value: bool) -> DashDownloader {
        self.fetch_subtitles = value;
        self
    }

    /// For multi-Period manifests, parameter `value` determines whether the content of multiple
    /// Periods is concatenated into a single output file where their resolutions, frame rate and
    /// aspect ratios are compatible, or kept in individual files.
    pub fn concatenate_periods(mut self, value: bool) -> DashDownloader {
        self.concatenate_periods = value;
        self
    }

    /// Don't check that the content-type of downloaded segments corresponds to audio or video
    /// content (may be necessary with poorly configured HTTP servers).
    pub fn without_content_type_checks(mut self) -> DashDownloader {
        self.content_type_checks = false;
        self
    }

    /// The upper limit on the number of non-transient network errors encountered for this download
    /// before we abort the download. Transient network errors such as an HTTP 408 "request timeout"
    /// are retried automatically with an exponential backoff mechanism, and do not count towards
    /// this upper limit. The default is to fail after 10 non-transient network errors.
    pub fn max_error_count(mut self, count: u32) -> DashDownloader {
        self.max_error_count = count;
        self
    }

    /// Specify a number of seconds to sleep between network requests (default 0).
    pub fn sleep_between_requests(mut self, seconds: u8) -> DashDownloader {
        self.sleep_between_requests = seconds;
        self
    }

    /// A maximal limit on the network bandwidth consumed to download media segments, expressed in
    /// octets (bytes) per second. No limit on bandwidth if set to zero (the default value).
    /// Limiting bandwidth below 50kB/s is not recommended, as the downloader may fail to respect
    /// this limit.
    pub fn with_rate_limit(mut self, bps: u64) -> DashDownloader {
        if bps < 10 * 1024 {
            warn!("Limiting bandwidth below 10kB/s is unlikely to be stable");
        }
        if self.verbosity > 1 {
            info!("Limiting bandwidth to {}kB/s", bps/1024);
        }
        self.rate_limit = bps;
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
    pub fn verbosity(mut self, level: u8) -> DashDownloader {
        self.verbosity = level;
        self
    }

    /// If `record` is true, record metainformation concerning the media content (origin URL, title,
    /// source and copyright metainformation) if present in the manifest as extended attributes in the
    /// output file.
    pub fn record_metainformation(mut self, record: bool) -> DashDownloader {
        self.record_metainformation = record;
        self
    }

    /// Specify the location of the `ffmpeg` application, if not located in PATH.
    ///
    /// Example
    /// ```rust
    /// #[cfg(target_os = "unix")]
    /// let ddl = ddl.with_ffmpeg("/opt/ffmpeg-next/bin/ffmpeg");
    /// ```
    pub fn with_ffmpeg(mut self, ffmpeg_path: &str) -> DashDownloader {
        self.ffmpeg_location = ffmpeg_path.to_string();
        self
    }

    /// Specify the location of the VLC application, if not located in PATH.
    ///
    /// # Example
    /// ```rust
    /// #[cfg(target_os = "windows")]
    /// let ddl = ddl.with_vlc("C:/Program Files/VideoLAN/VLC/vlc.exe");
    /// ```
    pub fn with_vlc(mut self, vlc_path: &str) -> DashDownloader {
        self.vlc_location = vlc_path.to_string();
        self
    }

    /// Specify the location of the mkvmerge application, if not located in PATH.
    ///
    /// # Argument
    ///
    /// `path` - the full path to the mkvmerge application, including the application name.
    pub fn with_mkvmerge(mut self, path: &str) -> DashDownloader {
        self.mkvmerge_location = path.to_string();
        self
    }

    /// Specify the location of the MP4Box application, if not located in PATH.
    ///
    /// # Argument
    ///
    /// `path` - the full path to the MP4Box application, including the application name.
    pub fn with_mp4box(mut self, path: &str) -> DashDownloader {
        self.mp4box_location = path.to_string();
        self
    }

    /// Specify the location of the Bento4 mp4decrypt application, if not located in PATH.
    ///
    /// # Argument
    ///
    /// `path` - the full path to the mp4decrypt application, including the application name.
    pub fn with_mp4decrypt(mut self, path: &str) -> DashDownloader {
        self.mp4decrypt_location = path.to_string();
        self
    }

    /// Download DASH streaming media content to the file named by `out`. If the output file `out`
    /// already exists, its content will be overwritten.
    ///
    /// Note that the media container format used when muxing audio and video streams depends on
    /// the filename extension of the path `out`. If the filename extension is `.mp4`, an MPEG-4
    /// container will be used; if it is `.mkv` a Matroska container will be used, and otherwise
    /// the heuristics implemented by ffmpeg will apply (e.g. an `.avi` extension will generate
    /// an AVI container).
    pub async fn download_to<P: Into<PathBuf>>(mut self, out: P) -> Result<PathBuf, DashMpdError> {
        self.output_path = Some(out.into());
        if self.http_client.is_none() {
            let client = reqwest::Client::builder()
                .timeout(Duration::new(30, 0))
                .build()
                .map_err(|_| DashMpdError::Network(String::from("building HTTP client")))?;
            self.http_client = Some(client);
        }
        fetch_mpd(&self).await
    }

    /// Download DASH streaming media content to a file in the current working directory and return
    /// the corresponding `PathBuf`. The name of the output file is derived from the manifest URL. The
    /// output file will be overwritten if it already exists.
    ///
    /// The downloaded media will be placed in an MPEG-4 container. To select another media container,
    /// see the `download_to` function.
    pub async fn download(mut self) -> Result<PathBuf, DashMpdError> {
        let cwd = env::current_dir()
            .map_err(|e| DashMpdError::Io(e, String::from("obtaining current directory")))?;
        let filename = generate_filename_from_url(&self.mpd_url);
        let outpath = cwd.join(filename);
        self.output_path = Some(outpath);
        if self.http_client.is_none() {
            let client = reqwest::Client::builder()
                .timeout(Duration::new(30, 0))
                .build()
                .map_err(|_| DashMpdError::Network(String::from("building HTTP client")))?;
            self.http_client = Some(client);
        }
        fetch_mpd(&self).await
    }
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
    // We currently default to an MP4 container (could default to Matroska which is more flexible,
    // but perhaps less commonly supported).
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
        tmp_file_path(&p).unwrap_or_else(|_| p.into())
    }
}

fn is_absolute_url(s: &str) -> bool {
    s.starts_with("http://") ||
        s.starts_with("https://") ||
        s.starts_with("file://") ||
        s.starts_with("ftp://")
}

// From the DASH-IF-IOP-v4.0 specification, "If the value of the @xlink:href attribute is
// urn:mpeg:dash:resolve-to-zero:2013, HTTP GET request is not issued, and the in-MPD element shall
// be removed from the MPD."
fn fetchable_xlink_href(href: &str) -> bool {
    (!href.is_empty()) && href.ne("urn:mpeg:dash:resolve-to-zero:2013")
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
    println!("  subs {stype:>18} | {lang:>10} |");
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
    println!("  {typ} {codec:17} | {:5} Kbps | {fmt:>9}", bw / 1024);
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

fn print_available_streams(mpd: &MPD) {
    let mut counter = 0;
    for p in &mpd.periods {
        let mut period_duration_secs: f64 = 0.0;
        if let Some(d) = mpd.mediaPresentationDuration {
            period_duration_secs = d.as_secs_f64();
        }
        if let Some(d) = &p.duration {
            period_duration_secs = d.as_secs_f64();
        }
        counter += 1;
        if let Some(id) = p.id.as_ref() {
            println!("Streams in period {id} (#{counter}), duration {period_duration_secs:.3}s:");
        } else {
            println!("Streams in period #{counter}, duration {period_duration_secs:.3}s:");
        }
        print_available_streams_period(p);
    }
}

async fn extract_init_pssh(client: &HttpClient, init_url: Url) -> Option<Vec<u8>> {
    if let Ok(resp) = client.get(init_url).send().await {
        if let Ok(bytes) = resp.bytes().await {
            let needle = b"pssh";
            if let Some(offset) = bytes.windows(needle.len()).position(|window| window == needle) {
                let start = offset - 4;
                let end = start + bytes[offset-1] as usize;
                let pssh = bytes.slice(start..end);
                return Some(pssh.to_vec());
            }
        }
    }
    None
}


// From https://dashif.org/docs/DASH-IF-IOP-v4.3.pdf:
// "For the avoidance of doubt, only %0[width]d is permitted and no other identifiers. The reason
// is that such a string replacement can be easily implemented without requiring a specific library."
//
// Instead of pulling in C printf() or a reimplementation such as the printf_compat crate, we reimplement
// this functionality directly.
//
// Example template: "$RepresentationID$/$Number%06d$.m4s"
fn resolve_url_template(template: &str, params: &HashMap<&str, String>) -> String {
    let mut result = template.to_string();
    for k in ["RepresentationID", "Number", "Time", "Bandwidth"] {
        // first check for simple case eg $Number$
        let ident = format!("${k}$");
        if result.contains(&ident) {
            if let Some(value) = params.get(k as &str) {
                result = result.replace(&ident, value);
            }
        }
        // now check for complex case eg $Number%06d$
        let re = format!("\\${k}%0([\\d])d\\$");
        let ident_re = Regex::new(&re).unwrap();
        if let Some(cap) = ident_re.captures(&result) {
            if let Some(value) = params.get(k as &str) {
                let width: usize = cap[1].parse::<usize>().unwrap();
                let count = format!("{value:0>width$}");
                let m = ident_re.find(&result).unwrap();
                result = result[..m.start()].to_owned() + &count + &result[m.end()..];
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

fn categorize_reqwest_error(e: reqwest::Error) -> backoff::Error<reqwest::Error> {
    if reqwest_error_transient_p(&e) {
        backoff::Error::retry_after(e, Duration::new(5, 0))
    } else {
        backoff::Error::permanent(e)
    }
}

fn notify_transient<E: std::fmt::Debug>(err: E, dur: Duration) {
    warn!("Transient error after {dur:?}: {err:?}");
}

fn network_error(why: &str, e: impl std::error::Error) -> DashMpdError {
    DashMpdError::Network(format!("{why}: {e}"))
}

fn parse_error(why: &str, e: impl std::error::Error) -> DashMpdError {
    DashMpdError::Parsing(format!("{why}: {e:#?}"))
}


// As per https://www.freedesktop.org/wiki/CommonExtendedAttributes/, set extended filesystem
// attributes indicating metadata such as the origin URL, title, source and copyright, if
// specified in the MPD manifest. This functionality is only active on platforms where the xattr
// crate supports extended attributes (currently Android, Linux, MacOS, FreeBSD, and NetBSD); on
// unsupported Unix platforms it's a no-op. On other non-Unix platforms the crate doesn't build.
//
// TODO: on Windows, could use NTFS Alternate Data Streams
// https://en.wikipedia.org/wiki/NTFS#Alternate_data_stream_(ADS)
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
            if let Some(pi) = &mpd.ProgramInformation {
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


// Walk the XML tree recursively to resolve any XLink references in any nodes.
#[async_recursion]
async fn resolve_xlink_references_element(
    downloader: &DashDownloader,
    redirected_url: &Url,
    element: &mut xmltree::Element) -> Result<(), DashMpdError>
{
    if let Some(href) = element.attributes.get_mut("href") {
        if fetchable_xlink_href(href) {
            let xlink_url = if is_absolute_url(href) {
                Url::parse(href)
                    .map_err(|e| parse_error(&format!("parsing XLink on {}", element.name), e))?
            } else {
                // Note that we are joining against the original/redirected URL for the MPD, and
                // not against the currently scoped BaseURL
                redirected_url.join(href)
                    .map_err(|e| parse_error(&format!("parsing XLink on {}", element.name), e))?
            };
            let client = downloader.http_client.as_ref().unwrap();
            let xml = client.get(xlink_url.clone())
                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                .header("Accept-Language", "en-US,en")
                .header("Sec-Fetch-Mode", "navigate")
                .send().await
                .map_err(|e| network_error(&format!("fetching XLink for {}", element.name), e))?
                .error_for_status()
                .map_err(|e| network_error(&format!("fetching XLink for {}", element.name), e))?
                .text().await
                .map_err(|e| network_error(&format!("resolving XLink on {}", element.name), e))?;
            if downloader.verbosity > 2 {
                println!("  Resolved onLoad XLink {xlink_url} on {} -> {} octets",
                         element.name, xml.len());
            }
            let ndoc = xmltree::Element::parse(xml.as_bytes())
                .map_err(|e| parse_error("xmltree parsing", e))?;
            // now replace
            *element = ndoc;
        }
    }
    for child in element.children.iter_mut() {
        if let Some(ce) = child.as_mut_element() {
            resolve_xlink_references_element(downloader, redirected_url, ce).await?;
        }
    }
    Ok(())
}

async fn resolve_xlink_references(
    downloader: &DashDownloader,
    redirected_url: &Url,
    xml: &[u8]) -> Result<MPD, DashMpdError>
{
    let mut doc = xmltree::Element::parse(xml)
        .map_err(|e| parse_error("xmltree parsing", e))?;
    resolve_xlink_references_element(downloader, redirected_url, &mut doc).await?;
    let mut buf = Vec::new();
    doc.write(&mut buf)
        .map_err(|e| parse_error("serializing rewritten manifest", e))?;
    let rewritten = std::str::from_utf8(&buf)
        .map_err(|e| parse_error("rewritten manifest as UTF-8", e))?;
    // Here using the quick-xml serde support to deserialize into Rust structs.
    parse(rewritten)
}


async fn do_period_audio(
    downloader: &DashDownloader,
    mpd: &MPD,
    period: &Period,
    period_counter: u8,
    base_url: Url
    ) -> Result<PeriodOutputs, DashMpdError>
{
    let mut fragments = Vec::new();
    let mut diagnostics = String::new();
    // The period_duration is specified either by the <Period> duration attribute, or by the
    // mediaPresentationDuration of the top-level MPD node.
    let mut period_duration_secs: f64 = 0.0;
    if let Some(d) = mpd.mediaPresentationDuration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(d) = period.duration {
        period_duration_secs = d.as_secs_f64();
    }
    // Handle the AdaptationSet with audio content. Note that some streams don't separate out
    // audio and video streams, so this might be None.
    let maybe_audio_adaptation = if let Some(ref lang) = downloader.language_preference {
        period.adaptations.iter().filter(is_audio_adaptation)
            .min_by_key(|a| adaptation_lang_distance(a, lang))
    } else {
        // returns the first audio adaptation found
        period.adaptations.iter().find(is_audio_adaptation)
    };
    if let Some(audio_adaptation) = maybe_audio_adaptation {
        let audio = audio_adaptation.clone();
        // The AdaptationSet may have a BaseURL (eg the test BBC streams). We use a local variable
        // to make sure we don't "corrupt" the base_url for the video segments.
        let mut base_url = base_url.clone();
        if !audio.BaseURL.is_empty() {
            let bu = &audio.BaseURL[0];
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)
                    .map_err(|e| parse_error("parsing AdaptationSet BaseURL", e))?;
            } else {
                base_url = base_url.join(&bu.base)
                    .map_err(|e| parse_error("joining with AdaptationSet BaseURL", e))?;
            }
        }
        // We rank according to the @qualityRanking attribute if it is present (quality
        // ranking may be different from bandwidth ranking when different codecs are used).
        let maybe_audio_repr = if audio.representations.iter()
            .all(|x| x.qualityRanking.is_some())
        {
            // rank according to the @qualityRanking attribute (lower values represent
            // higher quality content)
            if downloader.quality_preference == QualityPreference::Lowest {
                audio.representations.iter()
                    .max_by_key(|x| x.qualityRanking.unwrap())
            } else {
                audio.representations.iter()
                    .min_by_key(|x| x.qualityRanking.unwrap())
            }
        } else {
            // rank according to the bandwidth attribute
            if downloader.quality_preference == QualityPreference::Lowest {
                audio.representations.iter()
                    .min_by_key(|x| x.bandwidth.unwrap_or(1_000_000_000))
            } else {
                audio.representations.iter()
                    .max_by_key(|x| x.bandwidth.unwrap_or(0))
            }
        };
        if let Some(audio_repr) = maybe_audio_repr {
            if downloader.verbosity > 0 {
                let bw = if let Some(bw) = audio_repr.bandwidth {
                    format!("bw={} Kbps ", bw / 1024)
                } else {
                    String::from("")
                };
                let unknown = String::from("?");
                let lang = audio_repr.lang.as_ref()
                    .unwrap_or(audio.lang.as_ref()
                               .unwrap_or(&unknown));
                let codec = audio_repr.codecs.as_ref()
                    .unwrap_or(audio.codecs.as_ref()
                               .unwrap_or(&unknown));
                diagnostics += &format!("  Audio stream selected: {bw}lang={lang} codec={codec}\n");
                // Check for ContentProtection on the selected Representation/Adaptation
                for cp in audio_repr.ContentProtection.iter()
                    .chain(audio.ContentProtection.iter())
                {
                    diagnostics += &format!("  ContentProtection: {}\n", content_protection_type(cp));
                    if let Some(kid) = &cp.default_KID {
                        diagnostics += &format!("    KID: {}\n", kid.replace('-', ""));
                    }
                    for pssh in cp.cenc_pssh.iter() {
                        if let Some(pc) = &pssh.content {
                            diagnostics += &format!("    PSSH (from manifest): {pc}\n");
                        }
                    }
                }
            }
            // the Representation may have a BaseURL
            let mut base_url = base_url;
            if !audio_repr.BaseURL.is_empty() {
                let bu = &audio_repr.BaseURL[0];
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base)
                        .map_err(|e| parse_error("parsing Representation BaseURL", e))?;
                } else {
                    base_url = base_url.join(&bu.base)
                        .map_err(|e| parse_error("joining with Representation BaseURL", e))?;
                }
            }
            let mut opt_init: Option<String> = None;
            let mut opt_media: Option<String> = None;
            let mut opt_duration: Option<f64> = None;
            let mut timescale = 1;
            let mut start_number = 1;
            // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
            // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
            // nodes in an enclosed Representation node. Don't download media segments here, only
            // download for SegmentTemplate nodes that are children of a Representation node.
            if let Some(st) = &audio.SegmentTemplate {
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
            }
            let rid = match &audio_repr.id {
                Some(id) => id,
                None => return Err(
                    DashMpdError::UnhandledMediaStream(
                        "Missing @id on Representation node".to_string())),
            };
            let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
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
                    println!("  Using AdaptationSet>SegmentList addressing mode for audio representation");
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
                        let init_url = if is_absolute_url(&path) {
                            Url::parse(&path)
                                .map_err(|e| parse_error("parsing sourceURL", e))?
                        } else {
                            base_url.join(&path)
                                .map_err(|e| parse_error("joining with sourceURL", e))?
                        };
                        let mf = MediaFragment{
                            period: period_counter,
                            url: init_url,
                            start_byte, end_byte,
                            is_init: true
                        };
                        fragments.push(mf);
                    } else {
                        let mf = MediaFragment{
                            period: period_counter,
                            url: base_url.clone(),
                            start_byte, end_byte,
                            is_init: true
                        };
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
                        let u = base_url.join(m)
                            .map_err(|e| parse_error("joining media with baseURL", e))?;
                        let mf = make_fragment(period_counter, u, start_byte, end_byte);
                        fragments.push(mf);
                    } else if !audio_adaptation.BaseURL.is_empty() {
                        let bu = &audio_adaptation.BaseURL[0];
                        let base_url = if is_absolute_url(&bu.base) {
                            Url::parse(&bu.base)
                                .map_err(|e| parse_error("parsing Representation BaseURL", e))?
                        } else {
                            base_url.join(&bu.base)
                                .map_err(|e| parse_error("joining with Representation BaseURL", e))?
                        };
                        let mf = make_fragment(period_counter, base_url.clone(), start_byte, end_byte);
                        fragments.push(mf);
                    }
                }
            }
            if let Some(sl) = &audio_repr.SegmentList {
                // (1) Representation>SegmentList addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using Representation>SegmentList addressing mode for audio representation");
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
                        let init_url = if is_absolute_url(&path) {
                            Url::parse(&path)
                                .map_err(|e| parse_error("parsing sourceURL", e))?
                        } else {
                            base_url.join(&path)
                                .map_err(|e| parse_error("joining with sourceURL", e))?
                        };
                        let mf = MediaFragment{
                            period: period_counter,
                            url: init_url,
                            start_byte, end_byte,
                            is_init: true,
                        };
                        fragments.push(mf);
                    } else {
                        let mf = MediaFragment{
                            period: period_counter,
                            url: base_url.clone(),
                            start_byte, end_byte,
                            is_init: true,
                        };
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
                        let u = base_url.join(m)
                            .map_err(|e| parse_error("joining media with baseURL", e))?;
                        let mf = make_fragment(period_counter, u, start_byte, end_byte);
                        fragments.push(mf);
                    } else if !audio_repr.BaseURL.is_empty() {
                        let bu = &audio_repr.BaseURL[0];
                        let base_url = if is_absolute_url(&bu.base) {
                            Url::parse(&bu.base)
                                .map_err(|e| parse_error("parsing Representation BaseURL", e))?
                        } else {
                            base_url.join(&bu.base)
                                .map_err(|e| parse_error("joining with Representation BaseURL", e))?
                        };
                        let mf = make_fragment(period_counter, base_url.clone(), start_byte, end_byte);
                        fragments.push(mf);
                    }
                }
            } else if audio_repr.SegmentTemplate.is_some() || audio.SegmentTemplate.is_some() {
                // Here we are either looking at a Representation.SegmentTemplate, or a
                // higher-level AdaptationSet.SegmentTemplate
                let st;
                if let Some(it) = &audio_repr.SegmentTemplate {
                    st = it;
                } else if let Some(it) = &audio.SegmentTemplate {
                    st = it;
                } else {
                    panic!("unreachable");
                }
                if let Some(i) = &st.initialization {
                    opt_init = Some(i.to_string());
                }
                if let Some(m) = &st.media {
                    opt_media = Some(m.to_string());
                }
                if let Some(ts) = st.timescale {
                    timescale = ts;
                }
                if let Some(sn) = st.startNumber {
                    start_number = sn;
                }
                if let Some(stl) = &st.SegmentTimeline {
                    // (2) SegmentTemplate with SegmentTimeline addressing mode (also called
                    // "explicit addressing" in certain DASH-IF documents)
                    if downloader.verbosity > 1 {
                        println!("  Using SegmentTemplate+SegmentTimeline addressing mode for audio representation");
                    }
                    if let Some(init) = opt_init {
                        let path = resolve_url_template(&init, &dict);
                        let u = base_url.join(&path)
                            .map_err(|e| parse_error("joining init with BaseURL", e))?;
                        let mf = MediaFragment{
                            period: period_counter,
                            url: u,
                            start_byte: None,
                            end_byte: None,
                            is_init: true
                        };
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
                            let u = base_url.join(&path)
                                .map_err(|e| parse_error("joining media with BaseURL", e))?;
                            let mf = make_fragment(period_counter, u, None, None);
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
                                    } else if segment_time as f64 > end_time {
                                        break;
                                    }
                                    segment_time += segment_duration;
                                    let dict = HashMap::from([("Time", segment_time.to_string()),
                                                              ("Number", number.to_string())]);
                                    let path = resolve_url_template(&audio_path, &dict);
                                    let u = base_url.join(&path)
                                        .map_err(|e| parse_error("joining media with BaseURL", e))?;
                                    let mf = make_fragment(period_counter, u, None, None);
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
                    // (3) SegmentTemplate@duration addressing mode or (4)
                    // SegmentTemplate@index addressing mode (also called "simple
                    // addressing" in certain DASH-IF documents)
                    if downloader.verbosity > 1 {
                        println!("  Using SegmentTemplate addressing mode for audio representation");
                    }
                    let mut total_number = 0i64;
                    if let Some(init) = opt_init {
                        // The initialization segment counts as one of the $Number$
                        total_number -= 1;
                        let path = resolve_url_template(&init, &dict);
                        let u = base_url.join(&path)
                            .map_err(|e| parse_error("joining init with BaseURL", e))?;
                        let mf = MediaFragment{
                            period: period_counter,
                            url: u,
                            start_byte: None,
                            end_byte: None,
                            is_init: true
                        };
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
                            segment_duration = std / timescale as f64;
                        }
                        if segment_duration < 0.0 {
                            return Err(DashMpdError::UnhandledMediaStream(
                                "Audio representation is missing SegmentTemplate@duration attribute".to_string()));
                        }
                        total_number += (period_duration_secs / segment_duration).ceil() as i64;
                        let mut number = start_number;
                        for _ in 1..=total_number {
                            let dict = HashMap::from([("Number", number.to_string())]);
                            let path = resolve_url_template(&audio_path, &dict);
                            let u = base_url.join(&path)
                                .map_err(|e| parse_error("joining media with BaseURL", e))?;
                            let mf = make_fragment(period_counter, u, None, None);
                            fragments.push(mf);
                            number += 1;
                        }
                    }
                }
            } else if let Some(sb) = &audio_repr.SegmentBase {
                // (5) SegmentBase@indexRange addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using SegmentBase@indexRange addressing mode for audio representation");
                }
                // The SegmentBase@indexRange attribute points to a byte range in the media
                // file that contains index information (an sidx box for MPEG files, or a
                // Cues entry for a DASH-WebM stream). To be fully compliant, we should
                // download and parse these (for example using the sidx crate) then download
                // the referenced content segments. In practice, it seems that the
                // indexRange information is mostly provided by DASH encoders to allow
                // clients to rewind and fast-foward a stream, and is not necessary if we
                // download the full content specified by BaseURL.
                //
                // Our strategy: if there is a SegmentBase > Initialization > SourceURL
                // node, download that first, respecting the byte range if it is specified.
                // Otherwise, download the full content specified by the BaseURL for this
                // segment (ignoring any indexRange attributes).
                let mut start_byte: Option<u64> = None;
                let mut end_byte: Option<u64> = None;
                if let Some(init) = &sb.initialization {
                    if let Some(range) = &init.range {
                        let (s, e) = parse_range(range)?;
                        start_byte = Some(s);
                        end_byte = Some(e);
                    }
                    if let Some(su) = &init.sourceURL {
                        let path = resolve_url_template(su, &dict);
                        let u = if is_absolute_url(&path) {
                            Url::parse(&path)
                                .map_err(|e| parse_error("parsing sourceURL", e))?
                        } else {
                            base_url.join(&path)
                                .map_err(|e| parse_error("joining with sourceURL", e))?
                        };
                        let mf = MediaFragment {
                            period: period_counter,
                            url: u,
                            start_byte, end_byte,
                            is_init: true,
                        };
                        fragments.push(mf);
                    }
                }
                let mf = MediaFragment {
                    period: period_counter,
                    url: base_url.clone(),
                    start_byte: None,
                    end_byte: None,
                    is_init: true,
                };
                fragments.push(mf);
            } else if fragments.is_empty() && !audio_repr.BaseURL.is_empty() {
                // (6) plain BaseURL addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using BaseURL addressing mode for audio representation");
                }
                let u = if is_absolute_url(&audio_repr.BaseURL[0].base) {
                    Url::parse(&audio_repr.BaseURL[0].base)
                        .map_err(|e| parse_error("parsing BaseURL", e))?
                } else {
                    base_url.join(&audio_repr.BaseURL[0].base)
                        .map_err(|e| parse_error("joining Representation BaseURL", e))?
                };
                let mf = make_fragment(period_counter, u, None, None);
                fragments.push(mf);
            }
            if fragments.is_empty() {
                return Err(DashMpdError::UnhandledMediaStream(
                    "no usable addressing mode identified for audio representation".to_string()));
            }
        }
    }
    Ok(PeriodOutputs { fragments, diagnostics, subtitle_formats: Vec::new() })
}


async fn do_period_video(
    downloader: &DashDownloader,
    mpd: &MPD,
    period: &Period,
    period_counter: u8,
    base_url: Url
    ) -> Result<PeriodOutputs, DashMpdError> 
{
    let mut fragments = Vec::new();
    let mut diagnostics = String::new();
    let mut period_duration_secs: f64 = 0.0;
    if let Some(d) = mpd.mediaPresentationDuration {
        period_duration_secs = d.as_secs_f64();
    }
    if let Some(d) = period.duration {
        period_duration_secs = d.as_secs_f64();
    }
    // Handle the AdaptationSet which contains video content
    let maybe_video_adaptation = period.adaptations.iter().find(is_video_adaptation);
    if let Some(video_adaptation) = maybe_video_adaptation {
        let video = video_adaptation.clone();
        // The AdaptationSet may have a BaseURL. We use a local variable to make sure we
        // don't "corrupt" the base_url for the subtitle segments.
        let mut base_url = base_url.clone();
        if !video.BaseURL.is_empty() {
            let bu = &video.BaseURL[0];
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)
                    .map_err(|e| parse_error("parsing BaseURL", e))?;
            } else {
                base_url = base_url.join(&bu.base)
                    .map_err(|e| parse_error("joining base with BaseURL", e))?;
            }
        }
        // We rank according to the @qualityRanking attribute if it is present (quality
        // ranking may be different from bandwidth ranking when different codecs are used).
        let maybe_video_repr = if video.representations.iter()
            .all(|x| x.qualityRanking.is_some())
        {
            // rank according to the @qualityRanking attribute (lower values represent
            // higher quality content)
            if downloader.quality_preference == QualityPreference::Lowest {
                video.representations.iter()
                    .max_by_key(|x| x.qualityRanking.unwrap())
            } else {
                video.representations.iter()
                    .min_by_key(|x| x.qualityRanking.unwrap())
            }
        } else {
            // rank according to the bandwidth attribute
            if downloader.quality_preference == QualityPreference::Lowest {
                video.representations.iter()
                    .min_by_key(|x| x.bandwidth.unwrap_or(1_000_000_000))
            } else {
                video.representations.iter()
                    .max_by_key(|x| x.bandwidth.unwrap_or(0))
            }
        };
        if let Some(video_repr) = maybe_video_repr {
            if downloader.verbosity > 0 {
                let bw = if let Some(bw) = video_repr.bandwidth.or(video.maxBandwidth) {
                    format!("bw={} Kbps ", bw / 1024)
                } else {
                    String::from("")
                };
                let unknown = String::from("?");
                let w = video_repr.width.unwrap_or(video.width.unwrap_or(0));
                let h = video_repr.height.unwrap_or(video.height.unwrap_or(0));
                let fmt = if w == 0 || h == 0 {
                    String::from("")
                } else {
                    format!("resolution={w}x{h} ")
                };
                let codec = video_repr.codecs.as_ref()
                    .unwrap_or(video.codecs.as_ref().unwrap_or(&unknown));
                diagnostics += &format!("  Video stream selected: {bw}{fmt}codec={codec}\n");
                // Check for ContentProtection on the selected Representation/Adaptation
                for cp in video_repr.ContentProtection.iter()
                    .chain(video.ContentProtection.iter())
                {
                    diagnostics += &format!("  ContentProtection: {}\n", content_protection_type(cp));
                    if let Some(kid) = &cp.default_KID {
                        diagnostics += &format!("    KID: {}\n", kid.replace('-', ""));
                    }
                    for pssh in cp.cenc_pssh.iter() {
                        if let Some(pc) = &pssh.content {
                            diagnostics += &format!("    PSSH (from manifest): {pc}\n");
                        }
                    }
                }
            }
            if !video_repr.BaseURL.is_empty() {
                let bu = &video_repr.BaseURL[0];
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base)
                        .map_err(|e| parse_error("parsing BaseURL", e))?;
                } else {
                    base_url = base_url.join(&bu.base)
                        .map_err(|e| parse_error("joining base with BaseURL", e))?;
                }
            }
            let rid = match &video_repr.id {
                Some(id) => id,
                None => return Err(DashMpdError::UnhandledMediaStream(
                    "Missing @id on Representation node".to_string())),
            };
            let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
            if let Some(b) = &video_repr.bandwidth {
                dict.insert("Bandwidth", b.to_string());
            }
            let mut opt_init: Option<String> = None;
            let mut opt_media: Option<String> = None;
            let mut opt_duration: Option<f64> = None;
            let mut timescale = 1;
            let mut start_number = 1;
            // SegmentTemplate as a direct child of an Adaptation node. This can specify some common
            // attribute values (media, timescale, duration, startNumber) for child SegmentTemplate
            // nodes in an enclosed Representation node. Don't download media segments here, only
            // download for SegmentTemplate nodes that are children of a Representation node.
            if let Some(st) = &video.SegmentTemplate {
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
            }
            // Now the 6 possible addressing modes: (1) SegmentList,
            // (2) SegmentTemplate+SegmentTimeline, (3) SegmentTemplate@duration,
            // (4) SegmentTemplate@index, (5) SegmentBase@indexRange, (6) plain BaseURL
            if let Some(sl) = &video_adaptation.SegmentList {
                // (1) AdaptationSet>SegmentList addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using AdaptationSet>SegmentList addressing mode for video representation");
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
                        let u = if is_absolute_url(&path) {
                            Url::parse(&path)
                                .map_err(|e| parse_error("parsing sourceURL", e))?
                        } else {
                            base_url.join(&path)
                                .map_err(|e| parse_error("joining sourceURL with BaseURL", e))?
                        };
                        let mf = MediaFragment {
                            period: period_counter,
                            url: u,
                            start_byte, end_byte,
                            is_init: true,
                        };
                        fragments.push(mf);
                    }
                } else {
                    let mf = MediaFragment {
                        period: period_counter,
                        url: base_url.clone(),
                        start_byte, end_byte,
                        is_init: true
                    };
                    fragments.push(mf);
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
                        let u = base_url.join(m)
                            .map_err(|e| parse_error("joining media with BaseURL", e))?;
                        let mf = make_fragment(period_counter, u, start_byte, end_byte);
                        fragments.push(mf);
                    } else if !video_adaptation.BaseURL.is_empty() {
                        let bu = &video_adaptation.BaseURL[0];
                        let base_url = if is_absolute_url(&bu.base) {
                            Url::parse(&bu.base)
                                .map_err(|e| parse_error("parsing BaseURL", e))?
                        } else {
                            base_url.join(&bu.base)
                                .map_err(|e| parse_error("joining with BaseURL", e))?
                        };
                        let mf = make_fragment(period_counter, base_url.clone(), start_byte, end_byte);
                        fragments.push(mf);
                    }
                }
            }
            if let Some(sl) = &video_repr.SegmentList {
                // (1) Representation>SegmentList addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using Representation>SegmentList addressing mode for video representation");
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
                        let u = if is_absolute_url(&path) {
                            Url::parse(&path)
                                .map_err(|e| parse_error("parsing sourceURL", e))?
                        } else {
                            base_url.join(&path)
                                .map_err(|e| parse_error("joining sourceURL with BaseURL", e))?
                        };
                        let mf = MediaFragment {
                            period: period_counter,
                            url: u,
                            start_byte, end_byte,
                            is_init: true,
                        };
                        fragments.push(mf);
                    } else {
                        let mf = MediaFragment{
                            period: period_counter,
                            url: base_url.clone(),
                            start_byte, end_byte,
                            is_init: true
                        };
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
                        let u = base_url.join(m)
                            .map_err(|e| parse_error("joining media with BaseURL", e))?;
                        let mf = make_fragment(period_counter, u, start_byte, end_byte);
                        fragments.push(mf);
                    } else if !video_repr.BaseURL.is_empty() {
                        let bu = &video_repr.BaseURL[0];
                        let base_url = if is_absolute_url(&bu.base) {
                            Url::parse(&bu.base)
                                .map_err(|e| parse_error("parsing BaseURL", e))?
                        } else {
                            base_url.join(&bu.base)
                                .map_err(|e| parse_error("joining with BaseURL", e))?
                        };
                        let mf = make_fragment(period_counter, base_url.clone(), start_byte, end_byte);
                        fragments.push(mf);
                    }
                }
            } else if video_repr.SegmentTemplate.is_some() || video.SegmentTemplate.is_some() {
                // Here we are either looking at a Representation.SegmentTemplate, or a
                // higher-level AdaptationSet.SegmentTemplate
                let st;
                if let Some(it) = &video_repr.SegmentTemplate {
                    st = it;
                } else if let Some(it) = &video.SegmentTemplate {
                    st = it;
                } else {
                    panic!("impossible");
                }
                if let Some(i) = &st.initialization {
                    opt_init = Some(i.to_string());
                }
                if let Some(m) = &st.media {
                    opt_media = Some(m.to_string());
                }
                if let Some(ts) = st.timescale {
                    timescale = ts;
                }
                if let Some(sn) = st.startNumber {
                    start_number = sn;
                }
                if let Some(stl) = &st.SegmentTimeline {
                    // (2) SegmentTemplate with SegmentTimeline addressing mode
                    if downloader.verbosity > 1 {
                        println!("  Using SegmentTemplate+SegmentTimeline addressing mode for video representation");
                    }
                    if let Some(init) = opt_init {
                        let path = resolve_url_template(&init, &dict);
                        let u = base_url.join(&path)
                            .map_err(|e| parse_error("joining init with BaseURL", e))?;
                        let mf = MediaFragment{
                            period: period_counter,
                            url: u,
                            start_byte: None,
                            end_byte: None,
                            is_init: true
                        };
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
                            let u = base_url.join(&path)
                                .map_err(|e| parse_error("joining media with BaseURL", e))?;
                            let mf = make_fragment(period_counter, u, None, None);
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
                                    } else if segment_time as f64 > end_time {
                                        break;
                                    }
                                    segment_time += segment_duration;
                                    let dict = HashMap::from([("Time", segment_time.to_string()),
                                                              ("Number", number.to_string())]);
                                    let path = resolve_url_template(&video_path, &dict);
                                    let u = base_url.join(&path)
                                        .map_err(|e| parse_error("joining media with BaseURL", e))?;
                                    let mf = make_fragment(period_counter, u, None, None);
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
                        println!("  Using SegmentTemplate addressing mode for video representation");
                    }
                    let mut total_number = 0i64;
                    if let Some(init) = opt_init {
                        // The initialization segment counts as one of the $Number$
                        total_number -= 1;
                        let path = resolve_url_template(&init, &dict);
                        let u = base_url.join(&path)
                            .map_err(|e| parse_error("joining init with BaseURL", e))?;
                        let mf = MediaFragment{
                            period: period_counter,
                            url: u,
                            start_byte: None,
                            end_byte: None,
                            is_init: true
                        };
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
                            segment_duration = std / timescale as f64;
                        }
                        if segment_duration < 0.0 {
                            return Err(DashMpdError::UnhandledMediaStream(
                                "Video representation is missing SegmentTemplate@duration attribute".to_string()));
                        }
                        total_number += (period_duration_secs / segment_duration).ceil() as i64;
                        let mut number = start_number;
                        for _ in 1..=total_number {
                            let dict = HashMap::from([("Number", number.to_string())]);
                            let path = resolve_url_template(&video_path, &dict);
                            let u = base_url.join(&path)
                                .map_err(|e| parse_error("joining media with BaseURL", e))?;
                            let mf = make_fragment(period_counter, u, None, None);
                            fragments.push(mf);
                            number += 1;
                        }
                    }
                }
            } else if let Some(sb) = &video_repr.SegmentBase {
                // (5) SegmentBase@indexRange addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using SegmentBase@indexRange addressing mode for video representation");
                }
                let mut start_byte: Option<u64> = None;
                let mut end_byte: Option<u64> = None;
                if let Some(init) = &sb.initialization
                {
                    if let Some(range) = &init.range {
                        let (s, e) = parse_range(range)?;
                        start_byte = Some(s);
                        end_byte = Some(e);
                    }
                    if let Some(su) = &init.sourceURL {
                        let path = resolve_url_template(su, &dict);
                        let u = if is_absolute_url(&path) {
                            Url::parse(&path)
                                .map_err(|e| parse_error("parsing sourceURL", e))?
                        } else {
                            base_url.join(&path)
                                .map_err(|e| parse_error("joining with sourceURL", e))?
                        };
                        let mf = MediaFragment {
                            period: period_counter,
                            url: u,
                            start_byte, end_byte,
                            is_init: true
                        };
                        fragments.push(mf);
                    }
                }
                let mf = MediaFragment {
                    period: period_counter,
                    url: base_url.clone(),
                    start_byte: None,
                    end_byte: None,
                    is_init: true
                };
                fragments.push(mf);
            } else if fragments.is_empty() && !video_repr.BaseURL.is_empty() {
                // (6) BaseURL addressing mode
                if downloader.verbosity > 1 {
                    println!("  Using BaseURL addressing mode for video representation");
                }
                let u = if is_absolute_url(&video_repr.BaseURL[0].base) {
                    Url::parse(&video_repr.BaseURL[0].base)
                        .map_err(|e| parse_error("parsing BaseURL", e))?
                } else {
                    base_url.join(&video_repr.BaseURL[0].base)
                        .map_err(|e| parse_error("joining Representation BaseURL", e))?
                };
                let mf = make_fragment(period_counter, u, None, None);
                fragments.push(mf);
            }
            if fragments.is_empty() {
                return Err(DashMpdError::UnhandledMediaStream(
                    "no usable addressing mode identified for video representation".to_string()));
            }
        } else {
            // FIXME we aren't correctly handling manifests without a Representation node
            // eg https://raw.githubusercontent.com/zencoder/go-dash/master/mpd/fixtures/newperiod.mpd
            return Err(DashMpdError::UnhandledMediaStream(
                "Couldn't find lowest bandwidth video stream in DASH manifest".to_string()));
        }
    }
    Ok(PeriodOutputs { fragments, diagnostics, subtitle_formats: Vec::new() })
}

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
    if let Some(subtitle_adaptation) = maybe_subtitle_adaptation {
        let subtitle_format = subtitle_type(&subtitle_adaptation);
        subtitle_formats.push(subtitle_format);
        if downloader.verbosity > 1 {
            println!("  Retrieving subtitles in format {subtitle_format:?}");
        }
        // The AdaptationSet may have a BaseURL. We use a local variable to make sure we
        // don't "corrupt" the base_url for the subtitle segments.
        let mut base_url = base_url.clone();
        if !subtitle_adaptation.BaseURL.is_empty() {
            let bu = &subtitle_adaptation.BaseURL[0];
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)
                    .map_err(|e| parse_error("parsing BaseURL", e))?;
            } else {
                base_url = base_url.join(&bu.base)
                    .map_err(|e| parse_error("joining base with BaseURL", e))?;
            }
        }
        // We don't do any ranking on subtitle Representations, because there is probably only a single one.
        if let Some(rep) = subtitle_adaptation.representations.first() {
            if !rep.BaseURL.is_empty() {
                for st_bu in rep.BaseURL.iter() {
                    let st_url = if is_absolute_url(&st_bu.base) {
                        Url::parse(&st_bu.base)
                            .map_err(|e| parse_error("parsing subtitle BaseURL", e))?
                    } else {
                        base_url.join(&st_bu.base)
                            .map_err(|e| parse_error("joining subtitle BaseURL", e))?
                    };
                    let subs = client.get(st_url.clone())
                        .header("Referer", base_url.to_string())
                        .send().await
                        .map_err(|e| network_error("fetching subtitles", e))?
                        .error_for_status()
                        .map_err(|e| network_error("fetching subtitles", e))?
                        .bytes().await
                        .map_err(|e| network_error("retrieving subtitles", e))?;
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
                    let mut subs_file = File::create(subs_path.clone())
                        .map_err(|e| DashMpdError::Io(e, String::from("creating subtitle file")))?;
                    if downloader.verbosity > 2 {
                        println!("  Subtitle {st_url} -> {} octets", subs.len());
                    }
                    match subs_file.write_all(&subs) {
                        Ok(()) => {
                            if downloader.verbosity > 0 {
                                println!("  Downloaded subtitles ({subtitle_format:?}) to {}",
                                         subs_path.display());
                            }
                        },
                        Err(e) => {
                            error!("Unable to write subtitle file: {e:?}");
                            return Err(DashMpdError::Io(e, String::from("writing subtitle data")));
                        },
                    }
                    if subtitle_formats.contains(&SubtitleType::Wvtt) {
                        let mut out = subs_path.clone();
                        out.set_extension("srt");
                        // We can convert this to SRT format, which is more widely supported,
                        // using MP4Box. However, it's not a fatal error if MP4Box is not
                        // installed.
                        if let Ok(mp4box) = Command::new(downloader.mp4box_location.clone())
                            .args(["-srt", "1", "-out", &out.to_string_lossy(),
                                   &subs_path.to_string_lossy()])
                            .output()
                        {
                            let msg = String::from_utf8_lossy(&mp4box.stdout);
                            if msg.len() > 0 {
                                info!("MP4Box stdout: {msg}");
                            }
                            let msg = String::from_utf8_lossy(&mp4box.stderr);
                            if msg.len() > 0 {
                                info!("MP4Box stderr: {msg}");
                            }
                            if mp4box.status.success() {
                                info!("Converted WVTT subtitles to SRT");
                            } else {
                                warn!("Error running MP4Box to convert WVTT subtitles");
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
                }
                let rid = match &rep.id {
                    Some(id) => id,
                    None => return Err(
                        DashMpdError::UnhandledMediaStream(
                            "Missing @id on Representation node".to_string())),
                };
                let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
                if let Some(b) = &rep.bandwidth {
                    dict.insert("Bandwidth", b.to_string());
                }
                // Now the 6 possible addressing modes: (1) SegmentList,
                // (2) SegmentTemplate+SegmentTimeline, (3) SegmentTemplate@duration,
                // (4) SegmentTemplate@index, (5) SegmentBase@indexRange, (6) plain BaseURL

                // Though SegmentBase and SegmentList addressing modes are supposed to be
                // mutually exclusive, some manifests in the wild use both. So we try to work
                // around the brokenness.
                if let Some(sl) = &rep.SegmentList {
                    // (1) AdaptationSet>SegmentList addressing mode (can be used in conjunction
                    // with Representation>SegmentList addressing mode)
                    if downloader.verbosity > 1 {
                        println!("  Using AdaptationSet>SegmentList addressing mode for subtitle representation");
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
                            let init_url = if is_absolute_url(&path) {
                                Url::parse(&path)
                                    .map_err(|e| parse_error("parsing sourceURL", e))?
                            } else {
                                base_url.join(&path)
                                    .map_err(|e| parse_error("joining with sourceURL", e))?
                            };
                            let mf = MediaFragment{
                                period: period_counter,
                                url: init_url,
                                start_byte, end_byte,
                                is_init: true
                            };
                            fragments.push(mf);
                        } else {
                            let mf = MediaFragment{
                                period: period_counter,
                                url: base_url.clone(),
                                start_byte, end_byte,
                                is_init: true
                            };
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
                            let u = base_url.join(m)
                                .map_err(|e| parse_error("joining media with baseURL", e))?;
                            let mf = make_fragment(period_counter, u, start_byte, end_byte);
                            fragments.push(mf);
                        } else if !subtitle_adaptation.BaseURL.is_empty() {
                            let bu = &subtitle_adaptation.BaseURL[0];
                            let base_url = if is_absolute_url(&bu.base) {
                                Url::parse(&bu.base)
                                    .map_err(|e| parse_error("parsing Representation BaseURL", e))?
                            } else {
                                base_url.join(&bu.base)
                                    .map_err(|e| parse_error("joining with Representation BaseURL", e))?
                            };
                            let mf = make_fragment(period_counter, base_url.clone(), start_byte, end_byte);
                            fragments.push(mf);
                        }
                    }
                }
                if let Some(sl) = &rep.SegmentList {
                    // (1) Representation>SegmentList addressing mode
                    if downloader.verbosity > 1 {
                        println!("  Using Representation>SegmentList addressing mode for subtitle representation");
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
                            let init_url = if is_absolute_url(&path) {
                                Url::parse(&path)
                                    .map_err(|e| parse_error("parsing sourceURL", e))?
                            } else {
                                base_url.join(&path)
                                    .map_err(|e| parse_error("joining with sourceURL", e))?
                            };
                            let mf = MediaFragment{
                                period: period_counter,
                                url: init_url,
                                start_byte, end_byte,
                                is_init: true,
                            };
                            fragments.push(mf);
                        } else {
                            let mf = MediaFragment{
                                period: period_counter,
                                url: base_url.clone(),
                                start_byte, end_byte,
                                is_init: true,
                            };
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
                            let u = base_url.join(m)
                                .map_err(|e| parse_error("joining media with baseURL", e))?;
                            let mf = make_fragment(period_counter, u, start_byte, end_byte);
                            fragments.push(mf);
                        } else if !rep.BaseURL.is_empty() {
                            let bu = &rep.BaseURL[0];
                            let base_url = if is_absolute_url(&bu.base) {
                                Url::parse(&bu.base)
                                    .map_err(|e| parse_error("parsing Representation BaseURL", e))?
                            } else {
                                base_url.join(&bu.base)
                                    .map_err(|e| parse_error("joining with Representation BaseURL", e))?
                            };
                            let mf = make_fragment(period_counter, base_url.clone(), start_byte, end_byte);
                            fragments.push(mf);
                        }
                    }
                } else if rep.SegmentTemplate.is_some() ||
                    subtitle_adaptation.SegmentTemplate.is_some() {
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
                            opt_init = Some(i.to_string());
                        }
                        if let Some(m) = &st.media {
                            opt_media = Some(m.to_string());
                        }
                        if let Some(ts) = st.timescale {
                            timescale = ts;
                        }
                        if let Some(sn) = st.startNumber {
                            start_number = sn;
                        }
                        if let Some(stl) = &st.SegmentTimeline {
                            // (2) SegmentTemplate with SegmentTimeline addressing mode (also called
                            // "explicit addressing" in certain DASH-IF documents)
                            if downloader.verbosity > 1 {
                                println!("  Using SegmentTemplate+SegmentTimeline addressing mode for subtitle representation");
                            }
                            if let Some(init) = opt_init {
                                let path = resolve_url_template(&init, &dict);
                                let u = base_url.join(&path)
                                    .map_err(|e| parse_error("joining init with BaseURL", e))?;
                                let mf = MediaFragment{
                                    period: period_counter,
                                    url: u,
                                    start_byte: None,
                                    end_byte: None,
                                    is_init: true
                                };
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
                                    let u = base_url.join(&path)
                                        .map_err(|e| parse_error("joining media with BaseURL", e))?;
                                    let mf = make_fragment(period_counter, u, None, None);
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
                                            } else if segment_time as f64 > end_time {
                                                break;
                                            }
                                            segment_time += segment_duration;
                                            let dict = HashMap::from([("Time", segment_time.to_string()),
                                                                      ("Number", number.to_string())]);
                                            let path = resolve_url_template(&sub_path, &dict);
                                            let u = base_url.join(&path)
                                                .map_err(|e| parse_error("joining media with BaseURL", e))?;
                                            let mf = make_fragment(period_counter, u, None, None);
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
                                println!("  Using SegmentTemplate addressing mode for stpp subtitles");
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
                            let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
                            if let Some(b) = &rep.bandwidth {
                                dict.insert("Bandwidth", b.to_string());
                            }
                            let mut total_number = 0i64;
                            if let Some(init) = opt_init {
                                // The initialization segment counts as one of the $Number$
                                total_number -= 1;
                                let path = resolve_url_template(&init, &dict);
                                let u = base_url.join(&path)
                                    .map_err(|e| parse_error("joining init with BaseURL", e))?;
                                let mf = MediaFragment{
                                    period: period_counter,
                                    url: u,
                                    start_byte: None,
                                    end_byte: None,
                                    is_init: true
                                };
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
                                    let u = base_url.join(&path)
                                        .map_err(|e| parse_error("joining media with BaseURL", e))?;
                                    let mf = make_fragment(period_counter, u, None, None);
                                    fragments.push(mf);
                                    number += 1;
                                }
                            }
                        }
                    }
            }
            // TODO also implement SegmentBase addressing mode for subtitles
            // (sample MPD: https://usp-cmaf-test.s3.eu-central-1.amazonaws.com/tears-of-steel-ttml.mpd)
        }
    }
    Ok(PeriodOutputs { fragments, diagnostics: String::from(""), subtitle_formats })
}


async fn fetch_mpd(downloader: &DashDownloader) -> Result<PathBuf, DashMpdError> {
    let client = &downloader.http_client.clone().unwrap();
    let output_path = &downloader.output_path.as_ref().unwrap().clone();
    let fetch = || async {
        client.get(&downloader.mpd_url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .header("Accept-Language", "en-US,en")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Sec-Fetch-Mode", "navigate")
            .send().await
            .map_err(categorize_reqwest_error)?
            .error_for_status()
            .map_err(categorize_reqwest_error)
    };
    for observer in &downloader.progress_observers {
        observer.update(1, "Fetching DASH manifest");
    }
    if downloader.verbosity > 0 {
        println!("Fetching the DASH manifest");
    }
    // could also try crate https://lib.rs/crates/reqwest-retry for a "middleware" solution to retries
    // or https://docs.rs/again/latest/again/ with async support
    let response = retry_notify(ExponentialBackoff::default(), fetch, notify_transient)
        .await
        .map_err(|e| network_error("requesting DASH manifest", e))?;
    if !response.status().is_success() {
        let msg = format!("fetching DASH manifest (HTTP {})", response.status().as_str());
        return Err(DashMpdError::Network(msg));
    }
    let mut redirected_url = response.url().clone();
    let xml = response.bytes()
        .await
        .map_err(|e| network_error("fetching DASH manifest", e))?;
    let mut mpd: MPD = resolve_xlink_references(downloader, &redirected_url, &xml).await
        .map_err(|e| parse_error("parsing DASH XML", e))?;
    // From the DASH specification: "If at least one MPD.Location element is present, the value of
    // any MPD.Location element is used as the MPD request". We make a new request to the URI and reparse.
    if !mpd.locations.is_empty() {
        let new_url = &mpd.locations[0].url;
        if downloader.verbosity > 0 {
            println!("Redirecting to new manifest <Location> {new_url}");
        }
        let fetch = || async {
            client.get(new_url)
                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                .header("Accept-Language", "en-US,en")
                .header("Sec-Fetch-Mode", "navigate")
                .send().await
                .map_err(categorize_reqwest_error)?
                .error_for_status()
                .map_err(categorize_reqwest_error)
        };
        let response = retry_notify(ExponentialBackoff::default(), fetch, notify_transient)
            .await
            .map_err(|e| network_error("requesting relocated DASH manifest", e))?;
        if !response.status().is_success() {
            let msg = format!("fetching DASH manifest (HTTP {})", response.status().as_str());
            return Err(DashMpdError::Network(msg));
        }
        redirected_url = response.url().clone();
        let xml = response.text().await
            .map_err(|e| network_error("fetching relocated DASH manifest", e))?;
        mpd = parse(&xml)
            .map_err(|e| parse_error("parsing relocated DASH XML", e))?;
    }
    if let Some(mpdtype) = mpd.mpdtype.as_ref() {
        if mpdtype.eq("dynamic") {
            // TODO: look at algorithm used in function segment_numbers at
            // https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/dash_manifest.py
            return Err(DashMpdError::UnhandledMediaStream("Don't know how to download dynamic MPD".to_string()));
        }
    }
    let mut toplevel_base_url = redirected_url.clone();
    // There may be several BaseURL tags in the MPD, but we don't currently implement failover
    if !mpd.base_url.is_empty() {
        if is_absolute_url(&mpd.base_url[0].base) {
            toplevel_base_url = Url::parse(&mpd.base_url[0].base)
                .map_err(|e| parse_error("parsing BaseURL", e))?;
        } else {
            toplevel_base_url = redirected_url.join(&mpd.base_url[0].base)
                .map_err(|e| parse_error("parsing BaseURL", e))?;
        }
    }
    let mut audio_fragments = Vec::new();
    let mut video_fragments = Vec::new();
    let mut subtitle_fragments = Vec::new();
    let mut subtitle_formats = Vec::new();
    let mut have_audio = false;
    let mut have_video = false;
    let mut have_subtitles = false;
    if downloader.verbosity > 0 {
        let pcount = mpd.periods.len();
        println!("DASH manifest has {pcount} period{}", if pcount > 1 { "s" }  else { "" });
        print_available_streams(&mpd);
    }
    // Analyse the content of each Period in the manifest. We need to ensure that we associate media
    // segments with the correct period, because segments in each Period may use different codecs,
    // so they can't be concatenated together directly without reencoding.
    let mut period_counter = 0;
    for mpd_period in &mpd.periods {
        let period = mpd_period.clone();
        period_counter += 1;
        // let period_output_path = output_path_for_period(output_path, period_counter);
        // Accumulate some diagnostics information on the selected media stream
        if downloader.verbosity > 0 {
            if let Some(id) = period.id.as_ref() {
                println!("Preparing download for period {id} (#{period_counter})");
            } else {
                println!("Preparing download for period #{period_counter}");
            }
        }
        let mut base_url = toplevel_base_url.clone();
        // A BaseURL could be specified for each Period
        if !period.BaseURL.is_empty() {
            let bu = &period.BaseURL[0];
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base)
                    .map_err(|e| parse_error("parsing Period BaseURL", e))?;
            } else {
                base_url = base_url.join(&bu.base)
                    .map_err(|e| parse_error("joining with Period BaseURL", e))?;
            }
        }

        let audio_outputs = do_period_audio(downloader, &mpd, &period, period_counter, base_url.clone()).await?;
        for f in audio_outputs.fragments {
            audio_fragments.push(f);
        }
        let video_outputs = do_period_video(downloader, &mpd, &period, period_counter, base_url.clone()).await?;
        for f in video_outputs.fragments {
            video_fragments.push(f);
        }
        let subtitle_outputs = do_period_subtitles(downloader, &mpd, &period, period_counter, base_url.clone()).await?;
        for f in subtitle_outputs.fragments {
            subtitle_fragments.push(f);
        }
        for f in subtitle_outputs.subtitle_formats {
            subtitle_formats.push(f);
        }

        // Print some diagnostics information on the selected streams
        if downloader.verbosity > 0 {
            use base64::prelude::{Engine as _, BASE64_STANDARD};

            print!("{}", audio_outputs.diagnostics);
            for f in audio_fragments.iter().filter(|f| f.is_init) {
                if let Some(pssh) = extract_init_pssh(client, f.url.clone()).await {
                    println!("    PSSH (from init segment): {}", BASE64_STANDARD.encode(&pssh));
                }
            }
            print!("{}", video_outputs.diagnostics);
            for f in video_fragments.iter().filter(|f| f.is_init) {
                if let Some(pssh) = extract_init_pssh(client, f.url.clone()).await {
                    println!("    PSSH (from init segment): {}", BASE64_STANDARD.encode(&pssh));
                }
            }
        }
    } // loop over Periods

    // To collect the muxed audio and video segments for each Period in the MPD, before their
    // final concatenation-with-reencoding.
    let mut period_output_paths: Vec<PathBuf> = Vec::new();
    
    let mut download_errors = 0;
    // The additional +2 is for our initial .mpd fetch action and final muxing action
    let segment_count = audio_fragments.len() + video_fragments.len() + subtitle_fragments.len() + 2;
    let mut segment_counter = 0;
    // Our rate_limit is in bytes/second, but the governor::RateLimiter can only handle an u32 rate.
    // We express our cells in the RateLimiter in kB/s instead of bytes/second, to allow for numbing
    // future bandwidth capacities. We need to be careful to allow a quota burst size which
    // corresponds to the size (in kB) of the largest media segments we are going to be retrieving,
    // because that's the number of bucket cells that will be consumed for each downloaded segment.
    let kps = 1 + downloader.rate_limit / 1024;
    if kps > u32::MAX.into() {
        return Err(DashMpdError::Other("Bandwidth limit is too high".to_string()));
    }
    let bw_limit = NonZeroU32::new(kps as u32).unwrap();
    let bw_quota = Quota::per_second(bw_limit)
        .allow_burst(NonZeroU32::new(10 * 1024).unwrap());
    let bw_limiter = RateLimiter::direct(bw_quota);
    period_counter = 0;
    for _mpd_period in &mpd.periods {
        period_counter += 1;
        let period_output_path = output_path_for_period(output_path, period_counter);
        #[allow(clippy::collapsible_if)]
        if downloader.verbosity > 0 {
            if downloader.fetch_audio || downloader.fetch_video || downloader.fetch_subtitles {
                println!("Period #{period_counter}: fetching {} audio, {} video and {} subtitle segments",
                         audio_fragments.len(),
                         video_fragments.len(),
                         subtitle_fragments.len());
            }
        }
        let tmppath_audio = if let Some(ref path) = downloader.keep_audio {
            path.clone()
        } else {
            tmp_file_path("dashmpd-audio")?
        };
        let tmppath_video = if let Some(ref path) = downloader.keep_video {
            path.clone()
        } else {
            tmp_file_path("dashmpd-video")?
        };
        let tmppath_subs = tmp_file_path("dashmpd-subs")?;
        
        // Concatenate the audio segments to a file.
        //
        // FIXME: in DASH, the first segment contains headers that are necessary to generate a valid MP4
        // file, so we should always abort if the first segment cannot be fetched. However, we could
        // tolerate loss of subsequent segments.
        if downloader.fetch_audio {
            let start_audio_download = Instant::now();
            {
                // We need a local scope for our tmpfile_video File, so that the file is closed when
                // we later optionally call mp4decrypt (which requires exclusive access to its input file on Windows).
                let tmpfile_audio = File::create(tmppath_audio.clone())
                    .map_err(|e| DashMpdError::Io(e, String::from("creating audio tmpfile")))?;
                let mut tmpfile_audio = BufWriter::new(tmpfile_audio);
                // Optionally create the directory to which we will save the audio fragments.
                if let Some(ref fragment_path) = downloader.fragment_path {
                    let audio_fragment_dir = fragment_path.join("audio");
                    if !audio_fragment_dir.exists() {
                        fs::create_dir_all(audio_fragment_dir)
                            .map_err(|e| DashMpdError::Io(e, String::from("creating audio fragment dir")))?;
                    }
                }
                for frag in audio_fragments.iter().filter(|f| f.period == period_counter) {
                    // Update any ProgressObservers
                    segment_counter += 1;
                    let progress_percent = (100.0 * segment_counter as f32 / segment_count as f32).ceil() as u32;
                    for observer in &downloader.progress_observers {
                        observer.update(progress_percent, "Fetching audio segments");
                    }
                    let url = &frag.url;
                    /*
                    A manifest may use a data URL (RFC 2397) to embed media content such as the
                    initialization segment directly in the manifest (recommended by YouTube for live
                    streaming, but uncommon in practice).
                     */
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
                            println!("  Audio segment data URL -> {} octets", body.len());
                        }
                        if let Err(e) = tmpfile_audio.write_all(&body) {
                            error!("Unable to write DASH audio data: {e:?}");
                            return Err(DashMpdError::Io(e, String::from("writing DASH audio data")));
                        }
                        have_audio = true;
                    } else {
                        // We could download these segments in parallel, but that might upset some servers.
                        let fetch = || async {
                            // Don't use only "audio/*" in Accept header because some web servers
                            // (eg. media.axprod.net) are misconfigured and reject requests for
                            // valid audio content (eg .m4s)
                            let mut req = client.get(url.clone())
                                .header("Accept", "audio/*;q=0.9,*/*;q=0.5")
                                .header("Referer", redirected_url.to_string())
                                .header("Sec-Fetch-Mode", "navigate");
                            if let Some(sb) = &frag.start_byte {
                                if let Some(eb) = &frag.end_byte {
                                    req = req.header(RANGE, format!("bytes={sb}-{eb}"));
                                }
                            }
                            req.send().await
                                .map_err(categorize_reqwest_error)?
                                .error_for_status()
                                .map_err(categorize_reqwest_error)
                        };
                        let mut failure = None;
                        match retry_notify(ExponentialBackoff::default(), fetch, notify_transient).await {
                            Ok(response) => {
                                if response.status().is_success() {
                                    if !downloader.content_type_checks || content_type_audio_p(&response) {
                                        let dash_bytes = response.bytes().await
                                            .map_err(|e| network_error("fetching DASH audio segment bytes", e))?;
                                        if downloader.verbosity > 2 {
                                            if let Some(sb) = &frag.start_byte {
                                                if let Some(eb) = &frag.end_byte {
                                                    println!("  Audio segment {} range {sb}-{eb} -> {} octets",
                                                             &frag.url, dash_bytes.len());
                                                }
                                            } else {
                                                println!("  Audio segment {url} -> {} octets", dash_bytes.len());
                                            }
                                        }
                                        if downloader.rate_limit > 0 {
                                            let size = min(dash_bytes.len()/1024 + 1, u32::MAX as usize);
                                            if let Some(cells) = NonZeroU32::new(size as u32) {
                                                #[allow(clippy::redundant_pattern_matching)]
                                                if let Err(_) = bw_limiter.until_n_ready(cells).await {
                                                    return Err(DashMpdError::Other(
                                                        "Bandwidth limit is too low".to_string()));
                                                }
                                            }
                                        }
                                        if let Err(e) = tmpfile_audio.write_all(&dash_bytes) {
                                            error!("Unable to write DASH audio data: {e:?}");
                                            return Err(DashMpdError::Io(e, String::from("writing DASH audio data")));
                                        }
                                        if let Some(ref fragment_path) = downloader.fragment_path {
                                            if let Some(path) = frag.url.path_segments()
                                                .unwrap_or_else(|| "".split(' '))
                                                .last()
                                            {
                                                let af_file = fragment_path.clone().join("audio").join(path);
                                                let mut out = File::create(af_file)
                                                    .map_err(|e| DashMpdError::Io(e, String::from("creating audio fragment file")))?;
                                                out.write_all(&dash_bytes)
                                                    .map_err(|e| DashMpdError::Io(e, String::from("writing audio fragment")))?;
                                            }
                                        }
                                        have_audio = true;
                                    } else {
                                        warn!("Ignoring segment {url} with non-audio content-type");
                                    }
                                } else {
                                    failure = Some(format!("HTTP error {}", response.status().as_str()));
                                }
                            },
                            Err(e) => failure = Some(format!("{e}")),
                        }
                        if let Some(f) = failure {
                            if downloader.verbosity > 0 {
                                eprintln!("{f} fetching audio segment {url}");
                            }
                            download_errors += 1;
                            if download_errors > downloader.max_error_count {
                                return Err(DashMpdError::Network(
                                    String::from("more than max_error_count network errors")));
                            }
                        }
                    }
                    if downloader.sleep_between_requests > 0 {
                        tokio::time::sleep(Duration::new(downloader.sleep_between_requests.into(), 0)).await;
                    }
                }
                tmpfile_audio.flush().map_err(|e| {
                    error!("Couldn't flush DASH audio file: {e}");
                    DashMpdError::Io(e, String::from("flushing DASH audio file"))
                })?;
            } // end local scope for the FileHandle
            if !downloader.decryption_keys.is_empty() {
                if downloader.verbosity > 0 {
                    println!("  Attempting to decrypt audio stream");
                }
                let mut args = Vec::new();
                for (k, v) in downloader.decryption_keys.iter() {
                    args.push("--key".to_string());
                    args.push(format!("{k}:{v}"));
                }
                args.push(String::from(tmppath_audio.to_string_lossy()));
                let decrypted = tmp_file_path("dashmpd-decrypted-audio")?;
                args.push(String::from(decrypted.to_string_lossy()));
                let out = Command::new(downloader.mp4decrypt_location.clone())
                    .args(args)
                    .output()
                    .map_err(|e| DashMpdError::Io(e, String::from("spawning mp4decrypt")))?;
                if !out.status.success() {
                    warn!("mp4decrypt subprocess failed");
                    let msg = String::from_utf8_lossy(&out.stdout);
                    if msg.len() > 0 {
                        warn!("mp4decrypt stdout: {msg}");
                    }
                    let msg = String::from_utf8_lossy(&out.stderr);
                    if msg.len() > 0 {
                        warn!("mp4decrypt stderr: {msg}");
                    }
                }
                fs::rename(decrypted, tmppath_audio.clone())
                    .map_err(|e| DashMpdError::Io(e, String::from("renaming decrypted audio")))?;
            }
            if let Ok(metadata) = fs::metadata(tmppath_audio.clone()) {
                if downloader.verbosity > 1 {
                    let mbytes = metadata.len() as f64 / (1024.0 * 1024.0);
                    let elapsed = start_audio_download.elapsed();
                    println!("  Wrote {mbytes:.1}MB to DASH audio file ({:.1}MB/s)",
                             mbytes / elapsed.as_secs_f64());
                }
            }
        } // if downloader.fetch_audio

        // Now fetch the video segments and concatenate them to the video file
        if downloader.fetch_video {
            let start_video_download = Instant::now();
            {
                // We need a local scope for our tmpfile_video File, so that the file is closed when
                // we later call mp4decrypt (which requires exclusive access to its input file on Windows).
                let tmpfile_video = File::create(tmppath_video.clone())
                    .map_err(|e| DashMpdError::Io(e, String::from("creating video tmpfile")))?;
                let mut tmpfile_video = BufWriter::new(tmpfile_video);
                // Optionally create the directory to which we will save the video fragments.
                if let Some(ref fragment_path) = downloader.fragment_path {
                    let video_fragment_dir = fragment_path.join("video");
                    if !video_fragment_dir.exists() {
                        fs::create_dir_all(video_fragment_dir)
                            .map_err(|e| DashMpdError::Io(e, String::from("creating video fragment dir")))?;
                    }
                }
                for frag in video_fragments.iter().filter(|f| f.period == period_counter) {
                    // Update any ProgressObservers
                    segment_counter += 1;
                    let progress_percent = (100.0 * segment_counter as f32 / segment_count as f32).ceil() as u32;
                    for observer in &downloader.progress_observers {
                        observer.update(progress_percent, "Fetching video segments");
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
                            println!("  Video segment data URL -> {} octets", body.len());
                        }
                        if let Err(e) = tmpfile_video.write_all(&body) {
                            error!("Unable to write DASH video data: {e:?}");
                            return Err(DashMpdError::Io(e, String::from("writing DASH video data")));
                        }
                        have_video = true;
                    } else {
                        let fetch = || async {
                            let mut req = client.get(frag.url.clone())
                                .header("Accept", "video/*")
                                .header("Referer", redirected_url.to_string())
                                .header("Sec-Fetch-Mode", "navigate");
                            if let Some(sb) = &frag.start_byte {
                                if let Some(eb) = &frag.end_byte {
                                    req = req.header(RANGE, format!("bytes={sb}-{eb}"));
                                }
                            }
                            req.send().await
                                .map_err(categorize_reqwest_error)?
                                .error_for_status()
                                .map_err(categorize_reqwest_error)
                        };
                        let mut failure = None;
                        match retry_notify(ExponentialBackoff::default(), fetch, notify_transient).await {
                            Ok(response) => {
                                if response.status().is_success() {
                                    if !downloader.content_type_checks || content_type_video_p(&response) {
                                        let dash_bytes = response.bytes().await
                                            .map_err(|e| network_error("fetching DASH video segment", e))?;
                                        if downloader.verbosity > 2 {
                                            if let Some(sb) = &frag.start_byte {
                                                if let Some(eb) = &frag.end_byte {
                                                    println!("  Video segment {} range {sb}-{eb} -> {} octets",
                                                             &frag.url, dash_bytes.len());
                                                }
                                            } else {
                                                println!("  Video segment {} -> {} octets", &frag.url, dash_bytes.len());
                                            }
                                        }
                                        if downloader.rate_limit > 0 {
                                            let size = min(dash_bytes.len()/1024+1, u32::MAX as usize);
                                            if let Some(cells) = NonZeroU32::new(size as u32) {
                                                #[allow(clippy::redundant_pattern_matching)]
                                                if let Err(_) = bw_limiter.until_n_ready(cells).await {
                                                    return Err(DashMpdError::Other(
                                                        "Bandwidth limit is too low".to_string()));
                                                }
                                            }
                                        }
                                        if let Err(e) = tmpfile_video.write_all(&dash_bytes) {
                                            return Err(DashMpdError::Io(e, String::from("writing DASH video data")));
                                        }
                                        if let Some(ref fragment_path) = downloader.fragment_path {
                                            if let Some(path) = frag.url.path_segments()
                                                .unwrap_or_else(|| "".split(' '))
                                                .last()
                                            {
                                                let vf_file = fragment_path.clone().join("video").join(path);
                                                let mut out = File::create(vf_file)
                                                    .map_err(|e| DashMpdError::Io(e, String::from("creating video fragment file")))?;
                                                out.write_all(&dash_bytes)
                                                    .map_err(|e| DashMpdError::Io(e, String::from("writing video fragment")))?;
                                            }
                                        }
                                        have_video = true;
                                    } else {
                                        warn!("Ignoring segment {} with non-video content-type", &frag.url);
                                    }
                                } else {
                                    failure = Some(format!("HTTP error {}", response.status().as_str()));
                                }
                            },
                            Err(e) => failure = Some(format!("{e}")),
                        }
                        if let Some(f) = failure {
                            if downloader.verbosity > 0 {
                                eprintln!("{f} fetching video segment {}", &frag.url);
                            }
                            download_errors += 1;
                            if download_errors > downloader.max_error_count {
                                return Err(DashMpdError::Network(
                                    String::from("more than max_error_count network errors")));
                            }
                        }
                    }
                    if downloader.sleep_between_requests > 0 {
                        tokio::time::sleep(Duration::new(downloader.sleep_between_requests.into(), 0)).await;
                    }
                }
                tmpfile_video.flush().map_err(|e| {
                    error!("Couldn't flush video file: {e}");
                    DashMpdError::Io(e, String::from("flushing video file"))
                })?;
            } // end local scope for tmpfile_video File
            if !downloader.decryption_keys.is_empty() {
                if downloader.verbosity > 0 {
                    println!("  Attempting to decrypt video stream");
                }
                let mut args = Vec::new();
                for (k, v) in downloader.decryption_keys.iter() {
                    args.push("--key".to_string());
                    args.push(format!("{k}:{v}"));
                }
                args.push(String::from(tmppath_video.to_string_lossy()));
                let decrypted = tmp_file_path("dashmpd-decrypted-video")?;
                args.push(String::from(decrypted.to_string_lossy()));
                let out = Command::new(downloader.mp4decrypt_location.clone())
                    .args(args)
                    .output()
                    .map_err(|e| DashMpdError::Io(e, String::from("spawning mp4decrypt")))?;
                if ! out.status.success() {
                    warn!("mp4decrypt subprocess failed");
                    let msg = String::from_utf8_lossy(&out.stdout);
                    if msg.len() > 0 {
                        warn!("mp4decrypt stdout: {msg}");
                    }
                    let msg = String::from_utf8_lossy(&out.stderr);
                    if msg.len() > 0 {
                        warn!("mp4decrypt stderr: {msg}");
                    }
                }
                fs::rename(decrypted, tmppath_video.clone())
                    .map_err(|e| DashMpdError::Io(e, String::from("renaming decrypted video")))?;
            }
            if let Ok(metadata) = fs::metadata(tmppath_video.clone()) {
                if downloader.verbosity > 1 {
                    let mbytes = metadata.len() as f64 / (1024.0 * 1024.0);
                    let elapsed = start_video_download.elapsed();
                    println!("  Wrote {mbytes:.1}MB to DASH video file ({:.1}MB/s)",
                             mbytes / elapsed.as_secs_f64());
                }
            }
        } // if downloader.fetch_video

        // Here we handle subtitles that are distributed in fragmented MP4 segments, rather than as a
        // single .srt or .vtt file file. This is the case for WVTT (WebVTT) and STPP (which should be
        // formatted as EBU-TT for DASH media) formats.
        if downloader.fetch_subtitles {
            let start_subs_download = Instant::now();
            {
                let tmpfile_subs = File::create(tmppath_subs.clone())
                    .map_err(|e| DashMpdError::Io(e, String::from("creating subs tmpfile")))?;
                let mut tmpfile_subs = BufWriter::new(tmpfile_subs);
                for frag in &subtitle_fragments {
                    // Update any ProgressObservers
                    segment_counter += 1;
                    let progress_percent = (100.0 * segment_counter as f32 / segment_count as f32).ceil() as u32;
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
                            println!("  Subtitle segment data URL -> {} octets", body.len());
                        }
                        if let Err(e) = tmpfile_subs.write_all(&body) {
                            error!("Unable to write DASH subtitle data: {e:?}");
                            return Err(DashMpdError::Io(e, String::from("writing DASH subtitle data")));
                        }
                        have_subtitles = true;
                    } else {
                        let fetch = || async {
                            let mut req = client.get(frag.url.clone())
                                .header("Referer", redirected_url.to_string())
                                .header("Sec-Fetch-Mode", "navigate");
                            if let Some(sb) = &frag.start_byte {
                                if let Some(eb) = &frag.end_byte {
                                    req = req.header(RANGE, format!("bytes={sb}-{eb}"));
                                }
                            }
                            req.send().await
                                .map_err(categorize_reqwest_error)?
                                .error_for_status()
                                .map_err(categorize_reqwest_error)
                        };
                        let mut failure = None;
                        match retry_notify(ExponentialBackoff::default(), fetch, notify_transient).await {
                            Ok(response) => {
                                if response.status().is_success() {
                                    let dash_bytes = response.bytes().await
                                        .map_err(|e| network_error("fetching DASH subtitle segment", e))?;
                                    if downloader.verbosity > 2 {
                                        if let Some(sb) = &frag.start_byte {
                                            if let Some(eb) = &frag.end_byte {
                                                println!("  Subtitle segment {} range {sb}-{eb} -> {} octets",
                                                         &frag.url, dash_bytes.len());
                                            }
                                        } else {
                                            println!("  Subtitle segment {} -> {} octets", &frag.url, dash_bytes.len());
                                        }
                                    }
                                    if downloader.rate_limit > 0 {
                                        let size = min(dash_bytes.len()/1024+1, u32::MAX as usize);
                                        if let Some(cells) = NonZeroU32::new(size as u32) {
                                            #[allow(clippy::redundant_pattern_matching)]
                                            if let Err(_) = bw_limiter.until_n_ready(cells).await {
                                                return Err(DashMpdError::Other(
                                                    "Bandwidth limit is too low".to_string()));
                                            }
                                        }
                                    }
                                    if let Err(e) = tmpfile_subs.write_all(&dash_bytes) {
                                        return Err(DashMpdError::Io(e, String::from("writing DASH subtitle data")));
                                    }
                                    have_subtitles = true;
                                } else {
                                    failure = Some(format!("HTTP error {}", response.status().as_str()));
                                }
                            },
                            Err(e) => failure = Some(format!("{e}")),
                        }
                        if let Some(f) = failure {
                            if downloader.verbosity > 0 {
                                eprintln!("{f} fetching subtitle segment {}", &frag.url);
                            }
                            download_errors += 1;
                            if download_errors > downloader.max_error_count {
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
                    error!("Couldn't flush subs file disk: {e}");
                    DashMpdError::Io(e, String::from("flushing subtitle file"))
                })?;
            } // end local scope for tmpfile_subs File
            if have_subtitles {
                if let Ok(metadata) = fs::metadata(tmppath_subs.clone()) {
                    if downloader.verbosity > 1 {
                        let mbytes = metadata.len() as f64 / (1024.0 * 1024.0);
                        let elapsed = start_subs_download.elapsed();
                        println!("  Wrote {mbytes:.1}MB to DASH subtitle file ({:.1}MB/s)",
                                 mbytes / elapsed.as_secs_f64());
                    }
                }
                if subtitle_formats.contains(&SubtitleType::Wvtt) {
                    // We can extract these from the MP4 container in .srt format, using MP4Box.
                    if downloader.verbosity > 1 {
                        println!("  Running MP4Box to extract WVTT subtitles in SRT format");
                    }
                    let mut out = output_path.clone();
                    out.set_extension("srt");
                    if let Ok(mp4box) = Command::new(downloader.mp4box_location.clone())
                        .args(["-srt", "1", "-out", &out.to_string_lossy(), &tmppath_subs.to_string_lossy()])
                        .output()
                    {
                        let msg = String::from_utf8_lossy(&mp4box.stdout);
                        if msg.len() > 0 {
                            info!("MP4Box stdout: {msg}");
                        }
                        let msg = String::from_utf8_lossy(&mp4box.stderr);
                        if msg.len() > 0 {
                            info!("MP4Box stderr: {msg}");
                        }
                        if mp4box.status.success() {
                            info!("Extracted WVTT subtitles as SRT");
                        } else {
                            warn!("Error running MP4Box to extract WVTT subtitles");
                        }
                    } else {
                        warn!("Failed to spawn MP4Box to extract WVTT subtitles");
                    }
                }
            }
        } // if downloader.fetch_subtitles
        
        // The output file for this Period is either a mux of the audio and video streams, if both
        // are present, or just the audio stream, or just the video stream.
        if have_audio && have_video {
            for observer in &downloader.progress_observers {
                observer.update(99, "Muxing audio and video");
            }
            if downloader.verbosity > 1 {
                println!("  Muxing audio and video streams");
            }
            mux_audio_video(downloader, &period_output_path, &tmppath_audio, &tmppath_video)?;
            if subtitle_formats.contains(&SubtitleType::Stpp) {
                if downloader.verbosity > 1 {
                    println!("  Running MP4Box to merge STPP subtitles with output file");
                }
                // We can try to merge these with the MP4 container, using MP4Box.
                if let Ok(mp4box) = Command::new(downloader.mp4box_location.clone())
                    .args(["-add", &tmppath_subs.to_string_lossy(),
                           &period_output_path.clone().to_string_lossy()])
                    .output()
                {
                    let msg = String::from_utf8_lossy(&mp4box.stdout);
                    if msg.len() > 0 {
                        info!("MP4Box stdout: {msg}");
                    }
                    let msg = String::from_utf8_lossy(&mp4box.stderr);
                    if msg.len() > 0 {
                        info!("MP4Box stderr: {msg}");
                    }
                    if mp4box.status.success() {
                        info!("  Merged STPP subtitles with MP4 container");
                    } else {
                        warn!("Error running MP4Box to merge STPP subtitles");
                    }
                } else {
                    warn!("Failed to spawn MP4Box to merge STPP subtitles");
                }
            }
        } else if have_audio {
            // Copy the downloaded audio segments to the output file. We don't use fs::rename() because
            // it might fail if temporary files and our output are on different filesystems.
            let tmpfile_audio = File::open(&tmppath_audio)
                .map_err(|e| DashMpdError::Io(e, String::from("opening temporary audio output file")))?;
            let mut audio = BufReader::new(tmpfile_audio);
            let output_file = File::create(period_output_path.clone())
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file for video")))?;
            let mut sink = BufWriter::new(output_file);
            io::copy(&mut audio, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying audio stream to output file")))?;
        } else if have_video {
            let tmpfile_video = File::open(&tmppath_video)
                .map_err(|e| DashMpdError::Io(e, String::from("opening temporary video output file")))?;
            let mut video = BufReader::new(tmpfile_video);
            let output_file = File::create(period_output_path.clone())
                .map_err(|e| DashMpdError::Io(e, String::from("creating output file for video")))?;
            let mut sink = BufWriter::new(output_file);
            io::copy(&mut video, &mut sink)
                .map_err(|e| DashMpdError::Io(e, String::from("copying video stream to output file")))?;
        } else if downloader.fetch_video && downloader.fetch_audio {
            return Err(DashMpdError::UnhandledMediaStream("no audio or video streams found".to_string()));
        } else if downloader.fetch_video {
            return Err(DashMpdError::UnhandledMediaStream("no video streams found".to_string()));
        } else if downloader.fetch_audio {
            return Err(DashMpdError::UnhandledMediaStream("no audio streams found".to_string()));
        }
        #[allow(clippy::collapsible_if)]
        if downloader.keep_audio.is_none() && downloader.fetch_audio {
            if fs::remove_file(tmppath_audio).is_err() {
                info!("Failed to delete temporary file for audio stream");
            }
        }
        #[allow(clippy::collapsible_if)]
        if downloader.keep_video.is_none() && downloader.fetch_video {
            if fs::remove_file(tmppath_video).is_err() {
                info!("Failed to delete temporary file for video stream");
            }
        }
        if downloader.verbosity > 1 && (downloader.fetch_audio || downloader.fetch_video) {
            if let Ok(metadata) = fs::metadata(period_output_path.clone()) {
                println!("  Wrote {:.1}MB to media file", metadata.len() as f64 / (1024.0 * 1024.0));
            }
        }
        period_output_paths.push(period_output_path);
    } // Period iterator

    if period_output_paths.len() == 1 {
        // We already arranged to write directly to the requested output_path.
        maybe_record_metainformation(output_path, downloader, &mpd);
    } else {
        // If the streams for the different periods are all of the same resolution, we can
        // concatenate them (with reencoding) into a single media file. Otherwise, we can't
        // concatenate without rescaling and loss of quality, so we leave them in separate files.
        // This feature isn't implemented using libav instead of ffmpeg as a subprocess.
        #[allow(unused_mut)]
        let mut concatenated = false;
        #[cfg(not(feature = "libav"))]
        if downloader.concatenate_periods && video_containers_concatable(downloader, &period_output_paths) {
            info!("Preparing to concatenate multiple Periods into one output file");
            concat_output_files(downloader, &period_output_paths)?;
            for p in &period_output_paths[1..] {
                if fs::remove_file(p).is_err() {
                    info!("Failed to delete temporary file");
                }
            }
            concatenated = true;
            maybe_record_metainformation(&period_output_paths[0], downloader, &mpd);
        }
        if !concatenated {
            println!("Media content has been saved in a separate file for each period:");
            period_counter = 0;
            for p in period_output_paths {
                period_counter += 1;
                println!("  Period #{period_counter}: {}", p.display());
                maybe_record_metainformation(&p, downloader, &mpd);
            }
        }
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
