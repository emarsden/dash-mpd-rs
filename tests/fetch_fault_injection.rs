//! Test network error handling code using a fault-injecting HTTP proxy
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test fetch_fault_injection -- --show-output

// We use the toxiproxy fault-injecting proxy, via its Docker image. We could use noxious (Rust port
// of toxiproxy) as an alternative.


pub mod common;
use fs_err as fs;
use std::env;
use std::process::Command;
use tracing::trace;
use anyhow::{Context, Result};
use serde_json::json;
use ffprobe::ffprobe;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, setup_logging};



pub struct ToxiProxy {}

impl ToxiProxy {
    pub fn new() -> ToxiProxy {
        // https://github.com/Shopify/toxiproxy
        let pull = Command::new("podman")
            .env("LANG", "C")
            .args(["pull", "ghcr.io/shopify/toxiproxy"])
            .output()
            .expect("failed spawning podman");
        if !pull.status.success() {
            let stdout = String::from_utf8_lossy(&pull.stdout);
            if stdout.len() > 0 {
                println!("Podman stdout> {stdout}");
            }
            let stderr = String::from_utf8_lossy(&pull.stderr);
            if stderr.len() > 0 {
                println!("Podman stderr> {stderr}");
            }
        }
        assert!(pull.status.success());
        let _run = Command::new("podman")
            .env("LANG", "C")
            .args(["run", "--rm",
                   "--name", "toxiproxy",
                   "--env", "LOG_LEVEL=trace",
                   "-p", "8474:8474",
                   "-p", "8001:8001",
                   "ghcr.io/shopify/toxiproxy"])
            .spawn()
            .expect("failed spawning podman");
        trace!("Toxiproxy server started");
        ToxiProxy {}
    }
}

impl Drop for ToxiProxy {
    fn drop(&mut self) {
        // cleanup
        let _stop = Command::new("podman")
            .env("LANG", "C")
            .args(["stop", "toxiproxy"])
            .output()
            .expect("failed to spawn podman");
    }
}

impl Default for ToxiProxy {
    fn default() -> Self {
        Self::new()
    }
}


#[ignore]
#[cfg(not(feature = "libav"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn test_dl_resilience() -> Result<()> {
    setup_logging();
    if env::var("CI").is_ok() {
        return Ok(());
    }
    let _toxiproxy = ToxiProxy::new();
    let txclient = reqwest::Client::new();
    // Enable the toxiproxy proxy.
    txclient.post("http://localhost:8474/proxies")
        .json(&json!({
            "name": "dash-mpd-rs",
            "listen": "0.0.0.0:8001",
            // "upstream": "dash.akamaized.net:80",
            "upstream": "ftp.itec.aau.at:80",
            "enabled": true
        }))
        .send().await?;
    // Add a timeout Toxic with a very large timeout (amounts to a failure).
    txclient.post("http://localhost:8474/proxies/dash-mpd-rs/toxics")
        .json(&json!({
            "type": "timeout",
            "name": "fail",
            "toxicity": 0.3,
            "attributes": { "timeout": 4000000 },
        }))
        .send().await
        .expect("creating timeout toxic");
    // Add a data rate limitation Toxic.
    txclient.post("http://localhost:8474/proxies/dash-mpd-rs/toxics")
        .json(&json!({
            "type": "limit_data",
            "toxicity": 0.5,
            "attributes": { "bytes": 321 },
        }))
        .send().await
        .expect("creating timeout toxic");

    let _configer = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::new(0, 5000)).await;
        println!("Injecting toxics");
        let txfail = json!({
            "type": "timeout",
            "toxicity": 0.3,
            "attributes": { "timeout": 4000000 },
        });
        txclient.post("http://localhost:8474/proxies/dash-mpd-rs/toxics")
            .json(&txfail)
            .send().await
            .expect("creating timeout toxic");
        let txlimit = json!({
            "type": "limit_data",
            "toxicity": 0.5,
            "attributes": { "bytes": 321 },
        });
        txclient.post("http://localhost:8474/proxies/dash-mpd-rs/toxics")
            .json(&txlimit)
            .send().await
            .expect("creating timeout toxic");
        println!("Noxious toxics added");
    });

    let proxy = reqwest::Proxy::all("http://127.0.0.1:8001/")
        .context("connecting to HTTP proxy")?;
    let client = reqwest::ClientBuilder::new()
        .proxy(proxy)
        .build()
        .context("creating reqwest client")?;
    let mpd_url = "http://ftp.itec.aau.at/datasets/mmsys13/redbull_4sec.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("error-resilience.mkv");
    DashDownloader::new(mpd_url)
        .best_quality()
        .content_type_checks(false)
        .conformity_checks(false)
        .verbosity(3)
        .with_http_client(client)
        .download_to(out.clone()).await
        .unwrap();

    check_file_size_approx(&out, 71_342_249);
    let meta = ffprobe(out.clone()).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::MatroskaVideo);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);

    Ok(())
}
