//! The part of `store::seats` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AccountsAttentionStore;
use super::*;
pub use st3_store::store::seats::*;
pub use st3_store::store::seats::{EndedDeclaration};

/// Canonical order's first components: accepted time as stored, shorter first.
fn accepted_order(accepted: &str) -> (usize, &str) {
    (accepted.len(), accepted)
}

pub trait SeatsAttentionStore {
    fn desired_harness_login_candidates(&self) -> Result<Vec<DesiredSubject>>;
    /// Resume an agent's kept usage fold with only the usage claims after it, when every one of
    /// them sorts after the last claim it folded. `None` when one does not, such as a replicated
    /// claim accepted earlier, or two new claims tie on accepted time; the caller folds afresh.
    fn usage_fold_after(
            connection: &rusqlite::Connection,
            subject: &str,
            cached: &CachedUsageFold,
            through: u64,
        ) -> Result<Option<CachedUsageFold>>;
    /// Last substantive activity for an agent, excluding heartbeat and usage observations.
    fn agent_last_activity_at(
            &self,
            agent: &str,
            incarnation: Option<&str>,
            snapshot_index: u64,
        ) -> Result<Option<u128>>;
    /// The login directory a seat's harness runs with on `host`: its account's, or the pool
    /// account this node placed it on, else the harness's own default login on that host. Only
    /// the seat's own node knows which pool account it chose, so a pooled seat on another node
    /// stands alone rather than being merged with seats that may use another login.
    fn seat_login(&self, desired: &DesiredSubject, host: &str, driver: &str) -> Result<String>;
    fn harness_login_episode_key(
            &self,
            subject: &str,
            incarnation: &str,
        ) -> Result<(Option<ClaimRecord>, String)>;
    fn harness_auth_episode(
            &self,
            subject: &str,
            incarnation: &str,
            code: Option<&str>,
        ) -> Result<Option<ClaimRecord>>;
    /// Prefer a bound account person, then follow declaration authors and mission requesters.
    /// Missing or cyclic ownership remains unknown; it never selects a global operator.
    fn agent_person(&self, agent: &str) -> Result<Option<String>>;
    /// The seat's harness for the one question the login attention item asks: does it need a
    /// login? A seat that does not returns without the observation fold, which is what a
    /// full `current_harness` costs, so an attention read no longer folds every seat that ever
    /// had a login problem. The view a seat that does need one gets is `current_harness`'s,
    /// except that its `since` is not refined (attention reads the state, incarnation and
    /// observation time).
    fn current_harness_for_login(
            &self,
            subject: &str,
        ) -> Result<Option<crate::model::CurrentHarnessView>>;
}

impl SeatsAttentionStore for Store {

