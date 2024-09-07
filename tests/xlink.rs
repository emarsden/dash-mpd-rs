// Testing that we correctly resolve XLink references
//
// From the DASH IF specification: DASH "remote elements" are elements that are not fully contained
// in the MPD document but are referenced in the MPD with an HTTP URL using a simplified profile of
// XLink. Resolution (a.k.a. dereferencing) consists of two steps. Firstly, a DASH client issues an
// HTTP GET request to the URL contained in the @xlink:href, attribute of the in-MPD element, and
// the XLink resolver responds with a remote element entity in the response content. In case of
// error response or syntactically invalid remote element entity, the @xlink:href and @xlink:actuate
// attributes the client shall remove the in-MPD element.
//
//
// To run tests while enabling printing to stdout/stderr
//
//    RUST_LOG=info cargo test --test xlink -- --show-output
//
// What happens in this test:
//
//   - Start an axum HTTP server that serves an MPD manifest which includes several elements using
//   XLink references (that point to our server).
//
//   - Fetch the associated media content using DashDownloader, and check that each of the remote
//   elements is retrieved.
//
// This is a very demanding test, that is testing:
//
//   - the resolution of XLink references, including resolve-to-zero semantics.
//
//   - that an XLink fragment containing multiple elements is supported (e.g. a single XLinking
//   Period may resolve to two Period elements, as in this test).
//
//   - that a remote XLinked fragment can link to a further remote XLinked fragment (we only support
//   this to a limited depth, to avoid DoS loops).

pub mod common;
use fs_err as fs;
use std::env;
use std::time::Duration;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;
use axum::{routing::get, Router};
use axum::extract::State;
use axum::response::{Response, IntoResponse};
use axum::http::{header, StatusCode};
use axum::body::Body;
use dash_mpd::{MPD, Period, AdaptationSet, Representation, SegmentList};
use dash_mpd::{SegmentTemplate, SegmentURL};
use dash_mpd::fetch::{DashDownloader, parse_resolving_xlinks};
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

// This could be generalized to other namespaced elements such as xmlns:scte35 or xmlns:cenc, but
// that's not useful here.
fn add_xml_namespaces_recurse(element: &xmlem::Element, doc: &mut xmlem::Document) {
    if element.attribute(doc, "href").is_some() {
        element.set_attribute(doc, "xmlns:xlink", "http://www.w3.org/1999/xlink");
    }
    for child in element.children(doc).iter_mut() {
        add_xml_namespaces_recurse(child, doc);
    }
}

// Serving XLink content means serving XML fragments (for example a standalone Period element, not
// contained in a toplevel MPD element). Any elements in the XML fragments that use XLink attributes
// will need to specify the relevant XLink namespace. This is normally specified in the toplevel MPD
// element, but that is not present when serving a fragment.
//
// This function walks through the XML tree and adds xmlns:xlink attributes wherever they are
// required. This mechanism is preferable to the addition of almost-always unused @xmlns:xlink
// attributes (and eventually, xmlns:cenc and so on) to all the DASH structs that use those
// namespaces.
//
// Here we use a third (!) XML parsing crate xmlem only because it is sufficiently lax in parsing.
// We can't use xmltree for this tree rewriting because it refuses the malformed XML (missing
// namespace declaration).
fn add_xml_namespaces(xml: &str) -> Result<String> {
    let mut doc = xmlem::Document::from_str(xml).expect("xmlem parsing");
    add_xml_namespaces_recurse(&doc.root(), &mut doc);
    Ok(doc.to_string_pretty())
}

