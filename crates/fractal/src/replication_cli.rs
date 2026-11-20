use super::*;
// The same print macros as main.rs: CLI output goes through fractal's writer.
use fractal_daemon::{cli_print as print, cli_println as println};

#[derive(Args)]
pub(super) struct ArchiveCandidatesArgs {
    /// Existing offline SQLite copy; never discovered, copied from a host, or modified.
    #[arg(long)]
    source_copy: PathBuf,
    /// New private artifact directory; existing, overlapping, and symlink paths are refused.
    #[arg(long)]
    output: PathBuf,
    /// Explicit Unix time in milliseconds used for both retention windows; no clock default.
    #[arg(long)]
    as_of_unix_ms: u128,
    /// Acknowledge that the source is an offline copy and outputs are analysis-only, not daemon stores.
    #[arg(long, required = true)]
    offline_copy: bool,
}

pub(super) async fn run_archive_candidates(args: ArchiveCandidatesArgs) -> Result<()> {
    anyhow::ensure!(args.offline_copy, "--offline-copy acknowledgement is required");
    crate::stderr_message(
        "st",
        format_args!(
            "analysis only: constructing both 30-day and 14-day candidates in {}; no activation. \
             Never use these candidate stores with a daemon or deployment. Full JSON report follows on stdout.",
            args.output.display()
        ),
    );
    let report = tokio::task::spawn_blocking(move || {
        fractal_daemon::task_cpu::blocking("replication.archive_candidates", || {
            fractal_store::wire_archive_candidates::construct(
                &args.source_copy,
                &args.output,
                args.as_of_unix_ms,
            )
        })
    })
    .await
    .context("offline archive candidate construction worker stopped")??;
    // Always emit the complete auditable report, including in native (non --json) mode.
    print_value(&report, true)
}

