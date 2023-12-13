// decrypt.rs
//
// Run with `cargo run --example decrypt`


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

    let url = "https://bitmovin-a.akamaihd.net/content/art-of-motion_drm/mpds/11331.mpd";
    let ddl = DashDownloader::new(url)
        .worst_quality()
        .without_content_type_checks()
        .add_decryption_key("eb676abbcb345e96bbcf616630f1a3da".to_owned(),
                            "100b6c20940f779a4589152b57d2dacb".to_owned())
        .verbosity(2);
    match ddl.download().await {
        Ok(path) => println!("Downloaded to {path:?}"),
        Err(e) => {
            eprintln!("Download failed: {e:?}");
            process::exit(-1);
        },
    }
}
