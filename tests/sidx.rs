// Testing parsing of sidx boxes (MP4 and WebM)
//
// To run tests while enabling printing to stdout/stderr
//
//    RUST_LOG=info cargo test --test sidx -- --show-output
//

pub mod common;
use std::env;
use std::time::Duration;
use reqwest::header::{RANGE, CONTENT_LENGTH};
use tracing::warn;
use common::setup_logging;
use dash_mpd::sidx::{from_isobmff_sidx, from_webm_cue};

//         <BaseURL>v-0480p-1000k-libx264.mp4</BaseURL>
//         <SegmentBase indexRange="837-3532" timescale="12288">
//           <Initialization range="0-836"/>
//         </SegmentBase>

#[tokio::test]
async fn test_sidx_mp4() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let segment_url = "https://storage.googleapis.com/shaka-demo-assets/sintel/v-0480p-1000k-libx264.mp4";
    let s = 837;
    let e = 3532;
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .unwrap();
    let idx = client.get(segment_url)
        .header(RANGE, format!("bytes={s}-{e}"))
        .header("Sec-Fetch-Mode", "navigate")
        .send().await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes().await
        .unwrap();
    // TODO need to reject idx value if len() doesn't correspond to what we requested
    // (server may not accept Range requests).
    if e - s + 1 != idx.len() {
        warn!("sidx box length does not correspond to requested octet range");
    }
    // let sidx = SidxBox::parse(&idx).unwrap();
    let refs = from_isobmff_sidx(&idx, s as u64);
    /*
    println!("sidx box includes {} references", sidx.reference_count);
    let mut total_size = 0;
    for sref in sidx.references {
        // println!("{sref:?}");
        total_size += sref.referenced_size;
    }

    let resp = client.head(segment_url)
        .send().await
        .unwrap();
    let headers = resp.headers();
    let declared_size = headers.get(CONTENT_LENGTH).unwrap()
        .to_str().unwrap()
        .parse::<usize>().unwrap();
    println!("content-length of segment is {declared_size}, total_size = {total_size}");
    println!("content-length - start = {}", declared_size - e);
    */
}



//         <BaseURL>v-0240p-0300k-vp9.webm</BaseURL>
//         <SegmentBase indexRange="296-4111" timescale="1000000">
//           <Initialization range="0-295"/>
//         </SegmentBase>

#[tokio::test]
async fn test_cues_webm() {
    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    let segment_url = "https://storage.googleapis.com/shaka-demo-assets/sintel/v-0240p-0300k-vp9.webm";
    let s = 296;
    let e = 4111;
    let client = reqwest::Client::builder()
        .timeout(Duration::new(10, 0))
        .build()
        .unwrap();
    let idx = client.get(segment_url)
        .header(RANGE, format!("bytes={s}-{e}"))
        .header("Sec-Fetch-Mode", "navigate")
        .send().await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes().await
        .unwrap();
    // TODO need to reject idx value if len() doesn't correspond to what we requested
    // (server may not accept Range requests).
    if e - s + 1 != idx.len() {
        warn!("sidx box length does not correspond to requested octet range");
    }
    let _segments = from_webm_cue(&idx);
    let resp = client.head(segment_url)
        .send().await
        .unwrap();
    let headers = resp.headers();
    let declared_size = headers.get(CONTENT_LENGTH).unwrap()
        .to_str().unwrap()
        .parse::<usize>().unwrap();
    println!("content-length of WebM segment is {declared_size}");
}
