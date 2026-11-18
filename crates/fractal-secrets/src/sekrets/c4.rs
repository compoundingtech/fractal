//! Disposable C4 experiment, deliberately not connected to any daemon route.
//!
//! Only a privileged, trusted launcher may call `bind_launch`: identities come from its
//! launch plan, never a caller's environment. Each leaf requires a distinct Unix UID and
//! root-owned cgroup; multiplexed harness leaves are unsupported. Keys remain in this
//! root broker. Socket possession alone is insufficient: every proof rechecks the kernel
//! process incarnation. Restart revokes every old certificate before admitting new work.
use std::{collections::HashMap, fs, os::unix::{fs::{MetadataExt as _, OpenOptionsExt as _}, net::UnixStream}, path::{Path, PathBuf}};

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use smallclaims::fleet::keys::{MemberKey, verify_signature};

use super::{gateway::peer_credentials, identity::{process_cgroup, process_start}};

const CERT: &str = "org.fractal.session.v1";
const PROOF: &str = "org.fractal.session-proof.v1";
const SAFE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed {
    pub statement: String,
    pub signature: String,
}

/// Fully resolved native identity supplied by the trusted launcher, not a wire request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub machine: String,
    pub audience: String,
    pub leaf: String,
    pub incarnation: String,
    pub work_attempt: String,
}

