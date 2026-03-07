//! Tests for progress observation functionality
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test progress_observer -- --show-output
//


pub mod common;
use std::env;
use std::fs;
use file_format::FileFormat;
use dash_mpd::fetch::DashDownloader;
use common::{check_file_size_approx, setup_logging};



#[tokio::test]
async fn test_progress_observer_basic() {
    use dash_mpd::fetch::ProgressObserver;
    use std::sync::Arc;

    struct DownloadProgressionTest { }

    impl ProgressObserver for DownloadProgressionTest {
        fn update(&self, percent: u32, bandwidth: u64, _message: &str) {
            assert!(percent <= 100);
            // 1000 GB/s should be enough for everybody.
            assert!(bandwidth < 1_000_000_000_000);
        }
    }

    setup_logging();
    let mpd_url = "https://cloudflarestream.com/31c9291ab41fac05471db4e73aa11717/manifest/video.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("progress-basic.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .add_progress_observer(Arc::new(DownloadProgressionTest{}))
        .download_to(&out).await
        .unwrap();
    check_file_size_approx(&out, 410_218);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}




#[tokio::test]
async fn test_progress_observer_progress() {
    use dash_mpd::fetch::ProgressObserver;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    static CURRENT_PERCENT: AtomicU32 = AtomicU32::new(0);
    static MESSAGE_COUNTER: AtomicU32 = AtomicU32::new(0);
    static SEEN_P98: AtomicBool = AtomicBool::new(false);
    static SEEN_P99: AtomicBool = AtomicBool::new(false);
    static SEEN_P100: AtomicBool = AtomicBool::new(false);
    
    struct DownloadProgressionTest {
    }

    impl ProgressObserver for DownloadProgressionTest {
        fn update(&self, percent: u32, bandwidth: u64, message: &str) {
            assert!(percent <= 100);
            // The first progress notification we receive should be 1
            let current = CURRENT_PERCENT.load(Ordering::Relaxed);
            if current == 0 {
                assert!(percent == 1);
            } else {
                assert!(current <= percent);
            }
            match percent {
                98 => SEEN_P98.store(true, Ordering::Relaxed),
                99 => SEEN_P99.store(true, Ordering::Relaxed),
                100 => SEEN_P100.store(true, Ordering::Relaxed),
                _ => {},
            }
            CURRENT_PERCENT.store(percent.into(), Ordering::Relaxed);
            assert!(message.len() < 150);
            // 1000 GB/s should be enough for everybody.
            assert!(bandwidth < 1_000_000_000_000);
            MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        }
    }

    setup_logging();
    if env::var("CI").is_ok() {
        return;
    }
    // This stream has 1331 audio segments and 1312 video segments, so we should see a large number
    // of progress messages. We should also see the percent=99 message concerning the muxing of
    // audio and video, and the percent=100 mesage saying "Done".
    let mpd_url = "http://ftp.itec.aau.at/datasets/mmsys13/redbull_4sec.mpd";
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("progress-redbull.mp4");
    DashDownloader::new(mpd_url)
        .worst_quality()
        .verbosity(2)
        .add_progress_observer(Arc::new(DownloadProgressionTest{}))
        .download_to(&out).await
        .unwrap();
    assert!(MESSAGE_COUNTER.load(Ordering::Relaxed) > 100);
    assert!(SEEN_P98.load(Ordering::Relaxed) == true);
    assert!(SEEN_P99.load(Ordering::Relaxed) == true);
    assert!(SEEN_P100.load(Ordering::Relaxed) == true);
    check_file_size_approx(&out, 110_010_161);
    let format = FileFormat::from_file(&out).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


