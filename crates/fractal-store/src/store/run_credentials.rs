//! Run credentials (docs/st3/program-engine.md §3): the engine mints one per program attempt and
//! passes it to the attempt's effects as `ST_RUN_CREDENTIAL`; the daemon binds a request carrying
//! it to `mission-run/RUN`. It is a local, hashed store capability whose subject is the step run
//! and whose incarnation is the attempt. It needs no revocation: it binds only while that attempt
//! is the step run's current one and the step is still live, so it dies with its attempt.
use super::*;

/// The capability kind of a run credential.
pub const RUN_CREDENTIAL_KIND: &str = "program-run";
/// The environment variable a program attempt's effects carry their run credential in.
pub const RUN_CREDENTIAL_ENV: &str = "ST_RUN_CREDENTIAL";

/// The step statuses in which a program attempt runs, including one waiting on a person's answer.
const LIVE_STEP_STATES: [&str; 4] = ["ready", "working", "blocked", "waiting-person"];

/// The actor a run credential binds: `run` is `mission-run/RUN`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunBinding {
    pub run: String,
    pub step_run: String,
    pub attempt: u32,
}

fn ended(reason: &str) -> FractalError {
    FractalError::new("run-credential-ended", format!("the run credential no longer binds: {reason}"))
}

impl Store {
    /// Mint the credential one program attempt's effects carry, valid for at most `lifetime_ms`.
    pub fn issue_run_credential(&self, step_run: &str, attempt: u32, lifetime_ms: u64) -> Result<String> {
        self.issue_capability(RUN_CREDENTIAL_KIND, step_run, Some(&attempt.to_string()), lifetime_ms)
            .map(|(secret, _)| secret)
    }

    /// The run `secret` acts as, while its attempt is its step run's current, live attempt.
    pub fn run_binding(&self, secret: &str) -> Result<RunBinding, FractalError> {
        let capability = self.capability(secret, RUN_CREDENTIAL_KIND)?;
        let attempt = capability.incarnation_id.as_deref().and_then(|attempt| attempt.parse().ok())
            .ok_or_else(|| FractalError::new("invalid-capability", "the run credential names no attempt"))?;
        let step = self.step_run(&capability.subject).map_err(internal)?
            .ok_or_else(|| ended("its step run does not exist"))?;
        if step.attempt != attempt {
            return Err(ended("its attempt is no longer the step run's current attempt"));
        }
        if !LIVE_STEP_STATES.contains(&step.status.as_str()) {
            return Err(ended("its step run is no longer running"));
        }
        Ok(RunBinding { run: step.run, step_run: step.subject, attempt })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Store {
    /// Put a step run at `status` and `attempt`, as a dispatch, retry or end would.
    pub fn set_step_run_state_for_tests(&self, subject: &str, status: &str, attempt: u32) {
        self.connection.write().execute(
            /* sqlite-large-value: allow-for-now lv-h2ec0013f005e owner=fractal_store::store::run_credentials TODO=bound-or-reference-values-and-reclamation */ "UPDATE step_runs SET status=?2, attempt=?3 WHERE subject=?1",
            params![subject, status, attempt],
        ).unwrap();
    }

    /// How many run credentials were ever minted for `step_run`.
    pub fn run_credentials_issued_for_tests(&self, step_run: &str) -> usize {
        self.readers.get().query_row(
            "SELECT COUNT(*) FROM capabilities WHERE kind=?1 AND subject=?2",
            params![RUN_CREDENTIAL_KIND, step_run],
            |row| row.get(0),
        ).unwrap()
    }

    /// Publish a one-step mission that may write `custom/example/*` and `doc/runs/*` and publish
    /// `example/*`, start a run of it and put its step at attempt 1, working.
    pub fn start_program_step_for_tests(&self) -> StepRunView {
        crate::store::tests_shared_seats::publish_mission(self, r#"version 2
mission "land" state="ready" {
  goal "Land one change."
  authority { write "custom/example/*"; write "doc/runs/*"; publish "example/*"; }
  step "program" { agentless }
}"#, "declare-land");
        let run = self.create_mission_run_inner(&crate::model::MissionRunRequest {
            mission: "land".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/operator".into()),
            mode: None,
            inputs: Default::default(),
            idempotency_key: "run-land".into(),
        }, None, None, None).unwrap();
        let step = run.steps.into_iter().find(|step| step.step == "program").unwrap();
        self.set_step_run_state_for_tests(&step.subject, "working", 1);
        step
    }
}
