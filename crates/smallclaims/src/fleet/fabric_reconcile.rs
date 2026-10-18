//! Reconcile only this fleet's runtime policy; Fabric remains the policy authority.
//!
//! The provisional wire contract is documented in Fabric's docs/runtime-peer-api.md.
//! Socket access is authority. The owner is attribution, not an authentication boundary.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::FleetView;

const REQUEST_LIMIT: usize = 1024 * 1024;
const RESPONSE_LIMIT: u64 = 4 * 1024 * 1024;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ReconcileStatus {
    pub state: String,
    pub message: Option<String>,
    pub policy_revision: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PolicyError {
    code: String,
    class: String,
    message: String,
}
impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.code == "runtime_peers_disabled" {
            write!(
                f,
                "runtime_peers_disabled: restart Fabric with --runtime-peers; static peers remain unchanged and usable"
            )
        } else {
            write!(f, "Fabric {} ({}): {}", self.code, self.class, self.message)
        }
    }
}
impl std::error::Error for PolicyError {}

pub fn error_status(error: &anyhow::Error) -> ReconcileStatus {
    ReconcileStatus {
        state: error
            .downcast_ref::<PolicyError>()
            .map_or("error", |error| {
                if error.code == "runtime_peers_disabled" {
                    "runtime_peers_disabled"
                } else {
                    "error"
                }
            })
            .into(),
        message: Some(format!("{error:#}")),
        policy_revision: None,
    }
}

/// Match FabricHome's control endpoint, including isolated FABRIC_HOME services.
pub fn control_socket() -> Result<PathBuf> {
    let home = match std::env::var_os("FABRIC_HOME") {
        Some(home) => PathBuf::from(home),
        None => {
            PathBuf::from(std::env::var_os("HOME").context("HOME is unset; configure FABRIC_HOME")?)
                .join(".local/share/fabric")
        }
    };
    Ok(home.join("run/control.sock"))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Source {
    Runtime { owner: String },
    Static { path: PathBuf },
}
impl Source {
    fn owned_by(&self, owner: &str) -> bool {
        matches!(self, Self::Runtime { owner: source_owner } if source_owner == owner)
    }
    fn describe(&self) -> String {
        match self {
            Self::Runtime { owner } => format!("runtime owner {owner}"),
            Self::Static { path } => format!("static {}", path.display()),
        }
    }
}
#[derive(Debug, Deserialize)]
struct Peer {
    node_id: String,
    trust_sources: Vec<Source>,
    services: BTreeMap<String, Vec<Source>>,
}
#[derive(Debug, Deserialize)]
struct Snapshot {
    instance: String,
    policy_revision: u64,
    peers: Vec<Peer>,
    next_cursor: Option<Value>,
    fault: Option<String>,
}
#[derive(Debug, Deserialize)]
struct Cancellation {
    state: String,
}
#[derive(Debug, Deserialize)]
struct Mutation {
    current_revision: u64,
    cancellation: Cancellation,
}
#[derive(Debug, Deserialize)]
struct Reply {
    snapshot: Option<Snapshot>,
    mutation: Option<Mutation>,
    error: Option<PolicyError>,
}
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    reply: Reply,
}

