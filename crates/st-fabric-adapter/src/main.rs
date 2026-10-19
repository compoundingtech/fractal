//! A separate local policy adapter: st owns membership, Fabric owns authorization.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::{Deserialize, Serialize};
use smallclaims::fleet::FleetView;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};
mod reconcile;
mod pairing;

#[derive(Debug, Parser)]
#[command(about = "Reconcile verified local st fleet membership into Fabric runtime peers")]
struct Args {
    #[command(subcommand)]
    command: Option<PairingCommand>,
    /// Trusted local st3 HTTP socket (never the replication socket).
    #[arg(long)]
    st_socket: Option<PathBuf>,
    /// Fabric runtime-peers control socket; defaults to FABRIC_HOME/run/control.sock.
    #[arg(long)]
    fabric_socket: Option<PathBuf>,
    /// Adapter-owned durable state, separate from the st daemon's state.
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Reconcile once, print JSON status, and exit nonzero on failure.
    #[arg(long, conflicts_with = "status")]
    once: bool,
    /// Print the last persisted JSON status without contacting either daemon.
    #[arg(long)]
    status: bool,
    /// Poll interval in seconds.
    #[arg(long, default_value = "5", value_parser = clap::value_parser!(u64).range(1..))]
    interval: u64,
}

#[derive(Debug, clap::Subcommand)]
enum PairingCommand {
    /// Attach a restricted Fabric permit to an existing invite ({code} on stdin).
    PairingInvite,
    /// Exchange signed admission over Fabric's restricted pairing ALPN.
    PairingJoin,
}

#[derive(Debug, Deserialize)]
struct LeaveSnapshot {
    fleet_id: Option<String>,
    own_member_key: Option<String>,
    membership: FleetView,
    members: Vec<MemberLeave>,
    persistence_error: Option<String>,
}
#[derive(Debug, Deserialize)]
struct MemberLeave {
    member_key: String,
    state: String,
    grace_deadline_unix_ms: Option<u128>,
}
#[derive(Debug, Serialize, Deserialize)]
struct Status {
    observed_unix_ms: u128,
    fleet_id: Option<String>,
    #[serde(flatten)]
    policy: reconcile::ReconcileStatus,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").context("HOME is unset; pass --state-dir")?,
    ))
}
fn default_st_socket() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR")
            .context("XDG_RUNTIME_DIR is unset; pass --st-socket")?,
    )
    .join("st3.sock"))
}

/// Durable before any external mutation, including ownership discovery on first use.
fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let parent = path.parent().context("state file has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn owners(path: &Path) -> Result<BTreeSet<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(error.into()),
    }
}
fn fleet_directory(state: &Path, fleet: &str) -> PathBuf {
    // Fleet identifiers never become path components supplied by the daemon.
    use sha2::{Digest, Sha256};
    state.join(hex::encode(Sha256::digest(fleet.as_bytes())))
}
fn retained(snapshot: &LeaveSnapshot, now: u128) -> BTreeSet<String> {
    snapshot
        .members
        .iter()
        .filter(|member| {
            // Own departure remains CLI-owned until its receipt completes and
            // settings disappear; only receivers expire another member's carrier.
            snapshot.own_member_key.as_deref() == Some(member.member_key.as_str())
                || (member.state == "awaiting-confirmation"
                    && member.grace_deadline_unix_ms.is_some_and(|deadline| deadline > now))
        })
        .map(|member| member.member_key.clone())
        .collect()
}

fn fence_pairing(snapshot: &mut LeaveSnapshot, pending: &pairing::PairingState) {
    for member in &mut snapshot.membership.members {
        // A fresh confirmed receipt supersedes an older failed attempt only for
        // this exact current signed member and Fabric identity, never node-wide.
        let confirmed_node = member.node_id.as_deref().filter(|node| {
            member.state == "current"
                && snapshot.fleet_id.as_ref().is_some_and(|fleet| {
                    pending.confirmed.contains(&(
                        fleet.clone(), member.member_key.clone(), (*node).to_owned(),
                    ))
                })
        });
        if let (Some(fleet), Some(node)) = (
            &snapshot.fleet_id,
            confirmed_node.filter(|_| !member.endpoints_published),
        ) {
            member.endpoints.push(serde_json::json!({
                "transport":"fabric", "node":node,
                "protocol":format!("st3/fleet/{fleet}")
            }));
        }
        member.endpoints.retain(|endpoint| {
            endpoint["transport"] != "fabric"
                || endpoint["node"].as_str().is_none_or(|node| {
                    !pending.blocked.contains(node) || confirmed_node == Some(node)
                })
        });
    }
}

