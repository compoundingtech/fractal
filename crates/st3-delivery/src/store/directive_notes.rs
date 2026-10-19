//! The part of `store::directive_notes` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
pub use super::*;
use smallclaims::store::canonical;
use st3_schema::directive_notes::KIND;
pub use st3_store::store::directive_notes::*;
pub use st3_store::store::directive_notes::{DirectiveNote, create_schema, current, flush, project, rebuild, validate_publication};
#[cfg(test)]
mod tests;

pub trait DirectiveNotesStore {
    fn set_directive_note(&self, person: &str, actor: &str, text: Option<&str>, expires_at: Option<&str>) -> Result<Option<DirectiveNote>, St3Error>;
    fn directive_notes(&self, actor: &str) -> Result<Vec<DirectiveNote>, St3Error>;
    fn directive_note_people(&self, actor: &str) -> Result<BTreeSet<String>>;
}

impl DirectiveNotesStore for Store {

    fn set_directive_note(&self, person: &str, actor: &str, text: Option<&str>, expires_at: Option<&str>) -> Result<Option<DirectiveNote>, St3Error> {
        let mut fields = BTreeMap::from([("text".into(), json!(text))]);
        if let Some(at) = expires_at { fields.insert("expires_at".into(), json!(at)); }
        self.append_claim(&ClaimInput {
            subject: person.into(), kind: KIND.into(), actor: Some(actor.into()), fields,
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        })?;
        current(&self.readers.get(), person, now_ms()).map_err(internal)
    }

    fn directive_notes(&self, actor: &str) -> Result<Vec<DirectiveNote>, St3Error> {
        validate_actor(actor)?;
        self.read_snapshot(|_| {
            let people = self.directive_note_people(actor)?;
            let connection = self.readers.get();
            let now = now_ms();
            let mut notes = Vec::with_capacity(people.len());
            for person in people {
                if let Some(note) = current(&connection, &person, now)? { notes.push(note); }
            }
            Ok(notes)
        }).map_err(internal)
    }

    fn directive_note_people(&self, actor: &str) -> Result<BTreeSet<String>> {
        const MAX_HOPS: usize = 16;
        const MAX_RELATIONS: usize = 64;
        let mut people = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut work_relations = BTreeSet::new();
        let mut pending = VecDeque::from([(actor.to_owned(), 0)]);
        while let Some((actor, hops)) = pending.pop_front() {
            if actor.starts_with("person/") {
                if st3_schema::directive_notes::validate_actor(&actor, KIND, Some(&actor)).is_ok() {
                    people.insert(actor);
                }
                continue;
            }
            if !actor.starts_with("agent/") || hops >= MAX_HOPS || !seen.insert(actor.clone()) { continue; }
            anyhow::ensure!(seen.len() <= MAX_RELATIONS, "directive note ownership exceeds the bounded relation set");
            let Some((desired, author)) = self.desired_subject_with_writer(&actor)? else { continue; };
            let owner = match crate::accounts::harness_binding(&desired.desired).map(|binding| binding.binding) {
                Some(crate::accounts::Binding::Pool(person)) => Some(person),
                Some(crate::accounts::Binding::Account(account)) => self.desired_subject_with_writer(&format!("account/{account}"))?
                    .and_then(|(account, _)| crate::accounts::parse_account(&account.subject, &account.desired))
                    .and_then(|account| account.owner),
                None => None,
            };
            let requester = desired.owner_run.as_deref().map(|run| {
                smallclaims::touched::note_read(|| run.to_owned());
                self.readers.get().query_row(
                    "SELECT requester FROM mission_runs WHERE id=?1",
                    [run.trim_start_matches("mission-run/")],
                    |row| row.get::<_, String>(0),
                ).optional()
            }).transpose()?.flatten();
            // Union all relations. An account owner must not hide the person requesting work,
            // and requester precedence must not hide a different declaration author.
            for next in [owner, author, requester].into_iter().flatten() {
                pending.push_back((next, hops + 1));
            }
            // A top-level seat can also work for a run without being owned by that run.
            // Bound the indexed source rows before joining, including stale rows in the
            // budget: an oversized relation set fails closed rather than scanning history.
            smallclaims::touched::note_read(|| format!("actor:{actor}"));
            for column in ["assignee", "lease_owner"] {
                let connection = self.readers.get();
                let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h0132769160de owner=st3_store::store::directive_notes TODO=trace-statement-and-value-representation */ &format!(
                    "SELECT s.subject,r.requester FROM (
                         SELECT subject,run_id,generation_id FROM step_runs WHERE {column}=?1
                         AND status IN ('pending','ready','claimed','working','submitted','verifying','blocked','waiting-person')
                         LIMIT ?2
                     ) s LEFT JOIN mission_runs r ON r.id=s.run_id
                         AND r.current_generation_id=s.generation_id
                         AND r.status IN ('running','standing','blocked')
                         AND r.phase NOT LIKE 'cleanup-%' AND r.phase<>'terminal'"
                ))?;
                let rows = statement.query_map(params![actor, MAX_RELATIONS + 1], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?;
                for row in rows {
                    let (subject, requester) = row?;
                    work_relations.insert(subject);
                    anyhow::ensure!(work_relations.len() <= MAX_RELATIONS, "directive note work exceeds the bounded relation set");
                    if let Some(requester) = requester {
                        pending.push_back((requester, hops + 1));
                    }
                }
            }
        }
        Ok(people)
    }
}

