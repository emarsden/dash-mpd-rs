//! Support for downloading content from DASH MPD media streams.

use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::time::Duration;
use std::sync::Arc;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use regex::Regex;
use url::Url;
use backoff::{retry, retry_notify, ExponentialBackoff};
use anyhow::{Context, Result, anyhow};
use crate::{MPD, Period, Representation, AdaptationSet};
use crate::{parse, tmp_file_path, is_audio_adaptation, is_video_adaptation, mux_audio_video};


/// A blocking `Client` from the `reqwest` crate, that we use to download content over HTTP.
pub type HttpClient = reqwest::blocking::Client;

/// Receives updates concerning the progression of the download, and can display this information to
/// the user, for example using a progress bar.
pub trait ProgressObserver {
    fn update(&self, percent: u32, message: &str);
}


/// Preference for retrieving media representation with highest quality (and highest file size) or
/// lowest quality (and lowest file size).
#[derive(PartialEq)]
pub enum QualityPreference { Lowest, Highest }

impl Default for QualityPreference {
    fn default() -> Self { QualityPreference::Lowest }
}


/// The DashDownloader allows the download of streaming media content from a DASH MPD manifest. This
/// involves fetching the manifest file, parsing it, identifying the relevant audio and video
/// representations, downloading all the segments, concatenating them then muxing the audio and
/// video streams to produce a single video file including audio. This should work with both
/// MPEG-DASH MPD manifests (where the media segments are typically placed in MPEG-2 TS containers)
/// and for [WebM-DASH](http://wiki.webmproject.org/adaptive-streaming/webm-dash-specification).
pub struct DashDownloader {
    mpd_url: String,
    output_path: Option<PathBuf>,
    http_client: Option<HttpClient>,
    quality_preference: QualityPreference,
    progress_observers: Vec<Arc<dyn ProgressObserver>>,
    verbosity: u8,
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
/// let dl_path = DashDownloader::new(url)
///        .worst_quality()
///        .download();
/// println!("Downloaded to {:?}", dl_path);
/// ```
impl DashDownloader {
    /// Create a `DashDownloader` for the specified DASH manifest URL `mpd_url`.
    pub fn new(mpd_url: &str) -> DashDownloader {
        DashDownloader {
            mpd_url: String::from(mpd_url),
            output_path: None,
            http_client: None,
            quality_preference: QualityPreference::Lowest,
            progress_observers: vec![],
            verbosity: 0,
        }
    }

    /// Specify the reqwest Client to be used for HTTP requests that download the DASH streaming
    /// media content. Allows you to specify a proxy, the user agent, custom request headers,
    /// request timeouts, etc.
    ///
    /// Example
    /// ```rust
    /// use dash_mpd::fetch::DashDownloader;
    ///
    /// let client = reqwest::blocking::Client::builder()
    ///      .user_agent("Mozilla/5.0")
    ///      .timeout(Duration::new(10, 0))
    ///      .gzip(true)
    ///      .build()
    ///      .expect("Couldn't create reqwest HTTP client");
    ///  let url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    ///  let out = PathBuf::from(env::temp_dir()).join("cloudflarestream.mp4");
    ///  DashDownloader::new(url)
    ///      .with_http_client(client)
    ///      .download_to(out)
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

    /// Set the verbosity level of the download process. Possible values for level:
    /// - 0: no information is printed
    /// - 1: basic information on the number of Periods and bandwidth of selected representations
    /// - 2: information above + segment addressing mode
    /// - 3 or larger: information above + size of each downloaded segment
    pub fn verbosity(mut self, level: u8) -> DashDownloader {
        self.verbosity = level;
        self
    }

    /// Download DASH streaming media content to the file named by `out`. If the output file `out`
    /// already exists, its content will be overwritten.
    pub fn download_to<P: Into<PathBuf>>(mut self, out: P) -> Result<()> {
        self.output_path = Some(out.into());
        if self.http_client.is_none() {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::new(10, 0))
                .gzip(true)
                .build()
                .context("building reqwest HTTP client")?;
            self.http_client = Some(client);
        }
        fetch_mpd(self)
    }

    /// Download DASH streaming media content to a file in the current working directory and return
    /// the corresponding `PathBuf`. The name of the output file is derived from the manifest URL. The
    /// output file will be overwritten if it already exists.
    pub fn download(mut self) -> Result<PathBuf> {
        let cwd = env::current_dir().context("obtaining current dir")?;
        let filename = generate_filename_from_url(&self.mpd_url);
        let outpath = cwd.join(filename);
        self.output_path = Some(outpath.clone());
        if self.http_client.is_none() {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::new(10, 0))
                .gzip(true)
                .build()
                .context("building reqwest HTTP client")?;
            self.http_client = Some(client);
        }
        fetch_mpd(self)?;
        Ok(outpath)
    }
}


