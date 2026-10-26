//! Server-authored helper launch authority, scoped to the selected creation declaration.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HelperLauncherBinding {
    pub launcher: String,
    pub person: Option<String>,
    pub harness: String,
    pub credential_home: Option<String>,
    pub incarnation: String,
    pub declaration: String,
}

impl HelperLauncherBinding {
    /// Replicated receipts carry attribution, never host-local credential authority.
    pub fn attribution(&self) -> Value {
        json!({
            "launcher": self.launcher, "person": self.person, "harness": self.harness,
            "incarnation": self.incarnation, "declaration": self.declaration,
            "credential_home_label": self.credential_home.as_deref()
                .and_then(|home| Path::new(home).file_name())
                .map(|name| name.to_string_lossy()),
        })
    }
}

fn declaration_digest(desired: &DesiredSubject) -> Result<String> {
    canonical_hash(&canonical_json_value(&json!({
        "desired": desired.desired, "member": desired.member,
    })))
}

pub(super) fn binding_at(connection: &Connection, subject: &str) -> Result<Option<HelperLauncherBinding>> {
    smallclaims::touched::note_read(|| subject.to_owned());
    let Some((desired, writer)) = desired_subject_with_writer_at(connection, subject)? else {
        return Ok(None);
    };
    if desired.kind != "agent" {
        return Ok(None);
    }
    let Some(key) = desired.member.as_ref().and_then(|member| member.tags.get("st3.client.create-key")) else {
        return Ok(None);
    };
    let Some(writer) = writer.filter(|writer| writer.starts_with("agent/")) else {
        return Ok(None);
    };
    let digest = declaration_digest(&desired)?;
    let body: Option<String> = connection.query_row(
        "SELECT body FROM local_observations
         WHERE subject=?1 AND kind='agent.launcher-bound'
           AND actor=?2 AND json_extract(body, '$.fields.launcher')=?2
           AND json_extract(body, '$.fields.creation_key')=?3
           AND json_extract(body, '$.fields.desired_digest')=?4
         ORDER BY id DESC LIMIT 1",
        params![subject, writer, key, digest], |row| row.get(0)).optional()?;
    let Some(body) = body else { return Ok(None) };
    let body: Value = serde_json::from_str(&body)?;
    let fields = &body["fields"];
    let text = |key: &str| -> Result<String> {
        fields[key].as_str().map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("helper launcher binding has no {key}"))
    };
    let declaration = connection.query_row("SELECT claim_id FROM desired WHERE subject=?1", [subject], |row| row.get(0))?;
    Ok(Some(HelperLauncherBinding {
        launcher: text("launcher")?,
        person: fields["person"].as_str().map(str::to_owned),
        harness: text("harness")?,
        credential_home: fields["credential_home"].as_str().map(str::to_owned),
        incarnation: text("incarnation")?,
        declaration,
    }))
}

fn lifetime_binding_at(connection: &Connection, subject: &str) -> Result<Option<(String, String)>> {
    let mut statement = connection.prepare(
        "SELECT after_store_index, body FROM local_observations
         WHERE subject=?1 AND kind='agent.launcher-bound'
           AND actor=json_extract(body, '$.fields.launcher')
         ORDER BY id DESC")?;
    let historical = statement.query_map([subject], |row| Ok((
        row.get::<_, i64>(0)?, row.get::<_, String>(1)?,
    )))?.collect::<Result<Vec<_>, _>>()?;
    if historical.is_empty() { return Ok(None) }
    // Only bindings whose creation actually reached this selected declaration's own lineage
    // count. A prepublication binding abandoned after a lost CAS must not fence an unrelated seat.
    let mut statement = connection.prepare(
        "WITH RECURSIVE lineage(id) AS (
             SELECT claim_id FROM desired WHERE subject=?1
             UNION
             SELECT predecessor.value FROM lineage
             JOIN claims ON claims.id=lineage.id
             JOIN json_each(claims.predecessors) predecessor
             WHERE claims.subject=?1 AND claims.kind='intent.desired'
         )
         SELECT claims.id, claims.actor, claims.body FROM lineage JOIN claims ON claims.id=lineage.id
         WHERE claims.subject=?1 AND claims.kind='intent.desired'")?;
    let lineage = statement.query_map([subject], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?,
    )))?.collect::<Result<Vec<_>, _>>()?;
    let mut creations = Vec::new();
    let mut last_stop = None;
    for (id, actor, body) in lineage {
        let desired: DesiredSubject = serde_json::from_str(&body)?;
        if desired.kind == "stop" {
            let key: i64 = connection.query_row("SELECT store_index FROM claims WHERE id=?1", [&id], |row| row.get(0))?;
            if last_stop.as_ref().is_none_or(|previous| previous < &key) { last_stop = Some(key); }
        } else if let Some(key) = desired.member.as_ref().and_then(|member| member.tags.get("st3.client.create-key")) {
            creations.push((actor, key.clone(), declaration_digest(&desired)?));
        }
    }
    for (after_store_index, body) in historical {
        let body: Value = serde_json::from_str(&body)?;
        let fields = &body["fields"];
        let Some(launcher) = fields["launcher"].as_str() else { continue };
        let applies = creations.iter().any(|(actor, key, digest)| actor.as_deref() == Some(launcher)
            && fields["creation_key"] == *key && fields["desired_digest"] == *digest);
        if !applies { continue }
        // Ordinary edits retain run ownership; an explicit later stop permits identity recreation.
        if last_stop.as_ref().is_some_and(|stop| *stop > after_store_index) {
            return Ok(None);
        }
        let incarnation = fields["incarnation"].as_str()
            .ok_or_else(|| anyhow::anyhow!("helper launcher binding has no incarnation"))?;
        return Ok(Some((launcher.into(), incarnation.into())));
    }
    Ok(None)
}