async fn send(socket: &Path, request: &Value) -> Result<Reply> {
    let mut bytes = serde_json::to_vec(&json!({"type":"runtime_peers", "request":request}))?;
    bytes.push(b'\n');
    ensure!(
        bytes.len() <= REQUEST_LIMIT,
        "Fabric runtime request exceeds 1 MiB; policy was not changed"
    );
    // Retrying identical bytes preserves the transaction's durable receipt semantics.
    // Later reconciliation always reads current policy, never blindly replays an old plan.
    let mut last_error = None;
    for _ in 0..2 {
        let response = tokio::time::timeout(CONTROL_TIMEOUT, async {
            let mut stream = tokio::net::UnixStream::connect(socket).await?;
            stream.write_all(&bytes).await?;
            let mut response = Vec::new();
            BufReader::new(stream)
                .take(RESPONSE_LIMIT + 1)
                .read_until(b'\n', &mut response)
                .await?;
            ensure!(
                response.len() as u64 <= RESPONSE_LIMIT,
                "Fabric control response exceeds 4 MiB"
            );
            ensure!(
                response.last() == Some(&b'\n'),
                "Fabric control response is not a complete JSON line"
            );
            Ok::<_, anyhow::Error>(response)
        })
        .await;
        match response {
            Ok(Ok(response)) => {
                let envelope: Envelope = serde_json::from_slice(&response)
                    .context("Fabric runtime peer API unavailable or malformed; upgrade Fabric to a release containing #324")?;
                ensure!(
                    envelope.kind == "runtime_peers",
                    "Fabric returned an unexpected control response"
                );
                if let Some(error) = envelope.reply.error {
                    return Err(error.into());
                }
                return Ok(envelope.reply);
            }
            Ok(Err(error)) => last_error = Some(error),
            Err(error) => last_error = Some(error.into()),
        }
    }
    Err(last_error.expect("two control attempts")).with_context(|| {
        format!(
            "Fabric runtime peer reconciliation at {} failed; static peers are unchanged",
            socket.display()
        )
    })
}

async fn snapshot(socket: &Path) -> Result<Snapshot> {
    let mut first: Option<Snapshot> = None;
    let mut cursor = None;
    loop {
        let reply = send(socket, &json!({"operation":"list", "cursor":cursor})).await?;
        let mut page = reply.snapshot.context("Fabric list returned no snapshot")?;
        if let Some(fault) = &page.fault {
            bail!("Fabric policy is faulted: {fault}");
        }
        cursor = page.next_cursor.take();
        if let Some(first) = first.as_mut() {
            ensure!(
                first.instance == page.instance && first.policy_revision == page.policy_revision,
                "Fabric policy changed during pagination; reconciliation will retry with a fresh snapshot"
            );
            first.peers.append(&mut page.peers);
        } else {
            first = Some(page);
        }
        ensure!(
            first.as_ref().expect("first page").peers.len() <= 4096,
            "Fabric peer snapshot exceeds supported size"
        );
        if cursor.is_none() {
            return Ok(first.expect("first page"));
        }
    }
}

fn grants(fleet_id: &str) -> BTreeSet<String> {
    [
        format!("st3/fleet/{fleet_id}"),
        format!("st3/pty/{fleet_id}"),
        "st3-client-v0".into(),
    ]
    .into()
}

fn desired(
    view: &FleetView,
    own_key: &str,
    fleet_id: &str,
    retained_member_keys: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let eligible = |member: &super::MemberView| {
        member.state == "current"
            || (member.state == "ended"
                && member.ended.as_deref() == Some("left")
                && retained_member_keys.contains(&member.member_key))
    };
    ensure!(
        view.anchor.is_some(),
        "signed fleet membership is not available; no Fabric policy changes made"
    );
    if !view
        .members
        .iter()
        .any(|member| member.member_key == own_key && eligible(member))
    {
        return Ok(BTreeSet::new());
    }
    let mut nodes = BTreeSet::new();
    for member in view
        .members
        .iter()
        .filter(|member| eligible(member) && member.member_key != own_key)
    {
        for endpoint in &member.endpoints {
            if endpoint["transport"] != "fabric" {
                continue;
            }
            // Do not turn arbitrary signed endpoint names into Fabric grants.
            ensure!(
                endpoint["protocol"] == format!("st3/fleet/{fleet_id}"),
                "member {} advertises a nonstandard Fabric replication protocol; expected st3/fleet/{fleet_id}",
                member.name
            );
            let node = endpoint["node"]
                .as_str()
                .filter(|node| !node.is_empty())
                .context("signed Fabric endpoint has no NodeID")?;
            nodes.insert(node.to_owned());
        }
    }
    Ok(nodes)
}

type DesiredPolicy = BTreeMap<String, BTreeSet<String>>;

