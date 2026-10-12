//! Same-runtime cooperative scheduling. Every operation is independently fenced
//! by live work authority and Git CAS; metadata never authorizes a main update.
use super::discovery::Observation;
use super::service::Entry;
use super::transport::validate_advertisement;
use super::{Record, Revision, State, TrailerStatus, WorkBinding, change_id, lane, oid};
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create {
        source: String,
        base: String,
    },
    State {
        id: String,
        expected_current: String,
        state: State,
        next: Option<String>,
    },
    Claim {
        id: String,
        expected_current: String,
        lane: String,
    },
    Release {
        id: String,
        expected_current: String,
        nonce: String,
        next: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub repository: String,
    pub work: WorkBinding,
    #[serde(rename = "action")]
    pub operation: Operation,
}

#[derive(Debug, Serialize)]
pub struct Published {
    pub revision: Revision,
    pub work: WorkBinding,
    pub transport: &'static str,
}

impl Operation {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Create { source, base } => {
                oid(source)?;
                oid(base)?;
            }
            Self::State {
                id,
                expected_current,
                state,
                next,
            } => {
                change_id(id)?;
                oid(expected_current)?;
                ensure!(
                    matches!(state, State::Draft | State::Ready),
                    "only draft and ready are published by this adapter"
                );
                ensure!(
                    next.is_none() || *state == State::Ready,
                    "draft cannot have a next stage"
                );
                if let Some(next) = next {
                    scheduling_lane(next)?;
                }
            }
            Self::Claim {
                id,
                expected_current,
                lane,
            } => {
                change_id(id)?;
                oid(expected_current)?;
                scheduling_lane(lane)?;
            }
            Self::Release {
                id,
                expected_current,
                nonce,
                next,
            } => {
                change_id(id)?;
                oid(expected_current)?;
                ensure!(
                    nonce.len() == 32
                        && nonce
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                    "invalid claim nonce"
                );
                if let Some(next) = next {
                    scheduling_lane(next)?;
                }
            }
        }
        Ok(())
    }
}

fn scheduling_lane(value: &str) -> Result<()> {
    lane(value)?;
    ensure!(
        matches!(value, "build" | "review"),
        "this adapter supports build and review; landing stays with the curator and PR path"
    );
    Ok(())
}

fn pilot_started(entry: &Entry) -> Result<()> {
    ensure!(
        entry.configuration.mutations_enabled,
        "Git change mutations are disabled; use the PR path"
    );
    let started = chrono::DateTime::parse_from_rfc3339(&entry.configuration.pilot_started_at)?;
    let now = chrono::Utc::now();
    ensure!(
        started <= now && now.signed_duration_since(started) < chrono::Duration::days(14),
        "bounded Git change pilot is outside its 14-day window; use the PR path"
    );
    Ok(())
}

fn exact_current(
    store: &Store,
    entry: &Entry,
    observed: &Observation,
    id: &str,
    expected: &str,
) -> Result<Revision> {
    ensure!(
        !observed.offline && observed.error.is_none(),
        "fresh Git inventory unavailable; use the PR path"
    );
    let advertisement = observed
        .advertisement()
        .context("Git inventory unavailable; use the PR path")?;
    let current = advertisement
        .refs
        .get(&format!("{}{id}/current", super::PREFIX))
        .context("change is absent from Git")?;
    ensure!(
        current == expected,
        "current Git record changed; inspect the change again"
    );
    let indexed =
        store.indexed_git_changes(&entry.configuration.name, std::slice::from_ref(current))?;
    let revision = &indexed
        .get(current)
        .context("current Git object is not indexed; request sync")?
        .revision;
    ensure!(
        revision.record.change_id == id,
        "indexed change identity mismatch"
    );
    validate_advertisement(advertisement, revision)?;
    // Read the original objects again; an index alone cannot authorize publication.
    let loaded = entry
        .repository
        .as_ref()
        .context("Git transport unavailable")?
        .load(current)?;
    ensure!(
        &loaded == revision,
        "Git object differs from its indexed receipt"
    );
    Ok(loaded)
}

fn ready_dependencies(revision: &Revision) -> Result<()> {
    ensure!(
        revision.record.depends_on.is_empty(),
        "dependent changes require the existing PR composition path until independent dependency landing receipts are available"
    );
    Ok(())
}

impl Entry {
    /// Serialized by the sole configured repository owner. Network work never holds
    /// a graph writer or reader snapshot; the work fence is checked again before push.
    pub async fn mutate(
        self: &Arc<Self>,
        store: Arc<Store>,
        actor: String,
        request: Request,
    ) -> Result<Published> {
        let entry = self.clone();
        tokio::spawn(async move { entry.mutate_owned(store, actor, request).await })
            .await
            .context("Git mutation owner stopped")?
    }

