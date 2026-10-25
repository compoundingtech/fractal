//! Plan a checkpoint on a private, previously copied store and report what it would drop.
//! Usage: cargo run --release -p fractal --example retention_plan -- CLONE DAY [SCRATCH] [--trim]
//! DAY names the checkpoint's cut, such as 2026-10-10. With SCRATCH, the plan is also proved on a
//! copy written there, as a real checkpoint proves it. `--trim` requires SCRATCH and a passing
//! proof for this exact cut and drop plan, produced in this run. It then trims from CLONE as a
//! stable checkpoint does and reports writer holds and pages freed. Without `--trim`, even a
//! failed proof is reported without deletion. Never supply a live store or an immutable capture:
//! this opens and migrates CLONE.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use serde_json::json;
use smallclaims::store::checkpoint::{CheckpointProof, DropPlan, checkpoint_cut};
use fractal_daemon::store::Store;

// Keep the proof attached to the immutable plan actually submitted to the prover. A bare
// CheckpointProof contains no cut or drop identities, so its `passed` alone cannot authorize trim.
struct ProvedPlan<'a> {
    plan: &'a DropPlan,
    proof: CheckpointProof,
    elapsed_ms: u128,
}

fn require_trim_proof(proof: Option<&ProvedPlan<'_>>, cut: u128, plan: &DropPlan) -> Result<()> {
    let proof = proof.context("--trim requires a proof produced in this run; supply SCRATCH")?;
    ensure!(proof.proof.passed, "--trim refused: checkpoint proof failed");
    ensure!(
        proof.plan.cut_unix_ms == cut && proof.plan == plan,
        "--trim refused: checkpoint proof does not cover this exact cut and drop plan"
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let trim = args.iter().any(|arg| arg == "--trim");
    args.retain(|arg| arg != "--trim");
    let mut args = args.into_iter();
    let path = PathBuf::from(args.next().context("private clone path is required")?);
    let day = args
        .next()
        .context("the checkpoint day is required")?
        .into_string()
        .map_err(|_| anyhow::anyhow!("the day must be UTF-8"))?;
    let scratch = args.next().map(PathBuf::from);
    ensure!(args.next().is_none(), "expected CLONE DAY [SCRATCH] [--trim]");
    ensure!(!path.is_symlink(), "clone must not be a symlink");
    ensure!(
        path.metadata()?.permissions().mode() & 0o777 == 0o600,
        "clone must be mode 0600"
    );
    let cut = checkpoint_cut(&day)?;
    let opened = Instant::now();
    let store = Store::open(&path, "retention-plan")?;
    let open_ms = opened.elapsed().as_millis();
    let planned = Instant::now();
    let sealed = store.checkpoint_sealed_set(cut)?;
    let plan = store.runtime.plan_checkpoint_drops(&sealed);
    let plan_ms = planned.elapsed().as_millis();
    let proof = match &scratch {
        Some(scratch) => {
            let proved = Instant::now();
            let proof = store.prove_checkpoint_plan(&sealed, &plan, scratch)?;
            Some(ProvedPlan { plan: &plan, proof, elapsed_ms: proved.elapsed().as_millis() })
        }
        None => None,
    };
    let pages = |store: &Store| -> Result<(u64, u64)> {
        let connection = store.readers.get();
        Ok((
            connection.query_row("PRAGMA page_count", [], |row| row.get(0))?,
            connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?,
        ))
    };
    let trimmed = if trim {
        require_trim_proof(proof.as_ref(), cut, &plan)?;
        let holds = || {
            smallclaims::windows::store_series(smallclaims::windows::StoreWork::WriterHold)
                .snapshot(Instant::now())["1h"]
                .clone()
        };
        let before = pages(&store)?;
        let holds_before = holds();
        let trimming = Instant::now();
        let mut actions = Vec::new();
        store.trim_checkpoint(
            &format!("checkpoint/{day}"),
            cut,
            &plan.drop_digest,
            &plan.envelopes,
            &plan.claims,
            false,
            &mut actions,
        )?;
        let after = pages(&store)?;
        Some(json!({
            "elapsed_ms": trimming.elapsed().as_millis(),
            "actions": actions,
            "pages_before": {"page_count": before.0, "freelist_count": before.1},
            "pages_after": {"page_count": after.0, "freelist_count": after.1},
            "writer_holds_before_trim_1h": holds_before,
            "writer_holds_after_trim_1h": holds(),
        }))
    } else {
        None
    };
    let by_kind = plan
        .by_kind
        .iter()
        .map(|(kind, count)| (kind.clone(), json!({"sealed": count.sealed, "dropped": count.dropped})))
        .collect::<BTreeMap<_, _>>();
    let result = json!({
        "checkpoint": day,
        "cut_unix_ms": cut,
        "rules_digest": plan.rules_digest,
        "sealed_envelopes": plan.sealed_envelopes,
        "sealed_claims": plan.sealed_claims,
        "dropped_envelopes": plan.envelopes.len(),
        "dropped_claims": plan.claims.len(),
        "drop_digest": plan.drop_digest,
        "retained_digest": plan.retained_digest,
        "by_kind": by_kind,
        "store_open_ms": open_ms,
        "plan_ms": plan_ms,
        "proof": proof.as_ref().map(|proved| json!({
            "proof": proved.proof,
            "elapsed_ms": proved.elapsed_ms,
        })),
        "trim": trimmed,
        "release_build": !cfg!(debug_assertions),
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallclaims::store::checkpoint::EnvelopeTombstone;

    fn plan() -> DropPlan {
        DropPlan {
            cut_unix_ms: 100,
            rules_digest: "rules".into(),
            sealed_envelopes: 1,
            sealed_claims: 0,
            sealed_digest: "sealed".into(),
            envelopes: vec![EnvelopeTombstone {
                writer: "writer".into(),
                sequence: 1,
                envelope_hash: "hash".into(),
                accepted_at_unix_ms: 10,
            }],
            claims: Vec::new(),
            by_kind: BTreeMap::new(),
            drop_digest: "drop".into(),
            retained_digest: "retained".into(),
        }
    }

    fn proved(plan: &DropPlan, passed: bool) -> ProvedPlan<'_> {
        ProvedPlan {
            plan,
            proof: CheckpointProof {
                graph_digest_before: "graph".into(),
                graph_digest: "graph".into(),
                reader_digest_before: "readers".into(),
                reader_digest: "readers".into(),
                subjects: 0,
                passed,
                mismatches: Vec::new(),
            },
            elapsed_ms: 0,
        }
    }

    #[test]
    fn trim_refuses_missing_proof() {
        let plan = plan();
        assert!(require_trim_proof(None, plan.cut_unix_ms, &plan).is_err());
    }

    #[test]
    fn trim_refuses_failed_proof() {
        let plan = plan();
        assert!(require_trim_proof(Some(&proved(&plan, false)), plan.cut_unix_ms, &plan).is_err());
    }

    #[test]
    fn trim_refuses_proof_for_different_cut() {
        let plan = plan();
        assert!(require_trim_proof(Some(&proved(&plan, true)), plan.cut_unix_ms + 1, &plan).is_err());
    }

    #[test]
    fn trim_refuses_proof_for_different_drop_set() {
        let original = plan();
        let mut different = original.clone();
        different.envelopes.clear();
        // Even a reused digest must not authorize a different set of tombstones.
        assert!(require_trim_proof(Some(&proved(&original, true)), original.cut_unix_ms, &different).is_err());
    }

    #[test]
    fn trim_accepts_passing_proof_for_exact_plan() {
        let plan = plan();
        assert!(require_trim_proof(Some(&proved(&plan, true)), plan.cut_unix_ms, &plan).is_ok());
    }
}
