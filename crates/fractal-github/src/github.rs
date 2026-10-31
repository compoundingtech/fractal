use crate::model::{DesiredSubject, MissionInputKind, MissionRunView};
use crate::store::Store;
use crate::store::*;
use anyhow::Result;
use rusqlite::OptionalExtension as _;

pub trait GithubStore {
    /// Repository candidates on one host, excluding members with neither a checkout nor any
    /// published workspace evidence. Uses the host index rather than walking the fleet graph.
    fn agent_repository_subjects(&self, host: &str) -> Result<Vec<DesiredSubject>>;
    /// Find a mission-run PR resource for this discovered PR when that run already has a human
    /// review gate for its own PR. Intake uses this graph provenance to avoid a second reviewer.
    /// Why a pull request review that has not started must not start: since the request, its
    /// pull request closed or merged, went back to draft, or moved to a newer head. `None` while
    /// the requested head is still the open pull request's current head.
    fn stale_pull_request_request(&self, resource: &str, discovery: &str)
    -> Result<Option<String>>;
    /// Only subscription deliveries follow the current PR head. Authored missions keep their
    /// pinned inputs, including missions that opened or are changing a pull request themselves.
    fn stale_subscription_pull_request_run(&self, run: &MissionRunView) -> Result<Option<String>>;
    /// The members whose readers need the full facts of the GitHub repository `locator` recorded
    /// into `resource`: the host of each seat that watches it, each member that declared a
    /// subscription on one of its observers, and each member running a mission run pinned to one
    /// of its items. A poller keeps writing item snapshots until every one of them advertises
    /// the GitHub plugin.
    fn repository_dependents(&self, locator: &str, resource: &str) -> Result<std::collections::BTreeSet<String>>;
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
    ) -> Result<Option<String>> {
        stale_pull_request_request_tx(&self.readers.get(), resource, discovery)
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
        for input in run
            .inputs
            .values()
            .filter(|input| input.kind == MissionInputKind::Resource)
        {
            if let (Some(resource), Some(discovery)) = (&input.subject, &input.claim_id)
                && let Some(reason) = self.stale_pull_request_request(resource, discovery)?
            {
                return Ok(Some(reason));
            }
        }
        Ok(None)
    }

    fn repository_dependents(&self, locator: &str, resource: &str) -> Result<std::collections::BTreeSet<String>> {
        let observers = crate::WatchesStore::repository_observers(self)
            .map_err(|error| anyhow::anyhow!(error.message))?
            .into_iter()
            .filter(|(_, spec)| spec.resource == resource || spec.locator.eq_ignore_ascii_case(locator))
            .map(|(subject, _)| subject)
            .collect::<std::collections::BTreeSet<_>>();
        let connection = self.readers.get();
        let mut members = std::collections::BTreeSet::new();
        let origin = |subject: &str| -> Result<Option<String>> {
            Ok(connection
                .query_row(
                    "SELECT claims.origin FROM desired JOIN claims ON claims.id=desired.claim_id WHERE desired.subject=?1",
                    [subject],
                    |row| row.get::<_, String>(0),
                )
                .optional()?)
        };
        let rows = connection
            .prepare_cached("SELECT subject, body FROM desired WHERE kind='subscription' ORDER BY subject")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (subject, body) in rows {
            let Some(spec) = serde_json::from_str::<serde_json::Value>(&body).ok().as_ref().and_then(crate::graph::subscription_spec)
            else {
                continue;
            };
            if spec.stopped || !observers.contains(&spec.observer) {
                continue;
            }
            members.extend(origin(&subject)?);
            // A watch wakes its seat on the seat's host, which reads the item there.
            if spec.watch.is_some() && spec.to.starts_with("agent/") {
                let host = connection
                    .query_row(
                        "SELECT json_extract(member, '$.host') FROM desired WHERE subject=?1 AND kind='agent'",
                        [&spec.to],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten();
                match host {
                    Some(host) => {
                        members.insert(host);
                    }
                    None => members.extend(origin(&spec.to)?),
                }
            }
        }
        // A run pinned to one of the repository's items reads that claim on the member it runs on.
        let prefix = format!("{resource}/");
        let pinned = connection
            .prepare_cached(
                "SELECT DISTINCT created.origin FROM mission_runs, json_each(mission_runs.inputs) AS input
                 JOIN claims AS created ON created.subject='mission-run/' || mission_runs.id
                     AND created.kind='mission-run.created'
                 WHERE mission_runs.status NOT IN ('completed','failed','cancelled')
                   AND substr(json_extract(input.value, '$.subject'), 1, length(?1))=?1",
            )?
            .query_map([&prefix], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        members.extend(pinned);
        Ok(members)
    }
}
