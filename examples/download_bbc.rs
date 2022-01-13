// download_bbc.rs
//
// Run with `cargo run --example download_bbc`
//
// Check the extended attributes associated with the downloaded file (on Unix platforms)
// with "xattr -l <output-path>"

use dash_mpd::fetch::DashDownloader;
use env_logger::Env;

fn main () {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
    let url = "http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-ctv-events.mpd";
    let dl_path = DashDownloader::new(url)
        .worst_quality()
        .download();
    println!("Downloaded to {:?}", dl_path);
}
