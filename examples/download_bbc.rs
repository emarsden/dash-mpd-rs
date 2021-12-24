// download_bbc.rs
//
// Run with `cargo run --example download_bbc`
//
// Check the extended attributes associated with the downloaded file (on Unix platforms)
// with "xattr -l /tmp/BBC-MPD-test.mp4"

use std::time::Duration;
use dash_mpd::fetch_mpd;

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let url = "http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-ctv-events.mpd";
    if let Err(e) = fetch_mpd(&client, url, "/tmp/BBC-MPD-test.mp4") {
        eprintln!("Error downloading DASH MPD file: {:?}", e);
    }
}
