/// round_trip.rs -- check round trip from XML to Rust structs to XML.
//
// This tool fetches a DASH manifest, deserializes the content to our Rust structs, then serializes
// the result back to XML. It can be used to help identify attributes and nodes that are incorrectly
// defined in our Rust structs. If the "xmldiff" and/or "xdiff" (from
// https://github.com/ajankovic/xdiff) tools are installed, they will be used to show a treewise XML
// diff between the original and rewritten MPD manifests.
//
// You should expect attributes of type xs:duration to be flagged as differences by this tool,
// because there is no single canonical serialization format for them (for instance, we choose not
// to serialize trailing zeros in the floating point component of seconds, but many manifests in the
// wild include them). The order of elements in the reserialized XML may also differ from the
// original, because we lose ordering information when deserializing. Though in theory this can
// change the semantics of XML, it should AFAIK not affect DASH semantics. The order of attributes
// in the reserialized XML may also change, but this has no semantic meaning (and will be ignored by
// the xmldiff tool).
//
// To run this little tool:
//
//    cargo run --example round_trip URL
//
// Example URL: https://refapp.hbbtv.org/videos/00_llama_multiperiod_v1/manifest.mpd
//
// The xmldiff and xdiff output shows the location of differences using XPath expressions. You can
// identify the corresponding content using for example
//
//    xmllint --xpath "/*/*[11]/*[5]/*[6]/*[1]" /tmp/mpd-rewritten.xml


use std::env;
use std::io;
use std::io::Write;
use std::fs;
use std::process::Command;
use std::time::Duration;
use anyhow::{Result, Context};
use env_logger::Env;
use clap::Arg;
use url::Url;
use dash_mpd::MPD;
use dash_mpd::fetch::{DashDownloader, parse_resolving_xlinks};


#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
    let matches = clap::Command::new("round-trip")
        .arg(Arg::new("url")
             .num_args(1)
             .value_name("MPD-URL")
             .help("URL of the MPD manifest")
             .required(true)
             .index(1))
        .get_matches();
    let url = matches.get_one::<String>("url").unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::new(30, 0))
        .build()
        .context("creating HTTP client")?;
    let xml = client.get(url)
        .header("Accept", "application/dash+xml,video/vnd.mpeg.dash.mpd")
        .header("Accept-language", "en-US,en")
        .send().await
        .context("requesting DASH MPD")?
        .error_for_status()
        .context("requesting DASH MPD")?
        .text().await
        .context("fetching MPD content")?;
    let out1 = env::temp_dir().join("mpd-orig.xml");
    fs::write(&out1, &xml)?;
    // let mpd: MPD = parse(&xml).context("parsing MPD content")?;
    let dl = DashDownloader::new(url)
        .with_http_client(client.clone());
    let mpd: MPD = parse_resolving_xlinks(&dl, &Url::parse(url)?, &xml.into_bytes()).await
        .context("parsing DASH XML")?;
    let rewritten = mpd.to_string();
    let out2 = env::temp_dir().join("mpd-rewritten.xml");
    fs::write(&out2, rewritten)?;
    // We tried using the natural_xml_diff crate for this purpose, but its output is less convenient
    // to interpret.
    println!("==== xmldiff output ====");
    let cmd = Command::new("xmldiff")
        .args([out1.clone(), out2.clone()])
        .output()
        .context("executing xmldiff as a subprocess")?;
    io::stdout().write_all(&cmd.stdout).unwrap();
    println!("==== xdiff output ====");
    // The xdiff tool from https://github.com/ajankovic/xdiff provides more detail on the
    // differences between the two MPD files. xdiff can consume > 30GB of RAM on certain manifests
    // that contain many XML elements (for instance a large SegmentList), so we avoid calling it for
    // large manifests.
    if let Ok(meta) = fs::metadata(out1.clone()) {
        if meta.len() < 100_000 {
            let cmd = Command::new("xdiff")
                .args(["-left", &out1.to_string_lossy(), "-right", &out2.to_string_lossy()])
                .output()
                .context("executing xdiff as a subprocess")?;
            io::stdout().write_all(&cmd.stdout).unwrap();
        } else {
            println!("  skipping xdiff for this large MPD manifest");
        }
    }
    Ok(())
}

