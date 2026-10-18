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
    affected: Vec<Peer>,
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

fn desired(view: &FleetView, own_key: &str, fleet_id: &str) -> Result<BTreeSet<String>> {
    ensure!(
        view.anchor.is_some(),
        "signed fleet membership is not available; no Fabric policy changes made"
    );
    if !view
        .members
        .iter()
        .any(|member| member.member_key == own_key && member.state == "current")
    {
        return Ok(BTreeSet::new());
    }
    let mut nodes = BTreeSet::new();
    for member in view
        .members
        .iter()
        .filter(|member| member.state == "current" && member.member_key != own_key)
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

fn plan(
    snapshot: &Snapshot,
    owner: &str,
    desired: &BTreeSet<String>,
    grants: &BTreeSet<String>,
) -> Vec<Value> {
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
    for node in owned.keys().filter(|node| !desired.contains(**node)) {
        operations.push(json!({"op":"remove_peer", "node_id":node}));
    }
    for node in desired {
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

fn residual(
    peers: &[Peer],
    desired: &BTreeSet<String>,
    grants: &BTreeSet<String>,
) -> Option<String> {
    let remaining = peers
        .iter()
        .filter(|peer| !desired.contains(&peer.node_id))
        .flat_map(|peer| {
            peer.services
                .iter()
                .filter(|(service, _)| grants.contains(*service))
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

async fn apply(
    socket: &Path,
    fleet_id: &str,
    desired: BTreeSet<String>,
) -> Result<ReconcileStatus> {
    let owner = format!("st/fleet/{fleet_id}");
    let grants = grants(fleet_id);
    let snapshot = snapshot(socket).await?;
    let operations = plan(&snapshot, &owner, &desired, &grants);
    ensure!(
        operations.len() <= 128,
        "Fabric reconciliation requires {} operations (limit 128); no partial policy was installed",
        operations.len()
    );
    let mut revision = snapshot.policy_revision;
    if !operations.is_empty() {
        // Stable for identical plans/revisions; unrelated owners still use their own IDs.
        let digest = Sha256::digest(serde_json::to_vec(&json!([
            &owner,
            &snapshot.instance,
            revision,
            &operations
        ]))?);
        let request_id = format!("st-{}", hex::encode(digest));
        let reply = send(
            socket,
            &json!({"operation":"batch", "owner":owner, "request_id":request_id,
            "expected_revision":revision, "operations":operations}),
        )
        .await?;
        let mut mutation = reply
            .mutation
            .context("Fabric batch returned no mutation receipt")?;
        revision = mutation.current_revision;
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
            "Fabric revocation cancellation is {}; inspect Fabric receipt {request_id}",
            mutation.cancellation.state
        );
        if let Some(message) = residual(&mutation.affected, &desired, &grants) {
            return Ok(ReconcileStatus {
                state: "residual_access".into(),
                message: Some(message),
                policy_revision: Some(revision),
            });
        }
    }
    // Check ended identities too: after a restart our runtime record may already be gone,
    // but static policy must never be mistaken for successful effective revocation.
    Ok(ReconcileStatus {
        state: "applied".into(),
        message: None,
        policy_revision: Some(revision),
    })
}

pub async fn reconcile(
    socket: &Path,
    fleet_id: &str,
    own_key: &str,
    view: &FleetView,
) -> Result<ReconcileStatus> {
    let desired = desired(view, own_key, fleet_id)?;
    let mut status = apply(socket, fleet_id, desired.clone()).await?;
    let ended = view
        .members
        .iter()
        .filter(|member| member.state != "current")
        .flat_map(|member| member.endpoints.iter())
        .filter(|endpoint| endpoint["transport"] == "fabric")
        .filter_map(|endpoint| endpoint["node"].as_str())
        .collect::<BTreeSet<_>>();
    if !ended.is_empty() {
        let current = snapshot(socket).await?;
        let peers = current
            .peers
            .into_iter()
            .filter(|peer| ended.contains(peer.node_id.as_str()))
            .collect::<Vec<_>>();
        if let Some(message) = residual(&peers, &desired, &grants(fleet_id)) {
            status.state = "residual_access".into();
            status.message = Some(message);
        }
    }
    Ok(status)
}

/// Revoke this fleet's runtime records before local fleet settings are deleted.
pub async fn withdraw(socket: &Path, fleet_id: &str) -> Result<ReconcileStatus> {
    let status = apply(socket, fleet_id, BTreeSet::new()).await?;
    let current = snapshot(socket).await?;
    if let Some(message) = residual(&current.peers, &BTreeSet::new(), &grants(fleet_id)) {
        return Ok(ReconcileStatus {
            state: "residual_access".into(),
            message: Some(message),
            policy_revision: Some(current.policy_revision),
        });
    }
    Ok(status)
}

#[cfg(test)]
#[path = "fabric_reconcile_tests.rs"]
mod tests;
