// pprint_bbc_adaptive.rs
//
// Run with `cargo run --example pprint_bbc_adaptive`


use std::time::Duration;
use dash_mpd::{MPD, parse};

fn main() {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let xml = client.get("http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-events.mpd")
        .header("Accept", "application/dash+xml")
        .send()
        .expect("Requesting MPD content")
        .text()
        .expect("Fetching MPD content");
    let mpd: MPD = parse(&xml)
        .expect("parsing MPD");
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
            println!("Contains Period of duration {:?}", d);
        }
    }
}
