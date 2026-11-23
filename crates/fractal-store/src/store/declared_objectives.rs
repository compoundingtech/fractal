//! Declared objectives (docs/st3/objectives.md): each `objective/NAME` declaration, and the member
//! an objective's owner is declared on.

use super::*;
use crate::objective::Objective;

/// One current `objective/NAME` declaration. A body the kernel reader refuses is kept with its
/// error, so the objective's row says so instead of disappearing.
#[derive(Clone, Debug)]
pub struct DeclaredObjective {
    pub subject: String,
    pub revision: String,
    pub claim_id: String,
    pub objective: std::result::Result<Objective, String>,
}

impl DeclaredObjective {
    /// The objective's name: its subject after `objective/`.
    pub fn name(&self) -> &str {
        self.subject.strip_prefix("objective/").unwrap_or(&self.subject)
    }
}

pub fn declared_objectives_at(connection: &Connection) -> Result<Vec<DeclaredObjective>> {
    let mut statement = connection.prepare_cached(
        "SELECT subject, revision, claim_id, body FROM desired WHERE kind='objective' ORDER BY subject",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
    })?;
    let mut objectives = Vec::new();
    for row in rows {
        let (subject, revision, claim_id, body) = row?;
        let objective = parse_objective(&body);
        objectives.push(DeclaredObjective { subject, revision, claim_id, objective });
    }
    Ok(objectives)
}

/// Recorder-owned declaration inputs. A minute cut advances measurements, not declarations:
/// retain parsed unchanged objectives and apply the writer's committed desired-key deltas.
#[derive(Default)]
pub struct DeclaredObjectiveInputs {
    generation: Option<u64>,
    declarations: BTreeMap<String, DeclaredObjective>,
}

#[derive(Default)]
struct InputWork {
    full_reads: std::sync::atomic::AtomicU64,
    parsed: std::sync::atomic::AtomicU64,
}

impl DeclaredObjectiveInputs {
    pub fn refresh(&mut self, store: &Store) -> Result<()> {
        use std::sync::atomic::Ordering::Relaxed;
        let (generation, changed) = store.desired_changes_since(self.generation);
        let work = store.extension::<InputWork>();
        match changed {
            None => {
                let declarations = store.declared_objectives()?;
                work.full_reads.fetch_add(1, Relaxed);
                work.parsed.fetch_add(declarations.len() as u64, Relaxed);
                self.declarations = declarations.into_iter()
                    .map(|declaration| (declaration.subject.clone(), declaration)).collect();
            }
            Some(changed) if !changed.is_empty() => {
                let reader = store.readers.get();
                let mut statement = reader.prepare_cached(
                    "SELECT subject,revision,claim_id,body FROM desired WHERE subject=?1 AND kind='objective'",
                )?;
                // Do not publish a new cursor or partial inputs when any selected read fails.
                let mut replacements = Vec::with_capacity(changed.len());
                for subject in changed {
                    let row = statement./* sqlite-large-value: allow-for-now lv-h059cdabeb1e9 owner=fractal_store::store::declared_objectives TODO=trace-statement-and-value-representation */ query_row([&subject], |row| Ok((
                        row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    ))).optional()?;
                    let declaration = row.map(|(subject, revision, claim_id, body)| {
                        work.parsed.fetch_add(1, Relaxed);
                        DeclaredObjective { subject, revision, claim_id, objective: parse_objective(&body) }
                    });
                    replacements.push((subject, declaration));
                }
                for (subject, declaration) in replacements {
                    if let Some(declaration) = declaration {
                        self.declarations.insert(subject, declaration);
                    } else {
                        self.declarations.remove(&subject);
                    }
                }
            }
            Some(_) => {}
        }
        self.generation = Some(generation);
        Ok(())
    }

    pub fn declarations(&self) -> impl Iterator<Item = &DeclaredObjective> + Clone {
        self.declarations.values()
    }
}

fn parse_objective(body: &str) -> std::result::Result<Objective, String> {
    serde_json::from_str::<Value>(body)
        .map_err(|error| format!("its body is not JSON: {error}"))
        .and_then(|desired| crate::objective::parse(&desired)
            .map_err(|error| format!("{}: {}", error.code, error.message)))
}

