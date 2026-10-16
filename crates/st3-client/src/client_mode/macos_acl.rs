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
}

#[repr(C)]
#[derive(Default)]
struct NativeObservation {
    reason: u32,
    os_error: i32,
    entries: u32,
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
        os_error: (native.os_error != 0).then_some(native.os_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, process::Command};

    fn inspect_fixture(name: &str, ace: Option<&str>, private: bool) -> AclDisposition {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
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
            assert_ne!(
                inspect_fixture(name, Some(ace), false),
                AclDisposition::Accepted
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
            assert_ne!(
                inspect_fixture(name, Some(ace), true),
                AclDisposition::Accepted
            );
        }
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
        let root = tempfile::tempdir().unwrap();
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
        assert_ne!(
            inspect(&File::open(&path).unwrap(), true).disposition,
            AclDisposition::Accepted
        );
    }
}
