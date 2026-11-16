use super::*;

#[derive(Debug, PartialEq, Eq)]
enum Location {
    SourceRoot,
    DestinationRoot,
    NestedSource,
    NestedDestination,
}

#[derive(Debug)]
struct MountRefusal {
    path: PathBuf,
    old: PathBuf,
    new: PathBuf,
    location: Location,
    details: Option<(String, String)>,
}

impl std::fmt::Display for MountRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is a mount point", self.path.display())?;
        if let Some((source, fstype)) = &self.details {
            write!(f, " (source {source}, filesystem {fstype})")?;
        }
        write!(f, "; `fractal migrate host` performs only same-filesystem renames and never copies. ")?;
        if matches!(self.location, Location::DestinationRoot | Location::NestedDestination) {
            write!(f, "Unmount or relocate this mount before cutover; {} must be absent or an empty ordinary directory. Do not remount populated legacy data there and rerun: the mover requires a real legacy directory at {} and refuses a populated destination.", self.new.display(), self.old.display())
        } else {
            let suffix = self.path.strip_prefix(&self.old).map_err(|_| std::fmt::Error)?;
            let target = if suffix.as_os_str().is_empty() { self.new.clone() } else { self.new.join(suffix) };
            write!(f, "Stop the legacy units and move this mount out of {} before cutover, preserving its data in ordinary directories under {}. Restart the legacy daemon, run `fractal migrate host`, then stop the Fractal units and remount at {} without hiding migrated data before restarting Fractal. Remounting directly at {} with {} absent or a bridge symlink cannot complete cutover: the mover requires real legacy directories and refuses populated destinations.", self.old.display(), self.old.display(), target.display(), target.display(), self.old.display())
        }
    }
}

impl std::error::Error for MountRefusal {}

#[derive(Debug, PartialEq, Eq)]
struct Mount {
    path: PathBuf,
    source: String,
    fstype: String,
}

#[cfg(any(target_os = "linux", test))]
fn decode(field: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] == b'\\' {
            let octal = field.get(index + 1..index + 4).context("truncated mountinfo escape")?;
            ensure!(octal.iter().all(|byte| (b'0'..=b'7').contains(byte)), "invalid mountinfo escape");
            let value = u16::from(octal[0] - b'0') * 64 + u16::from(octal[1] - b'0') * 8 + u16::from(octal[2] - b'0');
            decoded.push(u8::try_from(value).context("mountinfo escape exceeds one byte")?);
            index += 4;
        } else {
            decoded.push(field[index]);
            index += 1;
        }
    }
    Ok(decoded)
}

#[cfg(any(target_os = "linux", test))]
fn parse_mountinfo(text: &[u8]) -> Result<Vec<Mount>> {
    use std::os::unix::ffi::OsStringExt as _;
    text.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()).map(|line| {
        let fields: Vec<_> = line.split(|byte| *byte == b' ').collect();
        let separator = fields.iter().position(|field| *field == b"-").context("mountinfo separator missing")?;
        ensure!(separator >= 6 && fields.len() >= separator + 4, "incomplete mountinfo record");
        Ok(Mount {
            path: PathBuf::from(std::ffi::OsString::from_vec(decode(fields[4])?)),
            fstype: String::from_utf8(decode(fields[separator + 1])?)?,
            source: String::from_utf8_lossy(&decode(fields[separator + 2])?).into_owned(),
        })
    }).collect()
}

#[cfg(test)]
fn check_mounts(old: &Path, new: &Path, mounts: &[Mount]) -> Result<()> {
    check_mounts_for(old, new, mounts, None, false)
}

