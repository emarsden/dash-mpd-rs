// download_progressbar.rs
//
// Run with `cargo run --example download_progressbar -- --quality=best <URL>`
//

use std::sync::Arc;
use env_logger::Env;
use clap::Arg;
use indicatif::ProgressBar;
use dash_mpd::fetch::DashDownloader;
use dash_mpd::fetch::ProgressObserver;


struct DownloadProgressBar {
    bar: ProgressBar,
}

impl DownloadProgressBar {
    pub fn new() -> Self {
        Self { bar: ProgressBar::new(100) }
    }
}

impl ProgressObserver for DownloadProgressBar {
    fn update(&self, percent: u32) {
        if percent <= 100 {
            self.bar.set_position(percent.into());
        }
    }
}

fn main () {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
    let matches = clap::App::new("downloader_progressbar")
        .about("Download content from a DASH streaming media manifest")
        .arg(Arg::new("quality")
             .long("quality")
             .takes_value(true)
             .possible_value("best")
             .possible_value("worst"))
        .arg(Arg::new("url")
             .takes_value(true)
             .value_name("MPD-URL")
             .required(true)
             .index(1))
        .get_matches();
    let url = matches.value_of("url").unwrap();
    let mut dl = DashDownloader::new(url)
        .add_progress_observer(Arc::new(DownloadProgressBar::new()));
    if let Some(q) = matches.value_of("quality") {
        if q.eq("best") {
            dl = dl.best_quality();
        }
    }
    let dl_path = dl.download();
    println!("Downloaded to {:?}", dl_path);
}
