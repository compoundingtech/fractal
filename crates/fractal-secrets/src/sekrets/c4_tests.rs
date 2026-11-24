use super::*;

fn issuer() -> MemberKey {
    // Deliberately public deterministic disposable fixture, never a production key.
    MemberKey::from_pkcs8(&hex::decode(concat!(
        "3053020101300506032b657004220420",
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        "a12303210003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"
    )).unwrap()).unwrap()
}
fn identity(name: &str) -> Identity {
    Identity { machine: "fixture-machine".into(), audience: "fixture-service".into(), leaf: format!("session/fixture/{name}"), incarnation: format!("fixture-incarnation-{name}"), work_attempt: String::new() }
}
fn process(pid: i32) -> Process {
    Process { pid, uid: 100_000 + u32::try_from(pid).unwrap(), start: 123, cgroup: format!("/fixture/{pid}"), parent: 1 }
}
fn setup() -> Broker { Broker::initialize(Connection::open_in_memory().unwrap(), issuer()).unwrap() }
fn issue(broker: &mut Broker, name: &str, parent: &str) -> Signed {
    let pid = 42 + i32::try_from(broker.sessions.len()).unwrap();
    broker.issue(identity(name), process(pid), parent.into(), 1_000, 61_000).unwrap()
}
fn proof(broker: &Broker, cert: &Signed, nonce: u8) -> Signed {
    let c = certificate(cert, broker.issuer.public()).unwrap();
    signed(&Proof(PROOF.into(), c.1.clone(), c.4, "a".repeat(64), URL_SAFE_NO_PAD.encode([nonce; 32]), 2_000), &broker.sessions[&c.1].key).unwrap()
}
fn admit(broker: &mut Broker, cert: &Signed, proof: &Signed, name: &str, parent: &str, at: u64) -> Result<()> {
    broker.admit_at(cert, proof, &identity(name), parent, &"a".repeat(64), at)
}

#[test]
fn deterministic_cross_repository_vector_has_real_signatures() {
    let vector: serde_json::Value = serde_json::from_str(include_str!("c4-vector.json")).unwrap();
    let cert: Signed = serde_json::from_value(vector["certificate"].clone()).unwrap();
    let p: Signed = serde_json::from_value(vector["proof"].clone()).unwrap();
    let c = certificate(&cert, issuer().public()).unwrap();
    assert_eq!(signed(&c, &issuer()).unwrap(), cert);
    assert!(verify_signature(&c.9, p.statement.as_bytes(), &p.signature));
    let mut broker = setup();
    broker.db.execute("INSERT INTO certificates VALUES (?1,?2,?3,?4,0,?5,100042,'/fixture/42')", params![c.1, c.8, cert.statement, c.11, c.6]).unwrap();
    broker.admit_at(&cert, &p, &identity("root"), "", &"a".repeat(64), 1_800_000_001_000).unwrap();
    assert!(broker.admit_at(&cert, &p, &identity("root"), "", &"a".repeat(64), 1_800_000_001_000).is_err());
}

#[test]
fn real_crypto_rejects_wrong_identity_digest_signer_expiry_and_replay() {
    let mut broker = setup();
    let cert = issue(&mut broker, "root", "");
    let p = proof(&broker, &cert, 1);
    for expected in [
        Identity { machine: "other".into(), ..identity("root") },
        Identity { audience: "other".into(), ..identity("root") },
        Identity { leaf: "other".into(), ..identity("root") },
        Identity { incarnation: "other".into(), ..identity("root") },
        Identity { work_attempt: "other".into(), ..identity("root") },
    ] { assert!(broker.admit_at(&cert, &p, &expected, "", &"a".repeat(64), 2_000).is_err()); }
    assert!(admit(&mut broker, &cert, &p, "root", "fake-parent", 2_000).is_err());
    assert!(broker.admit_at(&cert, &p, &identity("root"), "", &"b".repeat(64), 2_000).is_err());
    let wrong = Signed { signature: MemberKey::generate().unwrap().0.sign(p.statement.as_bytes()), ..p.clone() };
    assert!(admit(&mut broker, &cert, &wrong, "root", "", 2_000).is_err());
    let wrong_cert = Signed { signature: MemberKey::generate().unwrap().0.sign(cert.statement.as_bytes()), ..cert.clone() };
    assert!(admit(&mut broker, &wrong_cert, &p, "root", "", 2_000).is_err());
    let noncanonical = Signed { statement: format!(" {}", p.statement), ..p.clone() };
    assert!(admit(&mut broker, &cert, &noncanonical, "root", "", 2_000).is_err());
    assert!(admit(&mut broker, &cert, &p, "root", "", 61_000).is_err());
    assert!(admit(&mut broker, &cert, &p, "root", "", 7_001).is_err());
    admit(&mut broker, &cert, &p, "root", "", 2_000).unwrap();
    assert!(admit(&mut broker, &cert, &p, "root", "", 2_000).is_err());
}