    fn desired_harness_login_candidates(&self) -> Result<Vec<DesiredSubject>> {
        smallclaims::touched::note_read(|| "desired-kind:agent".to_owned());
        smallclaims::touched::note_read(|| "kind:harness.observed".to_owned());
        smallclaims::touched::note_read(|| "kind:harness.diagnostic".to_owned());
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject, kind, body, member, owner_run, owner_generation, owner_step
             FROM desired WHERE kind='agent'
               AND EXISTS (SELECT 1 FROM claims INDEXED BY claims_harness_login_candidate_index
                 WHERE claims.subject=desired.subject AND (
                   (kind='harness.observed' AND (
                     json_type(body, '$.fields.provider_auth')='false'
                     OR json_extract(body, CASE WHEN json_type(body, '$.fields') IS NULL
                         THEN '$.reason' ELSE '$.fields.reason' END)='providerAuth'
                     OR json_extract(body, CASE WHEN json_type(body, '$.fields') IS NULL
                         THEN '$.state' ELSE '$.fields.state' END)='needs-login'))
                   OR (kind='harness.diagnostic'
                     AND json_extract(body, '$.fields.code')='provider-auth-expired')))
             ORDER BY subject",
        )?;
        statement
            .query_map([], desired_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn usage_fold_after(
            connection: &rusqlite::Connection,
            subject: &str,
            cached: &CachedUsageFold,
            through: u64,
        ) -> Result<Option<CachedUsageFold>> {
        let mut statement = connection.prepare_cached(
            "SELECT store_index, body, accepted_at_unix_ms
             FROM claims INDEXED BY claims_subject_kind_index
             WHERE subject=?1 AND kind='harness.usage' AND store_index>?2 AND store_index<=?3",
        )?;
        let mut rows = statement
            .query_map(params![subject, cached.through, through], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.sort_by(|a, b| accepted_order(&a.2).cmp(&accepted_order(&b.2)));
        let mut last = cached.last_accepted.as_str();
        for row in &rows {
            if accepted_order(&row.2) <= accepted_order(last) {
                return Ok(None);
            }
            last = &row.2;
        }
        let mut fold = cached.fold.clone();
        for (store_index, body, accepted) in &rows {
            fold.apply(*store_index, body, accepted, None)?;
        }
        Ok(Some(CachedUsageFold { through, last_accepted: last.to_owned(), fold }))
    }

    fn agent_last_activity_at(
            &self,
            agent: &str,
            incarnation: Option<&str>,
            snapshot_index: u64,
        ) -> Result<Option<u128>> {
        let connection = self.readers.get();
        let mut times = Vec::new();
        if let Some(incarnation) = incarnation {
            let local: Option<i64> = connection.prepare_cached(
                "SELECT observed_at_unix_ms FROM local_observations
                 WHERE subject=?1 AND kind='harness.timeline'
                   AND json_extract(body, '$.fields.incarnation_id')=?2
                   AND json_extract(body, '$.fields.entry_type') IN ('message','content','tool_call','tool_result')
                   AND after_store_index<=?3 ORDER BY id DESC LIMIT 1")?.query_row(
                /* sqlite-large-value: allow-for-now lv-h5fc8fb69eeb6 owner=st3_attention::store::seats TODO=trace-statement-and-value-representation */ params![agent, incarnation, snapshot_index], |row| row.get(0),
            ).optional()?;
            if let Some(time) = local {
                times.push(time.max(0) as u128);
            }
            let replicated: Option<String> = connection.prepare_cached(
                "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_timeline_activity_time_index
                 WHERE subject=?1 AND kind='harness.timeline'
                   AND json_extract(body, '$.fields.incarnation_id')=?2
                   AND json_extract(body, '$.fields.entry_type') IN ('message','content','tool_call','tool_result')
                   AND store_index<=?3
                 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1")?.query_row(
                /* sqlite-large-value: allow-for-now lv-hf2a20de8e0c1 owner=st3_attention::store::seats TODO=trace-statement-and-value-representation */ params![agent, incarnation, snapshot_index], |row| row.get(0),
            ).optional()?;
            times.extend(replicated.and_then(|time| time.parse::<u128>().ok()));
        }
        // Only the timestamp is returned, so equal acceptance times need no identity tie
        // break. Length then decimal text preserves the full unsigned numeric range.
        for query in [
            "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_actor_activity_time_index WHERE actor=?1 AND kind IN ('work.progress','work.submitted') AND store_index<=?2 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1",
            "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_message_from_activity_time_index WHERE kind='message.sent' AND json_extract(body, '$.fields.from')=?1 AND store_index<=?2 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1",
            "SELECT accepted_at_unix_ms FROM claims INDEXED BY claims_message_to_activity_time_index WHERE kind='message.sent' AND json_extract(body, '$.fields.to')=?1 AND store_index<=?2 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC LIMIT 1",
        ] {
            let time: Option<String> = connection
                .prepare_cached(/* sqlite-large-value: allow-for-now lv-h487953a7fcc3 owner=st3_attention::store::seats TODO=trace-statement-and-value-representation */ query)?
                .query_row(/* sqlite-large-value: allow-for-now lv-he1143029829d owner=st3_attention::store::seats TODO=trace-statement-and-value-representation */ params![agent, snapshot_index], |row| row.get(0))
                .optional()?;
            times.extend(time.and_then(|time| time.parse::<u128>().ok()));
        }
        Ok(times.into_iter().max())
    }

    fn seat_login(&self, desired: &DesiredSubject, host: &str, driver: &str) -> Result<String> {
        let account = match crate::accounts::harness_binding(&desired.desired).map(|b| b.binding) {
            Some(crate::accounts::Binding::Account(name)) => name,
            Some(crate::accounts::Binding::Pool(_)) if host == self.origin => {
                match self.seat_account_choice(&desired.subject)? {
                    Some(name) => name,
                    None => return Ok(format!("the login of {}", desired.subject)),
                }
            }
            Some(crate::accounts::Binding::Pool(_)) => {
                return Ok(format!("the login of {}", desired.subject));
            }
            None => return Ok(format!("the default {driver} login")),
        };
        // One declaration read by its key, not a scan of every declared subject.
        Ok(self
            .desired_subject_with_writer(&format!("account/{account}"))?
            .and_then(|(declared, _)| {
                crate::accounts::parse_account(&declared.subject, &declared.desired)
            })
            .and_then(|declared| declared.login_for(host).map(str::to_owned))
            .unwrap_or(account))
    }

    fn harness_login_episode_key(
            &self,
            subject: &str,
            incarnation: &str,
        ) -> Result<(Option<ClaimRecord>, String)> {
        let base = format!("claude-auth-expired:{subject}:{incarnation}");
        let Some(latest) = self.harness_auth_episode(subject, incarnation, None)? else {
            return Ok((None, base));
        };
        if latest.body["fields"]["code"] == "provider-auth-restored" {
            return Ok((None, format!("{base}:{}", latest.id)));
        }
        let key = match latest.body["fields"]["auth_attention_key"].as_str() {
            Some(key) => key.to_owned(),
            None => self
                .harness_auth_episode(subject, incarnation, Some("provider-auth-restored"))?
                .map_or(base.clone(), |restored| format!("{base}:{}", restored.id)),
        };
        Ok((Some(latest), key))
    }

    fn harness_auth_episode(
            &self,
            subject: &str,
            incarnation: &str,
            code: Option<&str>,
        ) -> Result<Option<ClaimRecord>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        let id: Option<String> = connection.query_row(/* sqlite-large-value: allow-for-now lv-h4231ccfa09ca owner=st3_attention::store::seats TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT id FROM claims INDEXED BY claims_harness_auth_diagnostic_index
             WHERE subject=?1 AND kind='harness.diagnostic'
               AND json_extract(body, '$.fields.incarnation_id')=?2
               AND json_extract(body, '$.fields.code') IN ('provider-auth-expired','provider-auth-restored')
               AND (?3 IS NULL OR json_extract(body, '$.fields.code')=?3)
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"), params![subject, incarnation, code], |row| row.get(0)).optional()?;
        id.map(|id| self.claim_by_id(&id))
            .transpose()
            .map(Option::flatten)
    }

    fn agent_person(&self, agent: &str) -> Result<Option<String>> {
        let mut actor = agent.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..16 {
            if actor.starts_with("person/") {
                return Ok(Some(actor));
            }
            if !seen.insert(actor.clone()) {
                return Ok(None);
            }
            let Some((desired, writer)) = self.desired_subject_with_writer(&actor)? else {
                return Ok(None);
            };
            if actor == agent {
                let owner = match crate::accounts::harness_binding(&desired.desired)
                    .map(|binding| binding.binding)
                {
                    Some(crate::accounts::Binding::Pool(person)) => Some(person),
                    Some(crate::accounts::Binding::Account(account)) => self
                        .desired_subject_with_writer(&format!("account/{account}"))?
                        .and_then(|(account, _)| {
                            crate::accounts::parse_account(&account.subject, &account.desired)
                        })
                        .and_then(|account| account.owner),
                    None => None,
                };
                if let Some(owner) = owner.filter(|owner| owner.starts_with("person/")) {
                    return Ok(Some(owner));
                }
            }
            let requester = desired
                .owner_run
                .as_deref()
                .map(|run| self.mission_run(run))
                .transpose()?
                .flatten()
                .map(|run| run.requester);
            let Some(next) = requester.or(writer) else {
                return Ok(None);
            };
            actor = next;
        }
        Ok(None)
    }

    fn current_harness_for_login(
            &self,
            subject: &str,
        ) -> Result<Option<crate::model::CurrentHarnessView>> {
        smallclaims::touched::note_read(|| subject.to_owned());
        smallclaims::touched::note_read(|| format!("actor:{subject}"));
        let connection = self.readers.get();
        current_harness_fold_at(&connection, subject, None, true, true)
    }
}

