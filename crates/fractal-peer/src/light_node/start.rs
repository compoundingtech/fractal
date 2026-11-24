//! Immutable home-checked launch facts. Native admission still owns physical resource custody.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use crate::model::MemberSpec;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartAuthorization {
    pub revision: u64,
    pub content_sha256: String,
    pub desired_token: String,
    pub selected_desired_token: String,
    pub declaration_sha256: String,
    pub publication_sha256: String,
    pub helper: Option<StartHelperScope>,
    pub rollout: Option<StartRolloutScope>,
    pub member: MemberSpec,
    pub launch_member: MemberSpec,
    pub workspace_admission: StartWorkspaceAdmission,
    pub start_request: String,
    pub expected_incarnation: Option<String>,
    pub checkout: Option<StartCheckout>,
    pub render: Option<StartRender>,
    pub credentials: Option<StartCredentials>,
    pub continuation: Option<StartContinuation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartHelperScope { pub subject: String, pub lifetime: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRolloutScope { pub scope: String, pub permission: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StartWorkspaceAdmission { NonGit, Git { owner: String, repository: String } }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartCheckout { pub repository: String, pub base: String, pub branch: String, pub remove_at_run_end: bool }

/// The declaration is the existing render AST, not an opaque launch payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRender {
    pub declaration: serde_json::Value,
    pub documents: std::collections::BTreeMap<String, Vec<u8>>,
}

/// Host-local login selector only: credential secret bytes never cross this protocol.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartCredentials { pub driver: String, pub account: Option<String>, pub login: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartContinuation { pub session: String, pub path: Option<String> }

impl StartAuthorization {
    /// Seal only after graph-side admission has checked every retained fact.
    pub fn seal(&mut self) -> Result<()> { self.content_sha256 = self.digest()?; self.verify() }
    fn digest(&self) -> Result<String> {
        use sha2::{Digest as _, Sha256};
        let mut content = self.clone();
        content.content_sha256.clear();
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&content)?)))
    }
    pub fn verify(&self) -> Result<()> {
        ensure!(self.revision > 0 && self.revision <= i64::MAX as u64, "invalid Start authorization revision");
        ensure!(self.content_sha256 == self.digest()?, "Start authorization content digest differs");
        ensure!(!self.desired_token.is_empty() && !self.selected_desired_token.is_empty() && !self.start_request.is_empty(), "Start authorization lacks desired/request identity");
        for digest in [&self.declaration_sha256, &self.publication_sha256] {
            ensure!(digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()), "invalid Start declaration/publication content identity");
        }
        if let Some(helper) = &self.helper { ensure!(!helper.subject.is_empty() && !helper.lifetime.is_empty(), "invalid Start helper scope"); }
        if let Some(rollout) = &self.rollout { ensure!(!rollout.scope.is_empty() && !rollout.permission.is_empty(), "invalid Start rollout permission"); }
        if let StartWorkspaceAdmission::Git {owner, repository} = &self.workspace_admission { ensure!(!owner.is_empty() && !repository.is_empty(), "invalid Start workspace admission"); }
        if let Some(checkout) = &self.checkout {
            ensure!(bounded(&checkout.repository) && bounded(&checkout.base) && bounded(&checkout.branch), "invalid Start checkout");
        }
        ensure!(self.expected_incarnation.as_ref().is_none_or(|incarnation| bounded(incarnation)), "invalid expected Start incarnation");
        if let Some(credentials) = &self.credentials {
            ensure!(bounded(&credentials.driver) && bounded(&credentials.login)
                && credentials.account.as_ref().is_none_or(|account| bounded(account)), "invalid Start host credential selector");
        }
        if let Some(continuation) = &self.continuation {
            ensure!(bounded(&continuation.session) && continuation.path.as_ref().is_none_or(|path| bounded(path)), "invalid Start continuation");
        }
        Ok(())
    }
}

fn bounded(value: &str) -> bool { !value.is_empty() && value.len() <= 4096 && !value.contains('\0') }
