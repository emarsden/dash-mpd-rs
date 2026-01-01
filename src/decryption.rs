//! Support for decrypting media content

use std::path::Path;
use std::process::Command;
use std::ffi::OsStr;
use tokio::fs;
use tracing::{info, warn, error};
use crate::DashMpdError;
use crate::fetch::{DashDownloader, partial_process_output, tmp_file_path};


pub async fn decrypt_mp4decrypt(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    let mut args = Vec::new();
    for (k, v) in downloader.decryption_keys.iter() {
        args.push("--key".to_string());
        args.push(format!("{k}:{v}"));
    }
    args.push(inpath.to_string_lossy().to_string());
    args.push(outpath.to_string_lossy().to_string());
    if downloader.verbosity > 1 {
        info!("  Running mp4decrypt {}", args.join(" "));
    }
    let out = Command::new(downloader.mp4decrypt_location.clone())
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning mp4decrypt")))?;
    let mut no_output = false;
    if let Ok(metadata) = fs::metadata(outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        if metadata.len() == 0 {
            no_output = true;
        }
    } else {
        no_output = true;
    }
    if !out.status.success() || no_output {
        error!("  mp4decrypt subprocess failed");
        let msg = partial_process_output(&out.stdout);
        if !msg.is_empty() {
            warn!("  mp4decrypt stdout: {msg}");
        }
        let msg = partial_process_output(&out.stderr);
        if !msg.is_empty() {
            warn!("  mp4decrypt stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} stream with mp4decrypt");
        warn!("  Undecrypted {media_type} stream left in {}", inpath.display());
        return Err(DashMpdError::Decrypting(format!("{media_type} stream")));
    }
    Ok(())
}


pub async fn decrypt_shaka(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    let mut args = Vec::new();
    let mut keys = Vec::new();
    if downloader.verbosity < 1 {
        args.push("--quiet".to_string());
    }
    args.push(format!("in={},stream={media_type},output={}", inpath.display(), outpath.display()));
    let mut drm_label = 0;
    #[allow(clippy::explicit_counter_loop)]
    for (k, v) in downloader.decryption_keys.iter() {
        keys.push(format!("label=lbl{drm_label}:key_id={k}:key={v}"));
        drm_label += 1;
    }
    args.push("--enable_raw_key_decryption".to_string());
    args.push("--keys".to_string());
    args.push(keys.join(","));
    if downloader.verbosity > 1 {
        info!("  Running shaka-packager {}", args.join(" "));
    }
    let out = Command::new(downloader.shaka_packager_location.clone())
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Io(e, String::from("spawning shaka-packager")))?;
    let mut no_output = true;
    if let Ok(metadata) = fs::metadata(outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        no_output = false;
    }
    if !out.status.success() || no_output {
        warn!("  shaka-packager subprocess failed");
        let msg = partial_process_output(&out.stdout);
        if !msg.is_empty() {
            warn!("  shaka-packager stdout: {msg}");
        }
        let msg = partial_process_output(&out.stderr);
        if !msg.is_empty() {
            warn!("  shaka-packager stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} stream with shaka-packager");
        warn!("  Undecrypted {media_type} left in {}", inpath.display());
        return Err(DashMpdError::Decrypting(format!("{media_type} stream")));
    }
    Ok(())
}

pub async fn decrypt_shaka_container(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    let mut args = Vec::new();
    let mut keys = Vec::new();
    args.push(String::from("run"));
    args.push(String::from("--rm"));
    args.push(String::from("-v"));
    args.push(format!("{}:/tmp/shakainput.mp4", inpath.display()));
    args.push(String::from("-v"));
    args.push(String::from("/tmp:/tmp"));
    args.push(String::from("docker.io/google/shaka-packager:latest"));
    args.push(String::from("packager"));
    // Without the --quiet option, shaka-packager prints debugging output to stderr
    args.push("--quiet".to_string());
    args.push(format!("in=/tmp/shakainput.mp4,stream={media_type},output={}", outpath.display()));
    let mut drm_label = 0;
    #[allow(clippy::explicit_counter_loop)]
    for (k, v) in downloader.decryption_keys.iter() {
        keys.push(format!("label=lbl{drm_label}:key_id={k}:key={v}"));
        drm_label += 1;
    }
    args.push("--enable_raw_key_decryption".to_string());
    args.push("--keys".to_string());
    args.push(keys.join(","));
    if downloader.verbosity > 1 {
        info!("  Running shaka-packager container {}", args.join(" "));
    }
    // https://github.com/shaka-project/shaka-packager/blob/main/docs/source/docker_instructions.md
    let out = Command::new("podman")
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("running shaka-packager container: {e:?}")))?;
    if !out.status.success() {
        return Err(DashMpdError::Decrypting(
            String::from("shaka-packager container exit status indicates failure")));
    }
    let mut no_output = false;
    if let Ok(metadata) = fs::metadata(outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        no_output = false;
    }
    if !out.status.success() || no_output {
        warn!("  shaka-packager container failed");
        let msg = partial_process_output(&out.stdout);
        if !msg.is_empty() {
            warn!("  shaka-packager stdout: {msg}");
        }
        let msg = partial_process_output(&out.stderr);
        if !msg.is_empty() {
            warn!("  shaka-packager stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} stream with shaka-packager container");
        warn!("  Undecrypted {media_type} left in {}", inpath.display());
        return Err(DashMpdError::Decrypting(format!("{media_type} stream")));
    }
    Ok(())
}


