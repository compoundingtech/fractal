//! Durable admission for this machine's GitHub observer requests.
//!
//! This is a local request ceiling, not a reservation of a shared token's quota.
//! No other machine is consulted. Failed sends and conditional 304s retain their
//! reservation. GraphQL requests are counted separately from core requests; their
//! provider point cost is not inferred from request count.
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

const HOUR: u128 = 3_600_000;
const CAP: usize = 1000;
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ROWS: usize = 32_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    at: u128,
    caller: String,
    resource: String,
    observer: bool,
}

#[derive(Default)]
struct State {
    path: Option<std::path::PathBuf>,
    rows: Vec<Reservation>,
    refused: BTreeMap<(String, String), u64>,
    gap: Option<String>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// Load durable counters at daemon configuration, so doctor does not wait for a
/// new request to show prior-hour reservations. A corrupt ledger stays a visible
/// gap and refuses observer sends; startup does not erase it or stop merge paths.
pub(crate) fn configure(path: std::path::PathBuf) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or(0);
    if let Ok(mut held) = state().lock() {
        match read(&path, now) {
            Ok(rows) => {
                held.rows = rows;
                held.path = Some(path);
                held.gap = None;
            }
            Err(error) => {
                held.path = None;
                held.gap = Some(error.to_string());
            }
        }
    }
}

fn read(path: &Path, now: u128) -> Result<Vec<Reservation>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("open GitHub admission ledger"),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BYTES,
        "invalid GitHub admission ledger file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "GitHub admission ledger exceeds byte bound"
    );
    let rows: Vec<Reservation> = serde_json::from_slice(&bytes)
        .context("decode GitHub admission ledger; no empty-ledger recovery was used")?;
    ensure!(
        rows.len() <= MAX_ROWS,
        "GitHub admission ledger exceeds row bound"
    );
    ensure!(
        rows.iter().all(|r| r.at <= now
            && !r.caller.is_empty()
            && r.caller.len() <= 512
            && matches!(r.resource.as_str(), "core" | "graphql" | "search")),
        "invalid GitHub admission reservation"
    );
    Ok(rows
        .into_iter()
        .filter(|r| now.saturating_sub(r.at) < HOUR)
        .collect())
}

fn publish(path: &Path, rows: &[Reservation]) -> Result<()> {
    ensure!(
        rows.len() <= MAX_ROWS,
        "GitHub admission ledger exceeds row bound"
    );
    let bytes = serde_json::to_vec(rows)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "GitHub admission ledger exceeds byte bound"
    );
    let parent = path
        .parent()
        .context("GitHub admission ledger has no parent")?;
    // State is already the daemon's private configured directory. A unique,
    // no-replace temporary file prevents concurrent writers from sharing bytes.
    let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".github-admission-{}-{sequence}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    // Only our unique temporary name is removed. A rename already committed the
    // reservation even if the parent fsync failed; in that case no send follows.
    if temporary.exists() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn admit(
    rows: &mut Vec<Reservation>,
    caller: &str,
    resource: &str,
    observer: bool,
    now: u128,
) -> Result<()> {
    ensure!(
        !caller.is_empty() && caller.len() <= 512,
        "invalid GitHub admission caller"
    );
    rows.retain(|r| now.saturating_sub(r.at) < HOUR);
    let relevant = rows
        .iter()
        .filter(|r| r.observer && r.resource == resource)
        .collect::<Vec<_>>();
    if observer
        && (relevant.len() >= CAP || relevant.iter().filter(|r| r.caller == caller).count() >= CAP)
    {
        let retry_at = relevant
            .iter()
            .map(|r| r.at)
            .min()
            .unwrap_or(now)
            .saturating_add(HOUR);
        anyhow::bail!(crate::resource::ProviderRateLimit {
            retry_at_unix_ms: retry_at,
            status: 429
        });
    }
    rows.push(Reservation {
        at: now,
        caller: caller.into(),
        resource: resource.into(),
        observer,
    });
    Ok(())
}

