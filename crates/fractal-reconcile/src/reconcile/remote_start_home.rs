//! Home-only graph decisions. Executors consume the sealed result, never these graph reads.
use super::*;
use fractal_peer::light_node::journal::{StartAuthorization, StartCheckout, StartContinuation,
    StartCredentials, StartHelperScope, StartRender, StartRolloutScope, StartWorkspaceAdmission};
use crate::store::run_credentials::ExecutionBinding;
use crate::model::FractalError;

fn invalid_start(message: impl Into<String>) -> FractalError {
    FractalError::new("light-node-start-authorization-changed", message)
}

impl<R: RuntimeControl> Reconciler<R> {
    pub fn prepare_home_start(&self, binding: &ExecutionBinding, revision: u64,
        start_request: &str, workspace: StartWorkspaceAdmission) -> Result<StartAuthorization> {
        let expected = self.store.latest_actual_value(&binding.subject)?.and_then(|actual|
            actual_field(&actual, "incarnation_id").and_then(Value::as_str).map(str::to_owned));
        let mut authorization = self.home_start_snapshot(binding, revision, start_request, workspace, expected, true, None)?;
        authorization.seal()?;
        Ok(authorization)
    }

    /// Call while holding the graph's exclusive writer through journal admission.
    pub fn validate_home_start(&self, binding: &ExecutionBinding, authorization: &StartAuthorization) -> Result<()> {
        if authorization.member.terminal {
            anyhow::ensure!(authorization.launch_member.terminal_size == self.launch_geometry()?,
                invalid_start("home launch geometry changed before remote start admission"));
        }
        self.validate_home_start_snapshot(binding, authorization, false)
    }

    /// Recheck ongoing authority, not launch-only preferences such as the current pane size.
    /// Hold the exclusive graph writer through any resulting durable withdrawal.
    pub fn validate_retained_home_start(&self, binding: &ExecutionBinding, authorization: &StartAuthorization) -> Result<()> {
        self.validate_home_start_snapshot(binding, authorization, true)
    }

    fn validate_home_start_snapshot(&self, binding: &ExecutionBinding, authorization: &StartAuthorization,
        retained: bool) -> Result<()> {
        authorization.verify()?;
        let mut current = self.home_start_snapshot(binding, authorization.revision,
            &authorization.start_request, authorization.workspace_admission.clone(),
            authorization.expected_incarnation.clone(), false, retained.then_some(&authorization.continuation))?;
        if current.member.terminal {
            current.launch_member.terminal_size = authorization.launch_member.terminal_size;
        }
        current.seal()?;
        anyhow::ensure!(current.content_sha256 == authorization.content_sha256,
            invalid_start("home graph authorization changed before remote start or withdrawal reconciliation"));
        Ok(())
    }

