//! Exact durable opening authority for discussion reads and retained writes.
//! A current assignment alone, an episode-shaped string, labels and about links prove nothing.
use super::*;
use super::thread_metadata::request_history::SourceEpisode;

/// Transaction-local proof. Only this module can construct it; never deserialize authority.
pub struct QualifiedDiscussionSource {
    item: AttentionItemView,
    evidence: Vec<String>,
    current_notice: bool,
}

impl QualifiedDiscussionSource {
    pub fn item(&self) -> &AttentionItemView { &self.item }
    pub fn evidence(&self) -> &[String] { &self.evidence }
    pub(super) fn is_current_notice(&self) -> bool { self.current_notice }
    pub(super) fn into_resolved(self) -> thread_metadata::inbox_source::ResolvedSource {
        thread_metadata::inbox_source::ResolvedSource { item: self.item, evidence: self.evidence }
    }
}

pub fn qualify(connection: &Connection, person: &str, key: &SourceEpisode, at: u128)
    -> Result<Option<QualifiedDiscussionSource>, FractalError>
{
    Ok(qualify_batch(connection, person, std::slice::from_ref(key), at)?.pop().flatten())
}

/// One keyed opening/current-policy lookup for the entire input, preserving input order.
/// Reserved notice proofs additionally reuse the native registration/owner-chain validator.
/// No per-row live Inbox fold, and no N calls to the single-source qualifier.
pub fn qualify_batch(connection: &Connection, person: &str, keys: &[SourceEpisode], at: u128)
    -> Result<Vec<Option<QualifiedDiscussionSource>>, FractalError>
{
    let mut result: Vec<Option<QualifiedDiscussionSource>> = (0..keys.len()).map(|_| None).collect();
    if keys.is_empty() { return Ok(result); }
    let person = match identities::resolve_reference_conn(connection, person) {
        Ok(person) => person,
        Err(error) if error.code == "lifecycle-unavailable" => return Ok(result),
        Err(error) => return Err(error),
    };
    if !person.starts_with("person/") { return Ok(result); }
    let input = serde_json::to_string(keys).map_err(internal)?;
    let snapshots = if keys.iter().any(|key| key.source_id.starts_with(fractal_schema::notice::SOURCE_PREFIX)) {
        notice_snapshots(connection, &input)?
    } else { BTreeMap::new() };
    let reviewer = human_review_reassignment::reviewer_sql("opening");
    let before = canonical::after_sql("opening", "earlier");
    let sql = canonical_sql(&format!(
        "SELECT CAST(input.key AS INTEGER), opening.id,opening.store_index,opening.batch_id,
         opening.subject,opening.kind,opening.origin,opening.actor,opening.body,
         opening.predecessors,opening.accepted_at_unix_ms, step.assignee,
         source.body,registration.manifest,registration.state,
         CASE WHEN opening.kind='gate.requested' THEN {reviewer} END,
         NOT EXISTS(SELECT 1 FROM claims earlier WHERE earlier.subject=opening.subject
           AND earlier.kind=opening.kind AND {before}
           AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=earlier.id AND state='repaired')),
         registration.claim_id, step.run_id
         FROM json_each(?1) input
         LEFT JOIN custom_sources source ON source.subject=json_extract(input.value,'$.source_id')
         LEFT JOIN custom_registrations registration ON registration.hash=source.registration
         JOIN claims opening ON opening.id=CASE
           WHEN json_type(registration.manifest,'$.attention')='object' THEN (
             SELECT candidate.id FROM claims candidate
             WHERE candidate.subject=source.subject
               AND candidate.kind=json_extract(registration.manifest,'$.creation_kind')
               AND json_extract(candidate.body,'$.fields._registration')=source.registration
               AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=candidate.id AND state='repaired')
             ORDER BY CANONICAL_ASC(candidate) LIMIT 1)
           ELSE json_extract(input.value,'$.episode') END
         LEFT JOIN step_runs step ON step.subject=json_extract(input.value,'$.source_id')
         WHERE NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=opening.id AND state='repaired')"
    ));
    let mut statement = connection.prepare(&sql).map_err(internal)?;
    let mut rows = statement.query([input]).map_err(internal)?;
    while let Some(row) = rows.next().map_err(internal)? {
        let index: usize = row.get(0).map_err(internal)?;
        let key = &keys[index];
        if key.source_id.is_empty() || key.source_id.len() > 512 || key.episode.is_empty() || key.episode.len() > 512 { continue; }
        // claim_from_row expects column zero, so decode the opening's shifted columns here.
        let body: String = row.get(8).map_err(internal)?;
        let fields: Value = serde_json::from_str(&body).map_err(internal)?;
        let fields = &fields["fields"];
        let subject: String = row.get(4).map_err(internal)?;
        let kind: String = row.get(5).map_err(internal)?;
        let opening_at: String = row.get(10).map_err(internal)?;
        let opening_at: u128 = opening_at.parse().map_err(internal)?;
        if opening_at > at { continue; }
        let same_person = |recipient| matches_person(connection, &person, recipient);
        let canonical: bool = row.get(16).map_err(internal)?;
        let opening_id: String = row.get(1).map_err(internal)?;
        let mut evidence = vec![opening_id.clone()];
        let mut current_notice = false;
        let mut item = if kind == "work.person-asked" && subject == key.source_id && canonical {
            let assignee: Option<String> = row.get(11).map_err(internal)?;
            if !same_person(fields["person"].as_str())? || !same_person(assignee.as_deref())? { continue; }
            let mut item = source_item(key, &person, "person-step", fields, opening_at)?;
            let actor: Option<String> = row.get(7).map_err(internal)?;
            item.conversation = actor.as_deref().and_then(attention_snapshot::agent);
            item.requester_id = actor;
            item
        } else if kind == "gate.requested" && fields["owner"] == key.source_id {
            let reviewer: Option<String> = row.get(15).map_err(internal)?;
            if !same_person(fields["reviewer"].as_str())? || !same_person(reviewer.as_deref())? { continue; }
            let mut item = source_item(key, &person, "human-gate", fields, opening_at)?;
            item.review_mode = fields["mode"].as_str().map(str::to_owned);
            item.mission_run = if key.source_id.starts_with("mission-run/") {
                Some(key.source_id.clone())
            } else {
                row.get::<_, Option<String>>(18).map_err(internal)?
                    .map(|run| format!("mission-run/{}", run.strip_prefix("mission-run/").unwrap_or(&run)))
            };
            item
        } else if fractal_schema::notice::is_reserved_subject(&key.source_id) {
            let Some((item, proof, current)) = notice_source(connection, key, &person, fields,
                opening_at, at, snapshots.get(&key.episode).map(Vec::as_slice).unwrap_or_default())? else { continue };
            evidence = proof;
            current_notice = current;
            item
        } else if subject == key.source_id && key.source_id.starts_with("custom/") && canonical {
            if fractal_schema::notice::is_reserved_kind(&kind) { continue; }
            let body: Option<String> = row.get(12).map_err(internal)?;
            let manifest: Option<String> = row.get(13).map_err(internal)?;
            let state: Option<String> = row.get(14).map_err(internal)?;
            let (Some(body), Some(manifest), Some("ready")) = (body, manifest, state.as_deref()) else { continue };
            let body: Value = serde_json::from_str(&body).map_err(internal)?;
            let manifest: fractal_schema::custom::Manifest = serde_json::from_str(&manifest).map_err(internal)?;
            let Some(attention) = manifest.attention.as_ref() else { continue };
            if kind != manifest.creation_kind || body["state"] != "ready"
                || body["attention"]["episode"] != key.episode
                || fields["_registration"] != body["registration"]
                || !manifest.slots.iter().any(|(name, slot)| slot.kind == manifest.creation_kind
                    && body["provenance"][name]["claim_id"] == opening_id)
                || !same_person(fields[&attention.recipient_field].as_str())?
                || !same_person(body["attention"]["recipient"].as_str())? { continue; }
            for proof in body["provenance"].as_object().into_iter().flat_map(|proof| proof.values()) {
                if let Some(id) = proof["claim_id"].as_str() { evidence.push(id.into()); }
            }
            custom::attention_item(&body, opening_at).map_err(internal)?
        } else {
            // Authored step-run.state openings do not retain the opening recipient.
            // Harness, login, planning, revisions and faults have no supported retained proof.
            continue;
        };
        if item.kind.starts_with("custom.") || item.kind == "notice" {
            evidence.extend(row.get::<_, Option<String>>(17).map_err(internal)?);
        }
        item.actions.clear(); // Discussion qualification never grants answer/mutation authority.
        evidence.sort(); evidence.dedup();
        result[index] = Some(QualifiedDiscussionSource { item, evidence, current_notice });
    }
    Ok(result)
}