fn make_segment_list(urls: Vec<&str>) -> SegmentList {
    let mut segment_urls = Vec::new();
    for u in urls {
        segment_urls.push(SegmentURL { media: Some(String::from(u)), ..Default::default() });
    }
    SegmentList { segment_urls, ..Default::default() }
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_xlink_retrieval() -> Result<()> {
    setup_logging();

    // Temporarily disable this test on CI machines, because the concatenation of our small
    // synthetic MP4 segments is failing with certain older ffmpeg versions.
    if env::var("CI").is_ok() {
        return Ok(());
    }
    let segment_template1 = SegmentTemplate {
        initialization: Some("/media/f1.mp4".to_string()),
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
        href: Some("http://localhost:6666/remote/representation.xml".to_string()),
        actuate: Some("onLoad".to_string()),
        ..Default::default()
    };
    let remote_rep = Representation {
        id: Some("rr1".to_string()),
        width: Some(600),
        height: Some(400),
        SegmentList: Some(make_segment_list(vec!("/media/f2.mp4", "/media/f3.mp4"))),
        ..Default::default()
    };
    let adapt1 = AdaptationSet {
        id: Some("1".to_string()),
        contentType: Some("video".to_string()),
        representations: vec!(rep1),
        ..Default::default()
    };
    let adapt2 = AdaptationSet {
        id: Some("2".to_string()),
        contentType: Some("video".to_string()),
        representations: vec!(rep2),
        ..Default::default()
    };
    let period1 = Period {
        id: Some("1".to_string()),
        duration: Some(Duration::new(5, 0)),
        adaptations: vec!(adapt1.clone()),
        ..Default::default()
    };
    // This is a remote XLinked Period that resolves into two Periods.
    let period2 = Period {
        id: Some("2".to_string()),
        href: Some("/remote/period2.xml".to_string()),
        actuate: Some("onLoad".to_string()),
        ..Default::default()
    };
    // This is an XLinked Period that resolves-to-zero, meaning the client should delete it from the
    // final parsed manifest.
    let period3 = Period {
        id: Some("3".to_string()),
        href: Some("urn:mpeg:dash:resolve-to-zero:2013".to_string()),
        ..Default::default()
    };
    let remote_period1 = Period {
        id: Some("r1".to_string()),
        duration: Some(Duration::new(5, 0)),
        adaptations: vec!(adapt1),
        ..Default::default()
    };
    let remote_period2 = Period {
        id: Some("r2".to_string()),
        duration: Some(Duration::new(5, 0)),
        adaptations: vec!(adapt2),
        ..Default::default()
    };
    let mpd = MPD {
        mpdtype: Some("static".to_string()),
        xlink: Some("http://www.w3.org/1999/xlink".to_string()),
        periods: vec!(period1, period2, period3),
        ..Default::default()
    };
    let xml = mpd.to_string();
    let xml = add_xml_namespaces(&xml)?;
    let remote_period1_xml = quick_xml::se::to_string(&remote_period1)?;
    let remote_period1_xml = add_xml_namespaces(&remote_period1_xml)?;
    let remote_period2_xml = quick_xml::se::to_string(&remote_period2)?;
    let remote_period2_xml = add_xml_namespaces(&remote_period2_xml)?;
    let remote_period_xml = remote_period1_xml.clone() + &remote_period2_xml;
    let remote_rep = quick_xml::se::to_string(&remote_rep)?;
    let remote_representation_xml = add_xml_namespaces(&remote_rep)?;

    // State shared between the request handlers. We are simply maintaining a counter of the number
    // of requests made, to check that each XLink reference has been resolved.
    let shared_state = Arc::new(AppState::new());

    // Create a minimal sufficiently-valid MP4 file to use for this period.
    async fn send_mp4(State(state): State<Arc<AppState>>) -> Response {
        state.counter.fetch_add(1, Ordering::SeqCst);
        let data = generate_minimal_mp4();
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Body::from(data))
            .unwrap()
    }

    async fn send_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
        ([(header::CONTENT_TYPE, "text/plain")], format!("{}", state.counter.load(Ordering::Relaxed)))
    }

    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml) }))
        .route("/remote/period2.xml", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], remote_period_xml) }))
        .route("/remote/representation.xml", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], remote_representation_xml) }))
        .route("/media/:seg", get(send_mp4))
        .route("/status", get(send_status))
        .with_state(shared_state);
    let server_handle = hyper_serve::Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        hyper_serve::bind("127.0.0.1:6666".parse().unwrap())
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
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("0"), "Expecting 0 segment requests, got {txt}");

    // Now fetch the manifest and parse with our XLink resolution semantics and count the number of
    // Period elements.
    let mpd_urls = "http://localhost:6666/mpd";
    let mpd_url = Url::parse(mpd_urls)?;
    let dl = DashDownloader::new(mpd_urls)
        .with_http_client(client.clone());
    let xml = client.get(mpd_url.clone())
        .send().await?
        .error_for_status()?
        .bytes().await
        .context("fetching status")?;
    let mpd: MPD = parse_resolving_xlinks(&dl, &xml).await
        .context("parsing DASH XML")?;
    // We expect to have period1, remote_period1 and remote_period2 which were xlinked from period2,
    // and nothing from period3 which resolved to zero.
    assert_eq!(mpd.periods.len(), 3);
    assert!(mpd.periods.iter().any(|p| p.id.as_ref().is_some_and(|id| id.eq("r2"))));

    // Now download the media content from the MPD and check that the expected number of segments
    // were requested.
    let outpath = env::temp_dir().join("xlinked.mp4");
    DashDownloader::new(mpd_urls)
        .verbosity(0)
        .download_to(outpath.clone()).await
        .unwrap();
    assert!(fs::metadata(outpath).is_ok());

    // Check that the remote segments were fetched: request counter should be 4
    //
    // period1 > adapt1 > rep1 > segment_template1 > f1.mp4             +1
    // period2 > remote_period1 + remote_period2
    // remote_period1 > adapt1 > rep1 > segment_template1 > f1.mp4      +1
    // remote_period2 > adapt2 > rep2 > remote_rep > f2 + f3            +2
    // period3 > (null)
    let txt = client.get("http://localhost:6666/status")
        .send().await?
        .error_for_status()?
        .text().await
        .context("fetching status")?;
    assert!(txt.eq("4"), "Expecting 4 segment requests, got {txt}");
    server_handle.shutdown();

    Ok(())
}


