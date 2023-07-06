// decrypt.rs
//
// Run with `cargo run --example decrypt`


use std::process;
use env_logger::Env;
use dash_mpd::fetch::DashDownloader;

#[tokio::main]
async fn main () {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
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