pub(super) async fn run_replication(
    client: &Client,
    config: &Config,
    command: ReplicationCommand,
    json_output: bool,
) -> Result<()> {
    match command {
        ReplicationCommand::ArchiveCandidates(_) => {
            unreachable!("archive candidates dispatches before configuration and daemon discovery")
        }
        ReplicationCommand::Replay { confirm } => {
            let mut client = client.clone();
            // Match planned checkpoint maintenance, rather than the interactive 15-second limit.
            client.deadlines.request = Duration::from_secs(30 * 60);
            if confirm {
                let estimate: Value = client.post("/v1/replication/replay", &json!({"confirm":false})).await?;
                // Print before the destructive request, including when stdout is JSON.
                eprintln!("{}", estimate["warning"].as_str().unwrap_or_default());
            }
            let report: Value = client.post("/v1/replication/replay", &json!({"confirm":confirm})).await?;
            if json_output {
                return print_value(&report, true);
            }
            if confirm {
                println!("replay\t{}\tduration-ms={}", report["result"], report["duration_ms"]);
            } else {
                println!("claims\t{}", report["claim_count"]);
                println!("last-replay-duration-ms\t{}", report["last_replay_duration_ms"].as_u64().map_or_else(|| "unknown".into(), |ms| ms.to_string()));
                println!("{}", report["warning"].as_str().unwrap_or_default());
                println!("Estimate only. Run fractal replication replay --confirm to replay.");
            }
            Ok(())
        }
        ReplicationCommand::Status => {
            let status: ReplicationStatus = client.get("/v1/replication/status").await?;
            if json_output {
                return print_value(&status, true);
            }
            println!(
                "fleet\t{}",
                status.fleet_id.as_deref().unwrap_or("local-only")
            );
            if let Some(removed) = &status.removed {
                println!("removed\t{}", removed.describe(status.fleet_id.as_deref()));
            }
            println!("authority-digest\t{}", status.authority_digest);
            println!("graph-digest\t{}", status.graph_digest);
            for (table, digest) in &status.projection_digests {
                println!("table-digest\t{table}\t{digest}");
            }
            println!("envelopes\t{}", status.received_envelopes);
            // Records count only admitted envelopes; held ones show only here.
            println!(
                "envelopes-waiting\tpending={} unsigned={} fenced={}",
                status.pending_envelopes, status.unsigned_envelopes, status.fenced_envelopes
            );
            println!(
                "records\tvalid={} pending={} waiting={} invalid={} repaired={} checkpointed={}",
                status.valid_records,
                status.pending_records,
                status.waiting_claims,
                status.invalid_records,
                status.repaired_records,
                status.checkpointed_envelopes
            );
            println!("unhealthy-projections\t{}", status.unhealthy_projections);
            if let Some(first) = &status.first_sync {
                println!("first-sync\t{}", render_first_sync(first, now_ms()));
            }
            for projection in &status.unhealthy {
                println!(
                    "unhealthy\t{}\t{}\t{}",
                    projection.aggregate,
                    projection.error_code.as_deref().unwrap_or(""),
                    projection.error_message.as_deref().unwrap_or("")
                );
            }
            let timings = &status.timings;
            println!(
                "timings\t{} exchanges, {} envelopes received; ms: round-trip={} export={} snapshot={} receipt={} admission={} (verify={}) projection={} repair={} signing={} sqlite={} ({} commits, {} ms)",
                timings.exchanges,
                timings.envelopes_received,
                timings.round_trip_ms,
                timings.export_ms,
                timings.snapshot_ms,
                timings.receipt_ms,
                timings.admission_ms,
                timings.verify_ms,
                timings.projection_ms,
                timings.repair_ms,
                timings.signing_ms,
                timings.sqlite_ms,
                timings.commits,
                timings.commit_ms
            );
            if let Some(fallbacks) = &status.stream_fallbacks {
                println!("stream-fallbacks\t{}", render_stream_fallbacks(fallbacks));
            }
            print!(
                "{}",
                render_replication_peers(&status.peers, &status.graph_digest, now_ms())
            );
            Ok(())
        }
        ReplicationCommand::Invalid { all } => {
            let records: Vec<ReplicaRecordView> = client
                .get(&format!(
                    "/v1/replication/records?unresolved={}",
                    if all { "false" } else { "true" }
                ))
                .await?;
            if json_output {
                return print_value(&records, true);
            }
            if records.is_empty() {
                println!(
                    "No {} replication records.",
                    if all { "stored" } else { "unresolved" }
                );
                return Ok(());
            }
            for record in records {
                println!(
                    "{}\t{}\t{}:{}\t{}\t{}",
                    record.state,
                    record.record_ref,
                    record.writer,
                    record.sequence,
                    record.subject.as_deref().unwrap_or("unknown-subject"),
                    record.error_message.as_deref().unwrap_or("")
                );
            }
            Ok(())
        }
        ReplicationCommand::Inspect { record } => {
            let record: ReplicaRecordView = client
                .get(&format!(
                    "/v1/replication/records/{}",
                    urlencoding::encode(record.strip_prefix("record/").unwrap_or(&record))
                ))
                .await?;
            if json_output {
                return print_value(&record, true);
            }
            println!("record\t{}", record.record_ref);
            println!("state\t{}", record.state);
            println!("writer\t{}", record.writer);
            println!("sequence\t{}", record.sequence);
            println!("envelope\t{}", record.envelope_hash);
            println!("position\t{}", record.position);
            println!("claim\t{}", record.claim_id.as_deref().unwrap_or(""));
            println!("subject\t{}", record.subject.as_deref().unwrap_or(""));
            println!("kind\t{}", record.kind.as_deref().unwrap_or(""));
            println!("error-code\t{}", record.error_code.as_deref().unwrap_or(""));
            println!("error\t{}", record.error_message.as_deref().unwrap_or(""));
            println!(
                "replacement\t{}",
                record.replacement_claim_id.as_deref().unwrap_or("")
            );
            Ok(())
        }
        ReplicationCommand::Diff { peer } => {
            let status: ReplicationStatus = client.get("/v1/replication/status").await?;
            let remote = status
                .peers
                .iter()
                .find(|item| item.peer == peer)
                .with_context(|| format!("peer `{peer}` is not configured"))?;
            let value = json!({
                "peer": peer,
                "status": remote.status,
                "authority": {
                    "local": status.authority_digest,
                    "remote": remote.authority_digest,
                    "equal": remote.authority_digest.as_deref() == Some(status.authority_digest.as_str()),
                },
                "differing_tables": remote.differing_tables,
                "graph": {
                    "local": status.graph_digest,
                    "remote": remote.graph_digest,
                    "waiting_reason": remote.projection_comparison_waiting_reason,
                    "equal": if remote.projection_digests.is_empty() || remote.projection_comparison_waiting { None } else { Some(peer_graphs_equal(remote, &status.graph_digest)) },
                    "compared_at_unix_ms": remote.projection_compared_at_unix_ms,
                    "coverage": if remote.projection_digests.is_empty() { "legacy-only" } else if remote.projection_comparison_waiting { "pending" } else { "all-shared-projections" },
                },
            });
            if json_output {
                return print_value(&value, true);
            }
            println!("peer\t{}\t{}", peer, remote.status);
            if !remote.differing_tables.is_empty() {
                println!("different-tables\t{}", remote.differing_tables.join(", "));
            }
            println!(
                "authority\t{}\t{}\t{}",
                if remote.authority_digest.as_deref() == Some(status.authority_digest.as_str()) {
                    "equal"
                } else {
                    "different"
                },
                status.authority_digest,
                remote.authority_digest.as_deref().unwrap_or("unknown")
            );
            println!(
                "graph\t{}\t{}\t{}",
                if remote.projection_digests.is_empty() {
                    "unverified (legacy peer)"
                } else if remote.projection_comparison_waiting {
                    "pending comparison"
                } else if peer_graphs_equal(remote, &status.graph_digest) {
                    "equal"
                } else {
                    "different"
                },
                status.graph_digest,
                remote.graph_digest.as_deref().unwrap_or("unknown")
            );
            Ok(())
        }
        ReplicationCommand::Repair {
            record,
            replacement_claim,
            reason,
            actor,
            idempotency_key,
        } => {
            let record_ref = if record.starts_with("record/") {
                record
            } else {
                format!("record/{record}")
            };
            let idempotency_key = idempotency_key.unwrap_or_else(|| {
                hex::encode(Sha256::digest(
                    format!("repair\0{record_ref}\0{replacement_claim}\0{reason}\0{actor}")
                        .as_bytes(),
                ))
            });
            let request = ReplicationRepairRequest {
                record_ref,
                replacement_claim_id: replacement_claim,
                reason,
                actor,
                idempotency_key,
            };
            let claim: ClaimRecord = client.post("/v1/replication/repair", &request).await?;
            if json_output {
                print_value(&claim, true)
            } else {
                println!("repaired\t{}", claim.id);
                Ok(())
            }
        }
        ReplicationCommand::Checkpoint {
            command: CheckpointCommand::Plan { cut },
        } => {
            let request = fractal_daemon::store::CheckpointPlanRequest { day: cut };
            let plan = checkpoint_plan_waiting_for_proof(client, &request).await?;
            if json_output {
                return print_value(&plan, true);
            }
            print!("{}", render_checkpoint_plan(&plan));
            Ok(())
        }
        ReplicationCommand::Checkpoint {
            command: CheckpointCommand::Status,
        } => {
            let status: fractal_daemon::store::CheckpointStatusView =
                client.get("/v1/checkpoint/status").await?;
            if json_output {
                return print_value(&status, true);
            }
            print!("{}", render_checkpoint_status(&status));
            Ok(())
        }
        ReplicationCommand::Checkpoint {
            command:
                CheckpointCommand::Excuse {
                    writer,
                    reason,
                    actor,
                },
        } => {
            let actor = fleet_person(actor, config)?;
            let request = fractal_daemon::store::CheckpointExcuseRequest {
                writer,
                reason,
                actor,
            };
            let claim: fractal_daemon::model::ClaimRecord =
                client.post("/v1/checkpoint/excuse", &request).await?;
            print_value(&claim, json_output)
        }
        ReplicationCommand::Checkpoint {
            command: CheckpointCommand::Resume { reason, actor },
        } => {
            let actor = fleet_person(actor, config)?;
            let request = fractal_daemon::store::CheckpointResumeRequest { reason, actor };
            let status: fractal_daemon::store::CheckpointStatusView =
                client.post("/v1/checkpoint/resume", &request).await?;
            if json_output {
                return print_value(&status, true);
            }
            print!("{}", render_checkpoint_status(&status));
            Ok(())
        }
        ReplicationCommand::Signatures {
            command: SignaturesCommand::Resume { reason, actor },
        } => {
            let actor = fleet_person(actor, config)?;
            let request = smallclaims::store::principals::SignatureResumeRequest { reason, actor };
            let resumed: Value = client.post("/v1/signatures/resume", &request).await?;
            if json_output {
                return print_value(&resumed, true);
            }
            match resumed.get("cleared").and_then(Value::as_str) {
                Some(fault) => println!("cleared signature fault: {fault}"),
                None => println!("no signature fault was persisted"),
            }
            let migration = &resumed["migration"];
            println!("migration phase {}, fault {}", migration["phase"].as_str().unwrap_or("?"), migration["fault"].as_str().unwrap_or("none"));
            Ok(())
        }
    }
}

