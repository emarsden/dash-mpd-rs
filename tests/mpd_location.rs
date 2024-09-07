// Testing that the MPD.Location element is handled correctly.
//
//
// To run this test while enabling printing to stdout/stderr
//
//    cargo test --test mpd_location -- --show-output


pub mod common;
use std::env;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use axum::{routing::get, Router};
use axum::extract::State;
use axum::response::{Response, IntoResponse};
use axum::http::{header, StatusCode};
use axum::body::Body;
use dash_mpd::{MPD, Period, AdaptationSet, Representation, SegmentTemplate, Location};
use dash_mpd::fetch::DashDownloader;
use anyhow::{Context, Result};
use common::{generate_minimal_mp4, setup_logging};


#[derive(Debug, Default)]
struct AppState {
    counter: AtomicUsize,
}

impl AppState {
    fn new() -> AppState {
        AppState { counter: AtomicUsize::new(0) }
    }
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mpd_location() -> Result<()> {
    let segment_template1 = SegmentTemplate {
        initialization: Some("/media/init.mp4".to_string()),
        ..Default::default()
    };
    let rep1 = Representation {
        id: Some("1".to_string()),
        mimeType: Some("video/mp4".to_string()),
        codecs: Some("avc1.640028".to_string()),
        width: Some(1920),
        height: Some(800),
        bandwidth: Some(1980081),
        SegmentTemplate: Some(segment_template1),
        ..Default::default()
    };
    let adap = AdaptationSet {
        id: Some("1".to_string()),
        contentType: Some("video".to_string()),
        representations: vec!(rep1),
        ..Default::default()
    };
    let period = Period {
        id: Some("p1".to_string()),
        duration: Some(Duration::new(5, 0)),
        adaptations: vec!(adap),
        ..Default::default()
    };
    let elsewhere = Location { url: "http://localhost:6667/relocated.mpd".to_string() };
    let orig_mpd = MPD {
        mpdtype: Some("static".to_string()),
        locations: vec!(elsewhere),
        periods: vec!(),
        ..Default::default()
    };
    let relocated_mpd = MPD {
        mpdtype: Some("static".to_string()),
        periods: vec!(period),
        ..Default::default()
    };
    let xml1 = orig_mpd.to_string();
    let xml2 = relocated_mpd.to_string();

    // State shared between the request handlers. We are simply maintaining a counter of the number
    // of requests made, to check that each XLink reference has been resolved.
    let shared_state = Arc::new(AppState::new());


    async fn send_segment(State(state): State<Arc<AppState>>) -> Response {
        state.counter.fetch_add(1, Ordering::SeqCst);
        let bytes = generate_minimal_mp4();
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Body::from(bytes))
            .unwrap()
    }

    async fn send_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/plain")], format!("{}", state.counter.load(Ordering::Relaxed)))
    }

    setup_logging();
    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml1) }))
        .route("/relocated.mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml2) }))
        .route("/media/:id", get(send_segment))
        .route("/status", get(send_status))
        .with_state(shared_state);
    let server_handle = hyper_serve::Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        hyper_serve::bind("127.0.0.1:6667".parse().unwrap())
            .handle(backend_handle)
            .serve(app.into_make_service()).await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Check that the initial value of our request counter is zero.
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .context("creating HTTP client")?;
    let txt = client.get("http://localhost:6667/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("0"), "Expecting 0 segment requests, got {txt}");

    let r = env::temp_dir().join("relocated.mp4");
    DashDownloader::new("http://localhost:6667/mpd")
        .best_quality()
        .verbosity(3)
        .with_http_client(client.clone())
        .download_to(r.clone()).await
        .unwrap();

    // Check the total number of requested media segments corresponds to what we expect. We expect
    // two requests for the init.mp4 segment because we are running in verbose mode, and the init
    // segment is fetched once just to extract and print the PSSH.
    let txt = client.get("http://localhost:6667/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("2"), "Expecting status=2, got {txt}");
    server_handle.shutdown();

    Ok(())
}