fn matches_person(connection: &Connection, person: &str, recipient: Option<&str>)
    -> Result<bool, FractalError>
{
    let Some(recipient) = recipient else { return Ok(false) };
    match identities::resolve_reference_conn(connection, recipient) {
        Ok(recipient) => Ok(recipient == person),
        Err(error) if error.code == "lifecycle-unavailable" => Ok(false),
        Err(error) => Err(error),
    }
}

fn source_item(key: &SourceEpisode, person: &str, kind: &str, fields: &Value, at: u128)
    -> Result<AttentionItemView, FractalError>
{
    serde_json::from_value(json!({
        "subject":key.source_id,"episode":key.episode,"person":person,"kind":kind,
        "priority":"normal","title":fields["title"].as_str().unwrap_or("Discussion"),
        "detail":fields["reason"].as_str().or_else(|| fields["question"].as_str()).unwrap_or(""),
        "request":fields.get("request"),"requested_at_unix_ms":at,
        "step":key.source_id.starts_with("step-run/").then_some(&key.source_id),
        "mission_run":fields["run"].as_str().or_else(|| fields["mission_run"].as_str())
    })).map_err(internal)
}

fn notice_snapshots(connection: &Connection, input: &str)
    -> Result<BTreeMap<String, Vec<ClaimRecord>>, FractalError>
{
    // The native invalidation edge is indexed by the pinned creation claim. Do not scan all
    // notice claims to discover each source's snapshot, or refold unrelated recipients.
    let sql = canonical_sql(
        "SELECT DISTINCT claims.id,claims.store_index,claims.batch_id,claims.subject,claims.kind,
         claims.origin,claims.actor,claims.body,claims.predecessors,claims.accepted_at_unix_ms
         FROM json_each(?1) input
         CROSS JOIN custom_dependencies dependency INDEXED BY custom_dependencies_input
           ON dependency.dependency=json_extract(input.value,'$.episode')
         JOIN custom_sources snapshot ON snapshot.subject=dependency.source AND snapshot.state='ready'
         JOIN claims ON claims.id=json_extract(snapshot.body,'$.provenance.creation.claim_id')
         WHERE claims.kind='custom.attention.snapshot-created'
           AND json_extract(claims.body,'$.fields.creation_claim')=json_extract(input.value,'$.episode')
           AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
         ORDER BY CANONICAL_DESC(claims)"
    );
    let mut statement = connection.prepare(&sql).map_err(internal)?;
    let claims = statement.query_map([input], claim_from_row).map_err(internal)?;
    let mut snapshots = BTreeMap::<String, Vec<ClaimRecord>>::new();
    for claim in claims {
        let claim = claim.map_err(internal)?;
        if let Some(episode) = claim.body["fields"]["creation_claim"].as_str() {
            snapshots.entry(episode.into()).or_default().push(claim);
        }
    }
    Ok(snapshots)
}