fn launcher_runtime(store: &Store, launcher: &str) -> Result<Option<(Value, String)>> {
    smallclaims::touched::note_read(|| launcher.to_owned());
    let connection = store.readers.get();
    let connection = connection.unchecked_transaction()?;
    let Some((desired, _)) = desired_subject_with_writer_at(&connection, launcher)? else { return Ok(None) };
    if desired.kind != "agent" { return Ok(None) }
    let (_, origin, conflict) = selected_actual_source_at(
        &connection, launcher, None, desired.member.as_ref().map(|member| member.host.as_str()),
    )?;
    if conflict || has_unknown_claim_at(&connection, launcher, None)?.is_some() {
        return Ok(None);
    }
    Ok(latest_actual_at(&connection, launcher, None)?.zip(origin))
}


impl Store {
    /// Return only the binding authored for the selected declaration by its authenticated launcher.
    /// Tags alone, another declaration, and client claims never establish launch authority.
    pub fn helper_launcher_binding(&self, subject: &str) -> Result<Option<HelperLauncherBinding>> {
        binding_at(&self.readers.get(), subject)
    }

    /// An edited helper remains run-owned even when it no longer matches its credential receipt.
    pub fn helper_launcher_has_lifetime_binding(&self, subject: &str) -> Result<bool> {
        Ok(lifetime_binding_at(&self.readers.get(), subject)?.is_some())
    }

    /// Capture references only, before publishing the declaration so reconciliation cannot race it.
    /// All authority fields come from the authenticated actor and current server runtime evidence.
    pub fn bind_helper_launcher(
        &self,
        desired: &DesiredSubject,
        creation_key: &str,
        launcher: &str,
        harness: &str,
    ) -> Result<()> {
        anyhow::ensure!(launcher.starts_with("agent/"), "helper launcher must be an authenticated agent");
        let (actual, launcher_host) = launcher_runtime(self, launcher)?
            .ok_or_else(|| anyhow::anyhow!("the launcher has no unambiguous current runtime"))?;
        let helper_host = desired.member.as_ref().map(|member| member.host.trim_start_matches("host/"));
        anyhow::ensure!(helper_host == Some(launcher_host.trim_start_matches("host/")),
            "cross-host helper credential inheritance is unsupported; create the helper on the launcher host");
        anyhow::ensure!(launcher_host.trim_start_matches("host/") == self.origin().trim_start_matches("host/"),
            "helper launcher authority is only available on its local host");
        let fields = actual.get("fields").unwrap_or(&actual);
        anyhow::ensure!(fields["status"] == "running", "the launcher runtime is not running");
        let incarnation = fields["incarnation_id"].as_str()
            .ok_or_else(|| anyhow::anyhow!("the launcher runtime has no incarnation"))?;
        let credential_home = if matches!(harness, "claude" | "codex") {
            let mut credential_home = None;
            for claim in self.observations_for(launcher, "runtime.action.succeeded")?.into_iter().rev() {
                // Actor-bearing start receipts may replicate. Only this daemon's local evidence
                // can grant access to an absolute credential directory.
                if !claim.id.starts_with(LOCAL_OBSERVATION_ID_PREFIX) { continue }
                let fields = claim.body.get("fields").unwrap_or(&claim.body);
                if fields["action"] == "start" && fields["incarnation_id"] == incarnation {
                    anyhow::ensure!(fields["harness"] == harness,
                        "credential inheritance requires the helper and launcher to use the same harness");
                    credential_home = fields["credential_home"].as_str().map(str::to_owned);
                    break;
                }
            }
            Some(credential_home.filter(|home| !home.is_empty() && Path::new(home).is_absolute())
                .ok_or_else(|| anyhow::anyhow!("the launcher has no credential directory launch evidence; relaunch it before creating a helper"))?)
        } else {
            None
        };
        let mut fields = BTreeMap::from([
            ("launcher".into(), json!(launcher)),
            ("harness".into(), json!(harness)),
            ("credential_home".into(), json!(credential_home)),
            ("incarnation".into(), json!(incarnation)),
            ("creation_key".into(), json!(creation_key)),
            ("desired_digest".into(), json!(declaration_digest(desired)?)),
        ]);
        if let Some(person) = self.agent_person(launcher)? {
            fields.insert("person".into(), json!(person));
        }
        self.append_claim(&ClaimInput {
            subject: desired.subject.clone(), kind: "agent.launcher-bound".into(),
            actor: Some(launcher.into()), fields, evidence: Vec::new(), expected_subject: None,
            idempotency_key: Some(format!("helper-launcher:{creation_key}")),
        })?;
        Ok(())
    }

