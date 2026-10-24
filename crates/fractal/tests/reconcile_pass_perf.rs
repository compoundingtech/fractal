//! Time quiet reconcile passes against a copy of a member's store.
//!
//! The reconciler runs a pass whenever the graph changes, on every member, so a pass's cost is
//! paid for every write in the fleet. This runs passes until they stop writing, then times quiet
//! ones and lists the statements one pass runs, most expensive first.
//!
//! The passes write to the copy and render into the workspaces it names, so run it on a copy
//! made for it, in a sandbox whose only writable directories are the copy's and a private `/tmp`:
//!
//! `ST3_PERF_NODE` is the member's node name. `ST3_PERF_RUNTIMES` names a JSON list of the member's
//! running runtimes (`runtime_id`, `incarnation_id`, `terminal`), taken from its
//! `/v1/client/runtimes`. `ST3_PERF_INCREMENTAL=1` times passes that skip mission runs whose
//! inputs did not change, as a daemon's do between its full passes.
//!
//! ```sh
//! bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --bind COPY_DIR COPY_DIR \
//!   --unshare-pid --unshare-net --die-with-parent env ST3_PERF_STORE=COPY_DIR/claims.sqlite3 ST3_PERF_NODE=NODE \
//!   INTEGRATION_TEST_BINARY reconcile_pass_perf:: --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::Result;
use fractal_daemon::reconcile::{Reconciler, RuntimeControl, RuntimeObservation};
use fractal_daemon::RuntimeStartsStore;
use fractal_daemon::model::{ClaimInput, MemberSpec};
use fractal_daemon::store::{RuntimeStartAdmission, RuntimeStartOperation, Store};
use serde_json::json;
use tokio::sync::Notify;

/// A runtime in which every member the reconciler starts keeps running.
#[derive(Default)]
struct Started {
    running: Mutex<BTreeMap<String, RuntimeObservation>>,
}

impl RuntimeControl for Started {
    fn snapshot_ptys(&self) -> Result<Vec<RuntimeObservation>> {
        Ok(self
            .running
            .lock()
            .unwrap()
            .values()
            .map(|observation| RuntimeObservation {
                runtime_id: observation.runtime_id.clone(),
                terminal: observation.terminal,
                status: observation.status.clone(),
                exit_code: None,
                incarnation_id: observation.incarnation_id.clone(),
            })
            .collect())
    }
    fn observe_exec(&self, _: &str) -> Result<Option<RuntimeObservation>> {
        Ok(None)
    }
    fn start(&self, member: &fractal_daemon::model::MemberSpec) -> Result<()> {
        let mut running = self.running.lock().unwrap();
        let incarnation = format!("{}:2026-10-01T00:00:00.000Z", 1000 + running.len());
        running.insert(
            member.runtime_id.clone(),
            RuntimeObservation {
                runtime_id: member.runtime_id.clone(),
                terminal: true,
                status: "running".into(),
                exit_code: None,
                incarnation_id: Some(incarnation),
            },
        );
        Ok(())
    }
    fn stop(&self, runtime_id: &str, _: bool, _: Option<&str>) -> Result<()> {
        self.running.lock().unwrap().remove(runtime_id);
        Ok(())
    }
    fn kill(&self, runtime_id: &str, _: bool, _: Option<&str>) -> Result<()> {
        self.running.lock().unwrap().remove(runtime_id);
        Ok(())
    }
    fn remove(&self, runtime_id: &str, _: bool) -> Result<()> {
        self.running.lock().unwrap().remove(runtime_id);
        Ok(())
    }
    fn screen(&self, _: &str) -> Result<String> {
        Ok(String::new())
    }
    fn send_key(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn read_exec_log(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn thread_cpu_ms() -> f64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time points to a valid timespec; the clock reads this thread only.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) };
    time.tv_sec as f64 * 1000.0 + time.tv_nsec as f64 / 1_000_000.0
}

/// Native PTY rows may be ended even though native work-scope retirement is still required.
#[derive(Default)]
struct RevokedTerminalRuntime {
    observations: Vec<RuntimeObservation>,
    operations: BTreeMap<String, (String, String)>,
    provenance_matches: bool,
    snapshots: AtomicUsize,
    provenance_reads: AtomicUsize,
    closure_reads: AtomicUsize,
    retirements: AtomicUsize,
    signals: AtomicUsize,
}

