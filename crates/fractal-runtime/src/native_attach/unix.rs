use super::*;
use crate::PeerProcessIdentity;
use anyhow::{Context as _, Result, ensure};
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    os::{fd::{AsRawFd as _, FromRawFd as _, OwnedFd}, unix::{fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _}, net::{UnixListener, UnixStream}}},
    path::Path,
    time::{Duration, Instant},
};

mod os;
const MAX_FRAME: usize = 32 * 1024;
const MAX_HEADER: u64 = 64 * 1024;
const MAX_TTL: Duration = Duration::from_secs(60);
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Retains both the incarnation witness and the opened transcript, not just serialized claims.
pub struct AttachWitness {
    evidence: AttachEvidence,
    process: PeerProcessIdentity,
    file: File,
    challenge_digest: String,
}
impl AttachWitness {
    pub fn evidence(&self) -> &AttachEvidence { &self.evidence }
    pub fn challenge_digest(&self) -> &str { &self.challenge_digest }
    pub fn revalidate(&self) -> Result<()> {
        self.process.validate_generation(self.evidence.process.pid, self.evidence.process.start_token)?;
        ensure!(os::process(self.process.pid())? == self.evidence.process, "process identity changed");
        let metadata = self.file.metadata()?;
        ensure!(metadata.is_file() && metadata.dev() == self.evidence.transcript.device
            && metadata.ino() == self.evidence.transcript.inode && metadata.uid() == self.evidence.transcript.uid,
            "held transcript identity changed");
        let current = transcript(&self.evidence.transcript.path, &self.evidence.transcript.native_id, self.evidence.process.uid)?;
        ensure!(current.1 == self.evidence.transcript, "transcript path or header changed");
        os::has_descriptor(self.process.pid(), &self.evidence.transcript)?;
        self.process.validate()
    }
}

/// Inspect a candidate without granting attach authority. Only accept authenticates its connector.
pub fn inspect(pid: u32, path: &Path, native_id: &str) -> Result<AttachEvidence> {
    let process = PeerProcessIdentity::capture_pid(pid)?;
    let evidence = os::process(pid)?;
    process.validate_generation(pid, evidence.start_token)?;
    let (_, transcript) = transcript(path, native_id, evidence.uid)?;
    os::has_descriptor(pid, &transcript)?;
    let observed_cwd = os::cwd(pid);
    process.validate()?;
    Ok(AttachEvidence { process: evidence, transcript, observed_cwd })
}

fn transcript(path: &Path, native_id: &str, uid: u32) -> Result<(File, TranscriptEvidence)> {
    ensure!(path.is_absolute() && fs::canonicalize(path)? == path, "transcript must be an absolute canonical non-symlink path");
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK).open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file() && metadata.uid() == uid, "transcript must be a regular file owned by the process UID");
    // positional reads leave the held descriptor offset unchanged, and stop at the first record.
    use std::os::unix::fs::FileExt as _;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = file.read_at(&mut buffer, bytes.len() as u64)?;
        ensure!(count > 0, "transcript header is incomplete");
        if let Some(end) = buffer[..count].iter().position(|byte| *byte == b'\n') {
            bytes.extend_from_slice(&buffer[..end]);
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        ensure!((bytes.len() as u64) < MAX_HEADER, "transcript header exceeds limit");
    }
    ensure!((bytes.len() as u64) <= MAX_HEADER, "transcript header exceeds limit");
    let header: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(header["type"] == "session" && header["id"].as_str() == Some(native_id) && !native_id.is_empty(), "full transcript native ID does not match");
    let header_cwd = match header.get("cwd") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(cwd)) => Some(PathBuf::from(cwd)),
        _ => anyhow::bail!("invalid transcript cwd"),
    };
    ensure!(fs::canonicalize(path)? == path, "transcript path changed during inspection");
    let now = fs::symlink_metadata(path)?;
    ensure!(now.is_file() && now.dev() == metadata.dev() && now.ino() == metadata.ino(), "transcript path was replaced");
    Ok((file, TranscriptEvidence { path: path.to_owned(), device: metadata.dev(), inode: metadata.ino(), uid, native_id: native_id.to_owned(), header_cwd }))
}