fn generate_filename_from_url(url: &str) -> PathBuf {
    use sanitise_file_name::sanitise;

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
    PathBuf::from(sanitise(path) + ".mp4")
}


fn is_absolute_url(s: &str) -> bool {
    s.starts_with("http://") ||
        s.starts_with("https://") ||
        s.starts_with("file://")
}

// From the DASH-IF-IOP-v4.0 specification, "If the value of the @xlink:href attribute is
// urn:mpeg:dash:resolve-to-zero:2013, HTTP GET request is not issued, and the in-MPD element shall
// be removed from the MPD."
fn fetchable_xlink_href(href: &str) -> bool {
    (!href.is_empty()) && href.ne("urn:mpeg:dash:resolve-to-zero:2013")
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
        let ident = format!("${}$", k);
        if result.contains(&ident) {
            if let Some(value) = params.get(k as &str) {
                result = result.replace(&ident, value);
            }
        }
        // now check for complex case eg $Number%06d$
        let re = format!("\\${}%0([\\d])d\\$", k);
        let ident_re = Regex::new(&re).unwrap();
        if let Some(cap) = ident_re.captures(&result) {
            if let Some(value) = params.get(k as &str) {
                let width: usize = cap[1].parse::<usize>().unwrap();
                let count = format!("{:0>width$}", value, width=width);
                let m = ident_re.find(&result).unwrap();
                result = result[..m.start()].to_owned() + &count + &result[m.end()..];
            }
        }
    }
    result
}