    /// A helper cannot outlive the exact launcher runtime captured at creation.
    pub fn helper_launcher_alive(&self, subject: &str) -> Result<bool> {
        let binding = self.helper_launcher_binding(subject)?;
        let lifetime = match binding {
            Some(binding) => Some((binding.launcher, binding.incarnation)),
            None => lifetime_binding_at(&self.readers.get(), subject)?,
        };
        let Some((launcher, incarnation)) = lifetime else { return Ok(true) };
        let Some((actual, _)) = launcher_runtime(self, &launcher)? else { return Ok(false) };
        let fields = actual.get("fields").unwrap_or(&actual);
        Ok(fields["status"] == "running" && fields["incarnation_id"] == incarnation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(name: &str) -> NormalizedIntent {
        crate::graph::parse_intent(
            &format!("version 2\nagent \"{name}\" {{ workspace \"/tmp\"; harness \"codex\" {{}}; }}"),
            "node",
        ).unwrap()
    }

    fn publish(store: &Store, intent: &NormalizedIntent, key: &str, actor: Option<&str>) {
        let expected = intent.subjects.keys().map(|subject| (
            subject.clone(), store.selected_desired_token(subject).unwrap().into_iter().collect(),
        )).collect();
        store.apply_as(intent, &expected, key, actor).unwrap();
    }

    fn append(store: &Store, subject: &str, kind: &str, fields: Value) {
        store.append_claim(&ClaimInput {
            subject: subject.into(), kind: kind.into(), actor: None,
            fields: serde_json::from_value(fields).unwrap(), evidence: Vec::new(),
            expected_subject: None, idempotency_key: None,
        }).unwrap();
    }

    fn launcher(store: &Store, owner: Option<&str>) {
        publish(store, &declaration("launcher"), "launcher", owner);
        append(store, "agent/node.launcher", "runtime.observed",
            json!({"status":"running", "incarnation_id":"launcher-1"}));
        append(store, "agent/node.launcher", "runtime.action.succeeded",
            json!({"action":"start", "incarnation_id":"launcher-1", "harness":"codex",
                "credential_home":"/logins/codex"}));
    }

    fn helper() -> NormalizedIntent {
        let mut intent = declaration("helper");
        intent.subjects.values_mut().next().unwrap().member.as_mut().unwrap()
            .tags.insert("st3.client.create-key".into(), "create-helper".into());
        intent
    }

    #[test]
    fn shared_start_receipts_cannot_supply_credential_authority() {
        let store = Store::open_memory("node").unwrap();
        publish(&store, &declaration("launcher"), "launcher", Some("person/ada"));
        append(&store, "agent/node.launcher", "runtime.observed",
            json!({"status":"running", "incarnation_id":"launcher-1"}));
        store.append_claim(&ClaimInput {
            subject: "agent/node.launcher".into(), kind: "runtime.action.succeeded".into(),
            actor: Some("agent/node.launcher".into()),
            fields: serde_json::from_value(json!({"action":"start", "incarnation_id":"launcher-1",
                "harness":"codex", "credential_home":"/forged/login"})).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        let helper = helper();
        assert!(store.bind_helper_launcher(helper.subjects.values().next().unwrap(),
            "create-helper", "agent/node.launcher", "codex").is_err());
    }

    #[test]
    fn every_noncredential_harness_is_bound_to_the_launcher_lifetime() {
        for harness in ["omp", "pi", "opencode"] {
            let store = Store::open_memory("node").unwrap();
            launcher(&store, Some("person/ada"));
            let helper = helper();
            let desired = helper.subjects.values().next().unwrap();
            store.bind_helper_launcher(desired, "create-helper", "agent/node.launcher", harness).unwrap();
            publish(&store, &helper, "helper", Some("agent/node.launcher"));
            let binding = store.helper_launcher_binding(&desired.subject).unwrap().unwrap();
            assert_eq!(binding.harness, harness);
            assert_eq!(binding.credential_home, None);
            assert!(store.helper_launcher_alive(&desired.subject).unwrap());
            append(&store, "agent/node.launcher", "runtime.observed",
                json!({"status":"running", "incarnation_id":"launcher-2"}));
            assert!(!store.helper_launcher_alive(&desired.subject).unwrap());
        }
    }

    #[test]
    fn replicated_launcher_authority_cannot_grant_person_home_or_lifetime() {
        let source = Store::open_memory("peer").unwrap();
        append(&source, "host/peer", "transport.observed", json!({"status":"up"}));
        let store = Store::open_memory("node").unwrap();
        launcher(&store, Some("person/ada"));
        let mut helper = helper();
        let desired = helper.subjects.values().next().unwrap();
        publish(&store, &helper, "helper", Some("agent/node.launcher"));
        let mut replication = source.export_replication(0).unwrap();
        let claim = &mut replication.batches[0].claims[0];
        claim.subject = desired.subject.clone();
        claim.kind = "agent.launcher-bound".into();
        claim.actor = Some("agent/node.launcher".into());
        claim.body = json!({"fields":{
            "launcher":"agent/node.launcher", "person":"person/mallory", "harness":"codex",
            "credential_home":"/peer/stolen-login", "incarnation":"forged",
            "creation_key":"create-helper", "desired_digest":declaration_digest(desired).unwrap()
        }});
        claim.id = canonical_hash(&(&claim.batch_id, &claim.subject, &claim.kind, &claim.origin,
            claim.actor.as_deref(), &claim.body, &claim.predecessors)).unwrap();
        assert_eq!(classify_replicated_claim_with_registry(claim, fractal_schema::registry())
            .err().unwrap().code, "invalid-replicated-claim");
        assert_eq!(store.import_replication("peer", &replication).unwrap_err().code, "invalid-replicated-claim");
        assert!(store.claim_by_id(&replication.batches[0].claims[0].id).unwrap().is_none());
        assert!(store.helper_launcher_binding(&desired.subject).unwrap().is_none());
        assert_eq!(store.agent_person(&desired.subject).unwrap(), None);
        assert!(!store.helper_launcher_has_lifetime_binding(&desired.subject).unwrap());
        // A declaration edit must not discover the rejected claim via the historical path.
        helper.subjects.values_mut().next().unwrap().member.as_mut().unwrap().cwd = "/edited".into();
        publish(&store, &helper, "edited", Some("agent/node.launcher"));
        assert!(!store.helper_launcher_has_lifetime_binding("agent/node.helper").unwrap());
        assert_eq!(store.agent_person("agent/node.helper").unwrap(), None);
    }

    #[test]
    fn owned_and_unowned_launchers_capture_person_without_fallback() {
        for person in [Some("person/ada"), None] {
            let store = Store::open_memory("node").unwrap();
            launcher(&store, person);
            let helper = helper();
            let desired = helper.subjects.values().next().unwrap();
            store.bind_helper_launcher(desired, "create-helper", "agent/node.launcher", "codex").unwrap();
            assert!(store.helper_launcher_binding(&desired.subject).unwrap().is_none());
            publish(&store, &helper, "helper", Some("agent/node.launcher"));
            let binding = store.helper_launcher_binding(&desired.subject).unwrap().unwrap();
            assert_eq!(binding.person.as_deref(), person);
            assert_eq!(binding.launcher, "agent/node.launcher");
            assert_eq!(binding.incarnation, "launcher-1");
            assert_eq!(binding.credential_home.as_deref(), Some("/logins/codex"));
            assert_eq!(Some(binding.declaration), store.selected_desired_token(&desired.subject).unwrap());
            assert_eq!(store.agent_person(&desired.subject).unwrap().as_deref(), person);
            assert_eq!(agent_person_at(&store.readers.get(), &desired.subject).unwrap().as_deref(), person);
        }
    }

    #[test]
    fn credentials_are_harness_specific_and_launcher_incarnation_is_fenced() {
        let store = Store::open_memory("node").unwrap();
        launcher(&store, Some("person/ada"));
        let helper = helper();
        let desired = helper.subjects.values().next().unwrap();
        assert!(store.bind_helper_launcher(desired, "create-helper", "agent/node.launcher", "claude").is_err());
        store.bind_helper_launcher(desired, "create-helper", "agent/node.launcher", "codex").unwrap();
        publish(&store, &helper, "helper", Some("agent/node.launcher"));
        assert_eq!(store.helper_launcher_binding(&desired.subject).unwrap().unwrap().credential_home.as_deref(), Some("/logins/codex"));
        assert!(store.helper_launcher_alive(&desired.subject).unwrap());
        append(&store, "agent/node.launcher", "runtime.observed",
            json!({"status":"running", "incarnation_id":"launcher-2"}));
        assert!(!store.helper_launcher_alive(&desired.subject).unwrap());
        // Historical receipt attribution survives; it cannot authorize the replacement run.
        assert_eq!(store.helper_launcher_binding(&desired.subject).unwrap().unwrap().incarnation, "launcher-1");
        assert!(store.helper_launcher_alive("agent/not-a-helper").unwrap());
    }

    #[test]
    fn client_binding_claims_and_forged_declaration_metadata_cannot_grant_ownership() {
        let store = Store::open_memory("node").unwrap();
        launcher(&store, Some("person/ada"));
        let helper = helper();
        let desired = helper.subjects.values().next().unwrap();
        store.bind_helper_launcher(desired, "create-helper", "agent/node.launcher", "codex").unwrap();
        let claim = store.observations_for(&desired.subject, "agent.launcher-bound").unwrap().pop().unwrap();
        let forged = ClaimInput {
            subject: desired.subject.clone(), kind: claim.kind,
            actor: Some("agent/node.launcher".into()),
            fields: serde_json::from_value(claim.body["fields"].clone()).unwrap(),
            evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        };
        assert_eq!(store.append_client_claim(&forged).unwrap_err().code, "claim-write-forbidden");
        publish(&store, &helper, "forged-helper", Some("agent/attacker"));
        assert!(store.helper_launcher_binding(&desired.subject).unwrap().is_none());
        assert_eq!(store.agent_person(&desired.subject).unwrap(), None);
    }

    #[test]
    fn a_changed_creation_declaration_does_not_reuse_the_binding() {
        let store = Store::open_memory("node").unwrap();
        launcher(&store, Some("person/ada"));
        let mut helper = helper();
        store.bind_helper_launcher(helper.subjects.values().next().unwrap(), "create-helper", "agent/node.launcher", "codex").unwrap();
        publish(&store, &helper, "helper", Some("agent/node.launcher"));
        let desired = helper.subjects.values_mut().next().unwrap();
        desired.member.as_mut().unwrap().cwd = "/elsewhere".into();
        publish(&store, &helper, "replacement", Some("agent/node.launcher"));
        assert!(store.helper_launcher_binding("agent/node.helper").unwrap().is_none());
        assert!(store.helper_launcher_has_lifetime_binding("agent/node.helper").unwrap());
        append(&store, "agent/node.launcher", "runtime.observed",
            json!({"status":"stopped", "incarnation_id":"launcher-1"}));
        assert!(!store.helper_launcher_alive("agent/node.helper").unwrap());
    }

    #[test]
    fn credential_directory_references_cannot_be_inherited_across_hosts() {
        let store = Store::open_memory("node").unwrap();
        launcher(&store, Some("person/ada"));
        let mut helper = helper();
        let desired = helper.subjects.values_mut().next().unwrap();
        desired.member.as_mut().unwrap().host = "elsewhere".into();
        let error = store.bind_helper_launcher(desired, "create-helper", "agent/node.launcher", "codex").unwrap_err();
        assert!(error.to_string().contains("cross-host"));
        assert!(store.latest_claim(&desired.subject, Some("agent.launcher-bound")).unwrap().is_none());
    }
}

#[cfg(test)]
mod orphan_binding_tests {
    use super::*;

    #[test]
    fn abandoned_prepublication_binding_does_not_fence_an_unrelated_declaration() {
        let store = Store::open_memory("node").unwrap();
        store.append_claim(&ClaimInput {
            subject: "agent/node.unrelated".into(), kind: "agent.launcher-bound".into(),
            actor: Some("agent/gone".into()),
            fields: serde_json::from_value(json!({
                "launcher":"agent/gone", "harness":"codex", "credential_home":"/logins/gone",
                "incarnation":"ended", "creation_key":"never-published", "desired_digest":"never-published"
            })).unwrap(), evidence: Vec::new(), expected_subject: None, idempotency_key: None,
        }).unwrap();
        assert!(store.helper_launcher_alive("agent/node.unrelated").unwrap());
    }
}
