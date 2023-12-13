// download_bbc.rs
//
// Run with `cargo run --example download_bbc`
//
// Check the extended attributes associated with the downloaded file (on Unix platforms)
// with "xattr -l <output-path>"

use std::process;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use dash_mpd::fetch::DashDownloader;

#[tokio::main]
async fn main () {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact();
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,reqwest=warn"))
        .unwrap();
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    // this is a 442MB file
    let url = "https://rdmedia.bbc.co.uk/testcard/vod/manifests/avc-ctv-stereo-en.mpd";
    let ddl = DashDownloader::new(url)
        .worst_quality()
        .verbosity(2);
    #[cfg(target_os = "windows")]
    let ddl = ddl.with_vlc("C:/Program Files/VideoLAN/VLC/vlc.exe");
    match ddl.download().await {
        Ok(path) => println!("Downloaded to {path:?}"),
        Err(e) => {
            eprintln!("Download failed: {e:?}");
            process::exit(-1);
        },
    }
}
