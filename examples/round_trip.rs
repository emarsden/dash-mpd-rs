// round_trip.rs -- check round trip from XML to Rust structs to XML.
//
// This tool fetches a DASH manifest, deserializes the content to our Rust structs, then serializes
// the result back to XML. It can be used to help identify attributes and nodes that are incorrectly
// defined in our Rust structs. Requires the "xmldiff" tool to be installed.
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
// The xmldiff shows the location of differences using XPath expressions. You can identify the
// corresponding content using for example
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
use dash_mpd::{MPD, parse};


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
        .send()
        .await
        .context("requesting DASH MPD")?
        .error_for_status()
        .context("requesting DASH MPD")?
        .text()
        .await
        .context("fetching MPD content")?;
    let out1 = env::temp_dir().join("mpd-orig.xml");
    fs::write(&out1, &xml)?;
    let mpd: MPD = parse(&xml).context("parsing MPD content")?;
    let rewritten = quick_xml::se::to_string(&mpd)
        .context("serializing MPD struct")?;
    let out2 = env::temp_dir().join("mpd-rewritten.xml");
    fs::write(&out2, &rewritten)?;
    // We tried using the natural_xml_diff crate for this purpose, but its output is less convenient
    // to interpret.
    let cmd = Command::new("xmldiff")
        .args([out1, out2])
        .output()
        .context("executing xmldiff as a subprocess")?;
    io::stdout().write_all(&cmd.stdout).unwrap();
    Ok(())
}

