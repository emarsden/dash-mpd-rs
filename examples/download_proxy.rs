// download_proxy.rs
//
// Run with `cargo run --example download_proxy`
//
// This example illustrates downloading DASH content by supplying a custom reqwest Client, instead
// of relying on the default client. Providing a custom Client allows you to:
//
//   - specify a proxy to be used for all HTTP requests
//   - specify custom headers that you may need to use for authentication
//   - modify the User-Agent header on all HTTP requests
//   - modify the default timeout on network requests
//   - enable gzip / brotli / deflate support
//   - choose the TLS implementation (native or rustls)
//   - add root certificates if you need to connect to servers that use a non-standard certificate chain
//

use std::env;
use std::process;
use std::time::Duration;
use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use reqwest::header;
use dash_mpd::fetch::DashDownloader;


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

    let mut headers = header::HeaderMap::new();
    headers.insert("X-MY-HEADER",  header::HeaderValue::from_static("foo"));
    // Look for the semi-standard environment variables that specify proxy details, or try to
    // fall back to using a local Tor proxy.
    let http_proxy = env::var("https_proxy")
        .unwrap_or_else(|_| env::var("http_proxy")
                   .unwrap_or_else(|_| "socks5://127.0.0.1:9050".to_string()));
    let proxy = reqwest::Proxy::http(http_proxy)
        .context("connecting to HTTP proxy")?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .user_agent("Mozilla/5.0")
        .default_headers(headers)
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .context("creating HTTP client")?;
    let url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = env::temp_dir().join("cloudflarestream.mkv");
    match DashDownloader::new(url)
        .with_http_client(client)
        .worst_quality()
        .download_to(out).await {
	Err(e) => {
          eprintln!("Download failed: {e:?}");
          process::exit(-1);
        },
	Ok(path) => {
	  println!("Stream downloaded to {}", path.display());
	},
    }
    Ok(())
}
