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
//   - modify the default timeout
//   - enable gzip / brotli / deflate support
//   - choose the TLS implementation (native or rustls)
//   - add root certificates if you need to connect to servers that use a non-standard certificate chain
//

use std::env;
use std::time::Duration;
use std::path::PathBuf;
use env_logger::Env;
use reqwest::header;
use dash_mpd::fetch::DashDownloader;
use anyhow::Result;


fn main () -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
    let mut headers = header::HeaderMap::new();
    headers.insert("X-MY-HEADER",  header::HeaderValue::from_static("foo"));
    // Look for the semi-standard environment variables that specify proxy details, or try to
    // fall back to using a local Tor proxy.
    let http_proxy = env::var("https_proxy")
        .unwrap_or(env::var("http_proxy")
                   .unwrap_or("socks5://127.0.0.1:9050".to_string()));
    let proxy = reqwest::Proxy::http(http_proxy)
        .expect("Can't connect to HTTP proxy");
    let client = reqwest::blocking::Client::builder()
        .proxy(proxy)
        .user_agent("Mozilla/5.0")
        .default_headers(headers)
        .timeout(Duration::new(10, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let out = PathBuf::from(env::temp_dir()).join("cloudflarestream.mp4");
    DashDownloader::new(url)
        .with_http_client(client)
        .worst_quality()
        .download_to(out)
}