impl Store {
    pub fn declared_objectives(&self) -> Result<Vec<DeclaredObjective>> {
        declared_objectives_at(&self.readers.get())
    }

    pub fn slo_history_input_work(&self) -> Value {
        use std::sync::atomic::Ordering::Relaxed;
        let work = self.extension::<InputWork>();
        json!({
            "declaration_full_reads": work.full_reads.load(Relaxed),
            "declarations_parsed": work.parsed.load(Relaxed),
        })
    }

    /// The member `agent` is declared on, when it is a current agent declaration.
    pub fn agent_host(&self, agent: &str) -> Result<Option<String>> {
        Ok(self
            .readers
            .get()
            .prepare_cached(
                "SELECT coalesce(
                    (SELECT nullif(json_extract(value,'$.arguments[0]'),'local')
                     FROM json_each(desired.body,'$.children')
                     WHERE json_extract(value,'$.name')='host'),
                    json_extract(member,'$.host'))
                 FROM desired WHERE subject=?1 AND kind='agent'",
            )?
            .query_row(/* sqlite-large-value: allow-for-now lv-hbb373c1f9535 owner=fractal_store::store::declared_objectives TODO=trace-statement-and-value-representation */ [agent], |row| row.get::<_, Option<String>>(0))
            .optional()?
            .flatten())
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn history_cuts_parse_only_changed_declarations() {
        let store = Store::open_memory("test").unwrap();
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            append_claim_tx(&tx, &store.origin, "resource/source", "resource.observed",
                None, &json!({"fields": {}}), &[], None).unwrap();
            tx.commit().unwrap();
        }
        for index in 0..45 {
            store.connection.write().execute(
                "INSERT INTO desired(subject,kind,revision,claim_id,body)
                 VALUES(?1,'objective','initial',(SELECT id FROM claims LIMIT 1),'{}')",
                [format!("objective/{index:02}")],
            ).unwrap();
        }
        let mut inputs = DeclaredObjectiveInputs::default();
        inputs.refresh(&store).unwrap();
        let initial = store.slo_history_input_work();
        assert_eq!(initial["declaration_full_reads"], 1);
        assert_eq!(initial["declarations_parsed"], 45);
        let first = inputs.declarations().next().unwrap() as *const DeclaredObjective;
        for _ in 0..10 {
            inputs.refresh(&store).unwrap();
        }
        drop(store.connection.write());
        inputs.refresh(&store).unwrap();
        assert_eq!(store.slo_history_input_work(), initial,
            "idle history cuts and no-op writer loans must not parse declarations");
        assert_eq!(first, inputs.declarations().next().unwrap() as *const DeclaredObjective);
        {
            let mut writer = store.connection.write();
            let tx = writer.transaction().unwrap();
            append_claim_tx(&tx, &store.origin, "resource/unrelated", "resource.observed",
                None, &json!({"fields": {}}), &[], None).unwrap();
            tx.commit().unwrap();
        }
        inputs.refresh(&store).unwrap();
        assert_eq!(store.slo_history_input_work(), initial,
            "an unrelated claim must not rebuild SLO declaration inputs");
        store.connection.write().execute(
            "UPDATE desired SET revision='changed',body='invalid-json' WHERE subject='objective/23'", [],
        ).unwrap();
        inputs.refresh(&store).unwrap();
        assert_eq!(store.slo_history_input_work()["declarations_parsed"], 46);
        assert_eq!(store.slo_history_input_work()["declaration_full_reads"], 1);
        let changed = inputs.declarations().find(|entry| entry.subject == "objective/23").unwrap();
        assert_eq!(changed.revision, "changed");
        assert!(changed.objective.as_ref().unwrap_err().contains("not JSON"));
        store.connection.write().execute("DELETE FROM desired WHERE subject='objective/23'", []).unwrap();
        inputs.refresh(&store).unwrap();
        assert_eq!(inputs.declarations().count(), 44);
        assert_eq!(store.slo_history_input_work()["declarations_parsed"], 46,
            "removing an objective must not reparse any retained objective");
    }
}