#[test]
fn persistent_parent_revocation_cascades_and_restart_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("issuer.sqlite");
    let mut broker = Broker::initialize(Connection::open(&path).unwrap(), issuer()).unwrap();
    let parent = issue(&mut broker, "root", "");
    let parent_id = certificate(&parent, broker.issuer.public()).unwrap().1;
    let child = issue(&mut broker, "child", &parent_id);
    let child_id = certificate(&child, broker.issuer.public()).unwrap().1;
    let grandchild = issue(&mut broker, "grandchild", &child_id);
    let p = proof(&broker, &child, 2);
    admit(&mut broker, &child, &p, "child", &parent_id, 2_000).unwrap();
    let p2 = proof(&broker, &child, 3);
    let p3 = proof(&broker, &grandchild, 4);
    broker.revoke(&parent_id).unwrap();
    assert!(admit(&mut broker, &child, &p2, "child", &parent_id, 2_000).is_err());
    assert!(admit(&mut broker, &grandchild, &p3, "grandchild", &child_id, 2_000).is_err());
    assert!(issue_again(&mut broker, "root").is_err());
    drop(broker);
    let mut restarted = Broker::initialize(Connection::open(&path).unwrap(), issuer()).unwrap();
    assert!(restarted.sessions.is_empty());
    assert!(admit(&mut restarted, &child, &p2, "child", &parent_id, 2_000).is_err());
    assert_eq!(restarted.db.query_row("SELECT COUNT(*) FROM nonces", [], |r| r.get::<_, u64>(0)).unwrap(), 1);
    assert!(issue_again(&mut restarted, "root").is_err());
    let other_issuer = MemberKey::generate().unwrap().0;
    drop(restarted);
    assert!(Broker::initialize(Connection::open(&path).unwrap(), other_issuer).is_err());
    let mut missing = setup();
    assert!(admit(&mut missing, &child, &p2, "child", &parent_id, 2_000).is_err());
}
fn issue_again(broker: &mut Broker, name: &str) -> Result<Signed> {
    broker.issue(identity(name), process(43), String::new(), 1_000, 61_000)
}

