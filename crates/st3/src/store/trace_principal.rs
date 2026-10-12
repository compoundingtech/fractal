//! Trace attribution follows declaration authors only. Never use this for authorization.
use super::*;

impl Store {
    /// The accountable person declared once by a fleet root. This does not answer whose
    /// accounts, prompts, asks or directive notes a seat uses. Missing roots and cycles
    /// resolve to none; account owners and mission requesters are never inferred here.
    pub fn trace_principal(&self, agent: &str) -> Result<Option<String>> {
        self.read_snapshot(|_| {
            let mut actor = agent.to_owned();
            let mut seen = BTreeSet::new();
            while actor.starts_with("agent/") && seen.insert(actor.clone()) {
                let Some((desired, author)) = self.desired_subject_with_writer(&actor)? else {
                    return Ok(None);
                };
                if desired.kind != "agent" {
                    return Ok(None);
                }
                if let Some(person) = &desired.accountable {
                    // Admission enforces this; also verify the selected root at the read cut.
                    st3_schema::accountable::validate_claim(
                        &actor,
                        "intent.desired",
                        &serde_json::to_value(&desired)?,
                        author.as_deref(),
                    )?;
                    return Ok(Some(person.clone()));
                }
                let Some(next) = author else {
                    return Ok(None);
                };
                actor = next;
            }
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declare(store: &Store, name: &str, child: &str, author: Option<&str>, key: &str) {
        let source = format!("version 2\nagent \"{name}\" {{ {child} command \"true\"; }}");
        let intent = crate::graph::parse_internal_intent(&source, "alder").unwrap();
        let expected = intent
            .subjects
            .keys()
            .map(|subject| {
                (
                    subject.clone(),
                    intent_leaves_tx(&store.readers.get(), subject).unwrap(),
                )
            })
            .collect();
        store.apply_as(&intent, &expected, key, author).unwrap();
    }

    #[test]
    fn trace_principal_follows_only_declaring_seats_to_an_explicit_root() {
        let store = Store::open_memory("alder").unwrap();
        declare(
            &store,
            "example/root",
            "accountable \"person/avery\";",
            Some("person/avery"),
            "root",
        );
        declare(
            &store,
            "example/parent",
            "",
            Some("agent/example/root"),
            "parent",
        );
        declare(
            &store,
            "example/child",
            "",
            Some("agent/example/parent"),
            "child",
        );
        for agent in [
            "agent/example/root",
            "agent/example/parent",
            "agent/example/child",
        ] {
            assert_eq!(
                store.trace_principal(agent).unwrap().as_deref(),
                Some("person/avery")
            );
        }
        // The authority walk's 16-hop budget does not truncate a valid declaring-seat chain.
        for i in 0..20 {
            let author = if i == 0 {
                "agent/example/child".into()
            } else {
                format!("agent/example/depth{}", i - 1)
            };
            declare(
                &store,
                &format!("example/depth{i}"),
                "",
                Some(&author),
                &format!("depth{i}"),
            );
        }
        assert_eq!(
            store
                .trace_principal("agent/example/depth19")
                .unwrap()
                .as_deref(),
            Some("person/avery")
        );
        assert_eq!(
            store.agent_person("agent/example/depth19").unwrap(),
            None,
            "the existing authority walk retains its 16-hop limit"
        );
    }

    #[test]
    fn trace_principal_cycles_missing_roots_retirement_and_person_authors_give_none() {
        let store = Store::open_memory("alder").unwrap();
        declare(&store, "example/a", "", Some("agent/example/b"), "a");
        declare(&store, "example/b", "", Some("agent/example/a"), "b");
        declare(
            &store,
            "example/self",
            "",
            Some("agent/example/self"),
            "self",
        );
        declare(
            &store,
            "example/missing",
            "",
            Some("agent/example/absent"),
            "missing",
        );
        declare(&store, "example/person", "", Some("person/avery"), "person");
        declare(&store, "example/no-author", "", None, "no-author");
        for agent in [
            "agent/example/a",
            "agent/example/b",
            "agent/example/self",
            "agent/example/missing",
            "agent/example/person",
            "agent/example/no-author",
            "agent/example/absent",
            "person/avery",
        ] {
            assert_eq!(store.trace_principal(agent).unwrap(), None, "{agent}");
        }
        declare(
            &store,
            "example/root",
            "accountable \"person/avery\";",
            Some("person/avery"),
            "root",
        );
        declare(
            &store,
            "example/child",
            "",
            Some("agent/example/root"),
            "child",
        );
        declare(
            &store,
            "example/root",
            "",
            Some("person/avery"),
            "remove-accountable",
        );
        assert_eq!(store.trace_principal("agent/example/child").unwrap(), None);
        declare(
            &store,
            "example/root",
            "accountable \"person/avery\";",
            Some("person/avery"),
            "restore-root",
        );
        let stopped =
            crate::graph::parse_internal_intent("version 2\nstop \"agent/example/root\"", "alder")
                .unwrap();
        let expected = BTreeMap::from([(
            "agent/example/root".into(),
            intent_leaves_tx(&store.readers.get(), "agent/example/root").unwrap(),
        )]);
        store
            .apply_as(&stopped, &expected, "retire-root", Some("person/avery"))
            .unwrap();
        assert_eq!(store.trace_principal("agent/example/child").unwrap(), None);
    }

    #[test]
    fn trace_principal_never_uses_bound_accounts_or_mission_requesters() {
        let store = Store::open_memory("alder").unwrap();
        let source = r#"version 2
agent "example/bound" { harness "codex" { account-pool "person/robin"; } }
mission "example/work" state="ready" {
  goal "Run one step."
  agent "worker" { command "true"; }
  step "work" { assigned-to "agent/${ST_MISSION_RUN}/worker"; goal "Work."; }
}"#;
        let intent = crate::graph::parse_internal_intent(source, "alder").unwrap();
        store.apply_internal(&intent, "bound-and-mission").unwrap();
        assert_eq!(
            store
                .agent_person("agent/example/bound")
                .unwrap()
                .as_deref(),
            Some("person/robin")
        );
        assert_eq!(store.trace_principal("agent/example/bound").unwrap(), None);
        let run = store
            .create_mission_run(&MissionRunRequest {
                mission: "example/work".into(),
                revision: None,
                workspace: "/tmp".into(),
                requester: Some("person/avery".into()),
                mode: Some("run".into()),
                inputs: BTreeMap::new(),
                idempotency_key: "person-work".into(),
            })
            .unwrap();
        let declarations = intent.missions["example/work"]
            .declarations_kdl
            .as_ref()
            .unwrap();
        let mut execution =
            crate::graph::parse_execution_intent(declarations, "alder", &run.id).unwrap();
        for desired in execution.subjects.values_mut() {
            desired.owner_generation = Some(run.generation.clone());
        }
        let expected = execution
            .subjects
            .keys()
            .map(|subject| (subject.clone(), Vec::new()))
            .collect();
        store
            .apply_as(
                &execution,
                &expected,
                "materialize-worker",
                Some(&run.requester),
            )
            .unwrap();
        let seat = format!("agent/{}/worker", run.id);
        assert_eq!(
            store.agent_person(&seat).unwrap().as_deref(),
            Some("person/avery")
        );
        assert_eq!(store.trace_principal(&seat).unwrap(), None);
    }
}