fn check_mounts_for(old: &Path, new: &Path, mounts: &[Mount], expected_store: Option<&Path>, rollback: bool) -> Result<()> {
    for mount in mounts {
        let location = if mount.path == old {
            Location::SourceRoot
        } else if mount.path == new {
            Location::DestinationRoot
        } else if mount.path.starts_with(old) {
            Location::NestedSource
        } else if mount.path.starts_with(new) {
            Location::NestedDestination
        } else {
            continue;
        };
        if rollback {
            bail!("{} is a mount point; unmount it before `fractal migrate host --rollback`; rollback never moves a mounted product root or submount", mount.path.display());
        }
        if expected_store == Some(mount.path.as_path()) && mount.path != old && mount.path != new { continue; }
        return Err(MountRefusal {
            path: mount.path.clone(), old: old.to_owned(), new: new.to_owned(), location,
            details: Some((mount.source.clone(), mount.fstype.clone())),
        }.into());
    }
    Ok(())
}

// Resolve symlinked XDG ancestors before comparing with kernel mount paths.
pub(super) fn resolved(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => normalize_resolved(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("product root has no parent")?;
            Ok(normalize_resolved(fs::canonicalize(parent)?)?.join(path.file_name().context("product root has no filename")?))
        }
        Err(error) => Err(error).with_context(|| format!("resolve product root {}", path.display())),
    }
}

#[cfg(any(target_os = "macos", test))]
fn data_volume_alias(path: &Path) -> Option<PathBuf> {
    let suffix = path.strip_prefix("/System/Volumes/Data").ok()?;
    Some(Path::new("/").join(suffix))
}