async fn poll(
    client: &reqwest::Client,
    socket: &Path,
    state: &Path,
) -> Result<(Option<String>, reconcile::ReconcileStatus)> {
    // A failed read is not an empty fleet and must never trigger withdrawal.
    let mut response = client
        .get("http://localhost/v1/internal/fleet/leave-state")
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "st leave snapshot exceeds 4 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let mut snapshot: LeaveSnapshot = serde_json::from_slice(&bytes).context(
        "read st leave-state contract; upgrade st to a version supporting local adapters",
    )?;
    ensure!(
        snapshot.persistence_error.is_none(),
        "st leave state is not durable: {}",
        snapshot.persistence_error.as_deref().unwrap_or_default()
    );
    // Admission publishes signed membership before Fabric confirmation. Fence
    // every pending bound identity so normal reconciliation cannot grant it early.
    let pending = pairing::tick(client, socket, state).await?;
    fence_pairing(&mut snapshot, &pending);
    let owners_path = state.join("owners.json");
    let mut known = owners(&owners_path)?;
    if let Some(fleet) = &snapshot.fleet_id {
        ensure!(!fleet.is_empty(), "st returned an empty fleet id");
        ensure!(
            snapshot.own_member_key.is_some(),
            "st fleet has no local member key"
        );
        if known.insert(fleet.clone()) {
            save(&owners_path, &known)?;
        }
    }
    let mut last = reconcile::ReconcileStatus {
        state: "idle".into(),
        message: None,
        policy_revision: None,
    };
    let mut residuals = Vec::new();
    for old in known
        .clone()
        .into_iter()
        .filter(|old| Some(old) != snapshot.fleet_id.as_ref())
    {
        last = reconcile::withdraw(&fleet_directory(state, &old), socket, &old).await?;
        // Keep residual owners visible without blocking the active fleet's reconciliation.
        if last.state == "residual_access" {
            residuals.push(format!(
                "{old}: {}",
                last.message.as_deref().unwrap_or("residual access")
            ));
        } else {
            known.remove(&old);
            save(&owners_path, &known)?;
        }
    }
    if let (Some(fleet), Some(own)) = (&snapshot.fleet_id, &snapshot.own_member_key) {
        last = reconcile::reconcile(
            &fleet_directory(state, fleet),
            socket,
            fleet,
            own,
            &snapshot.membership,
            &retained(&snapshot, now_ms()),
        )
        .await?;
        if last.state == "applied"
            && snapshot
                .members
                .iter()
                .any(|member| member.state == "unconfirmed")
        {
            last.state = "leave_unconfirmed".into();
            last.message = Some("Leave confirmation is absent or its carrier grace expired; no expired leave carrier is retained".into());
        }
    }
    if !residuals.is_empty() {
        last.state = "residual_access".into();
        if let Some(message) = last.message.take() {
            residuals.push(message);
        }
        last.message = Some(residuals.join("; "));
    }
    Ok((snapshot.fleet_id, last))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let state = match args.state_dir {
        Some(path) => path,
        None => home()?.join(".local/state/st-fabric-adapter"),
    };
    if args.status {
        let status: Status = serde_json::from_slice(
            &std::fs::read(state.join("status.json")).context("no adapter status recorded")?,
        )?;
        println!("{}", serde_json::to_string(&status)?);
        return Ok(());
    }
    let fabric_socket = match args.fabric_socket {
        Some(path) => path,
        None => reconcile::control_socket()?,
    };
    if matches!(args.command, Some(PairingCommand::PairingJoin)) {
        return pairing::join(&fabric_socket, &state).await;
    }
    let st_socket = match args.st_socket {
        Some(path) => path,
        None => default_st_socket()?,
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .unix_socket(st_socket)
        .timeout(Duration::from_secs(10))
        .build()?;
    if matches!(args.command, Some(PairingCommand::PairingInvite)) {
        return pairing::invite(&client, &fabric_socket, &state).await;
    }
    std::fs::create_dir_all(&state)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("adapter.lock"))?;
    lock.try_lock()
        .context("another adapter owns this state directory")?;
    loop {
        let result = poll(&client, &fabric_socket, &state).await;
        let status = match &result {
            Ok((fleet_id, policy)) => Status {
                observed_unix_ms: now_ms(),
                fleet_id: fleet_id.clone(),
                policy: policy.clone(),
            },
            Err(error) => Status {
                observed_unix_ms: now_ms(),
                fleet_id: None,
                policy: reconcile::error_status(error),
            },
        };
        save(&state.join("status.json"), &status)?;
        println!("{}", serde_json::to_string(&status)?);
        if args.once {
            return result.map(|_| ());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(args.interval)) => {},
            result = tokio::signal::ctrl_c() => { result?; return Ok(()); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn carrier_retention_is_bounded_and_confirmation_specific() {
        let mut snapshot = LeaveSnapshot {
            fleet_id: None,
            own_member_key: None,
            membership: FleetView::default(),
            persistence_error: None,
            members: vec![
                MemberLeave {
                    member_key: "waiting".into(),
                    state: "awaiting-confirmation".into(),
                    grace_deadline_unix_ms: Some(101),
                },
                MemberLeave {
                    member_key: "expired".into(),
                    state: "awaiting-confirmation".into(),
                    grace_deadline_unix_ms: Some(100),
                },
                MemberLeave {
                    member_key: "confirmed".into(),
                    state: "confirmed".into(),
                    grace_deadline_unix_ms: Some(101),
                },
            ],
        };
        assert_eq!(retained(&snapshot, 100), BTreeSet::from(["waiting".into()]));
        snapshot.own_member_key = Some("expired".into());
        assert_eq!(
            retained(&snapshot, 101),
            BTreeSet::from(["expired".into()])
        );
    }
    #[test]
    fn fleet_directory_cannot_escape_state_root() {
        assert_eq!(
            fleet_directory(Path::new("/adapter"), "../../st").parent(),
            Some(Path::new("/adapter"))
        );
    }

    async fn local_api(
        path: &Path,
        status: &str,
        body: serde_json::Value,
    ) -> tokio::task::JoinHandle<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::UnixListener::bind(path).unwrap();
        let body = serde_json::to_vec(&body).unwrap();
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let n = stream.read(&mut request).await.unwrap();
            assert!(
                String::from_utf8_lossy(&request[..n]).contains("/v1/internal/fleet/leave-state")
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        })
    }

    #[tokio::test]
    async fn failed_snapshot_never_withdraws_known_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("st.sock");
        let known = BTreeSet::from(["fleet-before".to_owned()]);
        save(&tmp.path().join("owners.json"), &known).unwrap();
        let server = local_api(
            &socket,
            "503 Service Unavailable",
            serde_json::json!({"error":"worker unavailable"}),
        )
        .await;
        let client = reqwest::Client::builder()
            .unix_socket(socket)
            .build()
            .unwrap();
        let error = poll(&client, &tmp.path().join("absent-fabric.sock"), tmp.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("503"));
        assert_eq!(owners(&tmp.path().join("owners.json")).unwrap(), known);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_only_snapshot_is_idle_without_contacting_fabric() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("st.sock");
        let server = local_api(
            &socket,
            "200 OK",
            serde_json::json!({
                "fleet_id":null, "own_member_key":null, "membership":FleetView::default(),
                "members":[], "persistence_error":null
            }),
        )
        .await;
        let client = reqwest::Client::builder()
            .unix_socket(socket)
            .build()
            .unwrap();
        let (fleet, status) = poll(&client, &tmp.path().join("absent-fabric.sock"), tmp.path())
            .await
            .unwrap();
        assert!(fleet.is_none());
        assert_eq!(status.state, "idle");
        server.await.unwrap();
    }
}