/// Called before BOTH local reqwest and authorized RPC sends. A task-local
/// observer subject is supplied by the reconciler, never from an HTTP header or
/// the shell's ST_AGENT. Unscoped calls are visible but not capped here: comments
/// and live merge gates need their own authenticated caller enrollment before
/// their write path may be throttled. They must not inherit an observer identity.
pub(crate) async fn before_send(
    path: std::path::PathBuf,
    caller: Option<String>,
    resource: String,
) -> Result<()> {
    let observer = caller.is_some();
    let caller = caller.unwrap_or_else(|| "unattributed".into());
    let result = tokio::task::spawn_blocking(move || {
        let mut held = state()
            .lock()
            .map_err(|_| anyhow::anyhow!("GitHub admission mutex poisoned"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("GitHub admission clock precedes epoch")?
            .as_millis();
        let result: Result<()> = (|| {
            if held.path.as_ref() != Some(&path) {
                held.rows = read(&path, now)?;
                held.path = Some(path.clone());
            }
            let mut rows = held.rows.clone();
            admit(&mut rows, &caller, &resource, observer, now)?;
            publish(&path, &rows)?;
            held.rows = rows;
            held.gap = None;
            Ok(())
        })();
        if let Err(error) = &result {
            // A rename may have committed before a directory fsync error. Force
            // the next admission to reload, so it never overwrites spent usage.
            held.path = None;
            *held
                .refused
                .entry((caller.clone(), resource.clone()))
                .or_default() += 1;
            held.gap = Some(error.to_string());
            // A merge-path request with no enrolled observer caller is never
            // refused because the observer's durable meter is broken.
            if !observer {
                tracing::warn!("GitHub unscoped request admitted with meter gap: {error}");
                return Ok(());
            }
        }
        result
    })
    .await
    .context("GitHub admission worker")
    .and_then(|result| result);
    if !observer && result.is_err() {
        tracing::warn!("GitHub unscoped request admitted with unavailable admission worker");
        return Ok(());
    }
    result
}

pub(crate) fn doctor_lines(now: u128) -> Vec<crate::model::DoctorCheck> {
    // Doctor must not wait for the journal writer's disk sync.
    let Ok(held) = state().try_lock() else {
        return vec![crate::model::DoctorCheck {
            name: "github-requests/admission".into(),
            status: "warn".into(),
            message: "local admission state unavailable".into(),
        }];
    };
    let mut counts = BTreeMap::<(String, String), usize>::new();
    for r in &held.rows {
        if now.saturating_sub(r.at) < HOUR {
            *counts
                .entry((r.caller.clone(), r.resource.clone()))
                .or_default() += 1;
        }
    }
    for key in held.refused.keys() {
        counts.entry(key.clone()).or_default();
    }
    let mut lines = counts.into_iter().map(|((caller, resource), count)| crate::model::DoctorCheck {
        name: format!("github-requests/admitted/{caller}/{resource}"),
        status: if held.gap.is_some() { "warn" } else { "pass" }.into(),
        message: format!("{count} durable request reservations in the last hour (including failed sends and 304s); {} admission refusals/meter failures since start; observer aggregate and per-caller limit {CAP}/{resource}/hour; GraphQL points unknown; unscoped callers metered but uncapped", held.refused.get(&(caller,resource)).copied().unwrap_or(0)),
    }).collect::<Vec<_>>();
    if let Some(gap) = &held.gap {
        lines.push(crate::model::DoctorCheck {
            name: "github-requests/admission".into(),
            status: "warn".into(),
            message: gap.clone(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_retains_spent_requests_and_refusal_does_not_append() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("admission.json");
        let now = 10_000_000;
        let mut rows = Vec::new();
        for _ in 0..CAP {
            admit(&mut rows, "observer/github/repo", "core", true, now).unwrap();
        }
        publish(&path, &rows).unwrap();
        let mut restarted = read(&path, now).unwrap();
        assert!(admit(&mut restarted, "observer/github/repo", "core", true, now).is_err());
        assert_eq!(restarted.len(), CAP);
        // A different caller cannot acquire a second aggregate1000 allocation.
        assert!(admit(&mut restarted, "observer/github/another", "core", true, now).is_err());
        admit(&mut restarted, "observer/github/repo", "graphql", true, now).unwrap();
        admit(&mut restarted, "unattributed", "core", false, now).unwrap();
        admit(
            &mut restarted,
            "observer/github/repo",
            "core",
            true,
            now + HOUR,
        )
        .unwrap();
        assert_eq!(restarted.len(), 1);
    }

    #[tokio::test]
    async fn exhausted_observer_reservation_returns_existing_rate_limit_without_spending() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("admission.json");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let rows = (0..CAP)
            .map(|_| Reservation {
                at: now,
                caller: "observer/github/repo".into(),
                resource: "core".into(),
                observer: true,
            })
            .collect::<Vec<_>>();
        publish(&path, &rows).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = before_send(
            path.clone(),
            Some("observer/github/repo".into()),
            "core".into(),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::resource::ProviderRateLimit>()
                .is_some()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[tokio::test]
    async fn failed_send_budget_remains_on_disk_without_a_response() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("admission.json");
        before_send(
            path.clone(),
            Some("observer/github/repo".into()),
            "graphql".into(),
        )
        .await
        .unwrap();
        // No response-recording call occurs. Reopening still sees the admitted request.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let rows = read(&path, now).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].resource, "graphql");
    }

    #[test]
    fn corrupt_or_future_ledger_never_resets_to_zero() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("admission.json");
        std::fs::write(&path, b"broken").unwrap();
        assert!(read(&path, 10).is_err());
        publish(
            &path,
            &[Reservation {
                at: 11,
                caller: "observer/test".into(),
                resource: "core".into(),
                observer: true,
            }],
        )
        .unwrap();
        assert!(read(&path, 10).is_err());
    }

    #[test]
    fn wrong_file_type_and_symlink_are_refused_without_blocking() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ledger");
        std::os::unix::fs::symlink(root.path().join("target"), &path).unwrap();
        assert!(read(&path, 10).is_err());
    }
}