/// Protected launcher configuration. One UID/cgroup per leaf, never shared with agents.
#[derive(Clone, Debug, Serialize)]
pub struct Launch {
    pub identity: Identity,
    pub uid: u32,
    pub cgroup: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Process { pid: i32, uid: u32, start: u64, cgroup: String, parent: i32 }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Certificate(String, String, String, String, String, String, String, String, String, String, u64, u64);
#[derive(Serialize, Deserialize)]
struct Proof(String, String, String, String, String, u64);

struct Session { process: Process, key: MemberKey }

/// Single-owner broker. Keep this value and its root inaccessible to caller UIDs.
/// SQLite is the authoritative admission/revocation store; errors never become trusted.
pub struct Broker { db: Connection, issuer: MemberKey, sessions: HashMap<String, Session>, state_file: Option<(PathBuf, u64, u64)> }

fn identifier(s: &str, empty: bool) -> bool {
    (empty && s.is_empty()) || (!s.is_empty() && s.len() <= 256 && s.bytes().all(|c|
        c.is_ascii_alphanumeric() || b"_./:@-".contains(&c)))
}
fn random() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("randomness: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn now() -> Result<u64> {
    Ok(u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis())?)
}
fn digest(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn signed<T: Serialize>(value: &T, key: &MemberKey) -> Result<Signed> {
    let statement = serde_json::to_string(value)?;
    let signature = key.sign(statement.as_bytes());
    Ok(Signed { statement, signature })
}
fn certificate(wire: &Signed, issuer: &str) -> Result<Certificate> {
    let c: Certificate = serde_json::from_str(&wire.statement)?;
    ensure!(serde_json::to_string(&c)? == wire.statement && c.0 == CERT && c.2 == issuer, "noncanonical certificate or issuer");
    ensure!([&c.1, &c.3, &c.4, &c.5, &c.6].iter().all(|s| identifier(s, false)) && identifier(&c.7, true) && identifier(&c.8, true), "invalid identity");
    ensure!(URL_SAFE_NO_PAD.decode(&c.9)?.len() == 32 && c.9.len() == 43, "invalid session key");
    ensure!(c.10 < c.11 && c.11 <= SAFE && c.11 - c.10 <= 60_000, "invalid lifetime");
    ensure!(verify_signature(issuer, wire.statement.as_bytes(), &wire.signature), "certificate signature");
    Ok(c)
}

fn observe(pid: i32) -> Result<Process> {
    ensure!(cfg!(target_os = "linux") && pid > 0, "Linux process required");
    let start = process_start(pid).context("process start unavailable")?;
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let parent = stat.rsplit_once(") ").context("process stat")?.1.split_whitespace().nth(1).context("process parent")?.parse()?;
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let uids = status.lines().find_map(|s| s.strip_prefix("Uid:")).context("process uid")?
        .split_whitespace().map(str::parse::<u32>).collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(uids.len() == 4 && uids.iter().all(|u| *u == uids[0]), "ambiguous process uid");
    let cgroup = process_cgroup(pid).context("process cgroup unavailable")?;
    ensure!(process_start(pid) == Some(start), "process changed during observation");
    Ok(Process { pid, uid: uids[0], start, cgroup, parent })
}

impl Broker {
    /// Open a pre-created disposable 0700 root owned by root. No production enrollment.
    /// Exclusive lifetime lock prevents a second broker from resetting live revocations.
    pub fn open(root: &Path) -> Result<Self> {
        ensure!(cfg!(target_os = "linux"), "C4 issuer requires Linux");
        // SAFETY: geteuid has no pointer or lifetime requirements.
        ensure!(unsafe { libc::geteuid() } == 0, "broker must run as root, separate from every leaf");
        let meta = fs::symlink_metadata(root)?;
        ensure!(meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o077 == 0, "unprotected broker root");
        for ancestor in root.ancestors().skip(1) {
            let meta = fs::symlink_metadata(ancestor)?;
            ensure!(meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o022 == 0, "broker root needs root-controlled ancestors");
        }
        let key_path = root.join("c4-issuer.key");
        if key_path.exists() {
            let meta = fs::symlink_metadata(&key_path)?;
            ensure!(meta.is_file() && meta.uid() == 0 && meta.mode() & 0o077 == 0, "unprotected issuer key");
        }
        let issuer = MemberKey::load_or_create(&key_path)?;
        let path = root.join("c4-experimental-v1.sqlite");
        if !path.exists() { fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?; }
        let meta = fs::symlink_metadata(&path)?;
        ensure!(meta.is_file() && meta.uid() == 0 && meta.mode() & 0o077 == 0, "unprotected database");
        let mut broker = Self::initialize(Connection::open(&path)?, issuer)?;
        broker.state_file = Some((path, meta.dev(), meta.ino()));
        Ok(broker)
    }

    fn initialize(db: Connection, issuer: MemberKey) -> Result<Self> {
        // One exclusive connection for the broker lifetime, including admission transactions.
        db.execute_batch("PRAGMA locking_mode=EXCLUSIVE; PRAGMA synchronous=FULL;
            BEGIN EXCLUSIVE;
            CREATE TABLE IF NOT EXISTS issuer(key TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS certificates(id TEXT PRIMARY KEY, parent TEXT NOT NULL, statement TEXT NOT NULL UNIQUE, expires INTEGER NOT NULL, revoked INTEGER NOT NULL, incarnation TEXT NOT NULL UNIQUE, uid INTEGER NOT NULL UNIQUE, cgroup TEXT NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS nonces(cert TEXT NOT NULL, nonce TEXT NOT NULL, PRIMARY KEY(cert,nonce));")?;
        let existing: Option<String> = db.query_row("SELECT key FROM issuer", [], |r| r.get(0)).optional()?;
        ensure!(existing.as_deref().is_none_or(|k| k == issuer.public()), "issuer key changed: use a fresh disposable root");
        db.execute("INSERT OR IGNORE INTO issuer VALUES (?)", [issuer.public()])?;
        db.execute_batch("UPDATE certificates SET revoked=1; COMMIT;")?;
        Ok(Self { db, issuer, sessions: HashMap::new(), state_file: None })
    }

    /// Public experimental trust pin; this does not enroll the issuer anywhere.
    pub fn issuer_key(&self) -> &str { self.issuer.public() }

    /// Trusted launcher seam only. For a root, the issuer must be the actual process parent.
    /// For a child, the parent socket, session PoP, and independently observed PPID must agree.
    /// The launcher must place the process in its protected cgroup before calling this method.
    pub fn bind_launch(&mut self, launch: &Launch, pid: i32, parent: Option<(&UnixStream, &Signed, &Signed)>) -> Result<Signed> {
        let process = observe(pid)?;
        ensure!(process.uid != 0 && process.uid == launch.uid && process.cgroup == launch.cgroup, "launch boundary mismatch");
        ensure!(self.sessions.values().all(|s| s.process.uid != process.uid && s.process.cgroup != process.cgroup && s.process.pid != pid), "shared UID, cgroup, or process cannot represent an independent leaf");
        ensure!(Some(process.cgroup.clone()) != process_cgroup(i32::try_from(std::process::id())?), "leaf must have a separate protected cgroup");
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if let Ok(other_pid) = entry.file_name().to_string_lossy().parse::<i32>() {
                if entry.metadata().is_ok_and(|m| m.uid() == process.uid) {
                    ensure!(process_cgroup(other_pid).as_deref() == Some(process.cgroup.as_str()), "leaf UID is shared outside its session boundary");
                }
            }
        }
        ensure!(process.cgroup.starts_with('/') && !process.cgroup.split('/').any(|s| s == ".."), "invalid cgroup path");
        let path = Path::new("/sys/fs/cgroup").join(process.cgroup.trim_start_matches('/'));
        for directory in path.ancestors() {
            let meta = fs::symlink_metadata(directory)?;
            ensure!(meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o022 == 0, "caller-writable cgroup boundary");
        }
        let at = now()?;
        let (parent_id, expires) = if let Some((socket, cert, proof)) = parent {
            let c = certificate(cert, self.issuer.public())?;
            self.check_peer(socket, &c.1)?;
            let parent_process = &self.sessions.get(&c.1).context("parent key missing")?.process;
            ensure!(process.parent == parent_process.pid, "not an observed child of parent");
            ensure!(launch.identity.machine == c.3 && launch.identity.audience == c.4, "child trust domain differs");
            self.admit_at(cert, proof, &Identity { machine: c.3.clone(), audience: c.4.clone(), leaf: c.5.clone(), incarnation: c.6.clone(), work_attempt: c.7.clone() }, &c.8, &Self::spawn_request(launch, pid)?, at)?;
            (c.1, c.11.min(at.saturating_add(60_000)))
        } else {
            ensure!(process.parent == i32::try_from(std::process::id())?, "root was not launched by issuer");
            (String::new(), at.saturating_add(60_000))
        };
        ensure!(observe(pid)? == process, "launch changed while authenticating");
        self.issue(launch.identity.clone(), process, parent_id, at, expires)
    }

    /// Digest for parent authorization: exact observed child incarnation and trusted launch plan.
    pub fn spawn_request(launch: &Launch, pid: i32) -> Result<String> {
        smallclaims::hash::canonical_hash(&("org.fractal.spawn.v1", launch, observe(pid)?))
    }

    fn issue(&mut self, identity: Identity, process: Process, parent: String, at: u64, expires: u64) -> Result<Signed> {
        let (key, _) = MemberKey::generate()?;
        let c = Certificate(CERT.into(), random()?, self.issuer.public().into(), identity.machine, identity.audience, identity.leaf, identity.incarnation, identity.work_attempt, parent, key.public().into(), at, expires);
        let wire = signed(&c, &self.issuer)?;
        certificate(&wire, self.issuer.public())?;
        self.db.execute("INSERT INTO certificates VALUES (?1,?2,?3,?4,0,?5,?6,?7)", params![c.1, c.8, wire.statement, c.11, c.6, process.uid, process.cgroup])?;
        self.sessions.insert(c.1, Session { process, key });
        Ok(wire)
    }

    fn check_peer(&self, socket: &UnixStream, id: &str) -> Result<()> {
        let (pid, uid) = peer_credentials(socket)?;
        let session = self.sessions.get(id).context("session key absent after restart or unknown session")?;
        ensure!(uid == session.process.uid && observe(pid)? == session.process, "peer process incarnation or boundary changed");
        Ok(())
    }

    /// Per-request broker PoP; the session private key never leaves this process.
    pub fn prove(&self, socket: &UnixStream, cert: &Signed, request_digest: &str) -> Result<Signed> {
        let c = certificate(cert, self.issuer.public())?;
        self.check_peer(socket, &c.1)?;
        let at = now()?;
        ensure!(digest(request_digest), "invalid request digest");
        self.live(&c, at)?;
        signed(&Proof(PROOF.into(), c.1.clone(), c.4, request_digest.into(), random()?, at), &self.sessions.get(&c.1).context("missing key")?.key)
    }

    fn live(&self, c: &Certificate, at: u64) -> Result<()> {
        if let Some((path, device, inode)) = &self.state_file {
            let meta = fs::symlink_metadata(path)?;
            ensure!(meta.is_file() && meta.dev() == *device && meta.ino() == *inode && meta.uid() == 0 && meta.mode() & 0o077 == 0, "revocation state missing, replaced, or unprotected");
        }
        ensure!(c.10 <= at && at < c.11, "certificate expired or not yet valid");
        let mut id = c.1.clone();
        let mut visited = std::collections::HashSet::new();
        loop {
            ensure!(visited.insert(id.clone()), "ancestry cycle");
            let (parent, statement, expires, revoked): (String, String, u64, bool) = self.db.query_row("SELECT parent,statement,expires,revoked FROM certificates WHERE id=?", [&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            ensure!(!revoked && at < expires, "certificate or ancestor revoked/expired");
            if id == c.1 { ensure!(statement == serde_json::to_string(c)?, "certificate not registered"); }
            if parent.is_empty() { return Ok(()); }
            id = parent;
        }
    }

    /// Online admission is the freshness boundary, not an offline trust cache. Consumes nonce
    /// atomically with current ancestry/revocation checks in the same exclusive SQLite owner.
    pub fn admit(&mut self, cert: &Signed, proof: &Signed, expected: &Identity, parent: &str, request_digest: &str) -> Result<()> {
        self.admit_at(cert, proof, expected, parent, request_digest, now()?)
    }
    fn admit_at(&mut self, cert: &Signed, proof: &Signed, expected: &Identity, parent: &str, request_digest: &str, at: u64) -> Result<()> {
        let c = certificate(cert, self.issuer.public())?;
        ensure!(Identity { machine: c.3.clone(), audience: c.4.clone(), leaf: c.5.clone(), incarnation: c.6.clone(), work_attempt: c.7.clone() } == *expected && c.8 == parent, "wrong expected identity or parent");
        let p: Proof = serde_json::from_str(&proof.statement)?;
        ensure!(serde_json::to_string(&p)? == proof.statement && p.0 == PROOF && p.1 == c.1 && p.2 == c.4 && digest(request_digest) && p.3 == request_digest, "wrong proof request");
        ensure!(p.4.len() == 43 && URL_SAFE_NO_PAD.decode(&p.4)?.len() == 32 && c.10 <= p.5 && p.5 < c.11 && at.abs_diff(p.5) <= 5_000, "invalid nonce or stale proof");
        ensure!(verify_signature(&c.9, proof.statement.as_bytes(), &proof.signature), "proof signature");
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            self.live(&c, at)?;
            self.db.execute("INSERT INTO nonces VALUES (?,?)", params![c.1, p.4])?;
            Ok(())
        })();
        if result.is_ok() { self.db.execute_batch("COMMIT")?; } else { self.db.execute_batch("ROLLBACK")?; }
        result
    }

    /// Parent revocation cascades persistently; restart never clears revocation or nonces.
    pub fn revoke(&mut self, id: &str) -> Result<()> {
        ensure!(self.db.execute("WITH RECURSIVE descendants(id) AS (SELECT id FROM certificates WHERE id=?1 UNION SELECT c.id FROM certificates c JOIN descendants d ON c.parent=d.id) UPDATE certificates SET revoked=1 WHERE id IN descendants", [id])? > 0, "unknown certificate");
        Ok(())
    }
}

#[cfg(test)]
#[path = "c4_tests.rs"]
mod tests;