    fn home_start_snapshot(&self, binding: &ExecutionBinding, revision: u64, start_request: &str,
        workspace_admission: StartWorkspaceAdmission, expected_incarnation: Option<String>,
        retain_account: bool, retained_continuation: Option<&Option<StartContinuation>>) -> Result<StartAuthorization> {
        anyhow::ensure!(binding.run.is_none() && binding.authorization_revision == revision.to_string(),
            invalid_start("remote Start requires its exact non-program authorization revision"));
        binding.validate()?;
        let desired = self.store.desired_subjects_named(std::slice::from_ref(&binding.subject))?
            .into_iter().next().ok_or_else(|| invalid_start("remote Start subject has no current declaration"))?;
        anyhow::ensure!(desired.subject == binding.subject && matches!(desired.kind.as_str(), "agent" | "exec"),
            invalid_start("remote Start needs a current physical member declaration"));
        let member = desired.member.as_ref().ok_or_else(|| invalid_start("remote Start declaration has no member"))?;
        anyhow::ensure!(member.host.trim_start_matches("host/") == binding.host
            && member.runtime_id == binding.runtime, invalid_start("remote Start member host/runtime differs from its grant"));
        self.store.owned_desired_guard(&desired)?;
        anyhow::ensure!(self.store.helper_launcher_alive(&desired.subject)?, invalid_start("helper launcher lifetime ended"));
        if let Some(run) = &desired.owner_run {
            anyhow::ensure!(!self.store.run_projection_pending(run)?, invalid_start("remote Start publication is incomplete"));
            anyhow::ensure!(!self.store.terminal_owned_runtime_subjects()?.contains(&desired.subject),
                invalid_start("remote Start belongs to a terminal owner"));
        }
        let selected_desired_token = self.store.selected_desired_token(&desired.subject)?
            .ok_or_else(|| invalid_start("remote Start selected declaration is missing"))?;
        let request = self.store.retained_runtime_start_request(start_request)?
            .ok_or_else(|| invalid_start("remote Start original request is no longer retained"))?;
        anyhow::ensure!(request.subject == desired.subject && request.kind == "runtime.action.requested"
            && request.body.pointer("/fields/action").and_then(Value::as_str) == Some("start")
            && request.body.pointer("/fields/runtime_id").and_then(Value::as_str) == Some(member.runtime_id.as_str())
            && request.body.pointer("/fields/terminal").and_then(Value::as_bool) == Some(member.terminal)
            && request.body.get("evidence").and_then(Value::as_array)
                .is_some_and(|evidence| evidence.iter().any(|id| id.as_str() == Some(selected_desired_token.as_str()))),
            invalid_start("remote Start original request does not retain this exact declaration/member"));
        let publication = crate::store::claim_by_id_tx(&self.store.readers.get(), &selected_desired_token)?
            .ok_or_else(|| invalid_start("remote Start publication content is missing"))?;
        let digest = |value: &Value| -> Result<String> {
            use sha2::{Digest as _, Sha256};
            Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
        };
        let declaration_sha256 = digest(&serde_json::to_value(&desired)?)?;
        let publication_sha256 = digest(&publication.body)?;
        let helper = self.store.helper_launcher_lifetime(&desired.subject)?
            .map(|(subject, lifetime)| StartHelperScope { subject, lifetime });
        let rollout = if let Some(id) = member.environment.get(crate::rollout::OPERATION_ENV) {
            let current = self.store.rollout(&desired.subject)?.ok_or_else(|| invalid_start("remote Start rollout is missing"))?;
            anyhow::ensure!(current.id == *id && current.phase == "starting", invalid_start("remote Start rollout permission changed"));
            Some(StartRolloutScope { scope: current.id, permission: current.phase })
        } else { None };
        let credentials = if let Some(helper) = self.store.helper_launcher_binding(&desired.subject)? {
            anyhow::ensure!(crate::accounts::harness_driver(&desired.desired) == Some(helper.harness.as_str()),
                invalid_start("helper harness differs from its admitted credential lifetime"));
            helper.credential_home.map(|login| StartCredentials { driver: helper.harness, account: None, login })
        } else if let Some(account) = crate::accounts::harness_binding(&desired.desired) {
            anyhow::ensure!(!self.store.helper_launcher_has_lifetime_binding(&desired.subject)?,
                invalid_start("helper declaration no longer matches its credential binding"));
            let selected = self.store.account_for_start(&desired.subject, &account, &member.host,
                now_ms(), retain_account).map_err(|reason| invalid_start(format!("remote Start account refused: {reason}")))?;
            Some(StartCredentials { driver: account.driver, account: Some(selected.account.name), login: selected.login })
        } else { None };
        let continuation = if let Some(retained) = retained_continuation { retained.clone() }
        else if desired.kind == "agent" && !member.environment.contains_key(crate::suspension::RESUME_ENV)
            && let Some(driver) = member.driver.as_deref() {
            let account = credentials.as_ref().and_then(|selected| selected.account.as_deref());
            crate::suspension::continue_session(&self.store, &desired.subject, driver, account,
                crate::native_resume::selection_scope(member).as_deref())?
                .map(|(session, path)| StartContinuation { session, path })
        } else { None };
        let checkout = Checkout::from_desired(&desired.desired).map(|checkout| StartCheckout {
            repository: checkout.repository.to_string_lossy().into_owned(), base: checkout.base,
            branch: checkout.branch, remove_at_run_end: checkout.remove_at_run_end,
        });
        let render = crate::render::children(&desired.desired).iter()
            .find(|node| crate::render::name(node) == Some("render"))
            .map(|declaration| -> Result<StartRender> {
                let mut documents = BTreeMap::new();
                for operation in crate::render::children(declaration) {
                    if crate::render::name(operation) != Some("copy") { continue; }
                    let Some(source) = operation.get("arguments").and_then(Value::as_array)
                        .and_then(|args| args.first()).and_then(Value::as_str) else { continue; };
                    if !source.starts_with("doc/") { continue; }
                    let (name, hash) = source.rsplit_once('@').ok_or_else(|| invalid_start("render document needs immutable @HASH"))?;
                    let bytes = self.store.get_document(name, hash)?.ok_or_else(|| invalid_start("remote Start render document is missing"))?;
                    documents.insert(source.to_owned(), bytes);
                }
                Ok(StartRender { declaration: declaration.clone(), documents })
            }).transpose()?;
        let mut launch_member = member.clone();
        if retain_account && launch_member.terminal {
            launch_member.terminal_size = self.launch_geometry()?;
        }
        Ok(StartAuthorization {
            revision, content_sha256: String::new(), desired_token: self.launch_token(&desired.subject)?,
            selected_desired_token, declaration_sha256, publication_sha256, helper, rollout,
            member: member.clone(), launch_member, workspace_admission,
            start_request: start_request.to_owned(), expected_incarnation, checkout, render, credentials, continuation,
        })
    }
}
