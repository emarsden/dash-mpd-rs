// pprint_bbc_adaptive.rs
//
// Run with `cargo run --example pprint_bbc_adaptive`


use std::time::Duration;
use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use dash_mpd::{MPD, parse};

#[tokio::main]
async fn main() -> Result<()> {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact();
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info,reqwest=warn"))
        .unwrap();
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .context("creating HTTP client")?;
    let xml = client.get("https://rdmedia.bbc.co.uk/testcard/vod/manifests/avc-ctv-stereo-en.mpd")
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .send().await
        .context("requesting DASH MPD")?
        .error_for_status()
        .context("requesting DASH MPD")?
        .text().await
        .context("fetching MPD content")?;
    let mpd: MPD = parse(&xml)
        .context("parsing MPD")?;
    if let Some(pi) = mpd.ProgramInformation {
        if let Some(t) = pi.Title {
            println!("Title: {:?}", t.content);
        }
        if let Some(source) = pi.Source {
            println!("Source: {:?}", source.content);
        }
    }
    for p in mpd.periods {
        if let Some(d) = p.duration {
            println!("Contains Period of duration {d:?}");
        }
    }
    mpd.Metrics.iter().for_each(
        |m| m.Reporting.iter().for_each(
            |r| println!("{} metrics reporting to {}",
                         m.metrics,
                         r.reportingUrl.as_ref().unwrap_or(&String::from("<unspecified>")))));
    Ok(())
}
