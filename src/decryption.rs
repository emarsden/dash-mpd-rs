//! Support for decrypting media content
//
// We provide implementations for decrypting using the following helper applications:
//
//   - the historical mp4decrypt application from the Bento4 suite
//   - shaka-packager
//   - shaka-packager running in a Podman/Docker container
//   - MP4Box from the GPAC suite
//   - MP4Box from the official GPAC Podman/Docker container
//
// The options for running a helper application in a container rely on being able to run the
// container in rootless mode, to ensure that the decypted media files are owned by the user running
// our library. This is the default configuration for Podman, so we default to using that. It is
// possible to configure Docker to run in rootless mode; if you prefer to use Docker you can set the
// DOCKER environment variable to "docker".


use std::env;
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


// Run shaka-packager via its official Docker container, as per
// https://github.com/shaka-project/shaka-packager/blob/main/docs/source/docker_instructions.md
//
// Given the complexity of Podman/Docker arguments, this would be a good candidate for a plugin
// mechanism or use of a scripting language.
pub async fn decrypt_shaka_container(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    // We need to pass inpath and outpath into the container, in a manner which works both on Linux
    // and on Windows. We assume the container is a Linux container. We can't map outpath directly
    // in Docker/Podman using the -v argument, because outpath does not exist yet. We know that both
    // inpath and outpath are created in the same system temporary directory (they are created using
    // tmp_file_path, which uses the tempfile crate). The solution chosen here is to map the
    // temporary directory of the host (the parent directory of the inpath) to /tmp in the Linux
    // container, and in the container to refer to files in /tmp with the same filenames as on the
    // host.
    let inpath_dir = inpath.parent()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("inpath parent")))?;
    let inpath_nondir = inpath.file_name()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("inpath file name")))?;
    let outpath_nondir = outpath.file_name()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("outpath file name")))?;
    let mut args = Vec::new();
    let mut keys = Vec::new();
    args.push(String::from("run"));
    args.push(String::from("--rm"));
    args.push(String::from("--network=none"));
    args.push(String::from("--userns=keep-id"));
    args.push(String::from("-v"));
    args.push(format!("{}:/tmp", inpath_dir.display()));
    args.push(String::from("docker.io/google/shaka-packager:latest"));
    args.push(String::from("packager"));
    // Without the --quiet option, shaka-packager prints debugging output to stderr
    args.push("--quiet".to_string());
    args.push(format!("in=/tmp/{},stream={media_type},output=/tmp/{}",
                      inpath_nondir.display(), outpath_nondir.display()));
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
    // TODO: make container runner a DashDownloader option.
    // TODO: perhaps use the bollard crate to use Docker API.
    let container_runtime = env::var("DOCKER").unwrap_or(String::from("podman"));
    let pull = Command::new(&container_runtime)
        .args(["pull", "docker.io/google/shaka-packager:latest"])
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("pulling shaka-packager container: {e:?}")))?;
    if !pull.status.success() {
        error!("  Unable to pull shaka-packager decryption container with {container_runtime}");
        let msg = partial_process_output(&pull.stdout);
        if !msg.is_empty() {
            info!("  {container_runtime} stdout: {msg}");
        }
        let msg = partial_process_output(&pull.stderr);
        if !msg.is_empty() {
            info!("  {container_runtime} stderr: {msg}");
        }
        return Err(DashMpdError::Decrypting(String::from("pulling container docker.io/google/shaka-packager:latest")));
    }
    let runner = Command::new(&container_runtime)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("running shaka-packager container: {e:?}")))?;
    let mut no_output = false;
    if let Ok(metadata) = fs::metadata(outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        no_output = false;
    }
    if !runner.status.success() || no_output {
        warn!("  shaka-packager container failed");
        let msg = partial_process_output(&runner.stdout);
        if !msg.is_empty() {
            warn!("  shaka-packager stdout: {msg}");
        }
        let msg = partial_process_output(&runner.stderr);
        if !msg.is_empty() {
            warn!("  shaka-packager stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} stream with shaka-packager container");
        error!("  Undecrypted {media_type} left in {}", inpath.display());
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


// Decrypt using MP4Box from the GPAC suite, using their official Docker/Podman container.
pub async fn decrypt_mp4box_container(
    downloader: &DashDownloader,
    inpath: &Path,
    outpath: &Path,
    media_type: &str) -> Result<(), DashMpdError>
{
    let inpath_dir = inpath.parent()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("inpath parent")))?;
    let inpath_nondir = inpath.file_name()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("inpath file name")))?;
    let outpath_nondir = outpath.file_name()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("outpath file name")))?;
    let mut args = Vec::new();
    let drmpath = tmp_file_path("mp4boxcrypt", OsStr::new("xml"))?;
    let drmpath_nondir = drmpath.file_name()
        .ok_or_else(|| DashMpdError::Decrypting(String::from("drmpath file name")))?;
    let mut drm_contents = String::from("<GPACDRM>\n  <CrypTrack>\n");
    for (k, v) in downloader.decryption_keys.iter() {
        drm_contents += &format!("  <key KID=\"0x{k}\" value=\"0x{v}\"/>\n");
    }
    drm_contents += "  </CrypTrack>\n</GPACDRM>\n";
    fs::write(&drmpath, drm_contents).await
        .map_err(|e| DashMpdError::Io(e, String::from("writing to MP4Box decrypt file")))?;
    args.push(String::from("run"));
    args.push(String::from("--rm"));
    args.push(String::from("--network=none"));
    args.push(String::from("--userns=keep-id"));
    args.push(String::from("-v"));
    args.push(format!("{}:/tmp", inpath_dir.display()));
    args.push(String::from("docker.io/gpac/ubuntu:latest"));
    args.push(String::from("MP4Box"));
    args.push("-decrypt".to_string());
    args.push(format!("/tmp/{}", drmpath_nondir.display()));
    args.push(format!("/tmp/{}", inpath_nondir.display()));
    args.push("-out".to_string());
    args.push(format!("/tmp/{}", outpath_nondir.display()));
    if downloader.verbosity > 1 {
        info!("  Running decryption container GPAC/MP4Box {}", args.join(" "));
    }
    let container_runtime = env::var("DOCKER").unwrap_or(String::from("podman"));
    let pull = Command::new(&container_runtime)
        .args(["pull", "docker.io/gpac/ubuntu:latest"])
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("pulling MP4Box container: {e:?}")))?;
    if !pull.status.success() {
        warn!("  Unable to pull MP4Box decryption container");
        return Err(DashMpdError::Decrypting(String::from("pulling container docker.io/gpac/ubuntu:latest")));
    }
    let runner = Command::new(&container_runtime)
        .args(args)
        .output()
        .map_err(|e| DashMpdError::Decrypting(format!("spawning MP4Box container: {e:?}")))?;
    let mut no_output = false;
    if let Ok(metadata) = fs::metadata(&outpath).await {
        if downloader.verbosity > 0 {
            info!("  Decrypted {media_type} stream of size {} kB.", metadata.len() / 1024);
        }
        if metadata.len() == 0 {
            no_output = true;
        }
    } else {
        no_output = true;
    }
    if !runner.status.success() || no_output {
        warn!("  MP4Box decryption container failed");
        let msg = partial_process_output(&runner.stdout);
        if !msg.is_empty() {
            warn!("  MP4Box stdout: {msg}");
        }
        let msg = partial_process_output(&runner.stderr);
        if !msg.is_empty() {
            warn!("  MP4Box stderr: {msg}");
        }
    }
    if no_output {
        error!("  Failed to decrypt {media_type} with MP4Box container");
        error!("  Undecrypted {media_type} stream left in {}", inpath.display());
        return Err(DashMpdError::Decrypting(format!("{media_type} stream")));
    }
    Ok(())
}
