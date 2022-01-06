// download_bbc.rs
//
// Run with `cargo run --example download_bbc`
//
// Check the extended attributes associated with the downloaded file (on Unix platforms)
// with "xattr -l /tmp/BBC-MPD-test.mp4"

use std::time::Duration;
use dash_mpd::fetch_mpd;
use tempfile::NamedTempFile;

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let outfile = NamedTempFile::new()
       .expect("creating temporary output file");
    let outpath = outfile.path().to_str()
       .expect("obtaining name of temporary file");
    let url = "http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-ctv-events.mpd";
    if let Err(e) = fetch_mpd(&client, url, outpath) {
        eprintln!("Error downloading DASH MPD file: {:?}", e);
    } else {
        println!("Output saved to temporary file {}", outpath);
    }
}
