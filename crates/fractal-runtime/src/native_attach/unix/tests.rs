use super::*;
use std::{io::{BufRead as _, BufReader}, process::{Child, ChildStdin, ChildStdout, Command, Stdio}};

const ID: &str = "native-full-identifier";

struct Peer { child: Child, input: ChildStdin, output: BufReader<ChildStdout> }
impl Peer {
    fn start(path: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_attach::unix::tests::proof_child", "--ignored", "--nocapture"])
            .env("OMP_SESSION_ID", "forged-env-id")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut peer = Self { child, input, output };
        peer.line(&path);
        peer.until("READY");
        peer
    }
    fn line(&mut self, value: &impl Serialize) {
        serde_json::to_writer(&mut self.input, value).unwrap();
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
    }
    fn until(&mut self, marker: &str) {
        loop {
            let mut line = String::new();
            assert!(self.output.read_line(&mut line).unwrap() > 0, "child exited before {marker}");
            if line.contains(marker) { return; }
        }
    }
    fn stop(&mut self) { let _ = self.child.kill(); self.child.wait().unwrap(); }
}
impl Drop for Peer { fn drop(&mut self) { self.stop(); } }

#[test]
#[ignore = "subprocess entry point, invoked by boundary tests"]
fn proof_child() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let path: PathBuf = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let _held = File::open(path).unwrap();
    println!("READY");
    std::io::stdout().flush().unwrap();
    let Some(Ok(line)) = lines.next() else { return; };
    let (challenge, request): (ProofChallenge, serde_json::Value) = serde_json::from_str(&line).unwrap();
    let mut stream = os::connect(&challenge.socket_path, IO_TIMEOUT).unwrap();
    let deadline = Instant::now() + IO_TIMEOUT;
    send(&mut stream, &request, deadline).unwrap();
    let _ = receive::<bool>(&mut stream, deadline);
    println!("REPORTED");
    std::io::stdout().flush().unwrap();
    let _ = lines.next();
}

fn fixture() -> (tempfile::TempDir, PathBuf, File) {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap().join("session.jsonl");
    fs::write(&path, format!("{{\"type\":\"session\",\"id\":\"{ID}\",\"cwd\":\"/workspace\"}}\n")).unwrap();
    let file = File::open(&path).unwrap();
    (dir, path, file)
}
fn request(challenge: &ProofChallenge, evidence: &AttachEvidence) -> serde_json::Value {
    serde_json::to_value(NonceReport { nonce: challenge.nonce.clone(), evidence: evidence.clone(), expiry: challenge.expiry }).unwrap()
}

#[test]
fn selected_descriptor_not_environment_prefix_or_copy() {
    let (_dir, path, _file) = fixture();
    let peer = Peer::start(&path);
    let evidence = inspect(peer.child.id(), &path, ID).unwrap();
    assert_eq!(evidence.transcript.native_id, ID);
    assert!(inspect(peer.child.id(), &path, "native-full").is_err());
    assert!(inspect(peer.child.id(), &path, "forged-env-id").is_err());
    let copy = path.with_file_name("copy.jsonl");
    fs::copy(&path, &copy).unwrap();
    assert!(inspect(peer.child.id(), &copy, ID).is_err());
}

#[test]
fn authentic_peer_proof_rechecks_symlink_copy_and_process_exit() {
    let (dir, path, _file) = fixture();
    let mut peer = Peer::start(&path);
    let evidence = inspect(peer.child.id(), &path, ID).unwrap();
    let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    let challenge = listener.challenge().clone();
    peer.line(&(challenge.clone(), request(&challenge, &evidence)));
    let witness = listener.accept().unwrap();
    assert_eq!(witness.challenge_digest(), listener.challenge_digest());
    witness.revalidate().unwrap();
    assert!(listener.accept().is_err(), "nonce must be one-shot");
    let moved = path.with_file_name("moved.jsonl");
    fs::rename(&path, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &path).unwrap();
    assert!(witness.revalidate().is_err());
    fs::remove_file(&path).unwrap();
    fs::copy(&moved, &path).unwrap();
    assert!(witness.revalidate().is_err());
    fs::remove_file(&path).unwrap();
    fs::rename(&moved, &path).unwrap();
    witness.revalidate().unwrap();
    peer.stop();
    assert!(witness.revalidate().is_err());
    let socket = challenge.socket_path;
    drop(listener);
    assert!(!socket.exists());
    assert!(!socket.parent().unwrap().exists());
}