    async fn mutate_owned(
        self: &Arc<Self>,
        store: Arc<Store>,
        actor: String,
        request: Request,
    ) -> Result<Published> {
        request.operation.validate()?;
        ensure!(
            actor.starts_with("agent/"),
            "stage publication requires a live local agent work binding"
        );
        store.validate_git_change_work(&actor, &request.work)?;
        ensure!(
            self.owner.is_some(),
            "this node does not own Git writes; use the PR path"
        );
        let _publishing = self
            .publishing
            .try_lock()
            .context("Git publication is in flight; retry explicitly or use the PR path")?;
        // Voluntary release remains possible after the pilot closes.
        if !matches!(request.operation, Operation::Release { .. }) {
            pilot_started(self)?;
        }
        let capacity = self.owner.as_ref().unwrap().capacity().await?;
        let (observed, managed_ref_count) = match capacity {
            super::discovery::CapacityAdmission::Completed {
                observation,
                managed_ref_count,
            } => (observation, managed_ref_count),
            super::discovery::CapacityAdmission::Deferred { retry_after_ms } => anyhow::bail!(
                "Git read budget deferred capacity check for {retry_after_ms} ms; use the PR path or retry explicitly"
            ),
            super::discovery::CapacityAdmission::InFlight => {
                anyhow::bail!("Git reader is in flight; retry explicitly or use the PR path")
            }
        };
        let entry = self.clone();
        let old_and_candidate = tokio::task::spawn_blocking({
            let store = store.clone();
            let actor = actor.clone();
            let request = request.clone();
            let observed = observed.clone();
            move || -> Result<(Option<Revision>, Revision)> {
                let repository = entry.repository.as_ref().context("Git transport unavailable")?;
                let old = match &request.operation {
                    Operation::Create { .. } => None,
                    Operation::State { id, expected_current, .. }
                    | Operation::Claim { id, expected_current, .. }
                    | Operation::Release { id, expected_current, .. } => Some(exact_current(&store, &entry, &observed, id, expected_current)?),
                };
                let candidate = match &request.operation {
                    Operation::Create { source, base } => {
                        let advertisement = observed.advertisement().context("fresh Git inventory unavailable")?;
                        ensure!(observed.error.is_none(), "fresh Git inventory unavailable");
                        let id = repository.trailer(source)?.context("source has no Change-Id; explicitly recover it in the owned workspace before publication")?;
                        ensure!(!observed.advertised_ids().contains_key(&id), "change already exists; inspect or revise it");
                        ensure!(store.latest_indexed_git_change(&entry.configuration.name, &id)?.is_none(), "change has historical receipts but missing remote refs; explicit owner repair is required, use the PR path");
                        repository.validate_grouping(base, source, &id)?;
                        ensure!(observed.advertised_ids().len() < 100, "bounded pilot reached 100 changes; use the PR path");
                        let mut ids: BTreeSet<String> = observed.advertised_ids().keys().cloned().collect();
                        // Previously indexed IDs count too; deleting a ref does not restart the pilot.
                        let mut after = String::new();
                        loop {
                            let page = store.indexed_git_change_ids(&entry.configuration.name, &after, 50)?;
                            if page.is_empty() { break; }
                            after = page.last().unwrap().clone();
                            ids.extend(page);
                            ensure!(ids.len() < 100, "bounded pilot reached 100 changes; use the PR path");
                        }
                        ensure!(ids.len() < 100, "bounded pilot reached 100 changes; use the PR path");
                        ensure!(advertisement.refs.len() < 5000, "change namespace has no pilot capacity");
                        let (source_commit, source_tree) = repository.source(source)?;
                        let now = chrono::Utc::now().to_rfc3339();
                        repository.prepare(Record { schema_version: 1, change_id: id, generation: 1, previous_record: None,
                            source_commit, source_tree, base_commit: base.clone(), trailer_status: TrailerStatus::Present,
                            state: State::Draft, next: None, depends_on: vec![], actor: actor.clone(), created_at: now.clone(), updated_at: now, claim: None, mirror: None })?
                    }
                    Operation::State { state, next, .. } => repository.prepare_state(old.as_ref().unwrap(), &actor, state.clone(), next.clone())?,
                    Operation::Claim { lane, .. } => {
                        let old = old.as_ref().unwrap();
                        ensure!(old.record.next.as_deref() == Some(lane), "change is not waiting for this lane");
                        ready_dependencies(old)?;
                        repository.prepare_claim(old, &actor, request.work.clone())?
                    }
                    Operation::Release { nonce, next, .. } => {
                        let old = old.as_ref().unwrap();
                        ensure!(old.record.claim.as_ref().is_some_and(|claim| claim.work == request.work), "release work binding differs from the owner binding");
                        repository.prepare_release(old, &actor, nonce, next.clone())?
                    }
                };
                Ok((old, candidate))
            }
        }).await.context("Git candidate builder stopped")??;
        let old_refs = old_and_candidate
            .0
            .as_ref()
            .map(super::transport::markers)
            .transpose()?
            .map_or(0, |refs| refs.len());
        let new_refs = super::transport::markers(&old_and_candidate.1)?.len();
        ensure!(
            new_refs <= old_refs || managed_ref_count.saturating_add(new_refs - old_refs) <= 5000,
            "bounded pilot reached 5000 managed change/evidence refs; use the PR path"
        );
        let entry = self.clone();
        tokio::task::spawn_blocking(move || -> Result<Published> {
            let (old, candidate) = old_and_candidate;
            store.validate_git_change_work(&actor, &request.work)?;
            entry.check_destination()?;
            let repository = entry
                .repository
                .as_ref()
                .context("Git transport unavailable")?;
            let release = match &request.operation {
                Operation::Release { nonce, .. } => Some((actor.as_str(), nonce.as_str())),
                _ => None,
            };
            repository.publish_guarded(&entry.configuration.remote, old.as_ref(), &candidate, release,
                &|| store.validate_git_change_work(&actor, &request.work))
                .with_context(|| format!("Git publication outcome unconfirmed for {} candidate {}; inspect remote refs and work/nonce before retrying", candidate.record.change_id, candidate.oid))?;
            store.index_git_change(&entry.configuration.name, &candidate).with_context(|| format!("Git publication {} succeeded but local indexing failed; inspect this exact record before retrying", candidate.oid))?;
            Ok(Published {
                revision: candidate,
                work: request.work,
                transport: "git",
            })
        })
        .await
        .context("Git publisher stopped")?
    }
}

#[cfg(test)]
mod tests;