fn plan(snapshot: &Snapshot, owner: &str, desired: &DesiredPolicy) -> Vec<Value> {
    let owned = snapshot
        .peers
        .iter()
        .filter(|peer| {
            peer.trust_sources
                .iter()
                .any(|source| source.owned_by(owner))
        })
        .map(|peer| (peer.node_id.as_str(), peer))
        .collect::<BTreeMap<_, _>>();
    let mut operations = Vec::new();
    // Withdraw first, in the same atomic publication as additions (including re-key).
    for node in owned.keys().filter(|node| !desired.contains_key(**node)) {
        operations.push(json!({"op":"remove_peer", "node_id":node}));
    }
    for (node, grants) in desired {
        if let Some(peer) = owned.get(node.as_str()) {
            let actual = peer
                .services
                .iter()
                .filter(|(_, sources)| sources.iter().any(|source| source.owned_by(owner)))
                .map(|(service, _)| service.clone())
                .collect::<BTreeSet<_>>();
            if actual == *grants {
                continue;
            }
        } else {
            // No label: avoid conflicting with a static source's user-selected name.
            operations.push(json!({"op":"add_peer", "node_id":node}));
        }
        operations.push(json!({"op":"set_grants", "node_id":node, "grants":grants}));
    }
    operations
}

fn residual(peers: &[Peer], desired: &DesiredPolicy, grants: &BTreeSet<String>) -> Option<String> {
    let remaining = peers
        .iter()
        .flat_map(|peer| {
            peer.services
                .iter()
                .filter(|(service, _)| {
                    grants.contains(*service)
                        && !desired
                            .get(&peer.node_id)
                            .is_some_and(|expected| expected.contains(*service))
                })
                .map(|(service, sources)| {
                    format!(
                        "{} {service}: {}",
                        peer.node_id,
                        sources
                            .iter()
                            .map(Source::describe)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
        })
        .collect::<Vec<_>>();
    (!remaining.is_empty()).then(|| format!("Residual Fabric access was not removed (static/other-owner policy is never edited): {}", remaining.join("; ")))
}

#[derive(Default, Deserialize, Serialize)]
struct Journal {
    pending: Option<Pending>,
    retired: BTreeSet<String>,
}

#[derive(Deserialize, Serialize)]
struct Pending {
    request: Value,
    created_unix_ms: u128,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

async fn load_journal(state_dir: &Path) -> Result<Journal> {
    let path = state_dir.join("fleet/fabric-policy.json");
    tokio::task::spawn_blocking(move || match std::fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("read {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Journal::default()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    })
    .await?
}

async fn save_journal(state_dir: &Path, journal: &Journal) -> Result<()> {
    let path = state_dir.join("fleet/fabric-policy.json");
    let bytes = serde_json::to_vec(journal)?;
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let directory = path.parent().expect("fleet directory");
        std::fs::create_dir_all(directory)?;
        let temporary = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temporary, &path)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

fn pending_matches_desired(pending: &Pending, desired: &DesiredPolicy) -> bool {
    pending.request["operations"]
        .as_array()
        .is_some_and(|operations| {
            operations.iter().all(|operation| {
                let Some(node) = operation["node_id"].as_str() else {
                    return false;
                };
                match operation["op"].as_str() {
                    Some("add_peer") => desired.contains_key(node),
                    Some("remove_peer") => !desired.contains_key(node),
                    Some("set_grants") => desired.get(node).is_some_and(|grants| {
                        operation["grants"].as_array().is_some_and(|services| {
                            services.len() == grants.len()
                                && grants.iter().all(|grant| {
                                    services
                                        .iter()
                                        .any(|service| service.as_str() == Some(grant.as_str()))
                                })
                        })
                    }),
                    _ => false,
                }
            })
        })
}

async fn complete_pending(
    socket: &Path,
    pending: &Pending,
    replay_allowed: bool,
) -> Result<Mutation> {
    let request_id = pending.request["request_id"]
        .as_str()
        .context("pending Fabric transaction has no request ID")?;
    let reply = if !replay_allowed
        || now_ms().saturating_sub(pending.created_unix_ms) >= 24 * 60 * 60 * 1000
    {
        // Changed membership and expired plans are receipt queries only. Never
        // create obsolete rights merely to find out whether an old write committed.
        send(
            socket,
            &json!({"operation":"cancellation", "request_id":request_id}),
        )
        .await?
    } else {
        send(socket, &pending.request).await?
    };
    let mut mutation = reply
        .mutation
        .context("Fabric returned no retained mutation receipt")?;
    if mutation.cancellation.state == "pending" {
        tokio::time::sleep(Duration::from_secs(2)).await;
        mutation = send(
            socket,
            &json!({"operation":"cancellation", "request_id":request_id}),
        )
        .await?
        .mutation
        .context("Fabric cancellation returned no receipt")?;
    }
    ensure!(
        mutation.cancellation.state == "complete",
        "Fabric revocation cancellation is {}; retained transaction {request_id} must be resolved before policy is reported applied",
        mutation.cancellation.state
    );
    Ok(mutation)
}

async fn resolve_pending(
    state_dir: &Path,
    socket: &Path,
    journal: &mut Journal,
    desired: &DesiredPolicy,
) -> Result<Option<Mutation>> {
    let Some(pending) = &journal.pending else {
        return Ok(None);
    };
    let replay_allowed = pending_matches_desired(pending, desired);
    match complete_pending(socket, pending, replay_allowed).await {
        Ok(mutation) => {
            journal.pending = None;
            save_journal(state_dir, journal).await?;
            Ok(Some(mutation))
        }
        Err(error) => {
            // A well-formed receipt query reports invalid_argument only for a
            // missing/expired ID. Inside guaranteed retention, an obsolete request
            // with no receipt must be discarded, not submitted as a fresh add.
            if !replay_allowed
                && now_ms().saturating_sub(pending.created_unix_ms) < 24 * 60 * 60 * 1000
                && error
                    .downcast_ref::<PolicyError>()
                    .is_some_and(|error| error.code == "invalid_argument")
            {
                journal.pending = None;
                save_journal(state_dir, journal).await?;
                return Ok(None);
            }
            // Within the guaranteed receipt-retention window a coded refusal means
            // this exact transaction was rejected, not an uncertain lost commit.
            // Revision conflicts are noncommitting even though Fabric classes them as
            // faults. Other uncertain faults and expired receipts remain pending.
            let rejected = error.downcast_ref::<PolicyError>().is_some_and(|error| {
                (error.code == "policy_revision_conflict"
                    || matches!(error.class.as_str(), "refusal" | "usage"))
                    && !matches!(
                        error.code.as_str(),
                        "runtime_peers_disabled" | "cancellation_timeout" | "request_id_conflict"
                    )
            }) && now_ms().saturating_sub(pending.created_unix_ms)
                < 24 * 60 * 60 * 1000;
            if rejected {
                journal.pending = None;
                save_journal(state_dir, journal).await?;
            }
            Err(error)
        }
    }
}

async fn apply(
    state_dir: &Path,
    socket: &Path,
    fleet_id: &str,
    desired: DesiredPolicy,
    ended: BTreeSet<String>,
    withdrawing: bool,
) -> Result<ReconcileStatus> {
    let owner = format!("st/fleet/{fleet_id}");
    let grants = grants(fleet_id);
    let mut journal = load_journal(state_dir).await?;
    resolve_pending(state_dir, socket, &mut journal, &desired).await?;
    let mut journal_changed = false;
    for node in ended {
        journal_changed |= journal.retired.insert(node);
    }
    let mut current = snapshot(socket).await?;
    let operations = plan(&current, &owner, &desired);
    ensure!(
        operations.len() <= 128,
        "Fabric reconciliation requires {} operations (limit 128); no partial policy was installed",
        operations.len()
    );
    if !operations.is_empty() {
        for operation in &operations {
            if operation["op"] == "remove_peer" {
                journal.retired.insert(
                    operation["node_id"]
                        .as_str()
                        .context("removal has no NodeID")?
                        .to_owned(),
                );
            }
        }
        let revision = current.policy_revision;
        let digest = Sha256::digest(serde_json::to_vec(&json!([
            &owner,
            &current.instance,
            revision,
            &operations
        ]))?);
        journal.pending = Some(Pending {
            request: json!({"operation":"batch", "owner":owner, "request_id":format!("st-{}", hex::encode(digest)),
                "expected_revision":revision, "operations":operations}),
            created_unix_ms: now_ms(),
        });
        // Durable before sending: restarting st must not forget an uncertain commit
        // or replace a pending/timed-out cancellation with an empty-diff success.
        save_journal(state_dir, &journal).await?;
        let mutation = resolve_pending(state_dir, socket, &mut journal, &desired)
            .await?
            .expect("pending transaction");
        journal_changed = false;
        current = snapshot(socket).await?;
        ensure!(
            current.policy_revision >= mutation.current_revision,
            "Fabric policy revision regressed after mutation"
        );
    }
    let retired_count = journal.retired.len();
    journal.retired.retain(|node| {
        !desired.contains_key(node)
            && current.peers.iter().any(|peer| {
                peer.node_id == *node
                    && peer.services.keys().any(|service| grants.contains(service))
            })
    });
    if journal_changed || journal.retired.len() != retired_count {
        save_journal(state_dir, &journal).await?;
    }
    let relevant = current
        .peers
        .into_iter()
        .filter(|peer| {
            withdrawing
                || journal.retired.contains(&peer.node_id)
                || desired
                    .get(&peer.node_id)
                    .is_some_and(|expected| expected.len() < grants.len())
        })
        .collect::<Vec<_>>();
    let message = residual(&relevant, &desired, &grants);
    Ok(ReconcileStatus {
        state: if message.is_some() {
            "residual_access"
        } else {
            "applied"
        }
        .into(),
        message,
        policy_revision: Some(current.policy_revision),
    })
}

/// Call under the fleet settings lock so journal writes and retirement cannot race.
pub async fn reconcile(
    state_dir: &Path,
    socket: &Path,
    fleet_id: &str,
    own_key: &str,
    view: &FleetView,
    retained_member_keys: &BTreeSet<String>,
) -> Result<ReconcileStatus> {
    let desired = desired(view, own_key, fleet_id, retained_member_keys)?;
    let own_current = view
        .members
        .iter()
        .any(|member| member.member_key == own_key && member.state == "current");
    let desired = desired
        .into_iter()
        .map(|node| {
            let peer_current = view.members.iter().any(|member| {
                member.state == "current"
                    && member.endpoints.iter().any(|endpoint| {
                        endpoint["transport"] == "fabric" && endpoint["node"] == node
                    })
            });
            let services = if own_current && peer_current {
                grants(fleet_id)
            } else {
                BTreeSet::from([format!("st3/fleet/{fleet_id}")])
            };
            (node, services)
        })
        .collect();
    let ended = view
        .members
        .iter()
        .filter(|member| member.state != "current")
        .flat_map(|member| member.endpoints.iter())
        .filter(|endpoint| endpoint["transport"] == "fabric")
        .filter_map(|endpoint| endpoint["node"].as_str().map(str::to_owned))
        .collect();
    let mut status = apply(state_dir, socket, fleet_id, desired, ended, false).await?;
    if status.state == "applied"
        && view.members.iter().any(|member| {
            member.state == "ended"
                && member.ended.as_deref() == Some("left")
                && retained_member_keys.contains(&member.member_key)
        })
    {
        status.state = "leaving".into();
        status.message = Some("Fabric carrier retained until authenticated leave receipt completes; ordinary replication still refuses ended members".into());
    }
    Ok(status)
}

/// Revoke this fleet's runtime records before local fleet settings are deleted.
/// Call under the fleet settings lock, also held by the replication worker.
pub async fn withdraw(state_dir: &Path, socket: &Path, fleet_id: &str) -> Result<ReconcileStatus> {
    apply(
        state_dir,
        socket,
        fleet_id,
        DesiredPolicy::new(),
        BTreeSet::new(),
        true,
    )
    .await
}

#[cfg(test)]
#[path = "fabric_reconcile_tests.rs"]
mod tests;
