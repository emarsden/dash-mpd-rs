// Tests for particular audio/video aspects of download support
//
// To run these tests while enabling printing to stdout/stderr
//
//    cargo test --test audio_video -- --show-output


use std::env;
use ffprobe::ffprobe;
use dash_mpd::fetch::DashDownloader;


#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_video_only() {
    let mpd_url = "http://amssamples.streaming.mediaservices.windows.net/69fbaeba-8e92-4740-aedc-ce09ae945073/AzurePromo.ism/manifest(format=mpd-time-csf)";
    let out = env::temp_dir().join("azure-promo-video.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .video_only()
        .download_to(out.clone()).await
        .unwrap();
    if let Ok(meta) = ffprobe(out.clone()) {
        assert_eq!(meta.streams.len(), 1);
        let stream = &meta.streams[0];
        assert_eq!(stream.codec_type, Some(String::from("video")));
        assert_eq!(stream.codec_name, Some(String::from("h264")));
        assert!(stream.width.is_some());
    }
}

#[tokio::test]
#[cfg(not(feature = "libav"))]
async fn test_dl_audio_only() {
    let mpd_url = "http://amssamples.streaming.mediaservices.windows.net/69fbaeba-8e92-4740-aedc-ce09ae945073/AzurePromo.ism/manifest(format=mpd-time-csf)";
    let out = env::temp_dir().join("azure-promo-audio.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .audio_only()
        .download_to(out.clone()).await
        .unwrap();
    if let Ok(meta) = ffprobe(out.clone()) {
        assert_eq!(meta.streams.len(), 1);
        let stream = &meta.streams[0];
        assert_eq!(stream.codec_type, Some(String::from("audio")));
        assert_eq!(stream.codec_name, Some(String::from("aac")));
        assert!(stream.width.is_none());
    }
}

#[tokio::test]
async fn test_dl_keep_audio_video() {
    let mpd_url = "http://amssamples.streaming.mediaservices.windows.net/69fbaeba-8e92-4740-aedc-ce09ae945073/AzurePromo.ism/manifest(format=mpd-time-csf)";
    let out = env::temp_dir().join("azure-promo.mp4");
    let out_audio = env::temp_dir().join("azure-promo-kept-audio.mp4");
    let out_video = env::temp_dir().join("azure-promo-kept-video.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .keep_audio_as(out_audio.clone())
        .keep_video_as(out_video.clone())
        .download_to(out.clone()).await
        .unwrap();
    if let Ok(meta) = ffprobe(out_audio) {
        assert_eq!(meta.streams.len(), 1);
        let stream = &meta.streams[0];
        assert_eq!(stream.codec_type, Some(String::from("audio")));
        assert_eq!(stream.codec_name, Some(String::from("aac")));
        assert!(stream.width.is_none());
    }
    if let Ok(meta) = ffprobe(out_video) {
        assert_eq!(meta.streams.len(), 1);
        let stream = &meta.streams[0];
        assert_eq!(stream.codec_type, Some(String::from("video")));
        assert_eq!(stream.codec_name, Some(String::from("h264")));
        assert!(stream.width.is_some());
    }
}

