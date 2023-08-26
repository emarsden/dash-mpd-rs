// Testing that we select the right streams corresponding to user preference ranking.
//
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test stream_ranking -- --show-output
//
// What happens in this test:
//
//   - Start an axum HTTP server that serves an MPD manifest which includes video representations
//   with different bandwidths and resolutions.
//
//   - For different quality preferences (best_quality, intermediate_quality etc.) and for different
//   preferred video widths and heights, check that the media returned corresponds to that
//   requested.


use fs_err as fs;
use std::env;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use axum::{routing::get, Router};
use axum::extract::{State, Path};
use axum::response::{Response, IntoResponse};
use axum::http::{header, StatusCode};
use axum::body::{Full, Bytes};
use dash_mpd::{MPD, Period, AdaptationSet, Representation, SegmentTemplate};
use dash_mpd::fetch::DashDownloader;
use anyhow::{Context, Result};


#[derive(Debug, Default)]
struct AppState {
    counter: AtomicUsize,
}

impl AppState {
    fn new() -> AppState {
        AppState { counter: AtomicUsize::new(0) }
    }
}

const QUALITY_BEST: u8 = 55;
const QUALITY_INTERMEDIATE: u8 = 66;
const QUALITY_WORST: u8 = 77;


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_preference_ranking() -> Result<()> {
    let segment_template1 = SegmentTemplate {
        initialization: Some(format!("/media/{QUALITY_BEST}")),
        ..Default::default()
    };
    let segment_template2 = SegmentTemplate {
        initialization: Some(format!("/media/{QUALITY_INTERMEDIATE}")),
        ..Default::default()
    };
    let segment_template3 = SegmentTemplate {
        initialization: Some(format!("/media/{QUALITY_WORST}")),
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
    let rep2 = Representation {
        id: Some("2".to_string()),
        mimeType: Some("video/mp4".to_string()),
        width: Some(600),
        height: Some(400),
        bandwidth: Some(23000),
        SegmentTemplate: Some(segment_template2),
        ..Default::default()
    };
    let rep3 = Representation {
        id: Some("3".to_string()),
        mimeType: Some("video/mp4".to_string()),
        width: Some(240),
        height: Some(120),
        bandwidth: Some(1500),
        SegmentTemplate: Some(segment_template3),
        ..Default::default()
    };
    let adap = AdaptationSet {
        id: Some("1".to_string()),
        contentType: Some("video".to_string()),
        representations: vec!(rep1, rep2, rep3),
        ..Default::default()
    };
    let period = Period {
        id: Some("p1".to_string()),
        duration: Some(Duration::new(5, 0)),
        adaptations: vec!(adap),
        ..Default::default()
    };
    let mpd = MPD {
        mpdtype: Some("static".to_string()),
        xlink: Some("http://www.w3.org/1999/xlink".to_string()),
        periods: vec!(period),
        ..Default::default()
    };
    let xml = quick_xml::se::to_string(&mpd)?;

    // State shared between the request handlers. We are simply maintaining a counter of the number
    // of requests made, to check that each XLink reference has been resolved.
    let shared_state = Arc::new(AppState::new());

    async fn send_segment(Path(id): Path<u8>, State(state): State<Arc<AppState>>) -> Response<Full<Bytes>> {
        state.counter.fetch_add(1, Ordering::SeqCst);
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Full::from(vec![42, id]))
            .unwrap()
    }

    async fn send_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/plain")], format!("{}", state.counter.load(Ordering::Relaxed)))
    }

    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml) }))
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
    tokio::time::sleep(Duration::from_millis(1000)).await;
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
    let wb = env::temp_dir().join("wanting-best.mp4");
    DashDownloader::new(mpd_url)
        .best_quality()
        .with_http_client(client.clone())
        .download_to(wb.clone()).await
        .unwrap();
    let got = fs::read(wb).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], 42);
    assert_eq!(got[1], QUALITY_BEST);

    let ww = env::temp_dir().join("wanting-worst.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .with_http_client(client.clone())
        .download_to(ww.clone()).await
        .unwrap();
    let got = fs::read(ww).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], 42);
    assert_eq!(got[1], QUALITY_WORST);

    let wi = env::temp_dir().join("wanting-intermediate.mp4");
    DashDownloader::new(mpd_url)
        .intermediate_quality()
        .with_http_client(client.clone())
        .download_to(wi.clone()).await
        .unwrap();
    let got = fs::read(wi).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], 42);
    assert_eq!(got[1], QUALITY_INTERMEDIATE);

    let w = env::temp_dir().join("wanting-w1920.mp4");
    DashDownloader::new(mpd_url)
        .prefer_video_width(1920)
        .with_http_client(client.clone())
        .download_to(w.clone()).await
        .unwrap();
    let got = fs::read(w).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], 42);
    assert_eq!(got[1], QUALITY_BEST);

    let w = env::temp_dir().join("wanting-h120.mp4");
    DashDownloader::new(mpd_url)
        .prefer_video_height(120)
        .with_http_client(client.clone())
        .download_to(w.clone()).await
        .unwrap();
    let got = fs::read(w).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], 42);
    assert_eq!(got[1], QUALITY_WORST);

    // Check the total number of requested media segments corresponds to what we expect.
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("5"));

    Ok(())
}
