//! Same-UID host check backing the owning execution-tree contract.
//! This is defense in depth, not proof against privileged or non-dumpable consumers.
use rustix::fd::OwnedFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HostScan {
    Clear { uninspectable: u32 },
    Hit { uninspectable: u32 },
    Unknown { uninspectable: u32 },
}

impl HostScan {
    pub(super) fn uninspectable(self) -> u32 {
        match self {
            Self::Clear { uninspectable } | Self::Hit { uninspectable }
                | Self::Unknown { uninspectable } => uninspectable,
        }
    }
}

#[cfg(target_os = "linux")]
fn gone(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
        || error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error())
}

fn link_hit(path: &std::path::Path, root: &[u8], reuse: &mut Vec<u8>) -> std::io::Result<bool> {
    let target = rustix::fs::readlinkat(rustix::fs::CWD, path, std::mem::take(reuse))?;
    let bytes = target.to_bytes();
    let hit = bytes.starts_with(root) && (bytes.len() == root.len() || bytes[root.len()] == b'/');
    *reuse = target.into_bytes();
    Ok(hit)
}

/// One sequential procfs pass. Only the exact allocation descriptor held by this
/// caller is exempt: other descriptors in this process remain possible hits.
#[cfg(target_os = "linux")]
pub(super) fn host_consumers(root: &OwnedFd) -> HostScan {
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    let unknown = || HostScan::Unknown { uninspectable: 0 };
    let Ok(allocation) = fs::read_link(format!("/proc/self/fd/{}", root.as_raw_fd())) else { return unknown(); };
    if !allocation.is_absolute() { return unknown(); }
    let allocation = allocation.as_os_str().as_bytes();
    let Ok(owner) = fs::metadata("/proc/self") else { return unknown(); };
    let Ok(processes) = fs::read_dir("/proc") else { return unknown(); };
    let own_pid = std::process::id();
    let exempt = root.as_raw_fd().to_string();
    let mut uninspectable = 0;
    let mut link = PathBuf::new();
    let mut target = Vec::new();
    for entry in processes {
        let Ok(entry) = entry else { return HostScan::Unknown { uninspectable }; };
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|name| name.parse::<u32>().ok()) else { continue; };
        let base = entry.path();
        match fs::metadata(&base) {
            Ok(metadata) if metadata.uid() == owner.uid() => {},
            Ok(_) => continue,
            Err(error) if gone(&error) => continue,
            Err(_) => return HostScan::Unknown { uninspectable },
        }
        match link_hit(&base.join("cwd"), allocation, &mut target) {
            Ok(true) => return HostScan::Hit { uninspectable },
            Ok(false) => {},
            Err(error) if gone(&error) => continue,
            Err(_) => { uninspectable += 1; continue; },
        }
        let descriptors = match fs::read_dir(base.join("fd")) {
            Ok(descriptors) => descriptors,
            Err(error) if gone(&error) => continue,
            Err(_) => { uninspectable += 1; continue; },
        };
        link.clear();
        link.push(&base);
        link.push("fd");
        for descriptor in descriptors {
            let descriptor = match descriptor {
                Ok(descriptor) => descriptor,
                Err(error) if gone(&error) => continue,
                Err(_) => { uninspectable += 1; break; },
            };
            let name = descriptor.file_name();
            if pid == own_pid && name == std::ffi::OsStr::new(&exempt) { continue; }
            link.push(name);
            let hit = link_hit(&link, allocation, &mut target);
            link.pop();
            match hit {
                Ok(true) => return HostScan::Hit { uninspectable },
                Ok(false) => {},
                Err(error) if gone(&error) => {},
                Err(_) => { uninspectable += 1; break; },
            }
        }
    }
    HostScan::Clear { uninspectable }
}


#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};

    struct Consumer(Child);
    impl Consumer {
        fn start(cwd: &Path, script: &str, argument: &Path) -> Self {
            let mut child = Command::new("/bin/sh").args(["-c", script, "consumer"])
                .arg(argument).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped())
                .spawn().unwrap();
            let mut ready = [0; 5];
            child.stdout.as_mut().unwrap().read_exact(&mut ready).unwrap();
            assert_eq!(&ready, b"ready");
            Self(child)
        }
    }
    impl Drop for Consumer {
        fn drop(&mut self) { self.0.kill().unwrap(); self.0.wait().unwrap(); }
    }

    fn directory(path: &Path) -> OwnedFd {
        rustix::fs::open(path, rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC, rustix::fs::Mode::empty()).unwrap()
    }

    #[test]
    fn clear_exempts_only_held_self_descriptor_and_checks_component_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let allocation = root.path().join("allocation");
        std::fs::create_dir(&allocation).unwrap();
        let held = directory(&allocation);
        let sibling = root.path().join("allocation-other");
        std::fs::create_dir(&sibling).unwrap();
        let _sibling = directory(&sibling);
        assert!(matches!(host_consumers(&held), HostScan::Clear { .. }));
        let other = directory(&allocation);
        assert!(matches!(host_consumers(&held), HostScan::Hit { .. }));
        drop(other);
        let file = std::fs::File::create(allocation.join("open-file")).unwrap();
        assert!(matches!(host_consumers(&held), HostScan::Hit { .. }));
        drop(file);
        assert!(matches!(host_consumers(&held), HostScan::Clear { .. }));
    }

    #[test]
    fn live_cwd_in_allocation_hits() {
        let root = tempfile::tempdir().unwrap();
        let held = directory(root.path());
        let _consumer = Consumer::start(root.path(), "printf ready; read answer", root.path());
        assert!(matches!(host_consumers(&held), HostScan::Hit { .. }));
    }

    #[test]
    fn live_open_file_in_allocation_hits() {
        let root = tempfile::tempdir().unwrap();
        let allocation = root.path().join("allocation");
        std::fs::create_dir(&allocation).unwrap();
        let file = allocation.join("file");
        std::fs::write(&file, b"held").unwrap();
        let held = directory(&allocation);
        let _consumer = Consumer::start(root.path(), "exec 3<\"$1\"; printf ready; read answer", &file);
        assert!(matches!(host_consumers(&held), HostScan::Hit { .. }));
    }

    #[test]
    fn current_kernel_path_catches_cwd_renamed_into_allocation() {
        let root = tempfile::tempdir().unwrap();
        let allocation = root.path().join("allocation");
        let outside = root.path().join("outside");
        std::fs::create_dir(&allocation).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let held = directory(&allocation);
        let consumer = Consumer::start(&outside, "printf ready; read answer", &outside);
        assert!(matches!(host_consumers(&held), HostScan::Clear { .. }));
        std::fs::rename(&outside, allocation.join("moved")).unwrap();
        assert!(matches!(host_consumers(&held), HostScan::Hit { .. }));
        drop(consumer);
        assert!(matches!(host_consumers(&held), HostScan::Clear { .. }));
    }
}