// Decrypt with MP4Box as per https://wiki.gpac.io/xmlformats/Common-Encryption/
//    MP4Box -decrypt drm_file.xml encrypted.mp4 -out decrypted.mp4
pub async fn decrypt_mp4box(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    let mut args = Vec::new();
    let drmfile = tmp_file_path("mp4boxcrypt", OsStr::new("xml"))?;
    let mut drmfile_contents = String::from("<GPACDRM>\n  <CrypTrack>\n");
    for (k, v) in downloader.decryption_keys.iter() {
        drmfile_contents += &format!("  <key KID=\"0x{k}\" value=\"0x{v}\"/>\n");
    }
    drmfile_contents += "  </CrypTrack>\n</GPACDRM>\n";
    fs::write(&drmfile, drmfile_contents).await
        .map_err(|e| DashMpdError::Io(e, String::from("writing to MP4Box decrypt file")))?;
    args.push("-decrypt".to_string());
    args.push(drmfile.display().to_string());
    args.push(String::from(inpath.to_string_lossy()));
    args.push("-out".to_string());
    args.push(String::from(outpath.to_string_lossy()));
    if downloader.verbosity > 1 {
        info!("  Running decryption application MP4Box {}", args.join(" "));
    }
    let out = Command::new(downloader.mp4box_location.clone())
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("spawning MP4Box: {e:?}")))?;
    let mut no_output = false;
    if let Ok(metadata) = fs::metadata(outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        if metadata.len() == 0 {
            no_output = true;
        }
    } else {
        no_output = true;
    }
    if !out.status.success() || no_output {
        warn!("  MP4Box decryption subprocess failed");
        let msg = partial_process_output(&out.stdout);
        if !msg.is_empty() {
            warn!("  MP4Box stdout: {msg}");
        }
        let msg = partial_process_output(&out.stderr);
        if !msg.is_empty() {
            warn!("  MP4Box stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} with MP4Box");
        warn!("  Undecrypted {media_type} stream left in {}", inpath.display());
        return Err(DashMpdError::Decrypting(format!("{media_type} stream")));
    }
    Ok(())
}


pub async fn decrypt_mp4box_container(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    let mut args = Vec::new();
    let drmfile = tmp_file_path("mp4boxcrypt", OsStr::new("xml"))?;
    let mut drmfile_contents = String::from("<GPACDRM>\n  <CrypTrack>\n");
    for (k, v) in downloader.decryption_keys.iter() {
        drmfile_contents += &format!("  <key KID=\"0x{k}\" value=\"0x{v}\"/>\n");
    }
    drmfile_contents += "  </CrypTrack>\n</GPACDRM>\n";
    fs::write(&drmfile, drmfile_contents).await
        .map_err(|e| DashMpdError::Io(e, String::from("writing to MP4Box decrypt file")))?;
    args.push(String::from("run"));
    args.push(String::from("--rm"));
    args.push(String::from("-v"));
    args.push(format!("{}:/tmp/mp4boxinput.mp4", inpath.display()));
    args.push(String::from("-v"));
    args.push(format!("{}:/tmp/mp4boxcrypt", drmfile.display()));
    args.push(String::from("-v"));
    args.push(String::from("/tmp:/tmp"));
    args.push(String::from("docker.io/gpac/ubuntu:latest"));
    args.push(String::from("MP4Box"));
    args.push("-decrypt".to_string());
    args.push(String::from("/tmp/mp4boxcrypt"));
    args.push(String::from(inpath.to_string_lossy()));
    args.push("-out".to_string());
    args.push(String::from(outpath.to_string_lossy()));
    if downloader.verbosity > 1 {
        info!("  Running decryption container GPAC/MP4Box {}", args.join(" "));
    }
    let out = Command::new("podman")
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("spawning MP4Box container: {e:?}")))?;
    let mut no_output = false;
    if let Ok(metadata) = fs::metadata(outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        if metadata.len() == 0 {
            no_output = true;
        }
    } else {
        no_output = true;
    }
    if !out.status.success() || no_output {
        warn!("  MP4Box decryption container failed");
        let msg = partial_process_output(&out.stdout);
        if !msg.is_empty() {
            warn!("  MP4Box stdout: {msg}");
        }
        let msg = partial_process_output(&out.stderr);
        if !msg.is_empty() {
            warn!("  MP4Box stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} with MP4Box container");
        warn!("  Undecrypted {media_type} stream left in {}", inpath.display());
        return Err(DashMpdError::Decrypting(format!("{media_type} stream")));
    }
    Ok(())
}
