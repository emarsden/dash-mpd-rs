// Example for debugging an mp4box muxing issue that arises on CI machines

use fs_err as fs;
use env_logger::Env;
use dash_mpd::fetch::DashDownloader;

#[tokio::main]
async fn main () {
    env_logger::Builder::from_env(Env::default().default_filter_or("info,reqwest=warn")).init();

    let mpd_url = "https://turtle-tube.appspot.com/t/t2/dash.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("audio-only.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .fetch_video(false)
        .fetch_subtitles(false)
        .with_muxer_preference("mp4", "mp4box")
        .download_to(out.clone()).await
        .unwrap();
    let meta = fs::metadata(out).unwrap();
    println!("Output turtle-tube audio: {} bytes", meta.len());
}
