use super::*;

// Every FFI buffer below is fixed-layout, initialized, and checked for a complete kernel result.
fn socket_option<T: Copy>(stream: &UnixStream, level: i32, option: i32) -> Result<T> {
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let mut length = std::mem::size_of::<T>() as libc::socklen_t;
    let result = unsafe { libc::getsockopt(stream.as_raw_fd(), level, option, value.as_mut_ptr().cast(), &mut length) };
    ensure!(result == 0 && length as usize == std::mem::size_of::<T>(), "kernel peer credentials unavailable: {}", std::io::Error::last_os_error());
    Ok(unsafe { value.assume_init() })
}

#[cfg(target_os = "linux")]
pub(super) fn peer(stream: &UnixStream) -> Result<(PeerProcessIdentity, u32)> {
    let credentials: libc::ucred = socket_option(stream, libc::SOL_SOCKET, libc::SO_PEERCRED)?;
    // SO_PEERPIDFD is deliberately mandatory: never substitute pidfd_open on a reported PID.
    let fd: i32 = socket_option(stream, libc::SOL_SOCKET, 77)?;
    ensure!(fd >= 0, "missing peer pidfd");
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let identity = PeerProcessIdentity::capture(u32::try_from(credentials.pid)?, fd)?;
    ensure!(process(identity.pid())?.uid == credentials.uid, "peer UID changed");
    Ok((identity, credentials.uid))
}

#[cfg(target_os = "macos")]
pub(super) fn peer(stream: &UnixStream) -> Result<(PeerProcessIdentity, u32)> {
    let token: [u32; 8] = socket_option(stream, libc::SOL_LOCAL, libc::LOCAL_PEERTOKEN)?;
    // audit_token_t pins pidversion, unlike LOCAL_PEERPID alone, which is reusable after exit.
    #[repr(C)]
    struct UniqueInfo { uuid: [u8; 16], unique: u64, parent: u64, version: i32, reserved: i32, reserved3: u64, reserved4: u64 }
    let identity = PeerProcessIdentity::capture_pid(token[5])?;
    let mut unique = std::mem::MaybeUninit::<UniqueInfo>::zeroed();
    let size = std::mem::size_of::<UniqueInfo>() as i32;
    let count = unsafe { libc::proc_pidinfo(i32::try_from(token[5])?, 17, 0, unique.as_mut_ptr().cast(), size) };
    ensure!(count == size, "Darwin peer pidversion unavailable");
    let unique = unsafe { unique.assume_init() };
    ensure!(unique.version as u32 == token[7], "Darwin peer PID reused");
    ensure!(process(identity.pid())?.uid == token[1], "Darwin peer UID changed");
    identity.validate()?;
    Ok((identity, token[1]))
}