/// A private, one-shot listener. Dropping it removes its socket and private directory.
pub struct ProofListener {
    listener: UnixListener,
    directory: PathBuf,
    challenge: ProofChallenge,
    expected: AttachEvidence,
    candidate: PeerProcessIdentity,
    _prepared_file: File,
    daemon: PeerProcessIdentity,
    deadline: Instant,
    consumed: bool,
}
impl ProofListener {
    pub fn bind(parent: &Path, expected: &AttachEvidence, ttl: Duration) -> Result<Self> {
        ensure!(!ttl.is_zero() && ttl <= MAX_TTL, "challenge lifetime must be in (0, 60s]");
        let parent = fs::canonicalize(parent)?;
        let candidate = PeerProcessIdentity::capture_pid(expected.process.pid)?;
        candidate.validate_generation(expected.process.pid, expected.process.start_token)?;
        ensure!(os::process(expected.process.pid)? == expected.process, "candidate process changed");
        let (prepared_file, prepared_transcript) = transcript(&expected.transcript.path, &expected.transcript.native_id, expected.process.uid)?;
        ensure!(prepared_transcript == expected.transcript, "prepared transcript changed");
        os::has_descriptor(expected.process.pid, &expected.transcript)?;
        candidate.validate()?;
        let daemon = PeerProcessIdentity::capture_pid(std::process::id())?;
        let daemon_evidence = os::process(std::process::id())?;
        ensure!(expected.process.uid == daemon_evidence.uid, "candidate UID differs from daemon owner");
        let random = random_hex()?;
        let nonce = random_hex()?;
        let deadline = Instant::now() + ttl;
        let expiry = now_ms()?.checked_add(u64::try_from(ttl.as_millis())?).context("challenge expiry overflow")?;
        // Keep the endpoint below Darwin's short sockaddr_un limit even under its
        // per-user temporary root. The separate nonce retains all 256 random bits.
        let directory = parent.join(format!("a-{}", &random[..16]));
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let socket_path = directory.join("p");
        let setup = (|| -> Result<UnixListener> {
            let listener = UnixListener::bind(&socket_path)?;
            fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            Ok(listener)
        })();
        let listener = match setup { Ok(listener) => listener, Err(error) => { let _ = fs::remove_file(&socket_path); let _ = fs::remove_dir(&directory); return Err(error); } };
        Ok(Self { listener, directory, challenge: ProofChallenge { socket_path, nonce, daemon: daemon_evidence, expiry }, expected: expected.clone(), candidate, _prepared_file: prepared_file, daemon, deadline, consumed: false })
    }
    pub fn challenge(&self) -> &ProofChallenge { &self.challenge }
    pub fn challenge_digest(&self) -> String { hex(&Sha256::digest(self.challenge.nonce.as_bytes())) }
    /// One attempt per challenge, including failures. Unrelated peers cannot spend a challenge.
    pub fn accept(&mut self) -> Result<AttachWitness> {
        ensure!(!self.consumed, "challenge already consumed");
        loop {
            ensure!(Instant::now() < self.deadline && now_ms()? < self.challenge.expiry, "challenge expired");
            self.candidate.validate()?;
            self.daemon.validate()?;
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            let mut poll = libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // poll receives one initialized descriptor and does not retain the pointer.
            let count = unsafe { libc::poll(&mut poll, 1, remaining.as_millis().min(i32::MAX as u128) as i32) };
            ensure!(count > 0, "challenge expired or listener poll failed");
            let (mut stream, _) = match self.listener.accept() { Ok(value) => value, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue, Err(error) => return Err(error.into()) };
            let (peer, uid) = os::peer(&stream)?;
            if peer.pid() != self.expected.process.pid || uid != self.expected.process.uid { continue; }
            self.consumed = true;
            peer.validate_generation(self.expected.process.pid, self.expected.process.start_token)?;
            let deadline = self.deadline.min(Instant::now() + IO_TIMEOUT);
            let request: NonceReport = receive(&mut stream, deadline)?;
            ensure!(Instant::now() < self.deadline && now_ms()? < self.challenge.expiry, "challenge expired");
            ensure!(request.expiry == self.challenge.expiry, "report expiry does not match challenge");
            ensure!(request.nonce == self.challenge.nonce, "invalid challenge nonce");
            ensure!(request.evidence == self.expected, "reported attach evidence does not match challenge");
            let (file, current) = transcript(&request.evidence.transcript.path, &request.evidence.transcript.native_id, uid)?;
            ensure!(current == self.expected.transcript, "transcript changed since preparation");
            let witness = AttachWitness { evidence: AttachEvidence { observed_cwd: os::cwd(peer.pid()), ..request.evidence }, process: peer, file, challenge_digest: self.challenge_digest() };
            witness.revalidate()?;
            ensure!(Instant::now() < self.deadline && now_ms()? < self.challenge.expiry, "challenge expired");
            send(&mut stream, &true, deadline)?;
            return Ok(witness);
        }
    }
}
impl Drop for ProofListener {
    fn drop(&mut self) { let _ = fs::remove_file(&self.challenge.socket_path); let _ = fs::remove_dir(&self.directory); }
}