fn notice_source(connection: &Connection, key: &SourceEpisode, person: &str,
    opening_fields: &Value, opening_at: u128, at: u128, snapshots: &[ClaimRecord])
    -> Result<Option<(AttentionItemView, Vec<String>, bool)>, FractalError>
{
    let notice = &fractal_schema::notice::manifests()[1];
    for snapshot in snapshots {
        let fields = custom::fields(&snapshot);
        let source = fields.get("source_id").and_then(Value::as_str).unwrap_or_default();
        if key.source_id != source || snapshot.accepted_at_unix_ms > at { continue; }
        let Some((hash, manifest, state)) = custom::registration(connection, &snapshot.subject).map_err(internal)? else { continue };
        if state != "ready" || serde_json::to_value(&manifest).map_err(internal)? != serde_json::to_value(notice).map_err(internal)?
            || fields.get("_registration").and_then(Value::as_str) != Some(hash.as_str()) { continue; }
        let position = notice_authority::Position::written(connection, snapshot).map_err(internal)?;
        if notice_authority::validate(connection, &snapshot.subject, &manifest, &snapshot.kind,
            snapshot.actor.as_deref().unwrap_or_default(), &fields, None, Some(position)).is_err() { continue; }
        // Native validation pins the immutable source recipient and the current snapshot's
        // recipient to the same person; neither a producer label nor an about link suffices.
        if !matches_person(connection, person, fields.get("recipient").and_then(Value::as_str))? { continue; }
        let mut item = source_item(key, person, "notice", opening_fields, opening_at)?;
        item.title = opening_fields["summary"].as_str().unwrap_or("Notice").into();
        item.detail = opening_fields["body"].as_str().unwrap_or_default().into();
        item.conversation = fields.get("producer").and_then(Value::as_str).filter(|agent| agent.starts_with("agent/")).map(str::to_owned);
        let registration: String = connection.query_row(
            "SELECT claim_id FROM custom_registrations WHERE hash=?1 AND state='ready'",
            [&hash], |row| row.get(0),
        ).map_err(internal)?;
        let owner = current_desired_row_tx(connection, crate::notice_owner::SUBJECT).map_err(internal)?;
        let mut evidence = vec![key.episode.clone(), snapshot.id.clone(), registration];
        evidence.extend(owner.map(|owner| owner.claim_id));
        let history = custom::claims(connection, &snapshot.subject).map_err(internal)?;
        let latest = history.iter().rev().find(|claim| claim.accepted_at_unix_ms <= at
            && claim.body["fields"]["_registration"] == hash
            && matches!(claim.kind.as_str(), "custom.attention.snapshot-created" | "custom.attention.snapshot-updated"));
        let current = latest.is_some_and(|claim| claim.body["fields"]["policy"]["disposition"] == "deliver");
        return Ok(Some((item, evidence, current)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom_open(store: &Store, suffix: &str, person: &str) -> SourceEpisode {
        let source = format!("custom/garden/review/v1/{suffix}");
        let opening = store.append_claim(&ClaimInput {
            subject: source.clone(), kind: "custom.garden.review.v1.requested".into(),
            actor: Some("agent/garden/seed".into()),
            fields: serde_json::from_value(json!({
                "title":"Retained review", "detail":"Original detail", "recipient":person,
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        SourceEpisode { source_id: source, episode: opening.id }
    }

    fn custom_store() -> Store {
        let store = Store::open_memory("discussion-qualifier").unwrap();
        let manifest = serde_json::from_str(include_str!("../../../../examples/fractal/custom-review.json")).unwrap();
        store.register_custom_kind(&custom::RegistrationRequest {
            manifest, actor: "agent/garden/seed".into(),
        }).unwrap();
        store
    }

    #[test]
    fn batch_qualifies_multiple_resolved_openings_without_admitting_foreign_or_fabricated_keys() {
        let store = custom_store();
        let first = custom_open(&store, "first", "person/reader");
        let second = custom_open(&store, "second", "person/reader");
        let foreign = custom_open(&store, "foreign", "person/other");
        for key in [&first, &second] {
            store.append_claim(&ClaimInput {
                subject: key.source_id.clone(), kind: "custom.garden.review.v1.answered".into(),
                actor: Some("person/reader".into()),
                fields: serde_json::from_value(json!({"request":key.episode,"selection":"keep"})).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        let fabricated = SourceEpisode { episode: "missing-opening".into(), ..first.clone() };
        let crossed = SourceEpisode { episode: second.episode.clone(), ..first.clone() };
        let conn = store.connection.write();
        let rows = qualify_batch(&conn, "person/reader",
            &[first.clone(), foreign, fabricated, second.clone(), crossed], now_ms()).unwrap();
        assert_eq!(rows.len(), 5);
        for (index, key) in [(0, &first), (3, &second)] {
            let proof = rows[index].as_ref().unwrap();
            assert_eq!(proof.item().subject, key.source_id);
            assert_eq!(proof.item().episode, key.episode);
            assert_eq!(proof.item().conversation.as_deref(), Some("agent/garden/seed"));
            assert!(proof.item().actions.is_empty());
            assert!(proof.evidence().contains(&key.episode));
        }
        assert!(rows[1].is_none() && rows[2].is_none() && rows[4].is_none());
        assert!(qualify_batch(&conn, "person/reader", &[], now_ms()).unwrap().is_empty());
    }

    #[test]
    fn custom_current_recipient_cannot_replace_the_opening_recipient() {
        let store = custom_store();
        let key = custom_open(&store, "recipient", "person/original");
        let conn = store.connection.write();
        conn.execute(
            "UPDATE custom_sources SET person='person/reassigned',
             body=json_set(body,'$.attention.recipient','person/reassigned') WHERE subject=?1",
            [&key.source_id],
        ).unwrap();
        assert!(qualify(&conn, "person/original", &key, now_ms()).unwrap().is_none());
        assert!(qualify(&conn, "person/reassigned", &key, now_ms()).unwrap().is_none());
    }

    #[test]
    fn runtime_opening_and_current_assignment_must_both_name_the_person() {
        let store = Store::open_memory("discussion-qualifier").unwrap();
        let key = SourceEpisode { source_id:"step-run/retained/ask".into(), episode:"opening".into() };
        let conn = store.connection.write();
        conn.execute_batch(
            "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,
               root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,
               created_at_unix_ms,updated_at_unix_ms)
             VALUES('retained','retained','rev','retained','rev','retained',
               '/tmp','agent/requester','{}','once','completed','done','1','1');
             INSERT INTO run_generations(id,run_id,revision,status,actor,reason,
               created_at_unix_ms,updated_at_unix_ms)
             VALUES('retained','retained','rev','completed','agent/requester','fixture','1','1');
             INSERT INTO step_runs(subject,run_id,generation_id,step_path,definition_hash,
               status,attempt,assignee,title,goals,created_at_unix_ms,updated_at_unix_ms)
             VALUES('step-run/retained/ask','retained','retained','ask','hash',
               'completed',1,'person/reader','CURRENT POISON','CURRENT POISON','1','1');
             INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES('opening','discussion-qualifier',1,'opening','1');
             INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES('opening','opening','step-run/retained/ask','work.person-asked',
               'discussion-qualifier','agent/requester',
               '{\"fields\":{\"person\":\"person/reader\",\"title\":\"Original title\",\"reason\":\"Original ask\"}}','[]','1');"
        ).unwrap();
        let proof = qualify(&conn, "person/reader", &key, now_ms()).unwrap().unwrap();
        assert_eq!(proof.item().title, "Original title");
        assert_eq!(proof.item().detail, "Original ask");
        assert_eq!(proof.item().conversation.as_deref(), Some("agent/requester"));
        assert_eq!(proof.evidence(), ["opening"]);
        conn.execute("UPDATE step_runs SET assignee='person/other' WHERE subject=?1", [&key.source_id]).unwrap();
        assert!(qualify(&conn, "person/reader", &key, now_ms()).unwrap().is_none());
        assert!(qualify(&conn, "person/other", &key, now_ms()).unwrap().is_none());
        let authored = SourceEpisode { episode:"run-generation/retained:99:99".into(), ..key };
        assert!(qualify(&conn, "person/other", &authored, now_ms()).unwrap().is_none());
    }
}

#[cfg(test)]
mod manifest_episode_tests {
    use super::*;

    #[test]
    fn manifest_episode_expression_is_not_assumed_to_be_a_claim_id() {
        let store = Store::open_memory("manifest-episode").unwrap();
        let mut manifest: fractal_schema::custom::Manifest =
            serde_json::from_str(include_str!("../../../../examples/fractal/custom-review.json")).unwrap();
        manifest.attention.as_mut().unwrap().episode =
            fractal_schema::custom::Expr::Constant { value: json!("review-episode") };
        store.register_custom_kind(&custom::RegistrationRequest {
            manifest, actor: "agent/garden/seed".into(),
        }).unwrap();
        let source = "custom/garden/review/v1/expression";
        let opening = store.append_claim(&ClaimInput {
            subject: source.into(), kind: "custom.garden.review.v1.requested".into(),
            actor: Some("agent/garden/seed".into()),
            fields: serde_json::from_value(json!({
                "title":"Expression episode", "detail":"Retained", "recipient":"person/reader",
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.append_claim(&ClaimInput {
            subject: source.into(), kind: "custom.garden.review.v1.answered".into(),
            actor: Some("person/reader".into()),
            fields: serde_json::from_value(json!({"request":opening.id,"selection":"keep"})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let key = SourceEpisode { source_id:source.into(), episode:"review-episode".into() };
        let wrong = SourceEpisode { episode:opening.id.clone(), ..key.clone() };
        let conn = store.connection.write();
        let proof = qualify(&conn, "person/reader", &key, now_ms()).unwrap().unwrap();
        assert_eq!(proof.item().episode, "review-episode");
        assert!(proof.evidence().contains(&opening.id));
        assert!(!proof.evidence().contains(&key.episode));
        assert!(qualify(&conn, "person/reader", &wrong, now_ms()).unwrap().is_none());
    }
}
