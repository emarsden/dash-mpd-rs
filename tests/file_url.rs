// Tests for downloading from file:// URLs.
//
// To run tests while enabling printing to stdout/stderr
//
//    cargo test --test file_url -- --show-output


pub mod common;
use fs_err as fs;
use std::env;
use std::path::PathBuf;
use url::Url;
use ffprobe::ffprobe;
use file_format::FileFormat;
use test_log::test;
use dash_mpd::fetch::DashDownloader;
use common::check_file_size_approx;


// This manifest has a single absolute BaseURL at the top level (MPD.BaseURL).
#[test(tokio::test)]
async fn test_file_mpd_baseurl() {
    if env::var("CI").is_ok() {
        return;
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("ad-insertion-testcase6-av1");
    path.set_extension("mpd");
    let mpd_url = Url::from_file_path(path).unwrap();
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("fileurl-mpd-baseurl.mp4");
    DashDownloader::new(&mpd_url.to_string())
        .worst_quality()
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 3_918_028);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
async fn test_file_period_baseurl() {
    if env::var("CI").is_ok() {
        return;
    }
    let xslt = r#"<xsl:stylesheet version="1.0"
     xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
     xmlns:mpd="urn:mpeg:dash:schema:mpd:2011">
  <xsl:template match="@*|node()"><xsl:copy><xsl:apply-templates select="@*|node()"/></xsl:copy></xsl:template>
  <xsl:template match="//mpd:Period[@id!='2']" />
</xsl:stylesheet>"#;
    let stylesheet = tempfile::Builder::new()
        .suffix(".xslt")
        .rand_bytes(7)
        .tempfile()
        .unwrap();
    fs::write(&stylesheet, xslt).unwrap();
    let (_, stylesheet_path) = stylesheet.keep().unwrap();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("vod-aip-unif-streaming");
    path.set_extension("mpd");
    let mpd_url = Url::from_file_path(path).unwrap();
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("fileurl-period-baseurl.mp4");
    DashDownloader::new(&mpd_url.to_string())
        .worst_quality()
        .verbosity(3)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .with_xslt_stylesheet(stylesheet_path)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 1_933_145);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}


#[test(tokio::test)]
async fn test_file_period_baseurl_thomson() {
    if env::var("CI").is_ok() {
        return;
    }
    let xslt = r#"<xsl:stylesheet version="1.0"
     xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
     xmlns:mpd="urn:mpeg:dash:schema:mpd:2011">
  <xsl:template match="@*|node()"><xsl:copy><xsl:apply-templates select="@*|node()"/></xsl:copy></xsl:template>
  <xsl:template match="//mpd:Period[@id!='1']" />
</xsl:stylesheet>"#;
    let stylesheet = tempfile::Builder::new()
        .suffix(".xslt")
        .rand_bytes(7)
        .tempfile()
        .unwrap();
    fs::write(&stylesheet, xslt).unwrap();
    let (_, stylesheet_path) = stylesheet.keep().unwrap();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("vod-aip-unif-streaming");
    path.set_extension("mpd");
    let mpd_url = Url::from_file_path(path).unwrap();
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("fileurl-period-baseurl-thomson.mp4");
    DashDownloader::new(&mpd_url.to_string())
        .worst_quality()
        .verbosity(3)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .with_xslt_stylesheet(stylesheet_path)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 2_777_692);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}



// This manifest has absolute URLs in each SegmentTemplate element. It's a multiperiod manifest with
// several DAI periods including segments from dai.google.com that have now expired; we only fetch
// the first Period which is identified by its id in an XPath query.
#[test(tokio::test)]
async fn test_file_segmenttemplate() {
    if env::var("CI").is_ok() {
        return;
    }
    // This drops all Period elements except for the first one identified by its @id.
    let xslt = r#"<xsl:stylesheet version="1.0"
     xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
     xmlns:mpd="urn:mpeg:dash:schema:mpd:2011">
  <xsl:template match="@*|node()">
    <xsl:copy>
      <xsl:apply-templates select="@*|node()"/>
    </xsl:copy>
  </xsl:template>
  <xsl:template match="//mpd:Period[@id!='96d40c7b-4de1-4f93-b622-77719e867588']" />
</xsl:stylesheet>"#;
    let stylesheet = tempfile::Builder::new()
        .suffix(".xslt")
        .rand_bytes(7)
        .tempfile()
        .unwrap();
    fs::write(&stylesheet, xslt).unwrap();
    let (_, stylesheet_path) = stylesheet.keep().unwrap();
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push("telenet-mid-ad-rolls");
    path.set_extension("mpd");
    let mpd_url = Url::from_file_path(path).unwrap();
    let tmpd = tempfile::tempdir().unwrap();
    let out = tmpd.path().join("fileurl-segment-template.mp4");
    DashDownloader::new(&mpd_url.to_string())
        .worst_quality()
        .verbosity(3)
        .with_concat_preference("mp4", "ffmpegdemuxer")
        .with_xslt_stylesheet(stylesheet_path)
        .download_to(out.clone()).await
        .unwrap();
    check_file_size_approx(&out, 120_695_762);
    let format = FileFormat::from_file(out.clone()).unwrap();
    assert_eq!(format, FileFormat::Mpeg4Part14Video);
    let meta = ffprobe(out).unwrap();
    assert_eq!(meta.streams.len(), 2);
    let audio = meta.streams.iter()
        .find(|s| s.codec_type.eq(&Some(String::from("audio"))))
        .expect("finding audio stream");
    assert_eq!(audio.codec_name, Some(String::from("aac")));
    let entries = fs::read_dir(tmpd.path()).unwrap();
    let count = entries.count();
    assert_eq!(count, 1, "Expecting a single output file, got {count}");
    let _ = fs::remove_dir_all(tmpd);
}