fn normalize_resolved(path: PathBuf) -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(alias) = data_volume_alias(&path) {
        match fs::metadata(&alias) {
            Ok(alias_metadata) => {
                let metadata = fs::metadata(&path)?;
                // Only fold a Data-volume spelling when both paths name the
                // same object; unrelated Data directories are not firmlinks.
                if metadata.dev() == alias_metadata.dev() && metadata.ino() == alias_metadata.ino() {
                    return Ok(fs::canonicalize(alias)?);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect macOS firmlink alias"),
        }
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn macos_mounts() -> Result<Vec<Mount>> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt as _;

    // getfsstat writes into caller-owned storage, unlike getmntinfo's shared
    // static buffer, which another thread could invalidate.
    // SAFETY: a null buffer with length zero only queries the mount count.
    let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count < 0 { return Err(std::io::Error::last_os_error()).context("count macOS mounts"); }
    let capacity = usize::try_from(count)? + 1;
    let mut entries = Vec::<libc::statfs>::with_capacity(capacity);
    let size = i32::try_from(capacity.checked_mul(std::mem::size_of::<libc::statfs>()).context("macOS mount buffer overflow")?)?;
    // SAFETY: the buffer has capacity for size bytes of statfs records. The
    // kernel initializes at most that many complete entries, whose count it returns.
    let count = unsafe { libc::getfsstat(entries.as_mut_ptr(), size, libc::MNT_NOWAIT) };
    if count < 0 { return Err(std::io::Error::last_os_error()).context("enumerate macOS mounts"); }
    let count = usize::try_from(count)?;
    ensure!(count < capacity, "macOS mount topology changed during preflight; rerun `fractal migrate host`");
    // SAFETY: getfsstat initialized exactly count entries within the allocation.
    unsafe { entries.set_len(count); }
    entries.iter().map(|entry| {
        // SAFETY: successful getfsstat returns NUL-terminated mount path, source
        // and filesystem-name arrays in each initialized statfs record.
        let (path, source, fstype) = unsafe {
            (
                CStr::from_ptr(entry.f_mntonname.as_ptr()),
                CStr::from_ptr(entry.f_mntfromname.as_ptr()),
                CStr::from_ptr(entry.f_fstypename.as_ptr()),
            )
        };
        Ok(Mount {
            path: resolved(&PathBuf::from(std::ffi::OsString::from_vec(path.to_bytes().to_vec())))?,
            source: source.to_string_lossy().into_owned(),
            fstype: fstype.to_string_lossy().into_owned(),
        })
    }).collect()
}

pub(super) fn preflight(pairs: &[(PathBuf, PathBuf)]) -> Result<()> {
    preflight_for(pairs, None, false)
}

pub(super) fn rollback_preflight(pairs: &[(PathBuf, PathBuf)]) -> Result<()> {
    preflight_for(pairs, None, true)
}

pub(super) fn committed_preflight(pairs: &[(PathBuf, PathBuf)], store: Option<&Path>) -> Result<()> {
    preflight_for(pairs, store, false)
}

fn preflight_for(pairs: &[(PathBuf, PathBuf)], store: Option<&Path>, rollback: bool) -> Result<()> {
    let store = store.map(resolved).transpose()?;
    #[cfg(target_os = "linux")]
    let mounts = parse_mountinfo(&fs::read("/proc/self/mountinfo").context("read mountinfo before host cutover")?)?;
    #[cfg(target_os = "macos")]
    let mounts = macos_mounts()?;
    for (old, new) in pairs {
        let old = resolved(old)?;
        let new = resolved(new)?;
        check_mounts_for(&old, &new, &mounts, store.as_deref(), rollback)?;
        // Device identity remains an independent root-mount backstop.
        for (path, location) in [(&old, Location::SourceRoot), (&new, Location::DestinationRoot)] {
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error).with_context(|| format!("inspect mount point {}", path.display())),
            };
            let parent = path.parent().context("product root has no parent")?;
            if metadata.is_dir() && metadata.dev() != fs::metadata(parent)?.dev() {
                if rollback {
                    bail!("{} is a mount point; unmount it before `fractal migrate host --rollback`; rollback never moves a mounted product root or submount", path.display());
                }
                return Err(MountRefusal { path: path.clone(), old: old.clone(), new: new.clone(), location, details: None }.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_store_mount_is_the_only_expected_submount() -> Result<()> {
        let old = Path::new("/state/st3");
        let new = Path::new("/state/fractal");
        let store = new.join("store");
        let mounts = [Mount { path: store.clone(), source: "pool/store".into(), fstype: "zfs".into() }];
        assert!(check_mounts_for(old, new, &mounts, None, false).is_err());
        check_mounts_for(old, new, &mounts, Some(&store), false)?;
        for path in [new.to_path_buf(), new.join("other"), store.join("nested"), old.join("store")] {
            let mounts = [Mount { path, source: "pool/other".into(), fstype: "zfs".into() }];
            assert!(check_mounts_for(old, new, &mounts, Some(&store), false).is_err());
        }
        Ok(())
    }

    #[test]
    fn rollback_refuses_all_product_mounts_with_unmount_guidance() -> Result<()> {
        let old = Path::new("/state/st3");
        let new = Path::new("/state/fractal");
        let store = new.join("store");
        for path in [old.to_path_buf(), new.to_path_buf(), old.join("store"), store.clone()] {
            let mounts = [Mount { path: path.clone(), source: "pool/store".into(), fstype: "zfs".into() }];
            let error = check_mounts_for(old, new, &mounts, Some(&store), true).unwrap_err();
            assert_eq!(error.to_string(), format!("{} is a mount point; unmount it before `fractal migrate host --rollback`; rollback never moves a mounted product root or submount", path.display()));
        }
        let mounts = [Mount { path: "/external/store".into(), source: "pool/store".into(), fstype: "zfs".into() }];
        check_mounts_for(old, new, &mounts, None, true)?;
        Ok(())
    }

    #[test]
    fn parses_mountinfo_escapes_and_optional_fields() -> Result<()> {
        let mounts = parse_mountinfo(b"36 25 0:32 / /state/st3/store\\040with\\011tab\\012line\\134slash rw shared:7 - zfs pool/data\\040set rw\n")?;
        assert_eq!(mounts, vec![Mount {
            path: PathBuf::from("/state/st3/store with\ttab\nline\\slash"),
            source: "pool/data set".into(), fstype: "zfs".into(),
        }]);
        for malformed in [b"bad".as_slice(), b"36 25 0:32 / /bad\\04 rw - zfs pool rw", b"36 25 0:32 / /bad\\999 rw - zfs pool rw"] {
            assert!(parse_mountinfo(malformed).is_err());
        }
        Ok(())
    }

    #[test]
    fn classifies_mounts_and_reports_exact_guidance() -> Result<()> {
        let old = Path::new("/state/st3");
        let new = Path::new("/state/fractal");
        for (path, location, target) in [
            ("/state/st3", Location::SourceRoot, "/state/fractal"),
            ("/state/fractal", Location::DestinationRoot, "/state/fractal"),
            ("/state/st3/store", Location::NestedSource, "/state/fractal/store"),
            ("/state/fractal/store", Location::NestedDestination, "/state/fractal/store"),
        ] {
            let text = format!("36 25 0:32 / {path} rw - zfs pool/state rw\n");
            let mounts = parse_mountinfo(text.as_bytes())?;
            let error = check_mounts(old, new, &mounts).unwrap_err();
            let refusal = error.downcast_ref::<MountRefusal>().unwrap();
            assert_eq!(refusal.location, location);
            let guidance = if matches!(location, Location::DestinationRoot | Location::NestedDestination) {
                "Unmount or relocate this mount before cutover; /state/fractal must be absent or an empty ordinary directory. Do not remount populated legacy data there and rerun: the mover requires a real legacy directory at /state/st3 and refuses a populated destination.".to_owned()
            } else {
                format!("Stop the legacy units and move this mount out of /state/st3 before cutover, preserving its data in ordinary directories under /state/st3. Restart the legacy daemon, run `fractal migrate host`, then stop the Fractal units and remount at {target} without hiding migrated data before restarting Fractal. Remounting directly at {target} with /state/st3 absent or a bridge symlink cannot complete cutover: the mover requires real legacy directories and refuses populated destinations.")
            };
            assert_eq!(error.to_string(), format!("{path} is a mount point (source pool/state, filesystem zfs); `fractal migrate host` performs only same-filesystem renames and never copies. {guidance}"));
        }
        let mounts = parse_mountinfo(b"36 25 0:32 / /state/st3-other/store rw - zfs pool/state rw\n")?;
        check_mounts(old, new, &mounts)?;
        Ok(())
    }

    #[test]
    fn macos_mount_records_use_the_shared_classifier() -> Result<()> {
        for root in ["/Users/example/.local/state", "/private/var/operator/state"] {
            let old = Path::new(root).join("st3");
            let new = Path::new(root).join("fractal");
            for (suffix, location) in [
                ("st3/store", Location::NestedSource),
                ("fractal/store", Location::NestedDestination),
                ("st3", Location::SourceRoot),
                ("fractal", Location::DestinationRoot),
            ] {
                let physical = Path::new("/System/Volumes/Data").join(root.strip_prefix('/').unwrap()).join(suffix);
                let alias = data_volume_alias(&physical).unwrap();
                assert_eq!(alias, Path::new(root).join(suffix));
                let mounts = [Mount { path: alias, source: "/dev/disk4s1".into(), fstype: "apfs".into() }];
                let error = check_mounts(&old, &new, &mounts).unwrap_err();
                assert_eq!(error.downcast_ref::<MountRefusal>().unwrap().location, location);
                assert!(error.to_string().contains("(source /dev/disk4s1, filesystem apfs)"));
            }
            for suffix in ["st3-other/store", "fractal-other/store"] {
                let mounts = [Mount { path: Path::new(root).join(suffix), source: "/dev/disk4s1".into(), fstype: "apfs".into() }];
                check_mounts(&old, &new, &mounts)?;
            }
        }
        assert_eq!(data_volume_alias(Path::new("/System/Volumes/Data-other/Users")), None);
        assert_eq!(data_volume_alias(Path::new("/Users/example")), None);
        Ok(())
    }

    #[test]
    fn root_device_fallback_omits_unavailable_mount_details() {
        let refusal = MountRefusal {
            path: "/state/st3".into(), old: "/state/st3".into(), new: "/state/fractal".into(),
            location: Location::SourceRoot, details: None,
        };
        assert!(refusal.to_string().starts_with("/state/st3 is a mount point; `fractal migrate host` performs only same-filesystem renames and never copies."));
    }
}
