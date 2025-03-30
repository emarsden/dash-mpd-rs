// Testing support for with_base_url() on DashDownloader
//
//
// To run tests while enabling printing to stdout/stderr
//
//    RUST_LOG=info cargo test --test base_url -- --show-output
//
// What happens in this test:
//
//   - Start an axum HTTP server that serves the manifest and our media segments.
//
//   - Fetch the associated media content using DashDownloader with a base_url different from that
//   present in the manifest, and check that the externally specified Base URL overrides that in the
//   manifest.


pub mod common;
use fs_err as fs;
use std::env;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use axum::{routing::get, Router};
use axum::extract::State;
use axum::response::{Response, IntoResponse};
use axum::http::header;
use axum::body::Body;
use dash_mpd::{MPD, Period, AdaptationSet, Representation, SegmentList, SegmentURL, BaseURL};
use dash_mpd::fetch::DashDownloader;
use anyhow::{Context, Result};
use common::{generate_minimal_mp4, setup_logging};


#[derive(Debug, Default)]
struct AppState {
    original_counter: AtomicUsize,
    updated_counter: AtomicUsize,
}

impl AppState {
    fn new() -> AppState {
        AppState {
            original_counter: AtomicUsize::new(0),
            updated_counter: AtomicUsize::new(0),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_base_url() -> Result<()> {
    // State shared between the request handlers. We are simply maintaining a counter of the number
    // of requests for the original segment path and for the updated segment path.
    let shared_state = Arc::new(AppState::new());

    async fn send_mpd() -> impl IntoResponse {
        let segment1 = SegmentURL {
            media: Some(String::from("segment1.m4v")),
            ..Default::default()
        };
        let segment2 = SegmentURL {
            media: Some(String::from("segment2.m4v")),
            ..Default::default()
        };
        let segment3 = SegmentURL {
            media: Some(String::from("segment3.m4v")),
            ..Default::default()
        };
        let segment_list = SegmentList {
            timescale: Some(1000),
            segment_urls: vec!(segment1, segment2, segment3),
            ..Default::default()
        };
        let rep = Representation {
            id: Some("1".to_string()),
            mimeType: Some("video/mp4".to_string()),
            codecs: Some("avc1.640028".to_string()),
            width: Some(1920),
            height: Some(800),
            bandwidth: Some(1980081),
            SegmentList: Some(segment_list),
            ..Default::default()
        };
        let adapt = AdaptationSet {
            id: Some("1".to_string()),
            contentType: Some("video".to_string()),
            representations: vec!(rep),
            ..Default::default()
        };
        let period = Period {
            id: Some("1".to_string()),
            duration: Some(Duration::new(5, 0)),
            adaptations: vec!(adapt.clone()),
            ..Default::default()
        };
        let original_base = BaseURL {
            base: String::from("/original/"),
            ..Default::default()
        };
        let mpd = MPD {
            xmlns: Some("urn:mpeg:dash:schema:mpd:2011".to_string()),
            mpdtype: Some("static".to_string()),
            base_url: vec!(original_base),
            periods: vec!(period),
            ..Default::default()
        };
        let xml = mpd.to_string();
        ([(header::CONTENT_TYPE, "application/dash+xml")], xml)
    }

    async fn send_mp4_original(State(state): State<Arc<AppState>>) -> Response {
        state.original_counter.fetch_add(1, Ordering::SeqCst);
        let data = generate_minimal_mp4();
        Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Body::from(data))
            .unwrap()
    }

    async fn send_mp4_updated(State(state): State<Arc<AppState>>) -> Response {
        state.updated_counter.fetch_add(1, Ordering::SeqCst);
        let data = generate_minimal_mp4();
        Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Body::from(data))
            .unwrap()
    }

    async fn send_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/plain")],
         format!("{} {}",
                 state.original_counter.load(Ordering::Relaxed),
                 state.updated_counter.load(Ordering::Relaxed)))
    }

    setup_logging();
    let app = Router::new()
        .route("/mpd", get(send_mpd))
        .route("/original/{seg}", get(send_mp4_original))
        .route("/updated/{seg}", get(send_mp4_updated))
        .route("/status", get(send_status))
        .with_state(shared_state);
    let server_handle = hyper_serve::Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        hyper_serve::bind("127.0.0.1:6666".parse().unwrap())
            .handle(backend_handle)
            .serve(app.into_make_service())
            .await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(1000)).await;
    // Check that the initial value of our request counters is zero.
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .context("creating HTTP client")?;
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("0 0"), "Expecting 0 original and 0 updated segment requests, got {txt}");

    let outpath = env::temp_dir().join("base_url.mp4");
    DashDownloader::new("http://localhost:6666/mpd")
        .with_base_url(String::from("http://localhost:6666/updated/"))
        .verbosity(2)
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(outpath).is_ok());
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("0 3"), "Expecting 0 original and 3 updated segment requests, got {txt}");
    server_handle.shutdown();

    Ok(())
}