#[test]
fn restart_revokes_previously_live_certificates_and_missing_state_denies() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("issuer.sqlite");
    let mut broker = Broker::initialize(Connection::open(&path).unwrap(), issuer()).unwrap();
    let cert = issue(&mut broker, "root", "");
    let p = proof(&broker, &cert, 1);
    drop(broker);
    let mut restarted = Broker::initialize(Connection::open(path).unwrap(), issuer()).unwrap();
    assert!(admit(&mut restarted, &cert, &p, "root", "", 2_000).is_err());
    restarted.db.execute_batch("DROP TABLE certificates").unwrap();
    assert!(admit(&mut restarted, &cert, &p, "root", "", 2_000).is_err());
    let mut missing = setup();
    let fresh = issue(&mut missing, "root", "");
    let fresh_proof = proof(&missing, &fresh, 9);
    missing.state_file = Some((root.path().join("missing.sqlite"), 0, 0));
    assert!(admit(&mut missing, &fresh, &fresh_proof, "root", "", 2_000).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn socket_peer_is_not_an_arbitrarily_registered_process_or_incarnation() {
    let mut broker = setup();
    let cert = issue(&mut broker, "root", "");
    let id = certificate(&cert, broker.issuer.public()).unwrap().1;
    let (peer, _client) = UnixStream::pair().unwrap();
    assert!(broker.check_peer(&peer, &id).is_err());
    let observed = observe(i32::try_from(std::process::id()).unwrap()).unwrap();
    let session = broker.sessions.get_mut(&id).unwrap();
    session.process = observed.clone();
    broker.check_peer(&peer, &id).unwrap();
    broker.sessions.get_mut(&id).unwrap().process.start += 1;
    assert!(broker.check_peer(&peer, &id).is_err());
    broker.sessions.get_mut(&id).unwrap().process = Process { cgroup: "/wrong".into(), ..observed };
    assert!(broker.check_peer(&peer, &id).is_err());
}

/// Run only in a disposable Linux VM as root with writable cgroup v2.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires disposable Linux root and writable cgroup v2"]
fn linux_protected_launch_and_verified_spawn() {
    use std::{io::Read as _, os::unix::fs::PermissionsExt as _, os::unix::net::UnixListener, process::Command};
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let runtime = tempfile::Builder::new().prefix("c4-fixture-").tempdir_in("/run").unwrap();
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let state = runtime.path().join("state");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = runtime.path().join("peer.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o777)).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "sekrets::c4::tests::linux_fixture_process"])
        .env("C4_FIXTURE_SOCKET", &socket).spawn().unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    let mut bytes = [0; 4];
    peer.read_exact(&mut bytes).unwrap();
    let child_pid = i32::from_le_bytes(bytes);
    struct Cleanup { parent: std::process::Child, child: Option<i32>, groups: Vec<PathBuf> }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(pid) = self.child { unsafe { libc::kill(pid, libc::SIGKILL); } }
            let _ = self.parent.kill();
            let _ = self.parent.wait();
            for path in &self.groups { let _ = fs::remove_dir(path); }
        }
    }
    let cg = Path::new("/sys/fs/cgroup").join(runtime.path().file_name().unwrap());
    fs::create_dir(&cg).unwrap();
    let parent_cg = cg.join("parent");
    let child_cg = cg.join("child");
    fs::create_dir(&parent_cg).unwrap();
    fs::create_dir(&child_cg).unwrap();
    let mut cleanup = Cleanup { parent: child, child: Some(child_pid), groups: vec![child_cg.clone(), parent_cg.clone(), cg] };
    fs::write(parent_cg.join("cgroup.procs"), cleanup.parent.id().to_string()).unwrap();
    fs::write(child_cg.join("cgroup.procs"), child_pid.to_string()).unwrap();
    let launch = |name, uid, path: &Path| Launch {
        identity: identity(name), uid,
        cgroup: format!("/{}", path.strip_prefix("/sys/fs/cgroup").unwrap().display()),
    };
    let mut broker = Broker::open(&state).unwrap();
    let parent = broker.bind_launch(&launch("root", 60_001, &parent_cg), i32::try_from(cleanup.parent.id()).unwrap(), None).unwrap();
    let child_launch = launch("child", 60_002, &child_cg);
    let digest = Broker::spawn_request(&child_launch, child_pid).unwrap();
    let pop = broker.prove(&peer, &parent, &digest).unwrap();
    assert!(broker.bind_launch(&child_launch, child_pid, None).is_err());
    let cert = broker.bind_launch(&child_launch, child_pid, Some((&peer, &parent, &pop))).unwrap();
    let parent_id = certificate(&parent, broker.issuer_key()).unwrap().1;
    assert_eq!(certificate(&cert, broker.issuer_key()).unwrap().8, parent_id);
    assert!(broker.bind_launch(&child_launch, child_pid, Some((&peer, &parent, &pop))).is_err());
    broker.revoke(&parent_id).unwrap();
    assert!(broker.prove(&peer, &parent, &digest).is_err());
    assert!(broker.live(&certificate(&cert, broker.issuer_key()).unwrap(), now().unwrap()).is_err());
    unsafe { libc::kill(child_pid, libc::SIGKILL); }
    drop(peer);
    cleanup.parent.wait().unwrap();
    // Parent reaps its child before exiting; cgroups are now empty.
    for path in &cleanup.groups { fs::remove_dir(path).unwrap(); }
    cleanup.groups.clear();
    cleanup.child = None;
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "subprocess helper for linux_protected_launch_and_verified_spawn"]
fn linux_fixture_process() {
    use std::{io::Write as _, io::Read as _, os::unix::process::CommandExt as _, process::Command};
    let socket = std::env::var("C4_FIXTURE_SOCKET").expect("fixture helper must be launched by privileged test");
    let mut child = Command::new("sleep").arg("60").uid(60_002).gid(60_002).spawn().unwrap();
    assert_eq!(unsafe { libc::setgroups(0, std::ptr::null()) }, 0);
    assert_eq!(unsafe { libc::setgid(60_001) }, 0);
    assert_eq!(unsafe { libc::setuid(60_001) }, 0);
    let mut peer = UnixStream::connect(socket).unwrap();
    peer.write_all(&i32::try_from(child.id()).unwrap().to_le_bytes()).unwrap();
    let mut byte = [0];
    peer.read_exact(&mut byte).expect_err("parent closes socket to end fixture");
    child.wait().unwrap();
}
