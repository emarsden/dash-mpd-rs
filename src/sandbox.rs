//! Security sandboxing support
//
// This module provides basic and experimental sandboxing support for the running thread. We
// tell the operating system that we do not intend to use some functionality such as listening on a
// network socket or writing files other than in specific directories, and the operating system
// blocks us from later doing so. The sandboxing is inherited by all child processes, and in
// particular by the helper applications that we use to mux audio and video streams, and to merge
// subtitles. The intent is to:
//
//   - reduce the damage that can be caused by buggy or malicious helper applications;
//
//   - reduce our attack surface: deny ourselves privileges that we know we won't need, so that if
//     an attacker is able to compromise this library, for example via a supply-chain attack on one
//     of our crate dependencies, the damage they can produce is limited.
//
// This sandboxing support is currently only implemented for Linux, using the landlock LSM
// (https://docs.kernel.org/userspace-api/landlock.html). See https://landlock.io/. There are many
// limitations to this sandboxing support; for example UDP sockets and raw sockets are not currently
// blocked by the landlock APIs. Running the application in a Docker/Podman container provides much
// more protection.
//
// Implementation of this feature is gated at compile time by the `sandbox` crate feature. When
// compiled in, it must be enabled at runtime by calling the `sandbox` method on `DashDownloader`
// with a true argument.
//
// Implementation notes:
//
// - We don't attempt to limit TCP connection attempts, because the Landlock API requires us to list
//   the ports that we will connect to. Although ports 80 and 443 (for HTTP and HTTPS), as well as the
//   port provided in the MPD URL, will cover most situations, it won't cover manifests that contain
//   XLink remote references to URLs with non-standard ports.
//
// - We block binding TCP sockets.
//
// - We allow readonly access to the user's config files (as specified by the XDG_CONFIG_HOME
//   environment variable, defaulting to $HOME/.config), and to the XDG_DATA_HOME directory tree
//   (defaulting to $HOME/.local). We also allow readonly access to the /etc directory tree,
//   /dev/zero and /proc/meminfo.
//
// - We allow read+write access to temporary directories, as specified by the TMPDIR environment
//   variable and the XDG-RUNTIME_DIR environment variable. It's important that this be the same
//   directory trees as used by the tempfile crate, which we use to create temporary files.
//
// - We allow execution of code (read+execute permissions) for subdirectories in the PATH and in
//   LD_LIBRARY_PATH, in /usr and /lib, and in the user-specified paths for our helper applications
//   (ffmpeg and so on), which may be installed in non-standard locations. This may cause
//   "permission denied" errors in some unusual setups where a helper application is installed to a
//   non-standard location and statically linked to libraries in a directory which is not a subtree
//   of the location of the binary. The rstrict crate runs ldd on binaries before sandboxing to find
//   their dependent libraries even outside of LD_LIBRARY_PATH, but that seems excessive for our use
//   case.
//
// TODO:
//
// - Look into adding support for the seccomp security module for Linux.
//
// - Look into sandboxing mechanisms for Microsoft Windows.



use std::env;
use landlock::{
    Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
    path_beneath_rules
};
use tracing::{trace, info, error};
use crate::DashMpdError;
use crate::fetch::DashDownloader;



