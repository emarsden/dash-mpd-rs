// Example for debugging an mp4box muxing issue that arises on CI machines

use std::fs;
use dash_mpd::fetch::DashDownloader;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

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

    let mpd_url = "https://turtle-tube.appspot.com/t/t2/dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("audio-only.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_video(false)
        .fetch_subtitles(false)
        .with_muxer_preference("mp4", "mp4box")
        .download_to(out.clone()).await
        .unwrap();
    let meta = fs::metadata(out).unwrap();
    println!("Output turtle-tube audio: {} bytes", meta.len());
}
