// Rewrite MPD manifest to remove ads before downloading using XSLT stylesheet
//
// Run with `cargo run --example rewrite_xslt`
//
//
// Some streaming services and OTT television providers use server-side ad insertion (SSAI) or
// dynamic ad insertion (DAI) to serve customized advertising to viewers. Whereas traditional
// television showed the same ads to all viewers, this technology can increase advertising revenue
// by targeting based on tracking your viewing habits, in the same way as advertising on the web
// works.
//
// These ads are additional Periods that are inserted in the MPD manifest, in pre-roll, mid-roll or
// post-roll position. This example shows how to use an XSLT stylesheet to rewrite the XML manifest
// before starting the download, removing Period elements that look like advertising we would prefer
// not to consume. We can detect advertising Periods by looking at the location they are served from
// (which is "https://dai.google.com/" in this example), or for example their low duration
// (generally around 30 seconds), or perhaps by some recognizable name given to their @id attribute.
//
// We are currently executing XSLT spreadsheets using the venerable (but widely ported/distributed)
// xsltproc commandline application. This only implements XSLT v1.0, which is considerably less
// powerful than the most recent version of the specification (XSLT 3.0 from 2017 which allows XPath
// 3.1). However, the only free software implementation of XSLT 3.0 is Saxon-HE, implemented in
// Java, which less convenient for users to install.
//
// XSLT is a very powerful XML rewriting language, with a lot of pedagogical material available
// online. However, it's not very widely adopted outside the XML processing world. Future version of
// dash-mpd will examine the use of other rewriting languages, perhaps including WebAssembly/WASI
// scripting which would allow rewrite scripts/filters to be implemented in a variety of languages.
//
// Note: this example requires xsltproc to be installed and in the PATH
//
//    sudo apt install xsltproc
//    choco install xsltproc
//    brew install libxslt


use fs_err as fs;
use std::env;
use std::time::Duration;
use std::path::{Path, PathBuf};
use axum::{routing::get, Router};
use axum::http::header;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use anyhow::Result;
use env_logger::Env;


fn check_file_size_approx(p: &Path, expected: u64) {
    let meta = fs::metadata(p).unwrap();
    let ratio = meta.len() as f64 / expected as f64;
    assert!(0.9 < ratio && ratio < 1.1, "File sizes: expected {expected}, got {}", meta.len());
}


#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();


    // Manifests with SSAI/DAI are generally not publically accessible on the web (they are
    // per-subscriber and only available to a network provider's customers, for example). We test
    // with a manifest that we have saved locally and serve by spinning up a little web server.
    // This points to remote segments which will probably disappear at some stage.
    let mut mpd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    mpd.push("tests");
    mpd.push("fixtures");
    mpd.push("telenet-mid-ad-rolls");
    mpd.set_extension("mpd");
    let mpd = fs::read_to_string(mpd).unwrap();
    let app = Router::new()
        .route("/mpd", get(
            || async { ([(header::CONTENT_TYPE, "application/dash+xml")], mpd) }));
    let server_handle = axum_server::Handle::new();
    let backend_handle = server_handle.clone();
    let backend = async move {
        axum_server::bind("127.0.0.1:6669".parse().unwrap())
            .handle(backend_handle)
            .serve(app.into_make_service()).await
            .unwrap()
    };
    tokio::spawn(backend);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let out = env::temp_dir().join("nothx.mp4");
    let mut xslt = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xslt.push("tests");
    xslt.push("fixtures");
    xslt.push("rewrite-drop-dai");
    xslt.set_extension("xslt");
    DashDownloader::new("http://localhost:6669/mpd")
        .worst_quality()
        .with_xslt_stylesheet(xslt)
        .with_muxer_preference("mp4", "mp4box")
        .download_to(out.clone()).await
        .unwrap();
    server_handle.shutdown();
    check_file_size_approx(&out, 256_234_645);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let video = &meta.streams[0];
    assert_eq!(video.codec_type, Some(String::from("video")));
    assert_eq!(video.codec_name, Some(String::from("h264")));
    println!("Your uninfested content is at {}", out.display());

    Ok(())
}