/// How long `checkpoint plan` asks again while another checkpoint proof runs, as one does right
/// after the daemon starts. The daemon waits up to a minute itself before it refuses a plan with
/// `checkpoint-proof-in-progress`, so this adds a few minutes more.
const CHECKPOINT_PLAN_PROOF_WAIT: Duration = Duration::from_secs(4 * 60);

async fn checkpoint_plan_waiting_for_proof(
    client: &Client,
    request: &fractal_daemon::store::CheckpointPlanRequest,
) -> Result<fractal_daemon::store::CheckpointPlanView> {
    let started = std::time::Instant::now();
    let mut announced = false;
    loop {
        match client.post("/v1/checkpoint/plan", request).await {
            Err(error)
                if fractal_daemon::client::api_error_code(&error) == Some("checkpoint-proof-in-progress")
                    && started.elapsed() < CHECKPOINT_PLAN_PROOF_WAIT =>
            {
                if !announced {
                    crate::stderr_message("st", format_args!("waiting for the running checkpoint proof to finish"));
                    announced = true;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            result => return result,
        }
    }
}

fn render_checkpoint_plan(plan: &fractal_daemon::store::CheckpointPlanView) -> String {
    let mut output = String::new();
    let percent = |part: usize, whole: usize| {
        if whole == 0 {
            0.0
        } else {
            part as f64 * 100.0 / whole as f64
        }
    };
    output.push_str(&format!(
        "CHECKPOINT  {} · dry run, nothing changed\n",
        plan.checkpoint
    ));
    output.push_str(&format!(
        "before the cut  {} envelopes · {} claims\n",
        plan.sealed_envelopes, plan.sealed_claims
    ));
    output.push_str(&format!(
        "would drop      {} envelopes · {} claims ({:.1}%)\n",
        plan.dropped_envelopes,
        plan.dropped_claims,
        percent(plan.dropped_claims, plan.sealed_claims)
    ));
    for (kind, count) in &plan.by_kind {
        output.push_str(&format!(
            "  {kind}  {} of {}\n",
            count.dropped, count.sealed
        ));
    }
    let proof = &plan.proof;
    if proof.passed {
        output.push_str(&format!(
            "proof           passed · graph and {} subjects' readers unchanged\n",
            proof.subjects
        ));
    } else {
        output.push_str(&format!(
            "proof           FAILED · a checkpoint would not verify: {}\n",
            proof.mismatches.join(", ")
        ));
    }
    output.push_str(&format!("rules-digest    {}\n", plan.rules_digest));
    output.push_str(&format!("sealed-digest   {}\n", plan.sealed_digest));
    output.push_str(&format!("drop-digest     {}\n", plan.drop_digest));
    output.push_str(&format!("graph-digest    {}\n", proof.graph_digest));
    output
}

#[test]
fn replication_replay_requires_explicit_confirm_flag() {
    for confirm in [false, true] {
        let mut args = vec!["fractal", "replication", "replay"];
        if confirm { args.push("--confirm"); }
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(matches!(cli.command, Command::Replication {
            command: ReplicationCommand::Replay { confirm: parsed }
        } if parsed == confirm));
    }
}

#[test]
fn archive_candidates_requires_explicit_offline_inputs() {
    let options = [
        "--source-copy", "fixture.sqlite",
        "--output", "candidates",
        "--as-of-unix-ms", "1795046400000",
        "--offline-copy",
    ];
    for omitted in [0..2, 2..4, 4..6, 6..7] {
        let args = ["st", "replication", "archive-candidates"]
            .into_iter()
            .chain(options.iter().enumerate().filter_map(|(index, value)| {
                (!omitted.contains(&index)).then_some(*value)
            }));
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn archive_candidates_parses_explicit_options_and_json() {
    let cli = Cli::try_parse_from([
        "st", "--json", "replication", "archive-candidates",
        "--source-copy", "fixture.sqlite",
        "--output", "candidates",
        "--as-of-unix-ms", "1795046400000",
        "--offline-copy",
    ]).unwrap();
    assert!(cli.json);
    let Command::Replication {
        command: ReplicationCommand::ArchiveCandidates(args),
    } = cli.command else {
        panic!("expected archive candidates");
    };
    assert_eq!(args.source_copy, PathBuf::from("fixture.sqlite"));
    assert_eq!(args.output, PathBuf::from("candidates"));
    assert_eq!(args.as_of_unix_ms, 1_795_046_400_000);
    assert!(args.offline_copy);
}

#[test]
fn archive_candidates_rejects_invalid_timestamps() {
    for timestamp in ["not-a-time", "-1", "1.5", "340282366920938463463374607431768211456"] {
        assert!(Cli::try_parse_from([
            "st", "replication", "archive-candidates",
            "--source-copy", "fixture.sqlite",
            "--output", "candidates",
            "--as-of-unix-ms", timestamp,
            "--offline-copy",
        ]).is_err());
    }
}

#[tokio::test]
async fn archive_candidates_dispatches_without_a_daemon() {
    let fixture = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let missing_source = fixture.path().join("missing.sqlite");
    let output = fixture.path().join("candidates");
    let cli = Cli::try_parse_from([
        "st", "--endpoint", "unix:///archive-candidates-no-daemon.sock",
        "replication", "archive-candidates",
        "--source-copy", missing_source.to_str().unwrap(),
        "--output", output.to_str().unwrap(),
        "--as-of-unix-ms", "1795046400000",
        "--offline-copy",
    ]).unwrap();
    let error = run(cli).await.unwrap_err();
    assert!(!daemon_is_unreachable(&error), "{error:#}");
    assert!(!missing_source.exists(), "must not create a missing source");
    assert!(!output.exists(), "invalid source must not publish output");
}

#[tokio::test]
async fn archive_candidates_builds_both_fixture_windows_without_daemon_discovery() {
    let fixture = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let source = fixture.path().join("source.sqlite");
    let output = fixture.path().join("candidates");
    drop(Store::open(&source, "archive-cli-fixture").unwrap());
    let before = std::fs::read(&source).unwrap();
    // Even an unusable endpoint must not be parsed or contacted by this offline command.
    let cli = Cli::try_parse_from([
        "st", "--json", "--endpoint", "not-a-valid-endpoint",
        "replication", "archive-candidates",
        "--source-copy", source.to_str().unwrap(),
        "--output", output.to_str().unwrap(),
        "--as-of-unix-ms", "1795046400000",
        "--offline-copy",
    ]).unwrap();
    run(cli).await.unwrap();
    let report: Value = serde_json::from_slice(
        &std::fs::read(output.join("report.json")).unwrap(),
    ).unwrap();
    assert_eq!(report["status"], "complete");
    assert_eq!(report["artifact_kind"], "offline-wire-archive-candidates");
    assert_eq!(report["activation_allowed"], false);
    let windows: Vec<_> = report["candidates"].as_array().unwrap().iter()
        .map(|candidate| candidate["window_days"].as_u64().unwrap())
        .collect();
    assert_eq!(windows, vec![30, 14]);
    assert_eq!(std::fs::read(&source).unwrap(), before);
}

#[tokio::test]
async fn archive_candidates_dispatch_refuses_missing_acknowledgement() {
    let fixture = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let source = fixture.path().join("source.sqlite");
    let output = fixture.path().join("candidates");
    let error = run_archive_candidates(ArchiveCandidatesArgs {
        source_copy: source.clone(),
        output: output.clone(),
        as_of_unix_ms: 1_795_046_400_000,
        offline_copy: false,
    }).await.unwrap_err();
    assert!(error.to_string().contains("--offline-copy"));
    assert!(!source.exists());
    assert!(!output.exists());
}
