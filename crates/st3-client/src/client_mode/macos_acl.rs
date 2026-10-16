//! Read-only descriptor inspection. This does not initialize authority, take an
//! exclusive lease, or decide Local/Client. The Mac-unavailable enforcement guard
//! remains until native fixtures and the complete call-site integration qualify.
use std::{fs::File, os::fd::AsRawFd};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AclDisposition {
    Accepted,
    QueryFailed,
    InvalidRepresentation,
    Unsupported,
    AllowGrant,
    UnresolvedQualifier,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AclObservation {
    pub disposition: AclDisposition,
    pub entries: u32,
    // OS errors contain no file contents, credential bytes, or account names.
    pub os_error: Option<i32>,
    pub observed_maximum: u64,
    pub expected_maximum: u64,
}

#[repr(C)]
#[derive(Default)]
struct NativeObservation {
    reason: u32,
    os_error: i32,
    entries: u32,
    observed_maximum: u64,
    expected_maximum: u64,
}

unsafe extern "C" {
    fn st_client_mode_acl_inspect(fd: i32, private_object: i32, out: *mut NativeObservation);
}

/// Inspect the exact retained descriptor; never reopen a path or alter its ACL.
/// `private_object` forbids all allow entries, including read/inherited grants.
pub fn inspect(file: &File, private_object: bool) -> AclObservation {
    let mut native = NativeObservation::default();
    // SAFETY: file keeps its descriptor alive; out is aligned writable storage of
    // the identical C layout, and the synchronous adapter retains neither pointer.
    unsafe {
        st_client_mode_acl_inspect(file.as_raw_fd(), i32::from(private_object), &mut native);
    }
    let disposition = match native.reason {
        0 => AclDisposition::Accepted,
        1 => AclDisposition::QueryFailed,
        2 => AclDisposition::InvalidRepresentation,
        3 => AclDisposition::Unsupported,
        4 => AclDisposition::AllowGrant,
        5 => AclDisposition::UnresolvedQualifier,
        _ => AclDisposition::Unsupported,
    };
    AclObservation {
        disposition,
        entries: native.entries,
        observed_maximum: native.observed_maximum,
        expected_maximum: native.expected_maximum,
        os_error: (native.os_error != 0).then_some(native.os_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, PermissionsExt},
        },
        process::Command,
    };

    fn fixture_root() -> tempfile::TempDir {
        // Fixed root-owned APFS temp venue, not caller-selected TMPDIR. Inspect
        // retained ancestors before creating anything; only /private/tmp may be
        // other-writable, with root-owned sticky semantics and no mutating ACL.
        let mut parent = File::open("/").unwrap();
        for (name, sticky_tmp) in [("", false), ("private", false), ("tmp", true)] {
            if !name.is_empty() {
                let name = std::ffi::CString::new(name).unwrap();
                let fd = unsafe {
                    libc::openat(
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                assert!(fd >= 0);
                parent = unsafe { File::from_raw_fd(fd) };
            }
            let metadata = parent.metadata().unwrap();
            assert_eq!(metadata.uid(), 0);
            assert!(metadata.mode() & 0o022 == 0 || (sticky_tmp && metadata.mode() & 0o1000 != 0));
            assert_eq!(
                inspect(&parent, false).disposition,
                AclDisposition::Accepted
            );
        }
        let root = tempfile::Builder::new()
            .prefix("st-client-acl-fixture-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/private/tmp")
            .unwrap();
        let name =
            std::ffi::CString::new(root.path().file_name().unwrap().as_encoded_bytes()).unwrap();
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(fd >= 0);
        let retained = unsafe { File::from_raw_fd(fd) };
        let metadata = retained.metadata().unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o777, 0o700);
        // Refuse inherited allow/deferred/unknown ACLs. Do not try to repair them
        // with path chmod before custody of the fixture root is established.
        assert_eq!(
            inspect(&retained, true).disposition,
            AclDisposition::Accepted
        );
        root
    }

    unsafe extern "C" {
        fn st_client_mode_acl_check_export(bytes: *const u8, size: usize) -> u32;
        fn st_client_mode_acl_export_layout(out: *mut usize);
        fn acl_copy_int_native(bytes: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn acl_set_fd_np(fd: i32, acl: *mut std::ffi::c_void, kind: i32) -> i32;
        fn acl_free(acl: *mut std::ffi::c_void) -> i32;
        fn st_client_mode_acl_query_result(
            result: i32,
            error: i32,
            out: *mut NativeObservation,
        ) -> u32;
        fn st_client_mode_acl_check_entry(
            flags: u32,
            rights: u64,
            tag: i32,
            accessor_rights: u64,
        ) -> u32;
    }

    #[test]
    fn malformed_exported_buffers_fail_closed() {
        let mut layout = [0usize; 8];
        unsafe { st_client_mode_acl_export_layout(layout.as_mut_ptr()) };
        let mut valid = vec![0u8; layout[0]];
        let put = |bytes: &mut [u8], offset: usize, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
        };
        put(&mut valid, layout[1], layout[6] as u32);
        put(&mut valid, layout[2], 1);
        put(&mut valid, layout[4], 2); // SDK ACL_EXTENDED_DENY
        let check =
            |bytes: &[u8]| unsafe { st_client_mode_acl_check_export(bytes.as_ptr(), bytes.len()) };
        assert_eq!(check(&valid), 0);
        let mut no_inherit = valid.clone();
        put(&mut no_inherit, layout[3], 1 << 17);
        assert_eq!(check(&no_inherit), 0, "known raw no-inherit flag");
        for (name, offset, value, reason) in [
            ("unknown-acl-flag", layout[3], 1 << 31, 3),
            ("reserved-raw-low-bit", layout[3], 1, 3),
            ("unsupported-deferred-inheritance", layout[3], 1 << 16, 3),
            ("unknown-entry-flag", layout[4], 2 | (1 << 31), 3),
            ("unknown-tag", layout[4], 15, 3),
            ("unknown-right", layout[5], 1 << 31, 3),
            ("wrong-count", layout[2], 2, 2),
            ("wrong-magic", layout[1], 0, 2),
        ] {
            let mut bytes = valid.clone();
            put(&mut bytes, offset, value);
            assert_eq!(check(&bytes), reason, "{name}");
        }
        assert_eq!(check(&valid[..valid.len() - 1]), 2);
        let mut unaligned = vec![0];
        unaligned.extend_from_slice(&valid);
        assert_eq!(check(&unaligned[1..]), 0);
        assert_eq!(unsafe { st_client_mode_acl_check_entry(2, 0, 2, 0) }, 0);
        assert_eq!(unsafe { st_client_mode_acl_check_entry(2, 2, 2, 0) }, 3);
        assert_eq!(unsafe { st_client_mode_acl_check_entry(2, 0, 1, 0) }, 3);
    }

    fn inspect_fixture(name: &str, ace: Option<&str>, private: bool) -> AclDisposition {
        let root = fixture_root();
        let path = root.path().join(name);
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
        }
        // Only the fresh, owned fixture directory is modified. No account-home
        // path, production authority, Keychain or credential operation is used.
        assert!(
            Command::new("/bin/chmod")
                .arg("-N")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        if let Some(ace) = ace {
            assert!(
                Command::new("/bin/chmod")
                    .args(["+a", ace])
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let file = File::open(&path).unwrap();
        let observation = inspect(&file, private);
        println!("fixture={name} private={private} observation={observation:?}");
        observation.disposition
    }

    #[test]
    fn empty_acl_and_stock_deny_delete_are_accepted() {
        assert_eq!(
            inspect_fixture("empty", None, false),
            AclDisposition::Accepted
        );
        assert_eq!(
            inspect_fixture("stock-home-deny", Some("everyone deny delete"), false),
            AclDisposition::Accepted
        );
        assert_eq!(
            inspect_fixture("private-deny", Some("everyone deny delete"), true),
            AclDisposition::Accepted
        );
    }

    #[test]
    fn mutation_grants_refuse_even_when_inherit_only() {
        for (name, ace) in [
            ("write", "everyone allow write"),
            ("delete-child", "everyone allow delete_child"),
            ("acl-write", "everyone allow writesecurity"),
            ("inherited", "everyone allow write,inherited"),
            (
                "inherit-only",
                "everyone allow write,file_inherit,only_inherit",
            ),
        ] {
            assert_eq!(
                inspect_fixture(name, Some(ace), false),
                AclDisposition::AllowGrant
            );
        }
    }

    #[test]
    fn private_objects_refuse_read_and_future_child_grants() {
        for (name, ace) in [
            ("read", "everyone allow read"),
            ("read-inherited", "everyone allow read,inherited"),
            (
                "future-file",
                "everyone allow read,file_inherit,only_inherit",
            ),
        ] {
            assert_eq!(
                inspect_fixture(name, Some(ace), true),
                AclDisposition::AllowGrant
            );
        }
    }

    #[test]
    fn non_file_descriptor_is_explicitly_unsupported() {
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut out = NativeObservation::default();
        unsafe { st_client_mode_acl_inspect(socket.as_raw_fd(), 0, &mut out) };
        assert_eq!(out.reason, 3);
    }

    #[test]
    fn unresolved_allow_qualifier_is_not_silently_accepted() {
        use std::os::unix::fs::OpenOptionsExt;
        let root = fixture_root();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.path().join("unresolved-uuid"))
            .unwrap();
        let mut layout = [0usize; 8];
        unsafe { st_client_mode_acl_export_layout(layout.as_mut_ptr()) };
        let mut bytes = vec![0u8; layout[0]];
        for (offset, value) in [
            (layout[1], layout[6] as u32),
            (layout[2], 1),
            (layout[4], 1),
            (layout[5], 1 << 1),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
        }
        // Fixed synthetic UUID, not a queried account or credential. A surprising
        // venue mapping must fail this fixture rather than be silently accepted.
        bytes[layout[7]..layout[7] + 16].copy_from_slice(&[0xa5; 16]);
        // The OS importer may access its public struct directly: provide aligned
        // storage, unlike our own parser which explicitly supports unaligned bytes.
        let mut aligned = vec![0u64; bytes.len().div_ceil(8)];
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                aligned.as_mut_ptr().cast::<u8>(),
                bytes.len(),
            )
        };
        let acl = unsafe { acl_copy_int_native(aligned.as_ptr().cast()) };
        assert!(!acl.is_null());
        let assigned = unsafe { acl_set_fd_np(file.as_raw_fd(), acl, 0x100) };
        unsafe { acl_free(acl) };
        assert_eq!(assigned, 0);
        assert_eq!(
            inspect(&file, false).disposition,
            AclDisposition::UnresolvedQualifier
        );
    }

    #[test]
    fn query_enoent_is_not_missing_property() {
        let mut out = NativeObservation::default();
        assert_eq!(
            unsafe { st_client_mode_acl_query_result(-1, libc::ENOENT, &mut out) },
            1
        );
        assert_eq!(out.reason, 1);
        assert_eq!(out.os_error, libc::ENOENT);
    }

    #[test]
    fn query_failure_cannot_be_empty_acl() {
        let mut out = NativeObservation::default();
        unsafe { st_client_mode_acl_inspect(-1, 0, &mut out) };
        assert_eq!(out.reason, 1);
        assert_ne!(out.os_error, 0);
    }

    #[test]
    fn replaced_path_cannot_redirect_retained_descriptor_inspection() {
        let root = fixture_root();
        let path = root.path().join("authority");
        std::fs::create_dir(&path).unwrap();
        assert!(
            Command::new("/bin/chmod")
                .arg("-N")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let retained = File::open(&path).unwrap();
        std::fs::rename(&path, root.path().join("original")).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            Command::new("/bin/chmod")
                .args(["+a", "everyone allow write"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            inspect(&retained, true).disposition,
            AclDisposition::Accepted
        );
        assert_eq!(
            inspect(&File::open(&path).unwrap(), true).disposition,
            AclDisposition::AllowGrant
        );
    }
}
