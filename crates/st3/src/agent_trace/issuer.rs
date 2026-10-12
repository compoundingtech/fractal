//! Publish launch facts and only the lease held by this execution.
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, ensure};
use crate::model::StepRunView;
use super::generated::*;
use super::Runtime;

#[derive(Clone, Debug)]
pub struct Facts {
    pub agent: String,
    pub principal: String,
    pub incarnation: String,
    pub harness_tool: String,
    pub harness_version: Option<String>,
    pub model_route: Option<String>,
    pub model_effort: Option<String>,
    pub session: Option<String>,
    pub host: Option<String>,
    pub worktree: Option<String>,
    pub mission_name: Option<String>,
}

/// Serialize lifecycle read/modify/write transactions across the driver and work CLI.
pub fn lock(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false)
        .read(true).write(true).mode(0o600)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    Ok(lock)
}

fn now_ms() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
}

fn trace_agent(subject: &str) -> &str {
    subject.strip_prefix("agent/")
        .filter(|name| name.contains('.') && !name.contains('/'))
        .unwrap_or(subject)
}

/// Build a runtime document from observed launch facts, without inventing a model.
pub fn build(facts: &Facts, lease: Option<&StepRunView>) -> Result<Runtime> {
    let mut runtime = Runtime {
        r#type: "agent-trace-runtime/v1".to_owned().try_into()?,
        who: AgentTraceRuntimeV1Who {
            agent: trace_agent(&facts.agent).to_owned().try_into()?,
            principal: facts.principal.clone().try_into()?,
            actor: None,
            persona: None,
            supervisor: None,
        },
        why: None,
        how: AgentTraceRuntimeV1How {
            harness: AgentTraceRuntimeV1HowHarness {
                tool: facts.harness_tool.clone().try_into()?,
                version: facts.harness_version.clone().map(TryInto::try_into).transpose()?,
            },
            model: None,
            tooling_profile: None,
            runtime_profile: None,
            skills_manifest: None,
            invocation_id: None,
            issuance: None,
        },
        r#where: AgentTraceRuntimeV1Where {
            session: facts.session.clone().map(TryInto::try_into).transpose()?,
            host: facts.host.clone().map(TryInto::try_into).transpose()?,
            worktree: facts.worktree.clone().map(TryInto::try_into).transpose()?,
        },
    };
    if let Some(route) = &facts.model_route {
        update_model(&mut runtime, route, facts.model_effort.as_deref())?;
    }
    update_lease(&mut runtime, &facts.agent, &facts.incarnation, lease, facts.mission_name.as_deref(), now_ms()?)?;
    Ok(runtime)
}

/// Lease references use the actual mission run and step generation, not claim IDs.
/// A completed, expired, foreign, or previous-incarnation claim is idle context.
pub fn update_lease(
    runtime: &mut Runtime,
    actor: &str,
    incarnation: &str,
    lease: Option<&StepRunView>,
    mission_name: Option<&str>,
    now_ms: u128,
) -> Result<()> {
    ensure!(runtime.who.agent.as_str() == trace_agent(actor), "trace belongs to another agent");
    let lease = lease.filter(|step| {
        matches!(step.status.as_str(), "claimed" | "working" | "verifying" | "blocked")
            && step.claimant.as_deref() == Some(actor)
            && step.claim_incarnation.as_deref() == Some(incarnation)
            && step.claim_expires_at_unix_ms.is_some_and(|expires| expires > now_ms)
    });
    let Some(step) = lease else {
        runtime.why = None;
        runtime.how.issuance = None;
        return Ok(());
    };
    let mission_run = step.run.strip_prefix("mission-run/").context("lease has no mission run reference")?;
    let step_run = step.generation.strip_prefix("run-generation/").context("lease has no step generation reference")?;
    let mission = AgentTraceRuntimeV1WhyMission {
        name: mission_name.context("claimed lease requires its mission name")?.to_owned().try_into()?,
        run: Some(mission_run.to_owned().try_into()?),
    };
    let why_step = AgentTraceRuntimeV1WhyStep {
        name: step.step.clone().try_into()?,
        run: Some(step_run.to_owned().try_into()?),
    };
    let issuance = AgentTraceRuntimeV1HowIssuance::Variant0(AgentTraceRuntimeV1HowIssuanceVariant0 {
        r#_tag: "st-issued".to_owned().try_into()?,
        step_run: step.subject.clone().try_into()?,
    });
    runtime.why = Some(AgentTraceRuntimeV1Why { mission: Some(mission), step: Some(why_step) });
    runtime.how.issuance = Some(issuance);
    Ok(())
}

