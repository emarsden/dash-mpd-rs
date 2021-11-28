// tests for MPD download support
//
// To run tests while enabling printing to stdout/stderr, "cargo test -- --show-output"
//
// Testing resources:
//
//   https://testassets.dashif.org/#testvector/list
//   https://ottverse.com/free-mpeg-dash-mpd-manifest-example-test-urls/
//   https://dash.itec.aau.at/dash-dataset/
//   https://github.com/streamlink/streamlink/tree/master/tests/resources/dash



#[test]
fn test_itec1() {
    use std::time::Duration;
    use dash_mpd::fetch_mpd;
    
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::new(30, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    let manifest_url = "http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd";
    assert!(fetch_mpd(&client, manifest_url, "/tmp/test-dash-out.mp4").is_ok());
}


// These tests retrieve content from some public MPD manifests and check that the content is
// identical to previous "known good" downloads. These checks are fragile because checksums and
// exact octet counts might change due to version changes in libav, that we use for muxing.
// Running this test downloads several hundred megabytes. 
#[test]
fn test_downloader() {
    use std::io;
    use std::time::Duration;
    use sha2::{Digest, Sha256};
    use hex_literal::hex;
    use ffprobe::ffprobe;
    use dash_mpd::{fetch_mpd, HttpClient};
    
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/86.0.4240.183 Safari/537.36")
        .timeout(Duration::new(10, 0))
        .gzip(true)
        .build()
        .expect("Couldn't create reqwest HTTP client");
    
    fn check_mpd(client: &HttpClient, mpd_url: &str, octets: u64, digest: &[u8]) {
        use tempfile::NamedTempFile;
        
        println!("Checking MPD URL {}", mpd_url);
        let tmp = NamedTempFile::new()
            .expect("Can't create tempfile");
        let (tmpfile, tmppath) = tmp.keep()
            .expect("Can't keep tempfile");
        let os_path = tmppath.to_str().expect("tmpfile path");
        if let Err(e) = fetch_mpd(client, mpd_url, os_path) {
            eprintln!("Failed to fetch MPD {}: {:?}", mpd_url, e);
        }
        tmpfile.sync_all()
            .expect("Syncing data to disk");
        // check that ffprobe identifies this as a media file
        let probed_meta = ffprobe(tmppath.clone());
        if let Ok(meta) = probed_meta {
            assert!(meta.streams.len() > 0);
            let stream = &meta.streams[0];
            // actually, ffprobe doesn't give a duration for WebM content
            // assert!(stream.duration.is_some());
            if let Some(duration) = &stream.duration {
                assert!(duration.parse::<f64>().unwrap() > 0.0);
            }
        } else {
            eprintln!("ffprobe failed on {}", mpd_url);
        }
        let mut sha256 = Sha256::new();
        // we reopen because rewinding the tmpfile doesn't seem to work
        let mut reopened = std::fs::File::open(os_path)
            .expect("Can't open media file");
        let octets_downloaded = io::copy(&mut reopened, &mut sha256)
            .expect("Couldn't read media file contents");
        assert_eq!(octets, octets_downloaded);
        let calculated = sha256.finalize();
        assert_eq!(calculated[..], digest[..]);
    }
    
    check_mpd(&client,
              "http://ftp.itec.aau.at/datasets/mmsys12/ElephantsDream/MPDs/ElephantsDreamNonSeg_6s_isoffmain_DIS_23009_1_v_2_1c2_2011_08_30.mpd",
              3_753_579,
              // sha256sum --binary filename.mp4
              &hex!("31350a74a5b0c7d8056a0b022678c645278ad746114414cb5a7fabe767b376c8"));

    check_mpd(&client,
              "https://res.cloudinary.com/demo-robert/video/upload/sp_16x9_vp9/yourPublicId.mpd",
              445_758,
              &hex!("7d6533d19a4a60c5c76cead7b2de1664f4ff856916037a574f641aad0324ee36"));

    // 35MB for this one
    check_mpd(&client,
              "http://rdmedia.bbc.co.uk/dash/ondemand/bbb/2/client_manifest-common_init.mpd",
              // we are saying 37816568 but streamlink says 38108591, probably an off-by-one
              // $Number$ count
              37_816_568,
              &hex!("4f5e0a7716b8b4305655946fbe4690469be6f92796ecac3b5e0327f59b70a07d"));

    // 120MB
    check_mpd(&client,
              "http://rdmedia.bbc.co.uk/dash/ondemand/testcard/1/client_manifest-ctv-events.mpd",
              183_208_569,
              &hex!("c2ef782035c80a6ef7003e59899d9b9f449aa70d1ab564fe3e701bbd32e2a1ae"));

    check_mpd(&client,
              "https://media.axprod.net/TestVectors/v8-MultiContent/Clear/Manifest.mpd",
              35_295_258,
              &hex!("62d50469b10f2e6e98dfb24b966b4c52d50008a46b370bb656fde92e59bea30f"));

    check_mpd(&client,
              "https://dash.akamaized.net/dash264/TestCasesHEVC/2a/13/tos_ondemand_multires_10bit_hvc.mpd",
              45_954_988,
              &hex!("d6c8e1547d7c6d2d0a4a9e74bd3aa7a497584b0820779b5ebbb9333856c8f861"));
    
    // streamlink is not able to download this one...
    // The resulting video is not playable by mplayer, but it is by VLC
    // Our ffprobe tests are expected to fail on this file
    check_mpd(&client,
              "http://www.digitalprimates.net/dash/streams/gpac/mp4-main-multi-mpd-AV-NBS.mpd",
              3_815_796,
              &hex!("1492b95fa1707ddce3c8685c4ababd0129e3812e4d5462093eda6823c896790c"));
    
    check_mpd(&client,
              "http://www.digitalprimates.net/dash/streams/mp4-live-template/mp4-live-mpd-AV-BS.mpd",
              3_751_736,
              &hex!("640949ee76302e075fe0f36670376d4eb19f5bfd393e6cd3e89488934ac7d9df"));
    
    check_mpd(&client,
              "http://yt-dash-mse-test.commondatastorage.googleapis.com/media/motion-20120802-manifest.mpd",
              3_737_571,
              &hex!("6e5e572d49714f3edc8e5c10fedbf346ad998e2ea3110607c8167db5f0392435"));

    // this one is audio-only, in xHE-AAC ("High Efficiency AAC") format, which can't currently
    // be decoded by mplayer or VLC
    check_mpd(&client,
              "https://dash.akamaized.net/dash264/TestCasesMCA/fraunhofer/xHE-AAC_Stereo/3/Sintel/sintel_audio_only_64kbps.mpd",
              7_320_216,
              &hex!("7e541627935139cd18a9099d712f8d963fa4d59ff1f1d27c4ccf5cfa81d9e1d8"));
    
    check_mpd(&client,
              "https://dash.akamaized.net/dash264/TestCasesMCA/dolby/2/1/ChID_voices_71_768_ddp.mpd",
              4_845_725,
              &hex!("4011ce0bc7aae5fedd83bd9971454cb1ae2e5db99bdc581b728f3710a7301e0c"));
}

