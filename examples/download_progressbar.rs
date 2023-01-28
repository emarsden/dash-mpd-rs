// download_progressbar.rs
//
// Run with `cargo run --example download_progressbar -- --quality=best <URL>`
//

use std::process;
use std::sync::Arc;
use env_logger::Env;
use clap::Arg;
use indicatif::{ProgressBar, ProgressStyle};
use colored::*;
use dash_mpd::fetch::DashDownloader;
use dash_mpd::fetch::ProgressObserver;


struct DownloadProgressBar {
    bar: ProgressBar,
}

impl DownloadProgressBar {
    pub fn new() -> Self {
        let b = ProgressBar::new(100)
            .with_style(ProgressStyle::default_bar()
                        .template("[{elapsed}] {bar:50.cyan/blue} {wide_msg}")
                        .expect("building progress bar")
                        .progress_chars("#>-"));
        Self { bar: b }
    }
}

impl ProgressObserver for DownloadProgressBar {
    fn update(&self, percent: u32, message: &str) {
        if percent <= 100 {
            self.bar.set_position(percent.into());
            self.bar.set_message(message.to_string());
        }
        if percent == 100 {
            self.bar.finish_with_message("Done");
        }
    }
}

#[tokio::main]
async fn main () {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
    let matches = clap::Command::new("downloader_progressbar")
        .about("Download content from a DASH streaming media manifest")
        .arg(Arg::new("quality")
             .long("quality")
             .num_args(1)
             .value_parser(["best", "worst"]))
        .arg(Arg::new("url")
             .num_args(1)
             .value_name("MPD-URL")
             .help("URL of the MPD manifest")
             .required(true)
             .index(1))
        .get_matches();
    let url = matches.get_one::<String>("url").unwrap();
    let mut dl = DashDownloader::new(url)
        .record_metainformation(false)
        .add_progress_observer(Arc::new(DownloadProgressBar::new()));
    if let Some(q) = matches.get_one::<String>("quality") {
        if q.eq("best") {
            dl = dl.best_quality();
        }
    }
    match dl.download_to("media.mkv").await {
        Ok(path) => println!("Downloaded to {path:?}"),
        Err(e) => {
            eprintln!("{}: {e:?}", "Download failed".red());
            process::exit(-1);
        },
    }
}
