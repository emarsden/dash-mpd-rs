// dash_stream_info.rs
//
// Run with  `cargo run --example dash_stream_info <URL>`
//
// Example URL: http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd


use std::process;
use std::time::Duration;
use dash_mpd::parse;
use dash_mpd::{MPD, is_audio_adaptation, is_video_adaptation};
use clap::Arg;
use anyhow::{Context, Result};
use env_logger::Env;


#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();
    let matches = clap::Command::new("dash_stream_info")
        .about("Show codec and bandwidth for audio and video streams specified in a DASH MPD")
        .arg(Arg::new("url")
             .num_args(1)
             .value_name("URL")
             .help("URL of the MPD manifest")
             .index(1)
             .required(true))
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
    let mpd: MPD = parse(&xml).context("parsing MPD content")?;
    if mpd.periods.is_empty() {
        println!("MPD file contains zero Period elements");
        process::exit(2);
    }
    let period = &mpd.periods[0];
    let unspecified = "<unspecified>".to_string();
    // Show audio tracks with the codecs and available bitrates. Note that MPD manifests used by
    // commercial streaming services often include separate audio tracks for the different dubbed
    // languages available, but audio may also be included with the video track.
    if let Some(audio_adaptation) = period.adaptations.iter().find(is_audio_adaptation) {
        for representation in audio_adaptation.representations.iter() {
            // Here we see some of the fun of dealing with the MPD format: the codec can be
            // specified on the <Representation> element, or on the relevant <AdaptationSet>
            // element, or not at all.
            let codec = representation.codecs.as_ref()
                .unwrap_or_else(|| audio_adaptation.codecs.as_ref()
                                .unwrap_or(&unspecified));
            let bw = match representation.bandwidth {
                Some(b) => b.to_string(),
                None => "<unspecified>".to_string(),
            };
            println!("Audio stream with codec {}, bandwidth {}", codec, bw);
        }
    }

    // Show video tracks with the codecs and available bitrates
    if let Some(video_adaptation) = period.adaptations.iter().find(is_video_adaptation) {
        for representation in video_adaptation.representations.iter() {
            let codec = representation.codecs.as_ref()
                .unwrap_or_else(|| video_adaptation.codecs.as_ref()
                                .unwrap_or(&unspecified));
            let bw = match representation.bandwidth {
                Some(b) => b.to_string(),
                None => "<unspecified>".to_string(),
            };
            println!("Video stream with codec {codec}, bandwidth {bw}");
        }
    }
    Ok(())
}