#[cfg(target_os = "linux")]
pub(super) fn process(pid: u32) -> Result<ProcessEvidence> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let uid = status.lines().find_map(|line| line.strip_prefix("Uid:")).and_then(|line| line.split_whitespace().nth(1)).context("missing process effective UID")?.parse()?;
    let (_, start_token) = crate::process::process_identity(pid)?;
    Ok(ProcessEvidence { pid, uid, start_token, boot_id: fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim().to_owned() })
}
#[cfg(target_os = "macos")]
pub(super) fn process(pid: u32) -> Result<ProcessEvidence> {
    let (_, start_token, uid) = crate::macos_process_identity(pid)?;
    let mut boot = std::mem::MaybeUninit::<libc::timeval>::zeroed();
    let mut size = std::mem::size_of::<libc::timeval>();
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let result = unsafe { libc::sysctl(mib.as_mut_ptr(), 2, boot.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
    ensure!(result == 0 && size == std::mem::size_of::<libc::timeval>(), "Darwin boot identity unavailable");
    let boot = unsafe { boot.assume_init() };
    Ok(ProcessEvidence { pid, uid, start_token, boot_id: format!("{}:{}", boot.tv_sec, boot.tv_usec) })
}
#[cfg(target_os = "linux")]
pub(super) fn cwd(pid: u32) -> Option<PathBuf> { fs::read_link(format!("/proc/{pid}/cwd")).ok() }
#[cfg(target_os = "macos")]
pub(super) fn cwd(pid: u32) -> Option<PathBuf> {
    let mut value = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let count = unsafe { libc::proc_pidinfo(i32::try_from(pid).ok()?, libc::PROC_PIDVNODEPATHINFO, 0, value.as_mut_ptr().cast(), size) };
    if count != size { return None; }
    let value = unsafe { value.assume_init() };
    let bytes = value.pvi_cdir.vip_path.as_flattened();
    let length = bytes.iter().position(|byte| *byte == 0)?;
    use std::os::unix::ffi::OsStringExt as _;
    Some(std::ffi::OsString::from_vec(bytes[..length].iter().map(|byte| *byte as u8).collect()).into())
}

#[cfg(target_os = "linux")]
pub(super) fn has_descriptor(pid: u32, transcript: &TranscriptEvidence) -> Result<()> {
    for (index, entry) in fs::read_dir(format!("/proc/{pid}/fd"))?.enumerate() {
        ensure!(index < 65_536, "process descriptor limit exceeded");
        let entry = entry?;
        // Following the kernel fd link observes the opened object, not the reported path.
        if let Ok(metadata) = fs::metadata(entry.path()) {
            if metadata.is_file() && metadata.dev() == transcript.device && metadata.ino() == transcript.inode && metadata.uid() == transcript.uid {
                return Ok(());
            }
        }
    }
    anyhow::bail!("process does not hold the selected transcript open")
}

#[cfg(target_os = "macos")]
pub(super) fn has_descriptor(pid: u32, transcript: &TranscriptEvidence) -> Result<()> {
    #[repr(C)]
    struct FileInfo { flags: u32, status: u32, offset: i64, kind: i32, guard: u32 }
    #[repr(C)]
    struct VnodeFdInfo { file: FileInfo, vnode: libc::vnode_info_path }
    let pid = i32::try_from(pid)?;
    let needed = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let entry_size = std::mem::size_of::<libc::proc_fdinfo>();
    ensure!(needed > 0 && needed as usize / entry_size < 65_536, "Darwin descriptor listing unavailable or exceeds limit");
    // One spare entry detects growth; a concurrently expanding list fails closed.
    let entries = needed as usize / entry_size + 1;
    let mut descriptors: Vec<libc::proc_fdinfo> = (0..entries).map(|_| libc::proc_fdinfo { proc_fd: 0, proc_fdtype: 0 }).collect();
    let size = (descriptors.len() * entry_size) as i32;
    let count = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, descriptors.as_mut_ptr().cast(), size) };
    ensure!(count > 0 && count < size && count as usize % std::mem::size_of::<libc::proc_fdinfo>() == 0, "Darwin descriptor listing unavailable or exceeds limit");
    for fd in &descriptors[..count as usize / std::mem::size_of::<libc::proc_fdinfo>()] {
        if fd.proc_fdtype != libc::PROX_FDTYPE_VNODE as u32 { continue; }
        let mut info = std::mem::MaybeUninit::<VnodeFdInfo>::zeroed();
        let size = std::mem::size_of::<VnodeFdInfo>() as i32;
        // PROC_PIDFDVNODEPATHINFO = 2, from Darwin sys/proc_info.h.
        let count = unsafe { libc::proc_pidfdinfo(pid, fd.proc_fd, 2, info.as_mut_ptr().cast(), size) };
        if count != size { continue; }
        let info = unsafe { info.assume_init() };
        let stat = info.vnode.vip_vi.vi_stat;
        if stat.vst_dev as u64 == transcript.device && stat.vst_ino == transcript.inode && stat.vst_uid == transcript.uid
            && stat.vst_mode & libc::S_IFMT == libc::S_IFREG { return Ok(()); }
    }
    anyhow::bail!("process does not hold the selected transcript open")
}

pub(super) fn connect(path: &Path, duration: Duration) -> Result<UnixStream> {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    ensure!(bytes.len() < address.sun_path.len(), "challenge socket path too long");
    address.sun_family = libc::AF_UNIX as _;
    for (output, input) in address.sun_path.iter_mut().zip(bytes) { *output = *input as _; }
    #[cfg(target_os = "macos")]
    { address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8; }
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    ensure!(raw >= 0, "cannot create challenge socket");
    let stream = unsafe { UnixStream::from_raw_fd(raw) };
    ensure!(unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } == 0, "cannot protect challenge descriptor");
    stream.set_nonblocking(true)?;
    let result = unsafe { libc::connect(raw, (&address as *const libc::sockaddr_un).cast(), std::mem::size_of::<libc::sockaddr_un>() as _) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        ensure!(matches!(error.raw_os_error(), Some(libc::EINPROGRESS) | Some(libc::EAGAIN)), "challenge connect failed: {error}");
        let mut poll = libc::pollfd { fd: raw, events: libc::POLLOUT, revents: 0 };
        ensure!(unsafe { libc::poll(&mut poll, 1, duration.as_millis().min(i32::MAX as u128) as i32) } > 0, "challenge connection timed out");
        let error: i32 = socket_option(&stream, libc::SOL_SOCKET, libc::SO_ERROR)?;
        ensure!(error == 0, "challenge connect failed: {}", std::io::Error::from_raw_os_error(error));
    }
    stream.set_nonblocking(false)?;
    Ok(stream)
}
