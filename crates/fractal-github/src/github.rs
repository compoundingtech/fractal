use crate::model::{DesiredSubject, MissionInputKind, MissionRunView};
use crate::store::Store;
use crate::store::*;
use anyhow::Result;

pub trait GithubStore {
    /// Repository candidates on one host, excluding members with neither a checkout nor any
    /// published workspace evidence. Uses the host index rather than walking the fleet graph.
    fn agent_repository_subjects(&self, host: &str) -> Result<Vec<DesiredSubject>>;
    /// Find a mission-run PR resource for this discovered PR when that run already has a human
    /// review gate for its own PR. Intake uses this graph provenance to avoid a second reviewer.
    /// Why a queued pull request snapshot is stale. Ready-head admission also requires an
    /// open, non-draft PR; relevant-changes admission retains only the head freshness fence.
    fn stale_pull_request_request(&self, resource: &str, discovery: &str, admission: Option<&str>)
    -> Result<Option<String>>;
    /// Only subscription deliveries follow the current PR head. Authored missions keep their
    /// pinned inputs, including missions that opened or are changing a pull request themselves.
    fn stale_subscription_pull_request_run(&self, run: &MissionRunView) -> Result<Option<String>>;
}

impl GithubStore for Store {
    fn agent_repository_subjects(&self, host: &str) -> Result<Vec<DesiredSubject>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step FROM desired
             WHERE kind='agent' AND json_extract(member, '$.host')=?1
               AND (EXISTS (SELECT 1 FROM json_each(body, '$.children') child WHERE json_extract(child.value, '$.name')='checkout')
                    OR EXISTS (SELECT 1 FROM claims WHERE claims.subject=desired.subject AND claims.kind='workspace.observed'))
             ORDER BY subject",
        )?;
        let rows = statement.query_map([host], desired_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn stale_pull_request_request(
        &self,
        resource: &str,
        discovery: &str,
        admission: Option<&str>,
    ) -> Result<Option<String>> {
        stale_pull_request_request_tx(&self.readers.get(), resource, discovery, admission)
    }

    fn stale_subscription_pull_request_run(&self, run: &MissionRunView) -> Result<Option<String>> {
        if !run.inputs.values().any(|input| {
            input
                .subject
                .as_deref()
                .is_some_and(|subject| subject.contains("/pull-request/"))
        }) {
            return Ok(None);
        }
        let from_subscription = run
            .parent_step_run
            .as_deref()
            .is_some_and(|parent| parent.starts_with("step-run/subscription/"));
        if !from_subscription {
            smallclaims::touched::note_read(|| format!("subscription-run:{}", run.subject));
            let started = subscription_started_run_tx(&self.readers.get(), &run.subject)?;
            if !started {
                return Ok(None);
            }
        }
        let created = self.latest_claim(&run.subject, Some("mission-run.created"))?;
        let admission = created.as_ref()
            .and_then(|claim| claim.body.pointer("/fields/admission"))
            .and_then(serde_json::Value::as_str);
        for input in run
            .inputs
            .values()
            .filter(|input| input.kind == MissionInputKind::Resource)
        {
            if let (Some(resource), Some(discovery)) = (&input.subject, &input.claim_id)
                && let Some(reason) = self.stale_pull_request_request(resource, discovery, admission)?
            {
                return Ok(Some(reason));
            }
        }
        Ok(None)
    }
}
