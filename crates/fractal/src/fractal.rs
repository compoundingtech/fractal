//! Local browser-session launching and installed executable ownership.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Only exact installer-created symlinks belong to this installation.
pub fn owns_alias(path: &Path) -> bool {
    std::fs::read_link(path)
        .is_ok_and(|target| target == Path::new("st3") || target == path.with_file_name("st3"))
}

pub fn may_remove(path: &Path) -> bool {
    if path.file_name() != Some(OsStr::new("fractal")) || owns_alias(path) {
        return true;
    }
    // A physical canonical binary (or signed-app link) belongs to this uninstall
    // only while it still identifies the executable running it. Never execute a
    // replacement command or treat an arbitrary regular file as ours.
    #[cfg(target_os = "linux")]
    {
        use fractal_drivers::reexec::ImageIdentity;
        ImageIdentity::of(path).ok().zip(ImageIdentity::running().ok())
            .is_some_and(|(installed, running)| installed == running)
    }
    #[cfg(target_os = "macos")]
    {
        macos_ownership::owns_running_image(path)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
mod macos_ownership {
    use std::os::unix::fs::MetadataExt as _;
    use std::path::Path;

    // Apple public ABI: https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h
    // libc already supplies vnode_info_path; only these two region wrappers and
    // the flavor constant are absent from the pinned version.
    const PROC_PIDREGIONPATHINFO: libc::c_int = 8;

    #[repr(C)]
    struct RegionInfo {
        protection: u32,
        max_protection: u32,
        inheritance: u32,
        flags: u32,
        offset: u64,
        behavior: u32,
        user_wired_count: u32,
        user_tag: u32,
        pages_resident: u32,
        pages_shared_now_private: u32,
        pages_swapped_out: u32,
        pages_dirtied: u32,
        ref_count: u32,
        shadow_depth: u32,
        share_mode: u32,
        private_pages_resident: u32,
        shared_pages_resident: u32,
        obj_id: u32,
        depth: u32,
        address: u64,
        size: u64,
    }

    #[repr(C)]
    struct RegionWithPath {
        region: RegionInfo,
        vnode: libc::vnode_info_path,
    }

    // This dyld function is deprecated only in libc in favor of an extra Mach
    // crate; it is the existing system ABI, and we need no additional dependency.
    #[allow(deprecated)]
    pub(super) fn owns_running_image(path: &Path) -> bool {
        // SAFETY: dyld image zero is this process's executable. Its header is
        // used only as an address for the read-only kernel region query.
        let header = unsafe { libc::_dyld_get_image_header(0) };
        if header.is_null() {
            return false;
        }
        let address = header as usize as u64;
        let mut info = std::mem::MaybeUninit::<RegionWithPath>::zeroed();
        let size = libc::c_int::try_from(std::mem::size_of::<RegionWithPath>())
            .expect("bounded Apple process-region ABI");
        // SAFETY: info is aligned writable storage of exactly size bytes. A
        // short/error response is never read, and zeroed vnode evidence refuses.
        let read = unsafe {
            libc::proc_pidinfo(libc::getpid(), PROC_PIDREGIONPATHINFO, address,
                info.as_mut_ptr().cast(), size)
        };
        if read != size {
            return false;
        }
        // SAFETY: the kernel returned the full structure, whose fields are
        // integer values/arrays. Zero initialization also covers missing vnode data.
        let info = unsafe { info.assume_init() };
        let stat = &info.vnode.vip_vi.vi_stat;
        if !address.checked_sub(info.region.address)
            .is_some_and(|offset| offset < info.region.size)
            || stat.vst_dev == 0 || stat.vst_ino == 0 || stat.vst_size <= 0
            || stat.vst_mode & libc::S_IFMT != libc::S_IFREG
        {
            return false;
        }
        let Ok(installed) = std::fs::metadata(path) else { return false };
        // Darwin dev_t is 32 bits; MetadataExt widens its signed representation.
        let device_bits = installed.dev() & u64::from(u32::MAX);
        installed.is_file() && device_bits == u64::from(stat.vst_dev)
            && installed.ino() == stat.vst_ino
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn public_apple_region_abi_layout_matches_lp64() {
            assert_eq!(std::mem::size_of::<RegionInfo>(), 96);
            assert_eq!(std::mem::offset_of!(RegionInfo, address), 80);
            assert_eq!(std::mem::offset_of!(RegionInfo, size), 88);
            assert_eq!(std::mem::size_of::<libc::vinfo_stat>(), 136);
            assert_eq!(std::mem::size_of::<libc::vnode_info_path>(), 1176);
            assert_eq!(std::mem::size_of::<RegionWithPath>(), 1272);
            assert_eq!(std::mem::offset_of!(RegionWithPath, vnode), 96);
        }
    }
}

/// Extend historical setup manifests when an upgrade installed the canonical name.
/// Recheck recorded canonical paths too: a foreign replacement never belongs to uninstall.
pub fn installed_binaries(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    let recorded = files.len();
    for index in 0..recorded {
        if files[index].file_name() == Some(OsStr::new("st3")) {
            let alias = files[index].with_file_name("fractal");
            if may_remove(&alias) && !files.contains(&alias) {
                files.push(alias);
            }
        }
    }
    files.retain(|path| may_remove(path));
    files
}

pub fn web(explicit_state: Option<PathBuf>) -> ExitCode {
    let url = match std::env::var("FRACTAL_WEB_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            let path = explicit_state.or_else(|| {
                fractal_daemon::config::Config::load_unvalidated(None).ok().map(|config| config.state_dir)
            }).map(|state| state.join("web-launch-url"));
            let result = path.ok_or_else(|| anyhow::anyhow!("cannot resolve daemon state directory"))
                .and_then(|path| {
                    use std::io::Read as _;
                    let mut file = fractal_daemon::config::open_token_file(&path)?;
                    let mut url = String::new();
                    file.read_to_string(&mut url)?;
                    Ok(url)
                });
            match result {
                Ok(url) => url,
                Err(error) => {
                    eprintln!("fractal web: {error:#}; start the daemon with built web assets first");
                    return ExitCode::from(1);
                }
            }
        }
        Err(error) => {
            eprintln!("fractal web: FRACTAL_WEB_URL: {error}");
            return ExitCode::from(2);
        }
    };
    // A URL is an opener argument, never a shell command or an option.
    if !reqwest::Url::parse(&url)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
    {
        eprintln!("fractal web: FRACTAL_WEB_URL must be an HTTP or HTTPS URL");
        return ExitCode::from(2);
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let opened = Command::new(opener)
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !opened {
        fractal_daemon::cli_println!("{url}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn parse(input: &[&str]) -> crate::Cli {
        use clap::Parser;
        crate::Cli::try_parse_from(input).unwrap()
    }

    #[test]
    fn fractal_dispatch_no_arguments_keeps_onboarding_and_terminal_guards() {
        let arguments = [OsString::from("/bin/fractal")];
        for (stdin, stdout, seat, expected) in [
            (true, true, false, true),
            (false, true, false, false),
            (true, false, false, false),
            (true, true, true, false),
        ] {
            assert_eq!(
                crate::plain_ui_requested(arguments.len(), stdin, stdout, seat),
                expected
            );
        }
    }

    #[test]
    fn tui_alias_reuses_ui_arguments_with_global_endpoint() {
        for arguments in [
            &["fractal", "tui", "--demo"][..],
            &["fractal", "--endpoint", "http://localhost:7410", "tui", "--demo"],
            &["fractal", "tui", "--endpoint", "http://localhost:7410", "--demo"],
        ] {
            let cli = parse(arguments);
            assert!(matches!(cli.command, crate::Command::Ui(options) if options.demo));
            if arguments.contains(&"--endpoint") {
                assert_eq!(cli.endpoint.as_deref(), Some("http://localhost:7410"));
            }
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn canonical_executable_is_owned_only_while_it_identifies_the_running_binary() {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().join("fractal");
        let running = std::env::current_exe().unwrap();
        std::os::unix::fs::symlink(&running, &canonical).unwrap();
        assert!(may_remove(&canonical));
        assert_eq!(installed_binaries(vec![canonical.clone()]), std::slice::from_ref(&canonical));
        std::fs::remove_file(&canonical).unwrap();
        std::fs::write(&canonical, b"foreign replacement").unwrap();
        assert!(!may_remove(&canonical));
        assert!(installed_binaries(vec![canonical]).is_empty());
    }

    #[test]
    fn installed_alias_is_discovered_for_old_manifests_and_only_removed_while_owned() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let st3 = directory.path().join("st3");
        let alias = directory.path().join("fractal");
        let legacy_files = vec![st3.clone(), directory.path().join("st")];
        for target in [PathBuf::from("st3"), st3] {
            symlink(&target, &alias).unwrap();
            let upgraded = installed_binaries(legacy_files.clone());
            assert_eq!(upgraded.last(), Some(&alias));
            assert!(may_remove(&alias));
            assert_eq!(installed_binaries(upgraded.clone()), upgraded);
            std::fs::remove_file(&alias).unwrap();
        }
        for target in ["foreign", "/missing/foreign"] {
            symlink(target, &alias).unwrap();
            let mut recorded = legacy_files.clone();
            recorded.push(alias.clone());
            assert_eq!(installed_binaries(recorded), legacy_files);
            assert!(!may_remove(&alias));
            std::fs::remove_file(&alias).unwrap();
        }
        std::fs::write(&alias, "foreign").unwrap();
        assert!(!may_remove(&alias));
        assert_eq!(installed_binaries(legacy_files.clone()), legacy_files);
        std::fs::remove_file(&alias).unwrap();
        std::fs::create_dir(&alias).unwrap();
        assert!(!may_remove(&alias));
    }
}