/// Decode with the authoritative schema, preserving every launch and model fact.
pub fn refresh(
    path: &Path,
    actor: &str,
    incarnation: &str,
    lease: Option<&StepRunView>,
    mission_name: Option<&str>,
) -> Result<Runtime> {
    let mut runtime: Runtime = serde_json::from_slice(&std::fs::read(path)?)?;
    update_lease(&mut runtime, actor, incarnation, lease, mission_name, now_ms()?)?;
    write_atomic(path, &runtime)?;
    Ok(runtime)
}

/// Replace model facts only when an actual selected route is known.
pub fn update_model(runtime: &mut Runtime, route: &str, effort: Option<&str>) -> Result<()> {
    let model = AgentTraceRuntimeV1HowModel {
        route: route.to_owned().try_into()?,
        effort: effort.map(str::to_owned).map(TryInto::try_into).transpose()?,
    };
    runtime.how.model = Some(model);
    Ok(())
}

/// Same-directory exclusive temporary creation and rename keep readers atomic.
/// The file is private even when replacing a predecessor with broader permissions.
pub fn write_atomic(path: &Path, runtime: &Runtime) -> Result<()> {
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new().prefix(".agent-trace-").tempfile_in(parent)?;
    temporary.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut temporary, runtime)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MISSION: &str = "0123456789abcdef0123456789abcdef";
    const GENERATION: &str = "abcdef0123456789abcdef0123456789";

    fn facts() -> Facts {
        Facts {
            agent: "agent/example".into(), incarnation: "execution-one".into(),
            principal: "person/example".into(),
            harness_tool: "omp".into(), harness_version: Some("1.0.0".into()),
            model_route: Some("openai/gpt-5".into()), model_effort: Some("high".into()),
            session: Some("demo.one".into()), host: Some("test-host".into()),
            worktree: Some("/workspace/example".into()), mission_name: Some("example-mission".into()),
        }
    }

    fn lease() -> StepRunView {
        serde_json::from_value(json!({
            "subject":format!("step-run/{GENERATION}/work"), "run":format!("mission-run/{MISSION}"),
            "generation":format!("run-generation/{GENERATION}"), "step":"work", "definition_hash":"definition",
            "status":"claimed", "attempt":1, "assigned_to":"agent/example", "agentless":false,
            "title":null, "worker_reported":false, "claimant":"agent/example", "claim_incarnation":"execution-one",
            "claim_expires_at_unix_ms":u64::MAX, "readiness_epoch":1, "blocked_reason":null,
            "not_before_unix_ms":null, "created_at_unix_ms":1, "updated_at_unix_ms":2
        })).unwrap()
    }

    #[test]
    fn idle_omits_why_issuance_and_unknown_model() {
        let mut facts = facts();
        facts.model_route = None;
        let value = serde_json::to_value(build(&facts, None).unwrap()).unwrap();
        assert!(value.get("why").is_none());
        assert!(value["how"].get("issuance").is_none());
        assert!(value["how"].get("model").is_none());
    }

    #[test]
    fn claimed_lease_maps_actual_references() {
        let value = serde_json::to_value(build(&facts(), Some(&lease())).unwrap()).unwrap();
        assert_eq!(value["why"]["mission"], json!({"name":"example-mission", "run":MISSION}));
        assert_eq!(value["why"]["step"], json!({"name":"work", "run":GENERATION}));
        assert_eq!(value["how"]["issuance"], json!({"_tag":"st-issued", "step_run":format!("step-run/{GENERATION}/work")}));
    }

    #[test]
    fn expired_foreign_previous_and_completed_claims_are_idle() {
        let facts = facts();
        for case in 0..4 {
            let mut step = lease();
            match case {
                0 => step.claim_expires_at_unix_ms = Some(10),
                1 => step.claimant = Some("agent/other".into()),
                2 => step.claim_incarnation = Some("execution-previous".into()),
                _ => step.status = "completed".into(),
            }
            let mut runtime = build(&facts, None).unwrap();
            update_lease(&mut runtime, &facts.agent, &facts.incarnation, Some(&step), Some("example-mission"), 10).unwrap();
            assert!(runtime.why.is_none());
            assert!(runtime.how.issuance.is_none());
        }
    }

    #[test]
    fn atomic_private_roundtrip_and_claim_complete_refresh() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let facts = facts();
        let initial = build(&facts, None).unwrap();
        write_atomic(&path, &initial).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let decoded: Runtime = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded, initial);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let claimed = refresh(&path, &facts.agent, &facts.incarnation, Some(&lease()), Some("example-mission")).unwrap();
        assert!(claimed.why.is_some());
        assert_eq!(claimed.how.model, initial.how.model);
        assert_eq!(claimed.how.harness, initial.how.harness);
        assert_eq!(claimed.r#where, initial.r#where);
        let completed = refresh(&path, &facts.agent, &facts.incarnation, None, None).unwrap();
        assert_eq!(completed, initial);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn refresh_rejects_foreign_agent_without_replacing_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let initial = build(&facts(), None).unwrap();
        write_atomic(&path, &initial).unwrap();
        assert!(refresh(&path, "agent/other", "execution-one", Some(&lease()), Some("example-mission")).is_err());
        let decoded: Runtime = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(decoded, initial);
    }

    #[test]
    fn logical_and_dotted_identities_keep_human_principal() {
        for (subject, expected) in [
            ("agent/example/worker", "agent/example/worker"),
            ("agent/demo.worker", "demo.worker"),
        ] {
            let mut facts = facts();
            facts.agent = subject.into();
            let mut runtime = build(&facts, None).unwrap();
            assert_eq!(runtime.who.agent.as_str(), expected);
            assert_eq!(runtime.who.principal.as_str(), "person/example");
            assert!(runtime.who.actor.is_none());
            let mut step = lease();
            step.claimant = Some(subject.into());
            update_lease(&mut runtime, subject, &facts.incarnation, Some(&step), Some("example-mission"), 10).unwrap();
            assert!(runtime.how.issuance.is_some());
        }
    }

    #[test]
    fn model_update_preserves_lease_and_launch_facts() {
        let mut runtime = build(&facts(), Some(&lease())).unwrap();
        let initial = runtime.clone();
        update_model(&mut runtime, "anthropic/claude-sonnet-4", None).unwrap();
        assert_eq!(runtime.why, initial.why);
        assert_eq!(runtime.how.issuance, initial.how.issuance);
        assert_eq!(runtime.how.harness, initial.how.harness);
        assert_eq!(runtime.who, initial.who);
        assert_eq!(runtime.r#where, initial.r#where);
        let model = runtime.how.model.as_ref().unwrap();
        assert_eq!(model.route.as_str(), "anthropic/claude-sonnet-4");
        assert!(model.effort.is_none());
        let updated = runtime.clone();
        assert!(update_model(&mut runtime, "not-a-route", None).is_err());
        assert_eq!(runtime, updated);
    }

    #[test]
    fn invalid_typed_refresh_keeps_predecessor_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        let mut value = serde_json::to_value(build(&facts(), None).unwrap()).unwrap();
        value["how"]["model"]["route"] = json!("unknown");
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(refresh(&path, "agent/example", "execution-one", Some(&lease()), Some("example-mission")).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn all_held_work_statuses_keep_issuance() {
        for status in ["claimed", "working", "verifying", "blocked"] {
            let mut step = lease();
            step.status = status.into();
            assert!(build(&facts(), Some(&step)).unwrap().how.issuance.is_some());
        }
    }
}