impl RuntimeControl for RevokedTerminalRuntime {
    fn snapshot_ptys(&self) -> Result<Vec<RuntimeObservation>> {
        self.snapshots.fetch_add(1, Ordering::Relaxed);
        Ok(self.observations.clone())
    }
    fn observe_exec(&self, _: &str) -> Result<Option<RuntimeObservation>> {
        Ok(None)
    }
    fn start(&self, _: &MemberSpec) -> Result<()> {
        anyhow::bail!("persisted revoked starts must not launch")
    }
    fn start_operation(&self, id: &str, terminal: bool, incarnation: &str) -> Result<Option<String>> {
        self.provenance_reads.fetch_add(1, Ordering::Relaxed);
        Ok(self.operations.get(id).and_then(|(expected, operation)| {
            (terminal && expected == incarnation).then(|| operation.clone())
        }))
    }
    fn start_operation_closed(&self, _: &MemberSpec) -> Result<bool> {
        self.closure_reads.fetch_add(1, Ordering::Relaxed);
        // NativeRuntime cannot certify producer closure just from an ended PTY row.
        Ok(false)
    }
    fn stop_with_retirement_proof(
        &self,
        id: &str,
        terminal: bool,
        incarnation: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            self.provenance_matches && terminal
                && self.operations.get(id).is_some_and(|(expected, _)| expected == incarnation),
            "retirement must target the exact operation-owned terminal incarnation"
        );
        self.retirements.fetch_add(1, Ordering::Relaxed);
        Ok(false)
    }
    fn stop(&self, _: &str, _: bool, _: Option<&str>) -> Result<()> {
        self.signals.fetch_add(1, Ordering::Relaxed);
        anyhow::ensure!(self.provenance_matches, "must not stop a different operation");
        // Successful signal submission cannot establish native retirement.
        Ok(())
    }
    fn kill(&self, _: &str, _: bool, _: Option<&str>) -> Result<()> {
        anyhow::bail!("the previous daemon already recorded its kill")
    }
    fn remove(&self, _: &str, _: bool) -> Result<()> {
        Ok(())
    }
    fn screen(&self, _: &str) -> Result<String> {
        Ok(String::new())
    }
    fn send_key(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn read_exec_log(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn seed_revoked_terminal_starts(store: &Store, count: usize) -> RevokedTerminalRuntime {
    let mut runtime = RevokedTerminalRuntime::default();
    for index in 0..count {
        let subject = format!("agent/example-revoked-{index}");
        let runtime_id = format!("example-revoked-{index}");
        let incarnation = format!("{}:2026-10-01T00:00:00.000Z", 1000 + index);
        let operation_id = format!("example-revoked-operation-{index}");
        let member = json!({
            "kind": "agent", "host": "test", "runtime_id": runtime_id,
            "workspace": "/tmp", "cwd": "/tmp", "terminal": true,
            "launch": {"type": "shell", "value": "true"},
            "environment": {}, "tags": {}, "display_name": null,
            "lifecycle": "service", "restart": "never",
            "restart_intensity": {"attempts": 3, "interval_ms": 60000, "delay_ms": 0, "mode": "delay"},
            "shutdown_timeout_ms": 0, "driver": null
        });
        // A retired declaration has no selected desired row. Admission supplies the genuine
        // retained start request and receipt; no private SQL or live worker is needed.
        let mut operation = RuntimeStartOperation {
            id: operation_id.clone(),
            subject: subject.clone(),
            owner: "previous-daemon".into(),
            body: json!({
                "subject": {"subject": subject, "kind": "agent", "desired": null, "member": member},
                "member": member, "launch_member": member,
                "reason": "persisted revoked start", "request": null,
                "desired_token": "previous-declaration", "selected_token": null,
                "placement_evidence": [], "start_request": "",
                "expected_incarnation": null, "resume_operation": null
            }),
            revoked_by: None,
            finished: false,
            result: None,
            applied: false,
            cleanup_verified: false,
            dispatched: false,
            receipted: false,
        };
        assert_eq!(
            store.admit_runtime_start(&mut operation, count).unwrap(),
            RuntimeStartAdmission::Admitted
        );
        assert!(store.dispatch_runtime_start(&operation_id, count).unwrap());
        let append = |kind: &str, fields, evidence, key| {
            store.append_claim(&ClaimInput {
                subject: subject.clone(),
                kind: kind.into(),
                actor: None,
                fields,
                evidence,
                expected_subject: None,
                idempotency_key: Some(key),
            }).unwrap()
        };
        let request = append(
            "runtime.action.requested",
            serde_json::from_value(json!({
                "action": "terminate", "runtime_id": runtime_id, "terminal": true,
                "incarnation_id": incarnation, "deadline_unix_ms": "0"
            })).unwrap(),
            Vec::new(),
            format!("terminate:{subject}:{incarnation}"),
        );
        let deadline_key = format!("stop-deadline:{subject}:{incarnation}");
        let deadline = append(
            "runtime.action.deadline-reached",
            serde_json::from_value(json!({"action": "terminate", "incarnation_id": incarnation})).unwrap(),
            vec![request.id.clone()],
            deadline_key.clone(),
        );
        append(
            "runtime.action.succeeded",
            serde_json::from_value(json!({
                "action": "kill", "incarnation_id": incarnation, "deadline_key": deadline_key
            })).unwrap(),
            vec![deadline.id],
            format!("kill:{subject}:{incarnation}"),
        );
        store.revoke_runtime_start(&operation_id, &request.id).unwrap();
        store.finish_runtime_start(
            &operation_id,
            &json!({"error": "launch owner ended before its result"}),
        ).unwrap();
        store.runtime_start_applied(&operation_id).unwrap();
        runtime.operations.insert(runtime_id.clone(), (incarnation.clone(), operation_id));
        runtime.observations.push(RuntimeObservation {
            runtime_id,
            terminal: true,
            status: if index % 2 == 0 { "exited" } else { "vanished" }.into(),
            exit_code: Some(0),
            incarnation_id: Some(incarnation),
        });
    }
    runtime
}

#[test]
fn revoked_terminal_starts_preserve_safety_with_one_registry_snapshot_per_pass() {
    let mut results = Vec::new();
    for (count, provenance_matches) in [(7, false), (50, false), (7, true), (50, true)] {
        let scenario = if provenance_matches { "matched" } else { "mismatched" };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let store = Store::open(&path, "test").unwrap();
        let mut runtime = seed_revoked_terminal_starts(&store, count);
        runtime.provenance_matches = provenance_matches;
        if !provenance_matches {
            for (_, operation) in runtime.operations.values_mut() {
                *operation = "example-different-operation".into();
            }
        }
        let runtime = Arc::new(runtime);
        drop(store);
        // Reopen so this exercises the durable shape inherited from a previous daemon.
        let store = Arc::new(Store::open(&path, "test").unwrap());
        let seeded = store.pending_runtime_starts().unwrap();
        assert_eq!(seeded.len(), count);
        assert!(seeded.iter().all(|operation| operation.finished
            && operation.applied && operation.dispatched
            && operation.revoked_by.is_some() && !operation.cleanup_verified));
        let reconciler = Reconciler::new(
            store.clone(), runtime.clone(), "test".into(), Arc::new(Notify::new()),
        );
        let mut passes = Vec::new();
        for pass in 1..=3 {
            let snapshots = runtime.snapshots.load(Ordering::Relaxed);
            let provenance = runtime.provenance_reads.load(Ordering::Relaxed);
            let closure = runtime.closure_reads.load(Ordering::Relaxed);
            let retirements = runtime.retirements.load(Ordering::Relaxed);
            let cpu_started = thread_cpu_ms();
            let result = reconciler.reconcile_once();
            let cpu_ms = thread_cpu_ms() - cpu_started;
            let snapshots = runtime.snapshots.load(Ordering::Relaxed) - snapshots;
            let provenance = runtime.provenance_reads.load(Ordering::Relaxed) - provenance;
            let closure = runtime.closure_reads.load(Ordering::Relaxed) - closure;
            let retirements = runtime.retirements.load(Ordering::Relaxed) - retirements;
            let pending = store.pending_runtime_starts().unwrap().len();
            println!(
                "revoked starts scenario={scenario} N={count} pass={pass}: pending={pending}, snapshots={snapshots}, provenance={provenance}, closure={closure}, retirements={retirements}, thread CPU={cpu_ms:.3} ms, result={result:?}"
            );
            passes.push((snapshots, pending, result));
        }
        results.push((
            count, scenario, provenance_matches, passes,
            runtime.retirements.load(Ordering::Relaxed),
            runtime.signals.load(Ordering::Relaxed),
        ));
    }
    // Report every size, scenario and pass before asserting, including on the baseline.
    for (count, scenario, provenance_matches, passes, retirements, signals) in results {
        for (snapshots, pending, result) in &passes {
            assert!(result.is_ok(), "{scenario} N={count}: {result:?}");
            assert!(
                *snapshots <= 1,
                "{scenario} N={count}: {snapshots} registry snapshots in one pass"
            );
            assert_eq!(
                *pending, count,
                "{scenario} N={count}: ended rows and accepted signals are not closure proof"
            );
        }
        if !provenance_matches {
            assert_eq!(retirements, 0, "N={count}: must not retire a different operation");
            assert_eq!(signals, 0, "N={count}: must not signal a different operation");
        }
    }
}

#[test]
#[ignore = "needs ST3_PERF_STORE pointing at a copy of a member's store, inside a sandbox"]
fn quiet_reconcile_passes_on_a_store_copy() {
    // With ST3_PROFILE_DIR set, the file profiler writes each reconcile section's time to
    // totals.json there.
    fractal_daemon::profile::init_from_env();
    let path = PathBuf::from(std::env::var("ST3_PERF_STORE").expect("set ST3_PERF_STORE"));
    let node = std::env::var("ST3_PERF_NODE").expect("set ST3_PERF_NODE to the member's node name");
    let store = Arc::new(fractal_daemon::store::Store::open(&path, node.clone()).unwrap());
    // The member's running runtimes, as `/v1/client/runtimes` lists them, so passes see the same
    // incarnations its seats report harness state for.
    let runtime = Started::default();
    if let Ok(path) = std::env::var("ST3_PERF_RUNTIMES") {
        let listed: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut running = runtime.running.lock().unwrap();
        for item in listed {
            let runtime_id = item["runtime_id"].as_str().unwrap().to_owned();
            running.insert(
                runtime_id.clone(),
                RuntimeObservation {
                    runtime_id,
                    terminal: item["terminal"].as_bool().unwrap_or(true),
                    status: "running".into(),
                    exit_code: None,
                    incarnation_id: item["incarnation_id"].as_str().map(str::to_owned),
                },
            );
        }
    }
    let reconciler = Reconciler::new(
        store.clone(),
        Arc::new(runtime),
        node,
        Arc::new(Notify::new()),
    )
    .skipping_unneeded(std::env::var("ST3_PERF_INCREMENTAL").as_deref() == Ok("1"));
    // Settle: start what is declared and record what changed, until a pass writes nothing.
    for settle in 0..30 {
        let before = store.index().unwrap();
        let started = Instant::now();
        let _ = reconciler.reconcile_once();
        let wrote = store.index().unwrap() - before;
        println!(
            "settling pass {settle}: {:.0} ms, {wrote} claims",
            started.elapsed().as_secs_f64() * 1000.0
        );
        if wrote == 0 {
            break;
        }
    }
    let queries = |report: &serde_json::Value| {
        report["queries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["kind"].as_str().unwrap().to_owned(),
                    (
                        row["count"].as_u64().unwrap(),
                        row["total_ms"].as_f64().unwrap(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let before = queries(&fractal_daemon::performance::snapshot());
    let passes = 10;
    let mut wall = Vec::new();
    let mut cpu = Vec::new();
    for _ in 0..passes {
        let started = Instant::now();
        let cpu_started = thread_cpu_ms();
        fractal_daemon::profile::task("task reconcile-pass", || reconciler.reconcile_once()).unwrap();
        cpu.push(thread_cpu_ms() - cpu_started);
        wall.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let after = queries(&fractal_daemon::performance::snapshot());
    // The loop reads the next deadline after every pass.
    let mut deadline_cpu = Vec::new();
    for _ in 0..passes {
        let cpu_started = thread_cpu_ms();
        fractal_daemon::profile::task("task reconcile-deadline", || reconciler.next_deadline());
        deadline_cpu.push(thread_cpu_ms() - cpu_started);
    }
    println!(
        "quiet pass: wall {:.0} ms, CPU on the pass thread {:.0} ms (mean of {passes})",
        wall.iter().sum::<f64>() / passes as f64,
        cpu.iter().sum::<f64>() / passes as f64
    );
    println!(
        "next deadline after a pass: CPU {:.0} ms (mean of {passes})",
        deadline_cpu.iter().sum::<f64>() / passes as f64
    );
    let mut rows = after
        .iter()
        .map(|(kind, (count, total))| {
            let (count_before, total_before) = before.get(kind).copied().unwrap_or_default();
            (
                (total - total_before) / passes as f64,
                (count - count_before) as f64 / passes as f64,
                kind,
            )
        })
        .filter(|(total, _, _)| *total > 0.0)
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("statements in one pass, by total time (top 20 of the report):");
    for (total, count, kind) in rows {
        println!(
            "  {total:8.1} ms  {count:7.1}x  {}",
            kind.chars().take(160).collect::<String>()
        );
    }
}
