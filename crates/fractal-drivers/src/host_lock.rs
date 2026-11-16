//! Process existence checks shared by native driver tests.

/// Whether `pid` is a live process. `kill(pid, 0)` succeeds for a live process we may signal, and
/// fails with `EPERM` for a live process we may NOT signal (still alive) — both count as alive.
pub fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let ret = unsafe { libc::kill(pid, 0) };
    if ret == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_alive_basics() {
        assert!(process_alive(std::process::id() as i32)); // ourselves
        assert!(!process_alive(0));
        assert!(!process_alive(-1));
        assert!(!process_alive(2_000_000_000)); // dead
    }
}
