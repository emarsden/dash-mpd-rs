// Testing that the MPD.Location element is handled correctly.
//
//
// To run this test while enabling printing to stdout/stderr
//
//    cargo test --test mpd_location -- --show-output


use fs_err as fs;
use std::env;
use std::process::Command;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use axum::{routing::get, Router};
use axum::extract::State;
use axum::response::{Response, IntoResponse};
use axum::http::{header, StatusCode};
use axum::body::{Full, Bytes};
use dash_mpd::{MPD, Period, AdaptationSet, Representation, SegmentTemplate, Location};
use dash_mpd::fetch::DashDownloader;
use anyhow::{Context, Result};
use env_logger::Env;


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
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();

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
    let elsewhere = Location { url: "http://localhost:6666/relocated.mpd".to_string() };
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
    let xml1 = quick_xml::se::to_string(&orig_mpd)?;
    let xml2 = quick_xml::se::to_string(&relocated_mpd)?;

    // State shared between the request handlers. We are simply maintaining a counter of the number
    // of requests made, to check that each XLink reference has been resolved.
    let shared_state = Arc::new(AppState::new());


    // Useful ffmpeg recipes: https://github.com/videojs/http-streaming/blob/main/docs/creating-content.md
    // ffmpeg -y -f lavfi -i testsrc=size=10x10:rate=1 -vf hue=s=0 -t 1 -metadata title=foobles1 tiny.mp4
    async fn send_segment(State(state): State<Arc<AppState>>) -> Response<Full<Bytes>> {
        state.counter.fetch_add(1, Ordering::SeqCst);
        let tmp = env::temp_dir().join("segment.mp4");
        let ffmpeg = Command::new("ffmpeg")
            .args(["-f", "lavfi",
                   "-y",  // overwrite output file if it exists
                   "-i", "testsrc=size=10x10:rate=1",
                   "-vf", "hue=s=0",
                   "-t", "1",
                   tmp.to_str().unwrap()])
            .output()
            .expect("spawning ffmpeg");
        assert!(ffmpeg.status.success());
        let bytes = fs::read(tmp).unwrap();
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Full::from(bytes))
            .unwrap()
    }

    async fn send_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/plain")], format!("{}", state.counter.load(Ordering::Relaxed)))
    }

    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml1) }))
        .route("/relocated.mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml2) }))
        .route("/media/:id", get(send_segment))
        .route("/status", get(send_status))
        .with_state(shared_state);
    let backend = async move {
        axum::Server::bind(&"127.0.0.1:6666".parse().unwrap())
            .serve(app.into_make_service())
            .await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Check that the initial value of our request counter is zero.
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .context("creating HTTP client")?;
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("0"));

    let mpd_url = "http://localhost:6666/mpd";
    let r = env::temp_dir().join("relocated.mp4");
    DashDownloader::new(mpd_url)
        .best_quality()
        .verbosity(3)
        .with_http_client(client.clone())
        .download_to(r.clone()).await
        .unwrap();

    // Check the total number of requested media segments corresponds to what we expect. We expect
    // two requests for the init.mp4 segment because we are running in verbose mode, and the init
    // segment is fetched once just to extract and print the PSSH.
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("2"), "Expecting status=2, got {}", txt);

    Ok(())
}
