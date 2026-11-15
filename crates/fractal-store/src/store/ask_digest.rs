//! Daily informational notices over an incremental, arrival-aware resolution inventory.
//! Coverage and included resolution identities live in authenticated immutable notice history.
use super::*;
use crate::ask_digest_clock::{ask_digest_policy, ask_digest_window};
use crate::external_sessions::timestamp;
use fractal_schema::notice;

const OVERRIDE_WINDOW_MS: u128 = 7 * 24 * 60 * 60_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AskDigestPass {
    pub changed: bool,
    pub next_deadline: Option<u128>,
}

impl Store {
    /// Every writer hold discovers at most 128 claims, indexes one payload page, or stages
    /// 128 resolutions. Immediate wakes are bounded-work continuations, not idle polling.
    pub fn reconcile_ask_digests_at(&self, now: u128) -> Result<AskDigestPass> {
        self.connection.batched(|tx| -> Result<AskDigestPass> {
            let Ok((owner, _)) = notice_authority::owner(tx) else { return Ok(AskDigestPass::default()) };
            if owner.member != self.origin { return Ok(AskDigestPass::default()); }
            let owner_claim = current_desired_row_tx(tx, crate::notice_owner::SUBJECT)?
                .context("notice owner disappeared")?.claim_id;
            ask_digest_cache::initialize(tx)?;
            let (at, timezone) = ask_digest_policy(tx)?;
            let window = ask_digest_window(now, &timezone, &at)?;
            let waiting = AskDigestPass { changed: false, next_deadline: Some(window.next_deadline) };
            let continued = AskDigestPass { changed: true, next_deadline: Some(now) };
            if reclaim_previous_owner(tx, &owner_claim)? || ask_digest_cache::ingest(tx)?
                || ask_digest_cache::index_source_page(tx)? { return Ok(continued); }

            let draft = tx.query_row("SELECT person,source_id,date,covered_through,payload
                FROM local_ask_digest_draft WHERE owner=?1 ORDER BY person LIMIT 1", [&owner_claim], |r| {
                Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?))
            }).optional()?;
            let (person, source_id, date, end, previous) = if let Some((person,id,date,end,payload)) = draft {
                // Another authenticated historical emission may have arrived during a handover.
                if source_exists(tx, &id)? {
                    if reset_staged_page(tx, &id)? { return Ok(continued); }
                    tx.execute(/* sqlite-large-value: allow-for-now lv-ha34874ede07a owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_draft WHERE person=?1", [&person])?;
                    return Ok(continued);
                }
                (person,id,date,u128::try_from(end)?,Some(serde_json::from_str::<Value>(&payload)?))
            } else {
                const SCAN_KEY: &str = "ask_digest_person_scan_v1";
                let generation: Option<String> = tx.query_row(
                    "SELECT value FROM meta WHERE key='ask_digest_pending_generation_v1'", [], |r| r.get(0)).optional()?;
                let token = format!("{owner_claim}/{}/{timezone}/{at}/{}", generation.as_deref().unwrap_or("0"), window.date);
                let saved: Option<String> = tx.query_row("SELECT value FROM meta WHERE key=?1", [SCAN_KEY], |r| r.get(0)).optional()?;
                let (saved_token, mut after_person, mut wake): (String,String,u128) = saved
                    .map(|value| serde_json::from_str(&value)).transpose()?.unwrap_or_default();
                if saved_token != token || wake <= now {
                    after_person.clear();
                    wake = window.next_deadline;
                }
                // Seek directly beyond each person's entire pending range. A future person
                // must not block a different person's missed digest; each continuation examines
                // one person, retaining the earliest future wake across the bounded traversal.
                let candidate: Option<(String,i64)> = tx.query_row(
                    "SELECT person,resolved_at FROM local_ask_digest_pending INDEXED BY local_ask_digest_due
                     WHERE staged_source IS NULL AND person>?1 ORDER BY person,resolved_at,resolution_id LIMIT 1", [&after_person],
                    |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
                let Some((person, earliest)) = candidate else {
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h167f521211c1 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key=?1", [SCAN_KEY])?;
                    return Ok(AskDigestPass { next_deadline: Some(wake), ..waiting });
                };
                let covered: Option<i64> = tx.query_row("SELECT covered_through FROM local_ask_digest_sources
                    WHERE person=?1 ORDER BY covered_through DESC LIMIT 1", [&person], |r| r.get(0)).optional()?;
                // A policy reload selects only the next end. Previously covered instants are
                // never recalculated under the new timezone; late replicas remain unreported IDs.
                let after = u128::try_from(covered.map_or(earliest, |covered| covered.max(earliest)))?;
                let mut end = next_boundary(after, &timezone, &at)?;
                let (id, date) = loop {
                    let date = ask_digest_window(end, &timezone, &at)?.date.to_string();
                    let id = format!("{}{person}/{date}", notice::DIGEST_SOURCE_PREFIX);
                    if !source_exists(tx, &id)? { break (id,date); }
                    end = next_boundary(end, &timezone, &at)?;
                };
                if end > now {
                    wake = wake.min(end);
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h02042a7dc741 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
                        params![SCAN_KEY,serde_json::to_string(&(token,person,wake))?])?;
                    return Ok(continued);
                }
                (person,id,date,end,None)
            };
            let ids = tx.prepare_cached("SELECT resolution_id,episode FROM local_ask_digest_pending INDEXED BY local_ask_digest_due
                WHERE person=?1 AND staged_source IS NULL AND resolved_at<?2 ORDER BY resolved_at,resolution_id LIMIT ?3")?
                .query_map(params![person,i64::try_from(end)?,ask_digest_cache::PAGE], |r| {
                    Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))
                })?.collect::<rusqlite::Result<Vec<_>>>()?;
            if !ids.is_empty() {
                let mut rows = Vec::with_capacity(ids.len());
                for (resolution, episode) in ids {
                    if let Some(row) = resolution_row(tx, &resolution, &episode, now)? {
                        rows.push(Ok(row));
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h5ba36b286ad6 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_ask_digest_pending SET staged_source=?2,staged_owner=?3 WHERE resolution_id=?1",
                            params![resolution,source_id,owner_claim])?;
                    } else {
                        tx.execute(/* sqlite-large-value: allow-for-now lv-h1541a6119949 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_pending WHERE resolution_id=?1", [&resolution])?;
                    }
                }
                let payload = if rows.is_empty() {
                    previous.unwrap_or_else(|| json!({"digest":[],"digest_count":"0"}))
                } else {
                    ask_digest_payload::append(tx, &self.origin, previous.as_ref(), rows)?
                };
                tx.execute(/* sqlite-large-value: allow-for-now lv-h6c93e6f5976d owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO local_ask_digest_draft VALUES(?1,?2,?3,?4,?5,?6)",
                    params![person,source_id,date,i64::try_from(end)?,owner_claim,serde_json::to_string(&payload)?])?;
                return Ok(continued);
            }
            let Some(mut payload) = previous else { return Ok(waiting) };
            if payload["digest_count"] == "0" {
                tx.execute(/* sqlite-large-value: allow-for-now lv-hd84171209f25 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_draft WHERE person=?1", [&person])?;
                return Ok(continued);
            }
            notice_authority::ensure_digest_registrations_tx(tx, &self.origin)?;
            payload["host"] = json!(format!("host/{}",self.origin));
            payload["covered_through"] = json!(timestamp(end));
            let suffix = format!("digest/{person}/{date}");
            let source = append_notice(tx, &self.origin, &source_id, "custom.attention.notice-created", json!({
                "schema_version":notice::PAYLOAD_VERSION,"dedupe_key":suffix,"resource":"resource/attention",
                "recipient":person,"intent":"fyi","urgency":"info","requested":false,
                "summary":format!("{} asks auto-resolved",payload["digest_count"].as_str().context("digest count missing")?),
                "links":[],"source":payload,"created_at":timestamp(end)
            }), &[])?;
            append_notice(tx, &self.origin, &format!("{}{suffix}",notice::SNAPSHOT_PREFIX),
                "custom.attention.snapshot-created", json!({
                    "source_id":source_id,"creation_claim":source.id,"producer":notice::RUNTIME_ACTOR,
                    "recipient":person,"runtime":format!("host/{}",self.origin),
                    "policy":{"disposition":"deliver"},"delivery":[]
                }), &[source.id])?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h226a33d52146 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_draft WHERE person=?1", [&person])?;
            Ok(continued)
        }).map_err(anyhow::Error::msg)?
    }
}

fn next_boundary(after: u128, timezone: &str, at: &str) -> Result<u128> {
    Ok(ask_digest_window(after, timezone, at)?.next_deadline)
}

fn source_exists(connection: &Connection, source: &str) -> Result<bool> {
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM local_ask_digest_sources WHERE source_id=?1)",
        [source], |r| r.get(0))?)
}

fn reset_staged_page(tx: &Transaction<'_>, source: &str) -> Result<bool> {
    Ok(tx.execute(/* sqlite-large-value: allow-for-now lv-h6e6217b2f6e1 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_ask_digest_pending SET staged_source=NULL,staged_owner=NULL WHERE resolution_id IN
        (SELECT resolution_id FROM local_ask_digest_pending WHERE staged_source=?1 LIMIT ?2)",
        params![source,ask_digest_cache::PAGE])? != 0)
}

fn reclaim_previous_owner(tx: &Transaction<'_>, current: &str) -> Result<bool> {
    const KEY: &str = "ask_digest_cache_owner_v1";
    let previous: Option<String> = tx.query_row("SELECT value FROM meta WHERE key=?1", [KEY], |r| r.get(0)).optional()?;
    if previous.as_deref() == Some(current) { return Ok(false); }
    if let Some(previous) = previous {
        let changed = tx.execute(/* sqlite-large-value: allow-for-now lv-h391171dd8797 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_ask_digest_pending SET staged_source=NULL,staged_owner=NULL WHERE resolution_id IN
            (SELECT resolution_id FROM local_ask_digest_pending WHERE staged_owner=?1 LIMIT ?2)",
            params![previous,ask_digest_cache::PAGE])?;
        if changed != 0 { return Ok(true); }
        tx.execute(/* sqlite-large-value: allow-for-now lv-h4e5cdd0bdcce owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_ask_digest_draft WHERE owner=?1", [&previous])?;
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-hf26fcae76171 owner=fractal_store::store::ask_digest TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![KEY,current])?;
    Ok(false)
}

fn append_notice(tx: &Transaction<'_>, origin: &str, subject: &str, kind: &str,
    fields: Value, predecessors: &[String]) -> Result<ClaimRecord> {
    let input = ClaimInput { subject:subject.into(),kind:kind.into(),actor:Some(notice::RUNTIME_ACTOR.into()),
        fields:serde_json::from_value(fields)?,evidence:Vec::new(),expected_subject:None,idempotency_key:None };
    let fields = custom::prepare(tx, origin, &input)?.context("notice registration is missing")?;
    append_claim_tx(tx, origin, subject, kind, Some(notice::RUNTIME_ACTOR), &json!({"fields":fields}), predecessors, None)
}

fn resolution_row(connection: &Connection, resolution: &str, episode: &str, now: u128) -> Result<Option<Value>> {
    let ask = claim_by_id_tx(connection, episode)?.context("digest ask disappeared")?;
    let applied = claim_by_id_tx(connection, resolution)?.context("digest resolution disappeared")?;
    let person = ask.body["fields"]["person"].as_str().context("digest ask person missing")?;
    let overridden: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM claims answer WHERE subject=?1
        AND kind IN ('work.person-override','work.person-done','work.person-cancelled')
        AND actor=?2 AND json_extract(body,'$.fields.episode')=?3)",params![ask.subject,person,episode],|r|r.get(0))?;
    if overridden || ask.kind != "work.person-asked" || applied.origin != ask.origin || applied.subject != ask.subject {
        return Ok(None);
    }
    let mut row = json!({"resolution_id":resolution,"title":ask.body["fields"]["title"],
        "requester_id":ask.actor.as_deref().unwrap_or_default(),"resolved_at":timestamp(applied.accepted_at_unix_ms),
        "target_id":ask.subject,"episode":episode});
    if let Some(answer) = applied.body["fields"].get("answer").filter(|a| !a.is_null()) {
        row["default_answer"] = json!({"id":answer["id"],"label":answer["label"]});
        if now <= applied.accepted_at_unix_ms.saturating_add(OVERRIDE_WINDOW_MS) {
            row["action_parameters"] = json!({"work.override":{"target_id":ask.subject,"episode":episode}});
        }
    }
    Ok(Some(row))
}

#[cfg(test)]
#[path = "ask_digest_tests.rs"]
mod tests;