// Test behaviour when xlinked resources are unavailable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_xlink_errors() -> Result<()> {
    // This XLinked Period that resolves to a success.
    let period1 = Period {
        id: Some("2".to_string()),
        href: Some("/remote/period.xml".to_string()),
        actuate: Some("onLoad".to_string()),
        ..Default::default()
    };
    let remote_period = Period {
        id: Some("r1".to_string()),
        href: Some("/remote/failure.xml".to_string()),
        actuate: Some("onLoad".to_string()),
        ..Default::default()
    };
    let mpd = MPD {
        mpdtype: Some("static".to_string()),
        xlink: Some("http://www.w3.org/1999/xlink".to_string()),
        periods: vec!(period1),
        ..Default::default()
    };
    let xml = mpd.to_string();
    let xml = add_xml_namespaces(&xml)?;
    let remote_period_xml = quick_xml::se::to_string(&remote_period)?;
    let remote_period_xml = add_xml_namespaces(&remote_period_xml)?;

    setup_logging();
    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], xml) }))
        .route("/remote/period.xml", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], remote_period_xml) }));
    let server_handle = hyper_serve::Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        hyper_serve::bind("127.0.0.1:6669".parse().unwrap())
            .handle(backend_handle)
            .serve(app.into_make_service()).await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Now fetch the manifest and check that we fail due to the non-existent remote Period. 
    let outpath = env::temp_dir().join("nonexistent.mp4");
    assert!(DashDownloader::new("http://localhost:6669/mpd")
            .download_to(outpath.clone()).await
            .is_err());
    server_handle.shutdown();
    Ok(())
}
