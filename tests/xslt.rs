//! Dedicated tests for XSLT stylesheet processing
//
// To run only these tests while enabling printing to stdout/stderr
//
//    cargo test --test xslt -- --show-output


pub mod common;
use std::fs;
use std::env;
use std::net::SocketAddr;
use std::time::Duration;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use axum::{routing::get, Router};
use axum::extract::{Path, State};
use axum::response::{Response, IntoResponse};
use axum::http::{header, StatusCode};
use axum::body::Body;
use axum_server::{Handle, bind};
use ffprobe::ffprobe;
use file_format::FileFormat;
use pretty_assertions::assert_eq;
use dash_mpd::fetch::DashDownloader;
use anyhow::{Context, Result};
use common::{check_file_size_approx, generate_minimal_mp4, setup_logging};


#[derive(Debug, Default)]
struct AppState {
    count_init: AtomicUsize,
    count_media: AtomicUsize,
}

impl AppState {
    fn new() -> AppState {
        AppState {
            count_init: AtomicUsize::new(0),
            count_media: AtomicUsize::new(0),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_xslt_rewrite_media() -> Result<()> {
    // State shared between the request handlers.
    let shared_state = Arc::new(AppState::new());

    async fn send_media(Path(segment): Path<String>, State(state): State<Arc<AppState>>) -> Response {
        if segment.eq("init.mp4") {
            state.count_init.fetch_add(1, Ordering::SeqCst);
        } else {
            state.count_media.fetch_add(1, Ordering::SeqCst);
        }
        let mp4 = generate_minimal_mp4();
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Body::from(mp4))
            .unwrap()
    }
    async fn send_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/plain")],
         format!("{} {}",
                 state.count_init.load(Ordering::Relaxed),
                 state.count_media.load(Ordering::Relaxed)))
    }

    setup_logging();
    let mut mpd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    mpd.push("tests");
    mpd.push("fixtures");
    mpd.push("jurassic-compact-5975");
    mpd.set_extension("mpd");
    let xml = fs::read_to_string(mpd).unwrap();
    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml) }))
        .route("/media/{segment}", get(send_media))
        .route("/status", get(send_status))
        .with_state(shared_state);
    let server_handle: Handle<SocketAddr> = Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        bind("127.0.0.1:6668".parse().unwrap())
            .handle(backend_handle)
            .serve(app.into_make_service()).await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(1000)).await;
    // Check that the initial value of our request counter is zero.
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .context("creating HTTP client")?;
    let txt = client.get("http://localhost:6668/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("0 0"), "Expecting status 0 0, got {txt}");

    let mpd_url = "http://localhost:6668/mpd";
    let mut xslt = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt.push("tests");
    xslt.push("fixtures");
    xslt.push("rewrite-init-media-segments");
    xslt.set_extension("xslt");
    let v = env::temp_dir().join("xslt_video.mp4");
    DashDownloader::new(mpd_url)
        .best_quality()
        .with_http_client(client.clone())
        .with_xslt_stylesheet(xslt)
        .download_to(v.clone()).await
        .unwrap();
    // Check the total number of requested media segments corresponds to what we expect.
    let txt = client.get("http://localhost:6668/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("1 927"), "Expecting 1 927, got {txt}");
    server_handle.shutdown();
    Ok(())
}



// This MPD manifest includes two AdaptationSets, one for the video streams and one for the audio
// stream. The rewrite-drop-audio.xslt stylesheet rewrites the XML manifest to remove the audio
// AdaptationSet. We check that the resulting media container only contains a video track.
#[tokio::test]
async fn test_xslt_drop_audio() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let out = env::temp_dir().join("envivio-dropped-audio.mp4");
    let mut xslt = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt.push("tests");
    xslt.push("fixtures");
    xslt.push("rewrite-drop-audio");
    xslt.set_extension("xslt");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .with_xslt_stylesheet(xslt)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 11_005_923);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert_eq!(video.width, Some(320));
}


// This XSLT stylesheet replaces @media and @initialization attributes to point to a beloved media
// segment.
#[tokio::test]
async fn test_xslt_rick() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "https://dash.akamaized.net/dash264/TestCases/4b/qualcomm/1/ED_OnDemand_5SecSeg_Subtitles.mpd";
    let out = env::temp_dir().join("ricked.mp4");
    let mut xslt = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt.push("tests");
    xslt.push("fixtures");
    xslt.push("rewrite-rickroll");
    xslt.set_extension("xslt");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        // This manifest is using SegmentBase@indexRange addressing. We rewrite all the BaseURL
        // elements to point to a different media container from the original, which means that the
        // byte ranges are no longer valid. Disable use of the sidx index range information to make
        // this test work.
        .use_index_range(false)
        .with_xslt_stylesheet(xslt)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 7_082_395);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 1);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert_eq!(video.width, Some(320));
}


#[tokio::test]
async fn test_xslt_multiple_stylesheets() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let mpd_url = "http://dash.edgesuite.net/envivio/dashpr/clear/Manifest.mpd";
    let out = env::temp_dir().join("ricked-cleaned.mp4");
    let mut xslt_rick = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt_rick.push("tests");
    xslt_rick.push("fixtures");
    xslt_rick.push("rewrite-rickroll");
    xslt_rick.set_extension("xslt");
    let mut xslt_clean = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt_clean.push("tests");
    xslt_clean.push("fixtures");
    xslt_clean.push("rewrite-drop-dai");
    xslt_clean.set_extension("xslt");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_xslt_stylesheet(xslt_rick)
        .with_xslt_stylesheet(xslt_clean)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 12_975_377);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    assert_eq!(video.width, Some(320));
}


// Note that the error message is structured differently on Unix and Microsoft Windows platforms.
#[tokio::test]
#[should_panic(expected = "xsltproc returned exit")]
async fn test_xslt_stylesheet_error() {
    let mpd_url = "https://dash.akamaized.net/akamai/test/index3-original.mpd";
    let out = env::temp_dir().join("unexist.mp4");
    let mut xslt = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt.push("tests");
    xslt.push("fixtures");
    xslt.push("rewrite-stylesheet-error");
    xslt.set_extension("xslt");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_xslt_stylesheet(xslt)
        .download_to(out.clone()).await
        .unwrap();
}


// https://github.com/Paligo/xee/blob/a767117c0e3e51b5bb6b8c37f2b8397df77f7117/xee-xslt-ast/src/staticeval.rs#L287
/*
#[tokio::test]
async fn test_xslt_xee () {
    use xot::{NameId, Node, Xot};
    use xee_xpath_compiler::{compile, context::Variables, sequence::Sequence};
    
    let xml = r#"
        <xsl:stylesheet xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="3.0">
            <xsl:param name="x" static="yes" select="'foo'"/>
        </xsl:stylesheet>
        "#;
    let mut xot = Xot::new();
    let (root, span_info) = xot.parse_with_span_info(xml).unwrap();
    let names = Names::new(&mut xot);
    let document_element = xot.document_element(root).unwrap();
    
    let name = xpath_ast::Name::name("x");
    let static_parameters = Variables::new();
    
    let mut state = State::new(xot, span_info, names);
    
    let mut xot = Xot::new();
    let variables =
        static_evaluate(&mut state, document_element, static_parameters, &mut xot).unwrap();
    assert_eq!(variables.len(), 1);
    
    assert_eq!(variables.get(&name), Some(&Item::from("foo").into()));
}
*/
