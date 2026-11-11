use std::{io, os::fd::{FromRawFd, OwnedFd, RawFd}};

pub(super) fn pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: writable descriptor slots; successful descriptors get unique owners.
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Publish only this process's identity, never open a pidfd from a received PID.
///
/// # Safety
/// `socket` must be the live private child endpoint. Call from pre_exec before
/// readiness/gate waiting; on false the caller must immediately `_exit`.
/// No allocation, locks, destructors or errno access occurs on this path.
pub(super) unsafe fn publish(socket: RawFd) -> bool {
    // SAFETY: syscall arguments and stack buffers below live through each call.
    unsafe {
        let pid = libc::syscall(libc::SYS_getpid) as libc::pid_t;
        let fd = libc::syscall(libc::SYS_pidfd_open, pid, 0_u32);
        if fd < 0 { return false; }
        let mut bytes = pid.to_ne_bytes();
        let mut iov = libc::iovec { iov_base: bytes.as_mut_ptr().cast(), iov_len: bytes.len() };
        // usize storage supplies cmsghdr alignment, including its trailing data.
        let mut control = [0_usize; 8];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(size_of::<RawFd>() as u32) as usize;
        let header = libc::CMSG_FIRSTHDR(&msg);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(size_of::<RawFd>() as u32) as usize;
        libc::CMSG_DATA(header).cast::<RawFd>().write(fd as RawFd);
        let sent = libc::syscall(libc::SYS_sendmsg, socket, &msg, libc::MSG_NOSIGNAL);
        libc::syscall(libc::SYS_close, fd);
        sent == bytes.len() as libc::c_long
    }
}

pub(super) fn receive(socket: RawFd) -> io::Result<(u32, OwnedFd)> {
    let mut bytes = [0; size_of::<libc::pid_t>()];
    let mut control = [0_usize; 8];
    let mut iov = libc::iovec { iov_base: bytes.as_mut_ptr().cast(), iov_len: bytes.len() };
    loop {
        // SAFETY: all pointers reference live, aligned buffers of the given sizes.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = size_of_val(&control);
        let received = unsafe { libc::recvmsg(socket, &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if received < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted { continue; }
            return Err(error);
        }
        let mut descriptor = None;
        let mut count = 0;
        let mut valid = received == bytes.len() as isize
            && msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) == 0;
        // SAFETY: the kernel constructs valid ancillary headers. Walk all of
        // them even after rejection, owning/closing every delivered rights fd.
        // Linux closes rights omitted from a truncated ancillary buffer itself.
        unsafe {
            let mut header = libc::CMSG_FIRSTHDR(&msg);
            while !header.is_null() {
                if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                    let length = (*header).cmsg_len - libc::CMSG_LEN(0) as usize;
                    valid &= length == size_of::<RawFd>();
                    for index in 0..length / size_of::<RawFd>() {
                        let fd = OwnedFd::from_raw_fd(libc::CMSG_DATA(header).cast::<RawFd>().add(index).read());
                        count += 1;
                        if descriptor.is_none() { descriptor = Some(fd); }
                    }
                } else { valid = false; }
                header = libc::CMSG_NXTHDR(&msg, header);
            }
        }
        let pid = libc::pid_t::from_ne_bytes(bytes);
        if valid && count == 1 && pid > 0
            && let Some(fd) = descriptor
        { return Ok((pid as u32, fd)); }
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid child pidfd publication"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn transfers_self_pidfd_cloexec() {
        let (parent, child) = pair().unwrap();
        assert!(unsafe { publish(child.as_raw_fd()) });
        let (pid, fd) = receive(parent.as_raw_fd()).unwrap();
        assert_eq!(pid, std::process::id());
        assert_ne!(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
        assert_eq!(unsafe { libc::syscall(libc::SYS_pidfd_send_signal, fd.as_raw_fd(), 0, std::ptr::null::<libc::siginfo_t>(), 0_u32) }, 0);
    }

    #[test]
    fn closed_peer_rejects_publication_and_receive() {
        let (parent, child) = pair().unwrap();
        drop(parent);
        assert!(!unsafe { publish(child.as_raw_fd()) });
        assert!(receive(child.as_raw_fd()).is_err());
    }
}
