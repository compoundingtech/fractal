//! Kernel evidence for a recorded PTY owner. PID existence and socket reachability
//! cannot prove an incarnation, or authorize signalling it.

use pty_core::registry::SessionInfo;

pub(crate) const BOOT_TAG: &str = "st3.boot-id";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Owner {
    Matching,
    Mismatch,
    Gone,
    Unknown,
}

pub(crate) fn boot_identity() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        let boot = boot.trim();
        valid_boot(boot).then(|| boot.to_owned())
    }
    #[cfg(target_os = "macos")]
    {
        let mut value = [0u8; 37];
        let mut length = value.len();
        let result = unsafe {
            libc::sysctlbyname(
                c"kern.bootsessionuuid".as_ptr(),
                value.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 || length != 37 || value[36] != 0 {
            return None;
        }
        let boot = std::str::from_utf8(&value[..36]).ok()?;
        valid_boot(boot).then(|| boot.to_owned())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn valid_boot(boot: &str) -> bool {
    boot.len() == 36
        && boot.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn valid_start(token: &str) -> bool {
    token
        .strip_prefix("linux:")
        .is_some_and(|value| value.parse::<u64>().is_ok())
        || token
            .strip_prefix("darwin:")
            .is_some_and(|value| !value.trim().is_empty())
}

/// Absent or malformed original evidence cannot be repaired from today's PID
/// or wall clock. Legacy records retain their registry and scope evidence.
pub(crate) fn missing_recorded_identity(session: &SessionInfo) -> Option<&'static str> {
    let metadata = session.metadata.as_ref();
    let start = metadata
        .and_then(|metadata| metadata.daemon_start_token())
        .is_some_and(valid_start);
    let boot = metadata
        .and_then(|metadata| metadata.tags.as_ref()?.get(BOOT_TAG))
        .is_some_and(|boot| valid_boot(boot));
    match (start, boot) {
        (true, true) => None,
        (false, true) => Some("recorded daemon start token"),
        (true, false) => Some("recorded boot identity (st3.boot-id)"),
        (false, false) => Some("recorded daemon start token and boot identity (st3.boot-id)"),
    }
}

pub(crate) fn owner(session: &SessionInfo) -> Owner {
    let pid = session
        .pid
        .or_else(|| session.metadata.as_ref()?.daemon_pid);
    let Some(pid) = pid.filter(|pid| *pid > 0) else {
        return Owner::Unknown;
    };
    #[cfg(target_os = "linux")]
    {
        linux_owner(
            session,
            pid,
            |name| std::fs::read_to_string(format!("/proc/{pid}/{name}")),
            boot_identity().as_deref(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        match pty_core::proctable::has_exited(pid) {
            pty_core::proctable::Answer::NotPresent | pty_core::proctable::Answer::Known(true) => {
                return Owner::Gone;
            }
            pty_core::proctable::Answer::Unknown(_) => return Owner::Unknown,
            pty_core::proctable::Answer::Known(false) => {}
        }
        let Some(start) = pty_core::registry::read_process_start_token(pid) else {
            return Owner::Unknown;
        };
        let metadata = session.metadata.as_ref();
        compare(
            pid,
            metadata.and_then(|metadata| metadata.daemon_pid),
            metadata.and_then(|metadata| metadata.daemon_start_token()),
            metadata
                .and_then(|metadata| metadata.tags.as_ref()?.get(BOOT_TAG))
                .map(String::as_str),
            &start,
            boot_identity().as_deref(),
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Owner::Unknown
    }
}

#[cfg(target_os = "linux")]
fn linux_owner(
    session: &SessionInfo,
    pid: i32,
    read: impl Fn(&str) -> std::io::Result<String>,
    boot: Option<&str>,
) -> Owner {
    let status = match read("status") {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Owner::Gone,
        Err(_) => return Owner::Unknown,
    };
    let Some(tgid) = status
        .lines()
        .find_map(|line| line.strip_prefix("Tgid:")?.trim().parse::<i32>().ok())
    else {
        return Owner::Unknown;
    };
    if tgid != pid {
        return Owner::Mismatch;
    }
    let stat = match read("stat") {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Owner::Gone,
        Err(_) => return Owner::Unknown,
    };
    let Some((_, tail)) = stat.rsplit_once(") ") else {
        return Owner::Unknown;
    };
    let fields: Vec<_> = tail.split_whitespace().collect();
    let Some(start) = fields.get(19).filter(|start| start.parse::<u64>().is_ok()) else {
        return Owner::Unknown;
    };
    if matches!(fields.first(), Some(&"Z" | &"X")) {
        return Owner::Gone;
    }
    let metadata = session.metadata.as_ref();
    let recorded_start = metadata.and_then(|metadata| metadata.daemon_start_token());
    let recorded_boot = metadata.and_then(|metadata| metadata.tags.as_ref()?.get(BOOT_TAG));
    compare(
        pid,
        metadata.and_then(|metadata| metadata.daemon_pid),
        recorded_start,
        recorded_boot.map(String::as_str),
        &format!("linux:{start}"),
        boot,
    )
}

fn compare(
    pid: i32,
    recorded_pid: Option<i32>,
    recorded_start: Option<&str>,
    recorded_boot: Option<&str>,
    start: &str,
    boot: Option<&str>,
) -> Owner {
    let recorded_start = recorded_start.filter(|token| valid_start(token));
    if recorded_pid.is_some_and(|recorded| recorded != pid)
        || recorded_start.is_some_and(|recorded| recorded != start)
        || recorded_boot
            .filter(|boot| valid_boot(boot))
            .zip(boot)
            .is_some_and(|(recorded, boot)| recorded != boot)
    {
        return Owner::Mismatch;
    }
    if recorded_start.is_none()
        || recorded_boot.is_none_or(|boot| !valid_boot(boot))
        || boot.is_none()
    {
        return Owner::Unknown;
    }
    Owner::Matching
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn unreadable_and_incomplete_evidence_is_unknown() {
        let boot = "01234567-89ab-cdef-0123-456789abcdef";
        assert_eq!(
            compare(42, Some(42), Some("linux:7"), Some(boot), "linux:7", None),
            Owner::Unknown
        );
        for missing in [None, Some(""), Some("malformed")] {
            assert_eq!(
                compare(
                    42,
                    Some(42),
                    Some("linux:7"),
                    missing,
                    "linux:7",
                    Some(boot)
                ),
                Owner::Unknown
            );
        }
        assert_eq!(
            compare(42, Some(42), None, Some(boot), "linux:7", Some(boot)),
            Owner::Unknown
        );
    }

    #[test]
    fn known_mismatch_is_never_downgraded_to_unknown() {
        assert_eq!(
            compare(42, Some(41), None, None, "linux:7", None),
            Owner::Mismatch
        );
        assert_eq!(
            compare(42, Some(42), Some("linux:6"), None, "linux:7", None),
            Owner::Mismatch
        );
    }

    #[test]
    fn an_unreadable_kernel_identity_never_proves_the_owner_live() {
        let session = SessionInfo {
            name: "orchid".into(),
            socket_path: "orchid.sock".into(),
            pid: Some(42),
            status: pty_core::registry::SessionStatus::Running,
            metadata: None,
        };
        for unreadable in ["status", "stat"] {
            let result = linux_owner(
                &session,
                42,
                |path| {
                    if path == unreadable {
                        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                    } else {
                        Ok("Tgid:\t42\n".into())
                    }
                },
                Some("01234567-89ab-cdef-0123-456789abcdef"),
            );
            assert_eq!(result, Owner::Unknown);
        }
    }
}