pub fn restrict_thread(downloader: &DashDownloader) -> Result<(), DashMpdError> {
    let mut ro_dirs = Vec::new();
    // The process may need to read /etc/resolv.conf, /etc/hosts, /etc/ssl/certs and so on
    ro_dirs.push(String::from("/etc"));
    ro_dirs.push(String::from("/dev/zero"));
    // This is used by MP4Box
    ro_dirs.push(String::from("/proc/meminfo"));
    // XDG_CONFIG_HOME, used for example for $HOME/.config/vlc/vlcrc
    if let Some(config_dir) = dir_spec::config_home() {
        let config_str = config_dir.into_os_string();
        ro_dirs.push(String::from(config_str.to_string_lossy()));
    }
    // XDG_DATA_HOME, used for example for $HOME/.local/share/vlc/ml.xspf
    if let Some(data_dir) = dir_spec::data_home() {
        let data_str = data_dir.into_os_string();
        ro_dirs.push(String::from(data_str.to_string_lossy()));
    }
    let mut rw_dirs = Vec::new();
    rw_dirs.push(String::from("/var/tmp"));
    let tmpdir = env::var("TMPDIR").unwrap_or_else(|_| String::from("/tmp"));
    rw_dirs.push(tmpdir);
    // The XDG_RUNTIME_DIR is normally something like /run/user/<uid>, or possibly a subdirectory of
    // $TMPDIR.
    if let Some(runtime_dir) = dir_spec::runtime() {
        let runtime_str = runtime_dir.into_os_string();
        let runtime_string = String::from(runtime_str.to_string_lossy());
        if !rw_dirs.contains(&runtime_string) {
            rw_dirs.push(runtime_string);
        }
    }
    if let Some(output_path) = downloader.output_path.clone() {
        let os_str = output_path.as_os_str().to_string_lossy();
        rw_dirs.push(String::from(os_str));
    }
    let cwd = env::current_dir()
        .map_err(|_| DashMpdError::Other(String::from("reading cwd")))?
        .into_os_string();
    rw_dirs.push(String::from(cwd.to_string_lossy()));
    trace!("Sandbox: allowing r+w filesystem access to directories {rw_dirs:?}");
    let mut rx_dirs = Vec::new();
    rx_dirs.push(String::from("/usr"));
    rx_dirs.push(String::from("/lib"));
    // We need to add all the directories on the $PATH environment variable, because the default
    // values of ffmpeg_location and so on are filenames without directory, so if they are in a
    // non-standard directory we need to make that accessible.
    if let Some(paths) = env::var_os("PATH") {
        for path in env::split_paths(&paths) {
            let path_str = path.into_os_string();
            rx_dirs.push(String::from(path_str.to_string_lossy()));
        }
    }
    // Likewise for libraries in directories in $LD_LIBRARY_PATH.
    if let Some(paths) = env::var_os("LD_LIBRARY_PATH") {
        for path in env::split_paths(&paths) {
            let path_str = path.into_os_string();
            rx_dirs.push(String::from(path_str.to_string_lossy()));
        }
    }
    // These will only be useful if they specify a fully qualified directory
    rx_dirs.push(downloader.ffmpeg_location.clone());
    rx_dirs.push(downloader.vlc_location.clone());
    rx_dirs.push(downloader.mkvmerge_location.clone());
    rx_dirs.push(downloader.mp4box_location.clone());
    rx_dirs.push(downloader.mp4decrypt_location.clone());
    rx_dirs.push(downloader.shaka_packager_location.clone());
    trace!("Sandbox: allowing r+x filesystem access to directories {rx_dirs:?}");
    trace!("Sandbox: allowing readonly filesystem access to {ro_dirs:?}");
    // https://landlock.io/rust-landlock/landlock/enum.AccessFs.html
    let fs_ro = AccessFs::from_read(landlock::ABI::V2) & !AccessFs::Execute;
    let fs_rx = AccessFs::from_read(landlock::ABI::V2) | AccessFs::Execute;
    let fs_rw = AccessFs::from_all(landlock::ABI::V2) & !AccessFs::Execute;
    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(landlock::ABI::V2))
        .map_err(|_| DashMpdError::Other(String::from("restricting filesystem access")))?
        .handle_access(AccessNet::BindTcp)
        .map_err(|_| DashMpdError::Other(String::from("restricting network access")))?
        .scope(Scope::from_all(landlock::ABI::V6))
        .map_err(|_| DashMpdError::Other(String::from("restricting signal scope")))?
        .create()
        .map_err(|_| DashMpdError::Other(String::from("creating Landlock ruleset")))?
        .add_rules(path_beneath_rules(ro_dirs, fs_ro))
        .map_err(|_| DashMpdError::Other(String::from("allowing readonly access to /etc and /dev/zero")))?
        .add_rules(path_beneath_rules(&["/dev/null"], AccessFs::from_all(landlock::ABI::V2)))
        .map_err(|_| DashMpdError::Other(String::from("allowing access to /dev/null")))?
        .add_rules(path_beneath_rules(rx_dirs, fs_rx))
        .map_err(|_| DashMpdError::Other(String::from("allowing limited read-exec access to binaries")))?
        .add_rules(path_beneath_rules(rw_dirs, fs_rw))
        .map_err(|_| DashMpdError::Other(String::from("allowing write access to output directories")))?
        .restrict_self()
        .map_err(|_| DashMpdError::Other(String::from("enforcing landlock sandboxing ruleset")))?;
    match status.ruleset {
        RulesetStatus::FullyEnforced => info!(" ✓ Sandboxing enabled."),
        RulesetStatus::PartiallyEnforced => info!("Partially sandboxed."),
        RulesetStatus::NotEnforced => error!("Not sandboxed! Please update your Linux kernel."),
    }
    Ok(())
}