fn reqwest_error_transient_p(e: &reqwest::Error) -> bool {
    if e.is_timeout() || e.is_connect() {
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
    log::info!("Transient error at {:?}: {:?}", dur, err);
}


fn fetch_mpd(downloader: DashDownloader) -> Result<()> {
    let client = downloader.http_client.unwrap();
    let output_path = &downloader.output_path.unwrap();
    let fetch = || {
        client.get(&downloader.mpd_url)
            .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
            .header("Accept-language", "en-US,en")
            .send()
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
    let backoff = ExponentialBackoff::default();
    let response = retry(backoff, fetch)
        .context("requesting DASH manifest")?;
    let redirected_url = response.url().clone();
    let xml = response.text()
        .context("fetching DASH manifest")?;
    let mpd: MPD = parse(&xml)
        .context("parsing DASH XML")?;
    if let Some(mpdtype) = mpd.mpdtype {
        if mpdtype.eq("dynamic") {
            // TODO: look at algorithm used in function segment_numbers at
            // https://github.com/streamlink/streamlink/blob/master/src/streamlink/stream/dash_manifest.py
            return Err(anyhow!("Don't know how to download dynamic MPD"));
        }
    }
    let mut toplevel_base_url = redirected_url.clone();
    // There may be several BaseURL tags in the MPD, but we don't currently implement failover
    if let Some(bases) = &mpd.base_urls {
        if is_absolute_url(&bases[0].base) {
            toplevel_base_url = Url::parse(&bases[0].base).context("parsing BaseURL")?;
        } else {
            toplevel_base_url = redirected_url.join(&bases[0].base).context("parsing BaseURL")?;
        }
    }
    let mut video_segment_urls = Vec::new();
    let mut audio_segment_urls = Vec::new();
    let tmppath_video = tmp_file_path("dashmpd-video-track");
    let tmppath_audio = tmp_file_path("dashmpd-audio-track");
    let tmpfile_video = File::create(tmppath_video.clone())
        .context("creating video tmpfile")?;
    let mut tmpfile_video = BufWriter::new(tmpfile_video);
    let tmpfile_audio = File::create(tmppath_audio.clone())
        .context("creating audio tmpfile")?;
    let mut tmpfile_audio = BufWriter::new(tmpfile_audio);
    let mut have_audio = false;
    let mut have_video = false;
    if downloader.verbosity > 0 {
        println!("DASH manifest has {} Periods", mpd.periods.len());
    }
    for mpd_period in &mpd.periods {
        let mut period = mpd_period.clone();
        // Resolve a possible xlink:href (though this seems in practice mostly to be used for ad
        // insertion, so perhaps we should implement an option to ignore these).
        if let Some(href) = &period.href {
            if fetchable_xlink_href(href) {
                let xlink_url;
                if is_absolute_url(href) {
                    xlink_url = Url::parse(href).context("parsing XLink URL")?;
                } else {
                    // Note that we are joining against the original/redirected URL for the MPD, and
                    // not against the currently scoped BaseURL
                    xlink_url = redirected_url.join(href).context("joining with XLink URL")?;
                }
                let xml = client.get(xlink_url)
                    .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                    .header("Accept-language", "en-US,en")
                    .send()
                    .context("fetching XLink on Period element")?
                    .text()
                    .context("resolving XLink on Period element")?;
                let linked_period: Period = quick_xml::de::from_str(&xml)
                    .context("parsing Period XLink XML")?;
                period.clone_from(&linked_period);
            }
        }
        // The period_duration is specified either by the <Period> duration attribute, or by the
        // mediaPresentationDuration of the top-level MPD node.
        let mut period_duration_secs: f64 = 0.0;
        if let Some(d) = mpd.mediaPresentationDuration {
            period_duration_secs = d.as_secs_f64();
        }
        if let Some(d) = &period.duration {
            period_duration_secs = d.as_secs_f64();
        }
        let mut base_url = toplevel_base_url.clone();
        // A BaseURL could be specified for each Period
        if let Some(bu) = &period.BaseURL {
            if is_absolute_url(&bu.base) {
                base_url = Url::parse(&bu.base).context("parsing BaseURL")?;
            } else {
                base_url = base_url.join(&bu.base).context("joining with BaseURL")?;
            }
        }
        // Handle the AdaptationSet with audio content. Note that some streams don't separate out
        // audio and video streams.
        let maybe_audio_adaptation = match &period.adaptations {
            Some(a) => a.iter().find(is_audio_adaptation),
            None => None,
        };
        // TODO: we could perhaps factor out the treatment of the audio adaptation and video
        // adaptation into a common handle_adaptation() function
        if let Some(period_audio) = maybe_audio_adaptation {
            let mut audio = period_audio.clone();
            // Resolve a possible xlink:href on the AdaptationSet
            if let Some(href) = &audio.href {
                if fetchable_xlink_href(href) {
                    let xlink_url;
                    if is_absolute_url(href) {
                        xlink_url = Url::parse(href).context("parsing XLink URL on AdaptationSet")?;
                    } else {
                        // Note that we are joining against the original/redirected URL for the MPD, and
                        // not against the currently scoped BaseURL
                        xlink_url = redirected_url.join(href).context("joining with XLink URL on AdaptationSet")?;
                    }
                    let xml = client.get(xlink_url)
                        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                        .header("Accept-language", "en-US,en")
                        .send()
                        .context("fetching XLink URL for AdaptationSet")?
                        .text()
                        .context("resolving XLink on AdaptationSet element")?;
                    let linked_adaptation: AdaptationSet = quick_xml::de::from_str(&xml)
                        .context("parsing XML for AdaptationSet XLink")?;
                    audio.clone_from(&linked_adaptation);
                }
            }
            // The AdaptationSet may have a BaseURL (eg the test BBC streams). We use a local variable
            // to make sure we don't "corrupt" the base_url for the video segments.
            let mut base_url = base_url.clone();
            if let Some(bu) = &audio.BaseURL {
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base).context("parsing BaseURL")?;
                } else {
                    base_url = base_url.join(&bu.base).context("joining with BaseURL")?;
                }
            }
            // Start by resolving any xlink:href elements on Representation nodes, which we need to
            // do before the selection based on the @bandwidth attribute below.
            let mut representations = Vec::<Representation>::new();
            if let Some(reps) = audio.representations {
                for r in reps.iter() {
                    if let Some(href) = &r.href {
                        if fetchable_xlink_href(href) {
                            let xlink_url;
                            if is_absolute_url(href) {
                                xlink_url = Url::parse(href).context("parsing XLink URL for Representation")?;
                            } else {
                                xlink_url = redirected_url.join(href)
                                    .context("joining with XLink URL for Representation")?;
                            }
                            let xml = client.get(xlink_url)
                                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                                .header("Accept-language", "en-US,en")
                                .send()?
                                .text()
                                .context("resolving XLink on Representation element")?;
                            let linked_representation: Representation = quick_xml::de::from_str(&xml)
                                .context("parsing XLink XML for Representation")?;
                            representations.push(linked_representation);
                        }
                    } else {
                        representations.push(r.clone());
                    }
                }
            }
            let maybe_audio_repr;
            if downloader.quality_preference == QualityPreference::Lowest {
                maybe_audio_repr = representations.iter()
                    .min_by_key(|x| x.bandwidth.unwrap_or(1_000_000_000))
                    .context("finding min bandwidth audio representation");
            } else {
                maybe_audio_repr = representations.iter()
                    .max_by_key(|x| x.bandwidth.unwrap_or(0))
                    .context("finding max bandwidth audio representation");
            }
            if let Ok(audio_repr) = maybe_audio_repr {
                if downloader.verbosity > 0 {
                    if let Some(bw) = audio_repr.bandwidth {
                        println!("Selected audio representation with bandwidth {}", bw);
                    }
                }
                // the Representation may have a BaseURL
                let mut base_url = base_url;
                if let Some(bu) = &audio_repr.BaseURL {
                    if is_absolute_url(&bu.base) {
                        base_url = Url::parse(&bu.base).context("parsing BaseURL")?;
                    } else {
                        base_url = base_url.join(&bu.base).context("joining with BaseURL")?;
                    }
                }
                let mut opt_init: Option<String> = None;
                let mut opt_media: Option<String> = None;
                let mut opt_duration: Option<f64> = None;
                let mut timescale = 1;
                let mut start_number = 0;
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
                        opt_duration = Some(d as f64);
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
                    None => return Err(anyhow!("Missing @id on Representation node")),
                };
                let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
                if let Some(b) = &audio_repr.bandwidth {
                    dict.insert("Bandwidth", b.to_string());
                }
                // Now the 6 possible addressing modes: SegmentBase@indexRange, SegmentList, SegmentTimeline,
                // SegmentTemplate@duration, SegmentTemplate@index
                if let Some(sb) = &audio_repr.SegmentBase {
                    // (1) SegmentBase@indexRange addressing mode
                    if downloader.verbosity > 1 {
                        println!("Using SegmentBase@indexRange addressing mode for audio representation");
                    }
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path).context("parsing sourceURL")?;
                            } else {
                                init_url = base_url.join(&path).context("joining with sourceURL")?;
                            }
                            audio_segment_urls.push(init_url);
                        }
                    }
                    // TODO: properly handle indexRange attribute
                    audio_segment_urls.push(base_url.clone());
                } else if let Some(sl) = &audio_repr.SegmentList {
                    // (2) SegmentList addressing mode
                    if downloader.verbosity > 1 {
                        println!("Using SegmentList addressing mode for audio representation");
                    }
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path).context("parsing sourceURL")?;
                            } else {
                                init_url = base_url.join(&path).context("joining with sourceURL")?;
                            }
                            audio_segment_urls.push(init_url);
                        } else {
                            audio_segment_urls.push(base_url.clone());
                        }
                    }
                    for su in sl.segment_urls.iter() {
                        if let Some(m) = &su.media {
                            let segment = base_url.join(m).context("joining media with baseURL")?;
                            audio_segment_urls.push(segment);
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
                        // (3) SegmentTemplate with SegmentTimeline addressing mode
                        if downloader.verbosity > 1 {
                            println!("Using SegmentTemplate+SegmentTimeline addressing mode for audio representation");
                        }
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            let url = base_url.join(&path).context("joining init with baseURL")?;
                            audio_segment_urls.push(url);
                        }
                        if let Some(media) = opt_media {
                            let audio_path = resolve_url_template(&media, &dict);
                            let mut segment_time = 0;
                            let mut segment_duration;
                            let mut number = start_number;
                            for s in &stl.segments {
                                // the URLTemplate may be based on $Time$, or on $Number$
                                let dict = HashMap::from([("Time", segment_time.to_string()),
                                                          ("Number", number.to_string())]);
                                let path = resolve_url_template(&audio_path, &dict);
                                let u = base_url.join(&path).context("joining media with baseURL")?;
                                audio_segment_urls.push(u);
                                number += 1;
                                if let Some(t) = s.t {
                                    segment_time = t;
                                }
                                segment_duration = s.d;
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
                                        let u = base_url.join(&path).context("joining media with baseURL")?;
                                        audio_segment_urls.push(u);
                                        number += 1;
                                    }
                                }
                                segment_time += segment_duration;
                            }
                        } else {
                            return Err(anyhow!("SegmentTimeline without a media attribute"));
                        }
                    } else { // no SegmentTimeline element
                        // (4) SegmentTemplate@duration addressing mode or (5) SegmentTemplate@index addressing mode
                        if downloader.verbosity > 1 {
                            println!("Using SegmentTemplate addressing mode for audio representation");
                        }
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            let u = base_url.join(&path).context("joining init with baseURL")?;
                            audio_segment_urls.push(u);
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
                                segment_duration = std as f64 / timescale as f64;
                            }
                            if segment_duration < 0.0 {
                                return Err(anyhow!("Audio representation is missing SegmentTemplate @duration attribute"));
                            }
                            let total_number: u64 = (period_duration_secs / segment_duration).ceil() as u64;
                            let mut number = start_number;
                            for _ in 1..=total_number {
                                let dict = HashMap::from([("Number", number.to_string())]);
                                let path = resolve_url_template(&audio_path, &dict);
                                let segment_uri = base_url.join(&path).context("joining media with baseURL")?;
                                audio_segment_urls.push(segment_uri);
                                number += 1;
                            }
                        }
                    }
                } else {
                    return Err(anyhow!("Need either a SegmentBase or a SegmentTemplate node"));
                }
            }
        }

        // Handle the AdaptationSet which contains video content
        let maybe_video_adaptation = period.adaptations.as_ref()
            .and_then(|a| a.iter().find(is_video_adaptation));
        if let Some(period_video) = maybe_video_adaptation {
            let mut video = period_video.clone();
            // Resolve a possible xlink:href.
            if let Some(href) = &video.href {
                if fetchable_xlink_href(href) {
                    let xlink_url;
                    if is_absolute_url(href) {
                        xlink_url = Url::parse(href).context("parsing XLink URL")?;
                    } else {
                        // Note that we are joining against the original/redirected URL for the MPD, and
                        // not against the currently scoped BaseURL
                        xlink_url = redirected_url.join(href).context("joining XLink URL with BaseURL")?;
                    }
                    let xml = client.get(xlink_url)
                        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                        .header("Accept-language", "en-US,en")
                        .send()?
                        .text()
                        .context("resolving XLink on AdaptationSet element")?;
                    let linked_adaptation: AdaptationSet = quick_xml::de::from_str(&xml)
                        .context("parsing XML for XLink AdaptationSet")?;
                    video.clone_from(&linked_adaptation);
                }
            }
            // the AdaptationSet may have a BaseURL (eg the test BBC streams)
            if let Some(bu) = &video.BaseURL {
                if is_absolute_url(&bu.base) {
                    base_url = Url::parse(&bu.base).context("parsing BaseURL")?;
                } else {
                    base_url = base_url.join(&bu.base).context("joining base with BaseURL")?;
                }
            }
            // Start by resolving any xlink:href elements on Representation nodes, which we need to
            // do before the selection based on the @bandwidth attribute below.
            let mut representations = Vec::<Representation>::new();
            if let Some(reps) = video.representations {
                for r in reps.iter() {
                    if let Some(href) = &r.href {
                        if fetchable_xlink_href(href) {
                            let xlink_url;
                            if is_absolute_url(href) {
                                xlink_url = Url::parse(href)?;
                            } else {
                                xlink_url = redirected_url.join(href)?;
                            }
                            let xml = client.get(xlink_url)
                                .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
                                .header("Accept-language", "en-US,en")
                                .send()?
                                .text()
                                .context("resolving XLink on Representation element")?;
                            let linked_representation: Representation = quick_xml::de::from_str(&xml)
                                .context("parsing XLink XML for Representation")?;
                            representations.push(linked_representation);
                        }
                    } else {
                        representations.push(r.clone());
                    }
                }
            }
            let maybe_video_repr;
            if downloader.quality_preference == QualityPreference::Lowest {
                maybe_video_repr = representations.iter()
                    .min_by_key(|x| x.bandwidth.unwrap_or(1_000_000_000))
                    .context("finding video representation with min bandwidth");
            } else {
                maybe_video_repr = representations.iter()
                    .max_by_key(|x| x.bandwidth.unwrap_or(0))
                    .context("finding video representation with max bandwidth ");
            }
            if let Ok(video_repr) = maybe_video_repr {
                if downloader.verbosity > 0 {
                    if let Some(bw) = video_repr.bandwidth {
                        println!("Selected video representation with bandwidth {}", bw);
                    }
                }
                if let Some(bu) = &video_repr.BaseURL {
                    if is_absolute_url(&bu.base) {
                        base_url = Url::parse(&bu.base).context("parsing BaseURL")?;
                    } else {
                        base_url = base_url.join(&bu.base)
                            .context("joining base with BaseURL")?;
                    }
                }
                let rid = match &video_repr.id {
                    Some(id) => id,
                    None => return Err(anyhow!("Missing @id on Representation node")),
                };
                let mut dict = HashMap::from([("RepresentationID", rid.to_string())]);
                if let Some(b) = &video_repr.bandwidth {
                    dict.insert("Bandwidth", b.to_string());
                }
                let mut opt_init: Option<String> = None;
                let mut opt_media: Option<String> = None;
                let mut opt_duration: Option<f64> = None;
                let mut timescale = 1;
                let mut start_number = 0;
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
                        opt_duration = Some(d as f64);
                    }
                    if let Some(ts) = st.timescale {
                        timescale = ts;
                    }
                    if let Some(s) = st.startNumber {
                        start_number = s;
                    }
                }
                // Now the 6 possible addressing modes: SegmentBase@indexRange, SegmentList,
                // SegmentTemplate+SegmentTimeline, SegmentTemplate@duration, SegmentTemplate@index
                if let Some(sb) = &video_repr.SegmentBase {
                    // (1) SegmentBase@indexRange addressing mode
                    if downloader.verbosity > 1 {
                        println!("Using SegmentBase@indexRange addressing mode for video representation");
                    }
                    if let Some(init) = &sb.initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path).context("parsing sourceURL")?;
                            } else {
                                init_url = base_url.join(&path).context("joining sourceURL with BaseURL")?;
                            }
                            video_segment_urls.push(init_url);
                        }
                    }
                    // TODO: properly handle indexRange attribute
                    video_segment_urls.push(base_url.clone());
                } else if let Some(sl) = &video_repr.SegmentList {
                    // (2) SegmentList addressing mode
                    if downloader.verbosity > 1 {
                        println!("Using SegmentList addressing mode for video representation");
                    }
                    if let Some(init) = &sl.Initialization {
                        if let Some(su) = &init.sourceURL {
                            let path = resolve_url_template(su, &dict);
                            let init_url;
                            if is_absolute_url(&path) {
                                init_url = Url::parse(&path).context("parsing sourceURL")?;
                            } else {
                                init_url = base_url.join(&path).context("joining sourceURL with BaseURL")?;
                            }
                            video_segment_urls.push(init_url);
                        } else {
                            video_segment_urls.push(base_url.clone());
                        }
                    }
                    for su in sl.segment_urls.iter() {
                        if let Some(m) = &su.media {
                            let segment = base_url.join(m)
                                .context("joining media with BaseURL")?;
                            video_segment_urls.push(segment);
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
                        // (3) SegmentTemplate with SegmentTimeline addressing mode
                        if downloader.verbosity > 1 {
                            println!("Using SegmentTemplate+SegmentTimeline addressing mode for video representation");
                        }
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            let u = base_url.join(&path).context("joining init with BaseURL")?;
                            video_segment_urls.push(u);
                        }
                        if let Some(media) = opt_media {
                            let video_path = resolve_url_template(&media, &dict);
                            let mut segment_time = 0;
                            let mut segment_duration;
                            let mut number = start_number;
                            for s in &stl.segments {
                                // the URLTemplate may be based on $Time$, or on $Number$
                                let dict = HashMap::from([("Time", segment_time.to_string()),
                                                          ("Number", number.to_string())]);
                                let path = resolve_url_template(&video_path, &dict);
                                let u = base_url.join(&path).context("joining media with BaseURL")?;
                                video_segment_urls.push(u);
                                number += 1;
                                if let Some(t) = s.t {
                                    segment_time = t;
                                }
                                segment_duration = s.d;
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
                                        let u = base_url.join(&path).context("joiing media with BaseURL")?;
                                        video_segment_urls.push(u);
                                        number += 1;
                                    }
                                }
                                segment_time += segment_duration;
                            }
                        } else {
                            return Err(anyhow!("SegmentTimeline without a media attribute"));
                        }
                    } else { // no SegmentTimeline element
                        // (4) SegmentTemplate@duration addressing mode or (5) SegmentTemplate@index addressing mode
                        if downloader.verbosity > 1 {
                            println!("Using SegmentTemplate addressing mode for video representation");
                        }
                        if let Some(init) = opt_init {
                            let path = resolve_url_template(&init, &dict);
                            let u = base_url.join(&path).context("joining init with BaseURL")?;
                            video_segment_urls.push(u);
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
                                segment_duration = std as f64 / timescale as f64;
                            }
                            if segment_duration < 0.0 {
                                return Err(anyhow!("Video representation is missing SegmentTemplate @duration attribute"));
                            }
                            let total_number: u64 = (period_duration_secs / segment_duration).ceil() as u64;
                            let mut number = start_number;
                            for _ in 1..=total_number {
                                let dict = HashMap::from([("Number", number.to_string())]);
                                let path = resolve_url_template(&video_path, &dict);
                                let segment_uri = base_url.join(&path).context("joining media with BaseURL")?;
                                video_segment_urls.push(segment_uri);
                                number += 1;
                            }
                        }
                    }
                } else {
                    return Err(anyhow!("Need either a SegmentBase or a SegmentTemplate node"));
                }
            } else {
                return Err(anyhow!("Couldn't find lowest bandwidth video stream in DASH manifest"));
            }
        }
    }
    // Concatenate the audio segments to a file on disk.
    // In DASH, the first segment contains necessary headers to generate a valid MP4 file,
    // so we should always abort if the first segment cannot be fetched. However, we could
    // tolerate loss of subsequent segments.
    let mut seen_urls: HashMap<Url, bool> = HashMap::new();
    if downloader.verbosity > 0 {
        println!("Preparing to fetch {} audio and {} video segments",
                 audio_segment_urls.len(),
                 video_segment_urls.len());
    }
    // The additional +2 is for our initial .mpd fetch action and final muxing action
    let segment_count = audio_segment_urls.len() + video_segment_urls.len() + 2;
    let mut segment_counter = 0;
    for url in &audio_segment_urls {
        // Update any ProgressObservers
        segment_counter += 1;
        let progress_percent = (100.0 * segment_counter as f32 / segment_count as f32).ceil() as u32;
        for observer in &downloader.progress_observers {
            observer.update(progress_percent, "Fetching audio segments");
        }
        // Don't download repeated URLs multiple times: they may be caused by a MediaRange parameter
        // on the SegmentURL, which we are currently not handling correctly
        // Example here
        // http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd
        if let Entry::Vacant(e) = seen_urls.entry(url.clone()) {
            e.insert(true);
            if url.scheme() == "data" {
                return Err(anyhow!("data URLs currently unsupported"));
            } else {
                // We could download these segments in parallel using reqwest in async mode,
                // though that might upset some servers.
                let backoff = ExponentialBackoff::default();
                let fetch = || {
                    client.get(url.clone())
                    // Don't use only "audio/*" in Accept header because some web servers
                    // (eg. media.axprod.net) are misconfigured and reject requests for
                    // valid audio content (eg .m4s)
                        .header("Accept", "audio/*;q=0.9,*/*;q=0.5")
                        .header("Referer", redirected_url.to_string())
                        .send()?
                        .bytes()
                        .map_err(categorize_reqwest_error)
                };
                let dash_bytes = retry(backoff, fetch)
                    .context("fetching DASH audio segment")?;
                if downloader.verbosity > 2 {
                    println!("Audio segment {} -> {} octets", url, dash_bytes.len());
                }
                if let Err(e) = tmpfile_audio.write_all(&dash_bytes) {
                    log::error!("Unable to write DASH audio data: {:?}", e);
                    return Err(anyhow!("Unable to write DASH audio data: {:?}", e));
                }
                have_audio = true;
            }
        }
    }
    tmpfile_audio.flush().map_err(|e| {
        log::error!("Couldn't flush DASH audio file to disk: {:?}", e);
        e
    })?;
    if let Ok(metadata) = fs::metadata(tmppath_audio.clone()) {
        if downloader.verbosity > 1 {
            println!("Wrote {:.1}MB to DASH audio stream", metadata.len() as f64 / (1024.0 * 1024.0));
        }
    }
    // Now fetch the video segments and concatenate them to the video file path
    for url in &video_segment_urls {
        // Update any ProgressObservers
        segment_counter += 1;
        let progress_percent = (100.0 * segment_counter as f32 / segment_count as f32).ceil() as u32;
        for observer in &downloader.progress_observers {
            observer.update(progress_percent, "Fetching video segments");
        }
        // Don't download repeated URLs multiple times: they may be caused by a MediaRange parameter
        // on the SegmentURL, which we are currently not handling correctly
        // Example here
        // http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd
        if let Entry::Vacant(e) = seen_urls.entry(url.clone()) {
            e.insert(true);
            let backoff = ExponentialBackoff::default();
            let fetch = || {
                client.get(url.clone())
                    .header("Accept", "video/*")
                    .header("Referer", redirected_url.to_string())
                    .send()?
                    .bytes()
                    .map_err(categorize_reqwest_error)
            };
            let dash_bytes = retry_notify(backoff, fetch, notify_transient)
                .context("fetching DASH video segment")?;
            if downloader.verbosity > 2 {
                println!("Video segment {} -> {} octets", url, dash_bytes.len());
            }
            if let Err(e) = tmpfile_video.write_all(&dash_bytes) {
                return Err(anyhow!("Unable to write video data: {:?}", e));
            }
            have_video = true;
        }
    }
    tmpfile_video.flush().map_err(|e| {
        log::error!("Couldn't flush video file to disk: {:?}", e);
        e
    })?;
    if let Ok(metadata) = fs::metadata(tmppath_video.clone()) {
        if downloader.verbosity > 1 {
            println!("Wrote {:.1}MB to DASH video file", metadata.len() as f64 / (1024.0 * 1024.0));
        }
    }
    for observer in &downloader.progress_observers {
        observer.update(99, "Muxing audio and video");
    }
    // Our final output file is either a mux of the audio and video streams, if both are present, or just
    // the audio stream, or just the video stream.
    if have_audio && have_video {
        if downloader.verbosity > 1 {
            println!("Muxing audio and video streams");
        }
        mux_audio_video(&tmppath_audio, &tmppath_video, output_path)
            .context("muxing audio and video streams")?;
        if fs::remove_file(tmppath_audio).is_err() {
            log::info!("Failed to delete temporary file for audio segments");
        }
        if fs::remove_file(tmppath_video).is_err() {
            log::info!("Failed to delete temporary file for video segments");
        }
    } else if have_audio {
        // Copy the downloaded audio segments to the output file. We don't use fs::rename() because
        // it might fail if temporary files and our output are on different filesystems.
        let tmpfile_audio = File::open(&tmppath_audio)
            .context("opening temporary audio output file")?;
        let mut audio = BufReader::new(tmpfile_audio);
        let output_file = File::create(output_path)
            .context("creating output file for video")?;
        let mut sink = BufWriter::new(output_file);
        io::copy(&mut audio, &mut sink)
            .context("copying from audio stream to output file")?;
    } else if have_video {
        let tmpfile_video = File::open(&tmppath_video)
            .context("opening temporary video output file")?;
        let mut video = BufReader::new(tmpfile_video);
        let output_file = File::create(output_path)
            .context("creating output file for video")?;
        let mut sink = BufWriter::new(output_file);
        io::copy(&mut video, &mut sink)
            .context("copying from video stream to output file")?;
    } else {
        return Err(anyhow!("No audio or video streams found"));
    }
    if downloader.verbosity > 1 {
        if let Ok(metadata) = fs::metadata(output_path) {
            println!("Wrote {:.1}MB to media file", metadata.len() as f64 / (1024.0 * 1024.0));
        }
    }
    // As per https://www.freedesktop.org/wiki/CommonExtendedAttributes/, set extended filesystem
    // attributes indicating metadata such as the origin URL, title, source and copyright, if
    // specified in the MPD manifest. This functionality is only active on platforms where the xattr
    // crate supports extended attributes (currently Linux, MacOS, FreeBSD, and NetBSD); on
    // unsupported Unix platforms it's a no-op. On other non-Unix platforms the crate doesn't build.
    let origin_url = Url::parse(&downloader.mpd_url)
        .context("Can't parse MPD URL")?;
    // Don't record the origin URL if it contains sensitive information such as passwords
    #[allow(clippy::collapsible_if)]
    if origin_url.username().is_empty() && origin_url.password().is_none() {
        #[cfg(target_family = "unix")]
        if xattr::set(&output_path, "user.xdg.origin.url", downloader.mpd_url.as_bytes()).is_err() {
            log::info!("Failed to set user.xdg.origin.url xattr on output file");
        }
    }
    if let Some(pi) = mpd.ProgramInformation {
        if let Some(t) = pi.Title {
            // this if is empty on non-Unix
            #[allow(unused_variables)]
            if let Some(tc) = t.content {
                #[cfg(target_family = "unix")]
                if xattr::set(&output_path, "user.dublincore.title", tc.as_bytes()).is_err() {
                    log::info!("Failed to set user.dublincore.title xattr on output file");
                }
            }
        }
        if let Some(source) = pi.Source {
            #[allow(unused_variables)]
            if let Some(sc) = source.content {
                #[cfg(target_family = "unix")]
                if xattr::set(&output_path, "user.dublincore.source", sc.as_bytes()).is_err() {
                    log::info!("Failed to set user.dublincore.source xattr on output file");
                }
            }
        }
        if let Some(copyright) = pi.Copyright {
            #[allow(unused_variables)]
            if let Some(cc) = copyright.content {
                #[cfg(target_family = "unix")]
                if xattr::set(&output_path, "user.dublincore.rights", cc.as_bytes()).is_err() {
                    log::info!("Failed to set user.dublincore.rights xattr on output file");
                }
            }
        }
    }
    for observer in &downloader.progress_observers {
        observer.update(100, "Done");
    }
    Ok(())
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