#[test]
fn forged_tuple_nonce_uid_and_generation_are_rejected() {
    for forgery in ["pid", "uid", "path", "prefix", "nonce", "start", "boot", "cwd", "expired", "extended"] {
        let (dir, path, _file) = fixture();
        let mut peer = Peer::start(&path);
        let evidence = inspect(peer.child.id(), &path, ID).unwrap();
        let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
        let challenge = listener.challenge().clone();
        let mut request = request(&challenge, &evidence);
        match forgery {
            "pid" => request["evidence"]["process"]["pid"] = 1.into(),
            "uid" => request["evidence"]["process"]["uid"] = (evidence.process.uid.wrapping_add(1)).into(),
            "path" => request["evidence"]["transcript"]["path"] = "/forged.jsonl".into(),
            "prefix" => request["evidence"]["transcript"]["native_id"] = "native-full".into(),
            "nonce" => request["nonce"] = "replayed-other-challenge".into(),
            "start" => request["evidence"]["process"]["start_token"] = 0.into(),
            "boot" => request["evidence"]["process"]["boot_id"] = "different-boot".into(),
            "cwd" => request["evidence"]["observed_cwd"] = "/different-workspace".into(),
            "expired" => request["expiry"] = 0.into(),
            "extended" => request["expiry"] = challenge.expiry.saturating_add(1).into(),
            _ => unreachable!(),
        }
        peer.line(&(challenge, request));
        assert!(listener.accept().is_err(), "accepted {forgery}");
        assert!(listener.accept().is_err(), "failed attempt must consume nonce");
    }
}

#[test]
fn expired_challenge_and_dead_candidate_fail_closed() {
    let (dir, path, _file) = fixture();
    let mut peer = Peer::start(&path);
    let evidence = inspect(peer.child.id(), &path, ID).unwrap();
    let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    listener.deadline = Instant::now();
    assert!(listener.accept().is_err());
    let mut wall_expired = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    wall_expired.challenge.expiry = 0;
    assert!(wall_expired.accept().is_err());
    assert!(report(wall_expired.challenge(), &path, ID).is_err());
    let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    peer.stop();
    assert!(listener.accept().is_err());
    assert!(ProofListener::bind(dir.path(), &evidence, Duration::from_secs(1)).is_err());
}

#[test]
fn simultaneous_unrelated_peer_cannot_claim_or_spend_challenge() {
    let (dir, path, _file) = fixture();
    let mut expected = Peer::start(&path);
    let mut attacker = Peer::start(&path);
    let evidence = inspect(expected.child.id(), &path, ID).unwrap();
    let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    let challenge = listener.challenge().clone();
    attacker.line(&(challenge.clone(), request(&challenge, &evidence)));
    expected.line(&(challenge.clone(), request(&challenge, &evidence)));
    let witness = listener.accept().unwrap();
    assert_eq!(witness.evidence().process.pid, expected.child.id());
    witness.revalidate().unwrap();
}

#[test]
fn client_pins_daemon_before_disclosing_challenge() {
    let (dir, path, _file) = fixture();
    let evidence = inspect(std::process::id(), &path, ID).unwrap();
    let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    let mut bad = listener.challenge().clone();
    bad.daemon.start_token = bad.daemon.start_token.wrapping_add(1);
    assert!(report(&bad, &path, ID).is_err());
    // The identity-checking client closes without a request; this attempt cannot grant a witness.
    assert!(listener.accept().is_err());
    let mut listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    let challenge = listener.challenge().clone();
    std::thread::scope(|scope| {
        let client = scope.spawn(|| report(&challenge, &path, ID));
        listener.accept().unwrap().revalidate().unwrap();
        assert_eq!(client.join().unwrap().unwrap().transcript, evidence.transcript);
    });
}

#[test]
fn public_report_schema_and_listener_inputs_fail_closed() {
    let (dir, path, _file) = fixture();
    let evidence = inspect(std::process::id(), &path, ID).unwrap();
    assert!(ProofListener::bind(dir.path(), &evidence, Duration::ZERO).is_err());
    assert!(ProofListener::bind(dir.path(), &evidence, Duration::from_secs(61)).is_err());
    let mut stale = evidence.clone();
    stale.process.start_token = stale.process.start_token.wrapping_add(1);
    assert!(ProofListener::bind(dir.path(), &stale, Duration::from_secs(10)).is_err());
    stale = evidence.clone();
    stale.process.uid = stale.process.uid.wrapping_add(1);
    assert!(ProofListener::bind(dir.path(), &stale, Duration::from_secs(10)).is_err());
    stale = evidence.clone();
    stale.process.boot_id.push_str("-stale");
    assert!(ProofListener::bind(dir.path(), &stale, Duration::from_secs(10)).is_err());
    let listener = ProofListener::bind(dir.path(), &evidence, Duration::from_secs(10)).unwrap();
    assert!(listener.challenge().expiry > now_ms().unwrap());
    let wire = request(listener.challenge(), &evidence);
    let decoded: NonceReport = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded.evidence, evidence);
    assert_eq!(decoded.expiry, listener.challenge().expiry);
    let mut missing = wire.clone();
    missing.as_object_mut().unwrap().remove("expiry");
    assert!(serde_json::from_value::<NonceReport>(missing).is_err());
    let mut extra = wire;
    extra["reported_pid"] = 1.into();
    assert!(serde_json::from_value::<NonceReport>(extra).is_err());
}