/// Call from the selected native process while it still holds the selected transcript open.
/// The daemon's kernel PID, UID, boot and incarnation are checked before disclosing the nonce.
pub fn report(challenge: &ProofChallenge, path: &Path, native_id: &str) -> Result<AttachEvidence> {
    ensure!(now_ms()? < challenge.expiry, "challenge expired");
    let metadata = fs::symlink_metadata(&challenge.socket_path)?;
    use std::os::unix::fs::FileTypeExt as _;
    ensure!(metadata.file_type().is_socket() && metadata.uid() == challenge.daemon.uid && metadata.mode() & 0o077 == 0, "unsafe challenge socket");
    let parent = challenge.socket_path.parent().context("socket has no parent")?;
    let directory = fs::symlink_metadata(parent)?;
    ensure!(directory.is_dir() && directory.uid() == challenge.daemon.uid && directory.mode() & 0o077 == 0, "unsafe challenge directory");
    let mut stream = os::connect(&challenge.socket_path, IO_TIMEOUT)?;
    let (daemon, uid) = os::peer(&stream)?;
    ensure!(uid == challenge.daemon.uid && os::process(daemon.pid())? == challenge.daemon, "daemon identity mismatch");
    daemon.validate_generation(challenge.daemon.pid, challenge.daemon.start_token)?;
    let evidence = inspect(std::process::id(), path, native_id)?;
    let deadline = Instant::now() + IO_TIMEOUT;
    ensure!(now_ms()? < challenge.expiry, "challenge expired");
    send(&mut stream, &NonceReport { nonce: challenge.nonce.clone(), evidence: evidence.clone(), expiry: challenge.expiry }, deadline)?;
    ensure!(receive::<bool>(&mut stream, deadline)?, "attach proof rejected");
    daemon.validate()?;
    ensure!(now_ms()? < challenge.expiry, "challenge expired");
    Ok(evidence)
}

fn now_ms() -> Result<u64> {
    Ok(u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis())?)
}

fn random_hex() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex(&bytes))
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes { let _ = write!(result, "{byte:02x}"); }
    result
}
fn timeout(deadline: Instant) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), "proof I/O deadline exceeded");
    Ok(remaining)
}
fn read_exact(stream: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_read_timeout(Some(timeout(deadline)?))?;
        let count = stream.read(bytes)?;
        ensure!(count > 0, "proof peer disconnected");
        bytes = &mut bytes[count..];
    }
    Ok(())
}
fn receive<T: DeserializeOwned>(stream: &mut UnixStream, deadline: Instant) -> Result<T> {
    let mut length = [0_u8; 4];
    read_exact(stream, &mut length, deadline)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(length <= MAX_FRAME, "proof frame exceeds limit");
    let mut bytes = vec![0; length];
    read_exact(stream, &mut bytes, deadline)?;
    Ok(serde_json::from_slice(&bytes)?)
}
fn send<T: Serialize>(stream: &mut UnixStream, value: &T, deadline: Instant) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_FRAME, "proof frame exceeds limit");
    for mut part in [&(bytes.len() as u32).to_be_bytes()[..], &bytes[..]] {
        while !part.is_empty() {
            stream.set_write_timeout(Some(timeout(deadline)?))?;
            let count = stream.write(part)?;
            ensure!(count > 0, "proof peer disconnected");
            part = &part[count..];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
