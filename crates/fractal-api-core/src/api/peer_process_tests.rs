//! The accepted socket stays alive after its original connector exits.
use super::*;
use std::io::{Read as _, Write as _};
use std::os::fd::FromRawFd as _;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

const CHILD_SOCKET: &str = "FRACTAL_PEER_PROCESS_TEST_SOCKET";
const EXIT_BEFORE_CAPTURE: &str = "FRACTAL_PEER_PROCESS_TEST_EXIT_BEFORE_CAPTURE";

// Sending the connected FD back to the server deterministically models a socket
// inherited by a surviving process, without fork in the multithreaded test runner.
#[test]
fn connector_socket_child() {
    let Some(path) = std::env::var_os(CHILD_SOCKET) else { return };
    let mut socket = UnixStream::connect(path).unwrap();
    let fd = socket.as_raw_fd();
    let mut byte = [1_u8];
    let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
    let mut control = [0_usize; 8];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of_val(&fd) as _) } as usize;
    unsafe {
        // The aligned control buffer has CMSG_SPACE bytes available.
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(&fd) as _) as usize;
        std::ptr::write_unaligned(libc::CMSG_DATA(header).cast::<libc::c_int>(), fd);
        assert_eq!(libc::sendmsg(fd, &message, 0), 1);
    }
    if std::env::var_os(EXIT_BEFORE_CAPTURE).is_none() {
        socket.read_exact(&mut byte).unwrap();
    }
}

struct Connector(std::process::Child);
impl Drop for Connector {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn connector_exit_invalidates_witness_while_socket_fd_survives() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("peer.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let mut child = Connector(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "api::peer_process_tests::connector_socket_child", "--nocapture"])
        .env(CHILD_SOCKET, &path).stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap());
    let (stream, _) = listener.accept().await.unwrap();
    let identity = local_peer_process(&stream).unwrap();
    assert_eq!(identity.pid(), child.0.id());
    identity.validate().unwrap();
    let mut stream = stream.into_std().unwrap();
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut byte = [0_u8];
    let mut surviving_socket = receive_socket(&stream);
    stream.write_all(b"x").unwrap();
    assert!(child.0.wait().unwrap().success());
    assert!(identity.validate().is_err());
    // The surviving holder can still issue a request, but cannot inherit the
    // original connector's authority. Socket liveness is not process liveness.
    surviving_socket.write_all(b"y").unwrap();
    stream.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"y");
}

fn receive_socket(stream: &UnixStream) -> UnixStream {
    let mut byte = [0_u8];
    let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
    let mut control = [0_usize; 8];
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&control);
    unsafe {
        assert_eq!(libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC), 1);
        assert_eq!(message.msg_flags & libc::MSG_CTRUNC, 0);
        let header = libc::CMSG_FIRSTHDR(&message);
        assert!(!header.is_null());
        assert_eq!((*header).cmsg_level, libc::SOL_SOCKET);
        assert_eq!((*header).cmsg_type, libc::SCM_RIGHTS);
        assert_eq!((*header).cmsg_len, libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as _) as usize);
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<libc::c_int>());
        UnixStream::from_raw_fd(fd)
    }
}

#[tokio::test]
async fn connector_exit_before_first_capture_refuses_surviving_socket_holder() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("peer.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let mut child = Connector(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "api::peer_process_tests::connector_socket_child", "--nocapture"])
        .env(CHILD_SOCKET, &path).env(EXIT_BEFORE_CAPTURE, "1")
        .stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap());
    let connector_pid = child.0.id();
    // The queued SCM_RIGHTS reference keeps the connection alive while its
    // connector exits in the accept backlog, before any witness is captured.
    assert!(child.0.wait().unwrap().success());
    let (stream, _) = listener.accept().await.unwrap();
    assert_eq!(local_peer_pid(&stream), Some(connector_pid));
    assert!(local_peer_process(&stream).is_err());
    let mut stream = stream.into_std().unwrap();
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut surviving_socket = receive_socket(&stream);
    assert_ne!(connector_pid, std::process::id());
    surviving_socket.write_all(b"y").unwrap();
    let mut byte = [0_u8];
    stream.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"y");
    // Neither socket traffic nor the live process now holding the transferred
    // FD can become a replacement credential witness for the dead connector.
    stream.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(stream).unwrap();
    assert_eq!(local_peer_pid(&stream), Some(connector_pid));
    assert!(local_peer_process(&stream).is_err());
}
