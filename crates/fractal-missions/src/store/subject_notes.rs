//! Current subject context uses mission assignment and authoritative runtime folds.
use super::*;
pub use fractal_store::store::subject_notes::SubjectNote;
use fractal_schema::subject_notes::KIND;

pub trait SubjectNotesStore {
    fn current_subject_note_incarnation(&self, target: &str) -> Result<Option<String>>;
    fn subject_note_agents(&self, target: &str) -> Result<BTreeSet<String>>;
    fn subject_note_readable(&self, note: &SubjectNote, reader: &str) -> Result<bool>;
    fn subject_note(&self, id: &str, reader: &str) -> Result<Option<SubjectNote>, St3Error>;
    fn list_subject_notes(&self, target: Option<&str>, author: Option<&str>, reader: &str) -> Result<Vec<SubjectNote>, St3Error>;
    fn set_subject_note(&self, id: &str, actor: &str, target: &str, text: Option<&str>, session: Option<&str>, expires_at: Option<&str>) -> Result<Option<SubjectNote>, St3Error>;
}

fn timestamp(milliseconds: u128) -> Result<String> {
    Ok(chrono::DateTime::from_timestamp_millis(i64::try_from(milliseconds)?)
        .context("note timestamp outside UTC range")?
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn current(store: &Store, id: &str, now: u128) -> Result<Option<SubjectNote>> {
    smallclaims::touched::note_read(|| id.to_owned());
    let connection = store.readers.get();
    let claim = connection.query_row(
        "SELECT c.id,c.store_index,c.batch_id,c.subject,c.kind,c.origin,c.actor,c.body,c.predecessors,c.accepted_at_unix_ms
         FROM subject_notes n JOIN claims c ON c.id=n.claim_id WHERE n.note=?1",
        [id], claim_from_row).optional()?;
    let Some(claim) = claim else { return Ok(None); };
    let fields = &claim.body["fields"];
    let Some(text) = fields["text"].as_str() else { return Ok(None); };
    let expires_at = fields["expires_at"].as_str();
    if let Some(at) = expires_at {
        let expires = fractal_schema::subject_notes::expiry(at).map_err(anyhow::Error::new)?;
        if i128::from(expires.timestamp()) * 1_000_000_000
            + i128::from(expires.timestamp_subsec_nanos())
            <= i128::try_from(now).unwrap_or(i128::MAX).saturating_mul(1_000_000) {
            return Ok(None);
        }
    }
    let target = fields["target"].as_str().context("note target missing")?;
    let session = fields["session"].as_str();
    if let Some(session) = session {
        if store.current_subject_note_incarnation(target)?.as_deref() != Some(session) {
            return Ok(None);
        }
    }
    let created: String = connection.query_row(
        &canonical_sql("SELECT accepted_at_unix_ms FROM claims WHERE subject=?1 AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM replica_records r WHERE r.claim_id=claims.id AND r.state='repaired')
        ORDER BY CANONICAL_ASC(claims) LIMIT 1"), [id], |row| row.get(0))?;
    Ok(Some(SubjectNote {
        id: id.to_owned(), target: target.to_owned(),
        author: fractal_schema::subject_notes::author(id).map_err(anyhow::Error::new)?.to_owned(),
        text: text.to_owned(), audience: fields["audience"].as_str().context("note audience missing")?.to_owned(),
        session: session.map(str::to_owned), created_at: timestamp(created.parse()?)?,
        updated_at: timestamp(claim.accepted_at_unix_ms)?,
        expires_at: expires_at.map(str::to_owned), revision: claim.id,
    }))
}

impl SubjectNotesStore for Store {
    /// Session scope requires the authoritative actual fold to report a running runtime.
    fn current_subject_note_incarnation(&self, target: &str) -> Result<Option<String>> {
        if !target.starts_with("agent/") { return Ok(None); }
        smallclaims::touched::note_read(|| target.to_owned());
        let connection = self.readers.get();
        let Some(authority) = Self::runtime_authority_on(&connection, target)? else { return Ok(None); };
        if authority.reachability != "reachable" { return Ok(None); }
        let Some(actual) = authority.actual else { return Ok(None); };
        if crate::model::projected_actual_status(Some(&actual)) != Some("running") { return Ok(None); }
        let fields = actual.get("fields").unwrap_or(&actual);
        Ok(fields.get("incarnation_id").or_else(|| fields.pointer("/facts/incarnation_id"))
            .and_then(Value::as_str).map(str::to_owned))
    }

    /// Current-generation, nonterminal steps are the only mission recipients.
    fn subject_note_agents(&self, target: &str) -> Result<BTreeSet<String>> {
        smallclaims::touched::note_read(|| target.to_owned());
        if target.starts_with("agent/") { return Ok(BTreeSet::from([target.to_owned()])); }
        let (predicate, key) = if let Some(id) = target.strip_prefix("mission/") {
            ("r.mission_id=?1", id)
        } else if let Some(id) = target.strip_prefix("mission-run/") {
            ("r.id=?1", id)
        } else if target.starts_with("step-run/") {
            ("s.subject=?1", target)
        } else { return Ok(BTreeSet::new()); };
        let connection = self.readers.get();
        let mut statement = connection.prepare(&format!(
            "SELECT s.subject,s.assignee,s.lease_owner FROM step_runs s JOIN mission_runs r ON r.id=s.run_id
             WHERE {predicate} AND r.current_generation_id=s.generation_id
             AND r.status IN ('running','standing','blocked')
             AND r.phase NOT LIKE 'cleanup-%' AND r.phase<>'terminal'
             AND s.status IN ('pending','ready','claimed','working','submitted','verifying','blocked','waiting-person')"))?;
        let rows = statement.query_map([key], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)))?;
        let mut agents = BTreeSet::new();
        for row in rows {
            let (subject, assignee, owner) = row?;
            smallclaims::touched::note_read(|| subject);
            for agent in [assignee, owner].into_iter().flatten().filter(|actor| actor.starts_with("agent/")) {
                agents.insert(agent);
            }
        }
        Ok(agents)
    }

    fn subject_note_readable(&self, note: &SubjectNote, reader: &str) -> Result<bool> {
        Ok(reader.starts_with("person/") || reader == note.author
            || (reader.starts_with("agent/") && self.subject_note_agents(&note.target)?.contains(reader)))
    }

    fn subject_note(&self, id: &str, reader: &str) -> Result<Option<SubjectNote>, St3Error> {
        validate_actor(reader)?;
        self.read_snapshot(|_| {
            let note = current(self, id, now_ms())?;
            match note {
                Some(note) if self.subject_note_readable(&note, reader)? => Ok(Some(note)),
                _ => Ok(None),
            }
        }).map_err(internal)
    }

    fn list_subject_notes(&self, target: Option<&str>, author: Option<&str>, reader: &str) -> Result<Vec<SubjectNote>, St3Error> {
        validate_actor(reader)?;
        self.read_snapshot(|_| {
            let connection = self.readers.get();
            let (sql, keys): (&str, Vec<&str>) = match (target, author) {
                (Some(target), Some(author)) => ("SELECT note FROM subject_notes WHERE target=?1 AND author=?2 ORDER BY head_key DESC", vec![target, author]),
                (Some(target), None) => ("SELECT note FROM subject_notes WHERE target=?1 ORDER BY head_key DESC", vec![target]),
                (None, Some(author)) => ("SELECT note FROM subject_notes WHERE author=?1 ORDER BY head_key DESC", vec![author]),
                (None, None) => ("SELECT note FROM subject_notes ORDER BY head_key DESC", vec![]),
            };
            let ids = connection.prepare(sql)?.query_map(rusqlite::params_from_iter(keys), |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let now = now_ms();
            let mut notes = Vec::new();
            for id in ids {
                if let Some(note) = current(self, &id, now)? {
                    if self.subject_note_readable(&note, reader)? { notes.push(note); }
                }
            }
            Ok(notes)
        }).map_err(internal)
    }

    fn set_subject_note(&self, id: &str, actor: &str, target: &str, text: Option<&str>,
        session: Option<&str>, expires_at: Option<&str>) -> Result<Option<SubjectNote>, St3Error> {
        if actor.starts_with("agent/") && actor != target {
            if !(target.starts_with("mission-run/") || target.starts_with("step-run/"))
                || !self.subject_note_agents(target).map_err(internal)?.contains(actor) {
                return Err(St3Error::new("note-target-access", "agents may note only themselves or assigned mission/step runs"));
            }
        }
        let mut fields = BTreeMap::from([
            ("target".into(), json!(target)), ("text".into(), json!(text)), ("audience".into(), json!("agent"))]);
        if let Some(session) = session { fields.insert("session".into(), json!(session)); }
        if let Some(at) = expires_at { fields.insert("expires_at".into(), json!(at)); }
        self.append_claim(&ClaimInput {
            subject: id.into(), kind: KIND.into(), actor: Some(actor.into()), fields,
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        })?;
        // Currency still applies to the returned projection after an archive or expiry.
        self.subject_note(id, actor)
    }
}

#[cfg(test)]
mod tests;
