//! Exact durable opening authority for discussion reads and retained writes.
//! A current assignment alone, an episode-shaped string, labels and about links prove nothing.
use super::*;
use super::thread_metadata::request_history::SourceEpisode;

/// Transaction-local proof. Only this module can construct it; never deserialize authority.
pub struct QualifiedDiscussionSource {
    item: AttentionItemView,
    evidence: Vec<String>,
    current_notice: bool,
    authored: Option<AuthoredEvidence>,
}

/// Retained coordinates, exposed only after the opening recipient has been proved.
pub struct AuthoredEvidence {
    pub generation: String,
    pub revision: String,
    pub opening_claim_id: String,
    pub attempt: u64,
    pub readiness_epoch: u64,
    /// Immutable variables at this generation's materialization, not the later
    /// requested retry. Missions owns interpolation of the retained payload.
    pub materialization_variables: BTreeMap<String, String>,
}

impl QualifiedDiscussionSource {
    pub fn item(&self) -> &AttentionItemView { &self.item }
    pub fn evidence(&self) -> &[String] { &self.evidence }
    pub fn authored_evidence(&self) -> Option<&AuthoredEvidence> { self.authored.as_ref() }
    pub(super) fn is_current_notice(&self) -> bool { self.current_notice }
    pub(super) fn into_resolved(self) -> thread_metadata::inbox_source::ResolvedSource {
        thread_metadata::inbox_source::ResolvedSource { item: self.item, evidence: self.evidence }
    }
}

/// Read-only authority can survive a payload budget refusal, but never grants Start.
pub(super) enum HistoryQualification {
    Qualified,
    PayloadOverBudget,
    Unqualified,
}

impl HistoryQualification {
    pub(super) fn is_authorized(&self) -> bool {
        !matches!(self, Self::Unqualified)
    }
}

enum ProofBudget {
    WithinBudget,
    OverBudget,
    InvalidKey,
}

/// Only runtime asks have a separate bounded immutable routing proof. Other source
/// families still require their existing full payload/evidence qualification.
pub(super) fn qualify_history(connection: &Connection, person: &str, key: &SourceEpisode, at: u128)
    -> Result<HistoryQualification, FractalError>
{
    match inputs_preflight(connection, key)? {
        ProofBudget::WithinBudget => {
            let source = qualify_batch_preflighted(connection, person, std::slice::from_ref(key), at,
                std::iter::once(Ok(ProofBudget::WithinBudget)))?.pop().flatten();
            Ok(if source.is_some() { HistoryQualification::Qualified } else { HistoryQualification::Unqualified })
        }
        ProofBudget::OverBudget => {
            Ok(if runtime_opening_header_authorized(connection, person, key, at)? {
                HistoryQualification::PayloadOverBudget
            } else { HistoryQualification::Unqualified })
        }
        ProofBudget::InvalidKey => Ok(HistoryQualification::Unqualified),
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
    qualify_batch_preflighted(connection, person, keys, at,
        keys.iter().map(|key| inputs_preflight(connection, key)))
}

fn qualify_batch_preflighted(
    connection: &Connection, person: &str, keys: &[SourceEpisode], at: u128,
    preflight: impl Iterator<Item = Result<ProofBudget, FractalError>>,
) -> Result<Vec<Option<QualifiedDiscussionSource>>, FractalError> {
    let mut result: Vec<Option<QualifiedDiscussionSource>> = (0..keys.len()).map(|_| None).collect();
    if keys.is_empty() { return Ok(result); }
    let person = match identities::resolve_reference_conn(connection, person) {
        Ok(person) => person,
        Err(error) if error.code == "lifecycle-unavailable" => return Ok(result),
        Err(error) => return Err(error),
    };
    if !person.starts_with("person/") { return Ok(result); }
    // Admission reads only metadata/byte counts. Rejected entries remain null so the
    // batched query retains the caller's indices without ever hydrating their bodies.
    let mut admitted = Vec::with_capacity(keys.len());
    for (key, budget) in keys.iter().zip(preflight) {
        admitted.push(if matches!(budget?, ProofBudget::WithinBudget) {
            serde_json::to_value(key).map_err(internal)?
        } else { Value::Null });
    }
    let input = serde_json::to_string(&admitted).map_err(internal)?;
    let snapshots = if keys.iter().any(|key| key.source_id.starts_with(fractal_schema::notice::SOURCE_PREFIX)) {
        notice_snapshots(connection, &input)?
    } else { BTreeMap::new() };
    let authored = authored_sources(connection, &person, keys, &input, at)?;
    let reviewer = human_review_reassignment::reviewer_sql("opening");
    let before = canonical::after_sql("opening", "earlier");
    let candidate_order = canonical::order_sql("candidate", false);
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
             ORDER BY {candidate_order} LIMIT 1)
           ELSE json_extract(input.value,'$.episode') END
         LEFT JOIN step_runs step ON step.subject=json_extract(input.value,'$.source_id')
         WHERE NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=opening.id AND state='repaired')"
    ));
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hc47c843b6e24 owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
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
            // Harness, login, planning, revisions and faults have no supported retained proof.
            continue;
        };
        if item.kind.starts_with("custom.") || item.kind == "notice" {
            evidence.extend(row.get::<_, Option<String>>(17).map_err(internal)?);
        }
        item.actions.clear(); // Discussion qualification never grants answer/mutation authority.
        evidence.sort(); evidence.dedup();
        result[index] = Some(QualifiedDiscussionSource { item, evidence, current_notice, authored: None });
    }
    for (index, source) in authored { result[index] = Some(source); }
    Ok(result)
}

/// Fold only the requested generation subjects, in the same order as durable projection.
/// The definition is the generation's pinned revision, never today's assignment or revision.
fn authored_sources(connection: &Connection, person: &str, keys: &[SourceEpisode], input: &str, at: u128)
    -> Result<BTreeMap<usize, QualifiedDiscussionSource>, FractalError>
{
    struct State {
        generation: String,
        revision: String,
        recipient: Option<String>,
        current_recipient: Option<String>,
        attempt: u64,
        epoch: Option<u64>,
        status: String,
        evidence: Vec<String>,
        materialization_variables: BTreeMap<String, String>,
    }
    let order = canonical::order_sql("claims", false);
    let candidate_order = canonical::order_sql("candidate", false);
    let accepted_rename = person_rename::accepted_reassignment_sql("claims", "previous_person", "person");
    let sql = format!(
        "SELECT CAST(input.key AS INTEGER), step.generation_id, generation.revision,
         revision.body, revision.claim_id, step.step_path, step.definition_hash, step.assignee,
         run.id, created.body, claims.id, claims.kind, claims.body, claims.accepted_at_unix_ms,
         CASE WHEN claims.kind='work.person-reassigned' THEN {accepted_rename} ELSE 0 END,
         created.id, generation_claim.id, generation.predecessor_id
         FROM json_each(?1) input
         JOIN step_runs step ON step.subject=json_extract(input.value,'$.source_id')
         JOIN run_generations generation ON generation.id=step.generation_id AND generation.run_id=step.run_id
         JOIN mission_runs run ON run.id=step.run_id
         JOIN mission_revisions revision ON revision.mission_id=run.mission_id AND revision.revision=generation.revision
         JOIN claims created ON created.id=(
           SELECT candidate.id FROM claims candidate
           WHERE candidate.subject='mission-run/' || run.id AND candidate.kind='mission-run.created'
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=candidate.id AND state='repaired')
           ORDER BY {candidate_order} LIMIT 1)
         JOIN claims generation_claim ON generation_claim.id=(
           SELECT candidate.id FROM claims candidate
           WHERE candidate.subject='run-generation/' || generation.id AND candidate.kind='run-generation.created'
             AND json_extract(candidate.body,'$.fields.revision')=generation.revision
             AND json_extract(candidate.body,'$.fields.run')='mission-run/' || run.id
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=candidate.id AND state='repaired')
           ORDER BY {candidate_order} LIMIT 1)
         JOIN claims ON claims.subject=step.subject
         WHERE claims.kind IN ('step-run.state','step-run.carried','step-run.retried',
             'work.released','work.handoff-returned','work.person-reassigned',
             'work.claimed','work.progress','work.submitted','work.failed',
             'work.person-done','work.person-cancelled','work.person-override')
           AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
         ORDER BY CAST(input.key AS INTEGER), {order}"
    );
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-he4c4feee0f53 owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
    let mut rows = statement.query([input]).map_err(internal)?;
    let mut states = BTreeMap::<usize, State>::new();
    let mut result = BTreeMap::new();
    let mut seen = BTreeSet::new();
    while let Some(row) = rows.next().map_err(internal)? {
        let index: usize = row.get(0).map_err(internal)?;
        let key = &keys[index];
        if !key.episode.starts_with("run-generation/") || result.contains_key(&index) { continue; }
        if let std::collections::btree_map::Entry::Vacant(entry) = states.entry(index) {
            let generation: String = row.get(1).map_err(internal)?;
            let revision: String = row.get(2).map_err(internal)?;
            let body: String = row.get(3).map_err(internal)?;
            let Ok(mission) = serde_json::from_str::<MissionSpec>(&body) else { continue };
            let mut unhashed = mission.clone();
            unhashed.revision.clear();
            if mission.revision != revision || revision != hex::encode(Sha256::digest(
                serde_json::to_vec(&unhashed).map_err(internal)?,
            )) { continue; }
            let created: String = row.get(9).map_err(internal)?;
            let Ok(created) = serde_json::from_str::<Value>(&created) else { continue };
            let fields = &created["fields"];
            // Reconstruct solely from admission facts, never the current run view.
            let run: String = row.get(8).map_err(internal)?;
            let historical = json!({
                "subject":format!("mission-run/{run}"), "id":run,
                "mission":fields["mission"], "generation":format!("run-generation/{generation}"),
                "initial_revision":fields["initial_revision"], "revision":revision,
                "root_revision":fields["root_revision"], "root_mission_run":fields["root_mission_run"],
                "parent_step_run":fields["parent_step_run"], "workspace":fields["workspace"],
                "requester":fields["requester"], "inputs":fields["inputs"],
                "mode":fields["mode"], "status":"running", "phase":"normal",
                "created_at_unix_ms":0, "updated_at_unix_ms":0
            });
            let Ok(historical) = serde_json::from_value::<MissionRunView>(historical) else { continue };
            let mut variables = mission_run_variables(&historical, &revision);
            // Successor materialization does not inherit the initial default.
            let predecessor: Option<String> = row.get(17).map_err(internal)?;
            let default = if predecessor.is_none() {
                match serde_json::from_value::<Option<WorkSelector>>(fields["default_selector"].clone()) {
                    Ok(default) => default, Err(_) => continue,
                }
            } else { None };
            let path: String = row.get(5).map_err(internal)?;
            let hash: String = row.get(6).map_err(internal)?;
            let mut flat = Vec::new();
            flatten_steps(&mission, default, &[], &mut flat);
            let recipient = flat.into_iter()
                .find(|(step, _, _)| step.path == path && step.definition_hash == hash)
                .and_then(|(_, selector, _)| interpolate_selector(&selector, &variables).ok())
                .and_then(|(assignee, _, _)| assignee);
            if predecessor.is_none() { variables.remove("PATH"); }
            variables.insert("ST_STEP".into(), path.clone());
            variables.insert("ST_STEP_RUN".into(), key.source_id.clone());
            variables.insert("ST_ATTEMPT".into(), "1".into());
            let materialized_assignee = recipient.as_deref()
                .map(|recipient| identities::resolve_reference_conn(connection, recipient))
                .transpose()?.unwrap_or_default();
            variables.insert("ST_ASSIGNEE".into(), materialized_assignee);
            variables.insert("ST_PARENT_STEP_RUN".into(),
                crate::mission::parent_step_path(&mission, &path)
                    .map(|parent| format!("step-run/{generation}/{parent}"))
                    .or_else(|| historical.parent_step_run.clone()).unwrap_or_default());
            entry.insert(State {
                generation: format!("run-generation/{generation}"), revision, recipient,
                current_recipient: row.get(7).map_err(internal)?, attempt: 1, epoch: None,
                status: "pending".into(), evidence: vec![row.get(4).map_err(internal)?,
                    row.get(15).map_err(internal)?, row.get(16).map_err(internal)?],
                materialization_variables: variables,
            });
        }
        let state = states.get_mut(&index).expect("initialized authored fold");
        let claim_id: String = row.get(10).map_err(internal)?;
        let kind: String = row.get(11).map_err(internal)?;
        let body: String = row.get(12).map_err(internal)?;
        let body: Value = serde_json::from_str(&body).map_err(internal)?;
        let fields = &body["fields"];
        let opening_at: String = row.get(13).map_err(internal)?;
        let opening_at: u128 = opening_at.parse().map_err(internal)?;
        let claim_attempt = fields["attempt"].as_u64();
        if matches!(kind.as_str(), "step-run.carried" | "step-run.retried") {
            if let Some(attempt) = claim_attempt { state.attempt = attempt; }
            if kind == "step-run.carried" && let Some(attempt) = claim_attempt {
                state.materialization_variables.insert("ST_ATTEMPT".into(), attempt.to_string());
            }
            // A carried ready state inherits its epoch from the exact source lifecycle
            // before the carry, never the source's current projection.
            state.epoch = if kind == "step-run.carried" && claim_attempt.is_some() {
                carried_ready_epoch(connection, &claim_id, state.attempt, &mut state.evidence)?
            } else { None };
        }
        if matches!(kind.as_str(), "work.released" | "work.handoff-returned")
            && claim_attempt.unwrap_or(1) == state.attempt
            && let Some(to) = fields["handoff_to"].as_str()
        {
            state.recipient = Some(to.into());
            state.evidence.push(claim_id.clone());
        }
        if kind == "work.person-reassigned" && row.get::<_, bool>(14).map_err(internal)?
            && matches!(state.status.as_str(), "pending" | "ready")
            && state.recipient.as_deref() == fields["previous_person"].as_str()
        {
            state.recipient = fields["person"].as_str().map(str::to_owned);
            state.evidence.push(claim_id.clone());
        }
        if let Some(status) = fields["status"].as_str() { state.status = status.into(); }
        let handoff_ready = matches!(kind.as_str(), "work.released" | "work.handoff-returned")
            && claim_attempt == Some(state.attempt)
            && fields["handoff_to"].as_str().is_some()
            && fields["status"] == "ready";
        if kind == "step-run.state" || handoff_ready {
            state.epoch = fields["readiness_epoch"].as_u64();
        }
        if kind == "step-run.state" && let Some(attempt) = claim_attempt { state.attempt = attempt; }
        let Some(epoch) = state.epoch else { continue };
        if !(matches!(kind.as_str(), "step-run.state" | "step-run.carried") || handoff_ready)
            || fields["status"] != "ready"
            || key.episode != format!("{}:{}:{epoch}", state.generation, state.attempt)
            || opening_at > at
            || !seen.insert(index)
            || !matches_person(connection, person, state.recipient.as_deref())?
            || !matches_person(connection, person, state.current_recipient.as_deref())?
        { continue; }
        let mut item = source_item(key, person, "person-step", fields, opening_at)?;
        item.mission_run = Some(format!("mission-run/{}", row.get::<_, String>(8).map_err(internal)?));
        state.evidence.push(claim_id.clone());
        state.evidence.sort(); state.evidence.dedup();
        result.insert(index, QualifiedDiscussionSource {
            item, evidence: state.evidence.clone(), current_notice: false,
            authored: Some(AuthoredEvidence {
                generation: state.generation.clone(), revision: state.revision.clone(),
                opening_claim_id: claim_id, attempt: state.attempt, readiness_epoch: epoch,
                materialization_variables: std::mem::take(&mut state.materialization_variables),
            }),
        });
    }
    Ok(result)
}

/// Recover coordinates omitted by the existing carried writer. This is not
/// authority: consumers must still qualify the resulting exact source/episode.
pub fn carried_episode(connection: &Connection, source: &str, opening: &str)
    -> Result<Option<String>, FractalError>
{
    let key = SourceEpisode { source_id: source.into(), episode: opening.into() };
    if !matches!(inputs_preflight(connection, &key)?, ProofBudget::WithinBudget) { return Ok(None); }
    let Some(claim) = claim_by_id_tx(connection, opening).map_err(internal)? else { return Ok(None) };
    if claim.subject != source || claim.kind != "step-run.carried"
        || claim.body["fields"]["status"] != "ready" { return Ok(None); }
    let Some(attempt) = claim.body["fields"]["attempt"].as_u64() else { return Ok(None) };
    let Some(epoch) = carried_ready_epoch(connection, opening, attempt, &mut Vec::new())?
        else { return Ok(None) };
    let generation: Option<String> = connection.query_row(
        "SELECT generation_id FROM step_runs WHERE subject=?1", [source], |row| row.get(0),
    ).optional().map_err(internal)?;
    Ok(generation.map(|generation| format!("run-generation/{generation}:{attempt}:{epoch}")))
}

/// A carried claim does not record an epoch. Follow its exact predecessor chain
/// until an actual READY write supplies one, folding retries before that boundary.
/// All visited subjects were byte/cardinality-admitted by inputs_preflight.
fn carried_ready_epoch(connection: &Connection, carry: &str, wanted_attempt: u64, evidence: &mut Vec<String>)
    -> Result<Option<u64>, FractalError>
{
    let before = canonical::after_sql("carry", "prior");
    let order = canonical::order_sql("prior", false);
    let sql = format!(
        "SELECT prior.id,prior.kind,json_extract(prior.body,'$.fields.status'),
         json_extract(prior.body,'$.fields.attempt'),
         json_extract(prior.body,'$.fields.readiness_epoch'),
         json_extract(prior.body,'$.fields.handoff_to')
         FROM claims carry JOIN claims prior
           ON prior.subject=json_extract(carry.body,'$.fields.source_step_run')
         WHERE carry.id=?1 AND carry.kind='step-run.carried' AND {before}
           AND EXISTS(SELECT 1 FROM step_runs source WHERE source.subject=prior.subject
             AND json_extract(carry.body,'$.fields.source_generation')='run-generation/' || source.generation_id)
           AND prior.kind IN ('step-run.state','step-run.carried','step-run.retried',
             'work.released','work.handoff-returned','work.person-done','work.person-cancelled',
             'work.person-override','work.claimed','work.progress','work.submitted','work.failed')
           AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=prior.id AND state='repaired')
         ORDER BY {order}"
    );
    let mut carry = carry.to_owned();
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PROOF_CLAIMS {
        if !seen.insert(carry.clone()) { return Ok(None); }
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hf94cc9554b5c owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
        let mut rows = statement.query([&carry]).map_err(internal)?;
        let mut attempt = 1;
        let mut status = String::from("pending");
        let mut ready = None;
        let mut inherited = None;
        while let Some(row) = rows.next().map_err(internal)? {
            let kind: String = row.get(1).map_err(internal)?;
            if matches!(kind.as_str(), "step-run.carried" | "step-run.retried") {
                let Some(recorded) = row.get::<_, Option<u64>>(3).map_err(internal)? else { return Ok(None) };
                attempt = recorded;
                ready = None;
                inherited = None;
            }
            let recorded: Option<u64> = row.get(3).map_err(internal)?;
            if recorded.is_some_and(|recorded| recorded != attempt) { continue; }
            let handoff_ready = matches!(kind.as_str(), "work.released" | "work.handoff-returned")
                && recorded == Some(attempt)
                && row.get::<_, Option<String>>(5).map_err(internal)?.is_some();
            if let Some(next) = row.get::<_, Option<String>>(2).map_err(internal)? { status = next; }
            if (kind == "step-run.state" || handoff_ready) && status == "ready" {
                ready = row.get::<_, Option<u64>>(4).map_err(internal)?
                    .map(|epoch| row.get::<_, String>(0).map(|id| (epoch, id))).transpose()
                    .map_err(internal)?;
                inherited = None;
            } else if kind == "step-run.carried" && status == "ready" {
                inherited = Some(row.get::<_, String>(0).map_err(internal)?);
            }
        }
        if attempt != wanted_attempt || status != "ready" { return Ok(None); }
        if let Some((epoch, opening)) = ready {
            evidence.extend(seen);
            evidence.push(opening);
            return Ok(Some(epoch));
        }
        let Some(previous) = inherited else { return Ok(None) };
        carry = previous;
    }
    Ok(None)
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

const MAX_PROOF_CLAIMS: usize = 512;
const MAX_PROOF_BYTES: u64 = 256 * 1024;

/// Prove only the original canonical runtime recipient and current policy. The
/// database may inspect the body for JSON fields, but no body crosses this boundary.
/// All returned header bytes together must fit the unchanged proof-byte budget.
fn runtime_opening_header_authorized(connection: &Connection, person: &str, key: &SourceEpisode, at: u128)
    -> Result<bool, FractalError>
{
    let person = match identities::resolve_reference_conn(connection, person) {
        Ok(person) if person.starts_with("person/") => person,
        Ok(_) => return Ok(false),
        Err(error) if error.code == "lifecycle-unavailable" => return Ok(false),
        Err(error) => return Err(error),
    };
    let before = canonical::after_sql("opening", "earlier");
    let sql = format!(
        "WITH header AS (
           SELECT json_extract(opening.body,'$.fields.person') AS recipient,
             step.assignee AS current_recipient, opening.accepted_at_unix_ms AS accepted
           FROM claims opening LEFT JOIN step_runs step ON step.subject=opening.subject
           WHERE opening.id=?2 AND opening.subject=?1 AND opening.kind='work.person-asked'
             AND json_type(opening.body,'$.fields.person')='text'
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=opening.id AND state='repaired')
             AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=opening.id)
             AND (json_type(opening.body,'$._operation') IS NULL OR EXISTS(
               SELECT 1 FROM operations
               WHERE operations.id=json_extract(opening.body,'$._operation.id')
                 AND operations.state='active' AND operations.canonical_claim_id=opening.id
                 AND operations.request_digest=json_extract(opening.body,'$._operation.request_digest')))
             AND NOT EXISTS(SELECT 1 FROM claims earlier
               WHERE earlier.subject=opening.subject AND earlier.kind=opening.kind AND {before}
                 AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=earlier.id AND state='repaired')
                 AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=earlier.id))
         ), bounded AS (
           SELECT *, length(CAST(recipient AS BLOB))+length(CAST(current_recipient AS BLOB))
             +length(CAST(accepted AS BLOB))<=?3 AS fits FROM header
         )
         SELECT CASE WHEN fits THEN recipient END,
           CASE WHEN fits THEN current_recipient END, CASE WHEN fits THEN accepted END
         FROM bounded"
    );
    let header: Option<(Option<String>, Option<String>, Option<String>)> = connection.query_row(
        /* sqlite-large-value: allow-for-now lv-h7c1ab81ff9e3 owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ &sql, params![key.source_id, key.episode, MAX_PROOF_BYTES],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional().map_err(internal)?;
    let Some((Some(recipient), Some(current_recipient), Some(accepted))) = header else { return Ok(false) };
    let accepted: u128 = accepted.parse().map_err(internal)?;
    Ok(accepted <= at && matches_person(connection, &person, Some(&recipient))?
        && matches_person(connection, &person, Some(&current_recipient))?)
}

/// Bound the complete proof read set, including repaired history, before selecting
/// any body. JSON extracts here return exact references only, never source prose.
fn inputs_preflight(connection: &Connection, key: &SourceEpisode) -> Result<ProofBudget, FractalError> {
    if key.source_id.is_empty() || key.source_id.len() > 512
        || key.episode.is_empty() || key.episode.len() > 512 { return Ok(ProofBudget::InvalidKey); }
    let mut statement = connection.prepare(
        /* sqlite-large-value: allow-for-now lv-he9e177bb34b5 owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ "WITH RECURSIVE subjects(subject) AS (
           SELECT ?1 UNION SELECT subject FROM claims WHERE id=?2
           UNION SELECT 'mission-run/' || step.run_id FROM step_runs step WHERE step.subject=?1
           UNION SELECT 'run-generation/' || step.generation_id FROM step_runs step WHERE step.subject=?1
           UNION SELECT dependency.source FROM custom_dependencies dependency
             WHERE dependency.dependency=?2
           UNION SELECT ?3 WHERE substr(?1,1,?4)=?5
           UNION SELECT json_extract(c.body,'$.fields.source_step_run')
             FROM subjects JOIN claims c ON c.subject=subjects.subject
             WHERE c.kind='step-run.carried'
           LIMIT ?6
         ), sizes(bytes) AS (
           SELECT 0 FROM subjects
           UNION ALL SELECT length(CAST(body AS BLOB))+length(CAST(predecessors AS BLOB))
             FROM claims WHERE subject IN subjects
           UNION ALL SELECT length(CAST(source.body AS BLOB)) FROM custom_sources source
             WHERE source.subject IN subjects
           UNION ALL SELECT length(CAST(registration.manifest AS BLOB))
             FROM custom_registrations registration
             WHERE EXISTS(SELECT 1 FROM subjects WHERE
               substr(subject,1,length(registration.prefix))=registration.prefix)
           UNION ALL SELECT length(CAST(revision.body AS BLOB)) FROM mission_revisions revision
             JOIN mission_runs run ON run.mission_id=revision.mission_id
             JOIN step_runs step ON step.run_id=run.id
             JOIN run_generations generation ON generation.id=step.generation_id
               AND generation.revision=revision.revision
             WHERE step.subject=?1
         ) SELECT bytes FROM sizes LIMIT ?6"
    ).map_err(internal)?;
    let mut rows = statement.query(params![key.source_id, key.episode,
        crate::notice_owner::SUBJECT, fractal_schema::notice::SOURCE_PREFIX.len(),
        fractal_schema::notice::SOURCE_PREFIX, MAX_PROOF_CLAIMS + 1]).map_err(internal)?;
    let mut count = 0;
    let mut bytes = 0_u64;
    while let Some(row) = rows.next().map_err(internal)? {
        count += 1;
        bytes = bytes.saturating_add(row.get::<_, Option<u64>>(0).map_err(internal)?.unwrap_or(0));
        if count > MAX_PROOF_CLAIMS || bytes > MAX_PROOF_BYTES { return Ok(ProofBudget::OverBudget); }
    }
    Ok(ProofBudget::WithinBudget)
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
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hef0e1c1afcc5 owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
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
        let fields = custom::fields(snapshot);
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
        // The snapshot history was metadata-admitted before hydration. Resolve one
        // canonical exact reference rather than materializing every historical body.
        let latest: Option<String> = connection.query_row(/* sqlite-large-value: allow-for-now lv-hd93815b8fb9e owner=fractal_store::store::discussion_source TODO=trace-statement-and-value-representation */ &canonical_sql(
            "SELECT claims.id FROM claims WHERE claims.subject=?1
             AND claims.kind IN ('custom.attention.snapshot-created','custom.attention.snapshot-updated')
             AND json_extract(claims.body,'$.fields._registration')=?2
             AND CAST(claims.accepted_at_unix_ms AS INTEGER)<=?3
             AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
             ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
            params![snapshot.subject, hash, at.to_string()], |row| row.get(0),
        ).optional().map_err(internal)?;
        let current = if let Some(latest) = latest {
            claim_by_id_tx(connection, &latest).map_err(internal)?
                .is_some_and(|claim| claim.body["fields"]["policy"]["disposition"] == "deliver")
        } else { false };
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
        let first = custom_open(&store, "first", "person/avery");
        let second = custom_open(&store, "second", "person/avery");
        let foreign = custom_open(&store, "foreign", "person/other");
        for key in [&first, &second] {
            store.append_claim(&ClaimInput {
                subject: key.source_id.clone(), kind: "custom.garden.review.v1.answered".into(),
                actor: Some("person/avery".into()),
                fields: serde_json::from_value(json!({"request":key.episode,"selection":"keep"})).unwrap(),
                evidence: vec![], expected_subject: None, idempotency_key: None,
            }).unwrap();
        }
        let fabricated = SourceEpisode { episode: "missing-opening".into(), ..first.clone() };
        let crossed = SourceEpisode { episode: second.episode.clone(), ..first.clone() };
        let conn = store.connection.write();
        let rows = qualify_batch(&conn, "person/avery",
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
        assert!(qualify_batch(&conn, "person/avery", &[], now_ms()).unwrap().is_empty());
    }

    #[test]
    fn custom_current_recipient_cannot_replace_the_opening_recipient() {
        let store = custom_store();
        let key = custom_open(&store, "recipient", "person/avery");
        let conn = store.connection.write();
        conn.execute(
            "UPDATE custom_sources SET person='person/robin',
             body=json_set(body,'$.attention.recipient','person/robin') WHERE subject=?1",
            [&key.source_id],
        ).unwrap();
        assert!(qualify(&conn, "person/avery", &key, now_ms()).unwrap().is_none());
        assert!(qualify(&conn, "person/robin", &key, now_ms()).unwrap().is_none());
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
               'completed',1,'person/avery','CURRENT POISON','CURRENT POISON','1','1');
             INSERT INTO batches(id,origin,replica_sequence,hash,accepted_at_unix_ms)
             VALUES('opening','discussion-qualifier',1,'opening','1');
             INSERT INTO claims(id,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms)
             VALUES('opening','opening','step-run/retained/ask','work.person-asked',
               'discussion-qualifier','agent/requester',
               '{\"fields\":{\"person\":\"person/avery\",\"title\":\"Original title\",\"reason\":\"Original ask\"}}','[]','1');"
        ).unwrap();
        let proof = qualify(&conn, "person/avery", &key, now_ms()).unwrap().unwrap();
        assert_eq!(proof.item().title, "Original title");
        assert_eq!(proof.item().detail, "Original ask");
        assert_eq!(proof.item().conversation.as_deref(), Some("agent/requester"));
        assert_eq!(proof.evidence(), ["opening"]);
        conn.execute("UPDATE step_runs SET assignee='person/other' WHERE subject=?1", [&key.source_id]).unwrap();
        assert!(qualify(&conn, "person/avery", &key, now_ms()).unwrap().is_none());
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
                "title":"Expression episode", "detail":"Retained", "recipient":"person/avery",
            })).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        store.append_claim(&ClaimInput {
            subject: source.into(), kind: "custom.garden.review.v1.answered".into(),
            actor: Some("person/avery".into()),
            fields: serde_json::from_value(json!({"request":opening.id,"selection":"keep"})).unwrap(),
            evidence: vec![], expected_subject: None, idempotency_key: None,
        }).unwrap();
        let key = SourceEpisode { source_id:source.into(), episode:"review-episode".into() };
        let wrong = SourceEpisode { episode:opening.id.clone(), ..key.clone() };
        let conn = store.connection.write();
        let proof = qualify(&conn, "person/avery", &key, now_ms()).unwrap().unwrap();
        assert_eq!(proof.item().episode, "review-episode");
        assert!(proof.evidence().contains(&opening.id));
        assert!(!proof.evidence().contains(&key.episode));
        assert!(qualify(&conn, "person/avery", &wrong, now_ms()).unwrap().is_none());
    }
}

#[cfg(test)]
mod authored_tests {
    use super::*;

    pub(super) fn fixture(recipient: &str) -> (Store, SourceEpisode) {
        let store = Store::open_memory("authored-qualifier").unwrap();
        let source = format!("version 2\nmission \"fixture\" state=\"ready\" {{ goal \"Review\"; step \"review\" {{ assigned-to {recipient:?}; goal \"Retained question\"; }} }}");
        let intent = crate::graph::parse_internal_intent(&source, store.origin()).unwrap();
        store.apply(&intent, &BTreeMap::from([("mission/fixture".into(), Vec::new())]),
            "proof-definition").unwrap();
        let run = store.create_mission_run_inner(&MissionRunRequest {
            mission: "fixture".into(), revision: None, workspace: "/tmp".into(),
            requester: Some("person/avery".into()), mode: Some("run".into()),
            inputs: BTreeMap::new(), idempotency_key: "proof-run".into(),
        }, None, None, None).unwrap();
        let key = SourceEpisode {
            source_id: run.steps[0].subject.clone(),
            episode: format!("{}:1:1", run.generation),
        };
        (store, key)
    }

    pub(super) fn claim(store: &Store, key: &SourceEpisode, kind: &str, fields: Value) -> String {
        let mut writer = store.connection.write();
        let tx = writer.transaction().unwrap();
        let claim = append_claim_tx(&tx, store.origin(), &key.source_id, kind,
            Some("person/avery"), &json!({"fields":fields}), &[], None).unwrap();
        tx.commit().unwrap();
        claim.id
    }

    #[test]
    fn literal_requires_exact_opening_and_keeps_completed_history() {
        let (store, key) = fixture("person/avery");
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());
        let opening = claim(&store, &key, "step-run.state", json!({"status":"ready","readiness_epoch":1}));
        claim(&store, &key, "step-run.state", json!({"status":"completed"}));
        let conn = store.readers.get();
        let proof = qualify(&conn, "person/avery", &key, now_ms()).unwrap().unwrap();
        assert_eq!(proof.authored_evidence().unwrap().opening_claim_id, opening);
        let never_opened = SourceEpisode { episode: key.episode.replace(":1:1", ":1:99"), ..key };
        assert!(qualify(&conn, "person/avery", &never_opened, now_ms()).unwrap().is_none());
    }

    #[test]
    fn current_assignment_cannot_replace_wrong_original_recipient() {
        let (store, key) = fixture("person/other");
        claim(&store, &key, "step-run.state", json!({"status":"ready","readiness_epoch":1}));
        store.connection.write().execute("UPDATE step_runs SET assignee='person/avery' WHERE subject=?1", [&key.source_id]).unwrap();
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());
    }

    #[test]
    fn canonical_assignment_mutations_before_opening_are_authority() {
        let (store, key) = fixture("person/other");
        let transfer = claim(&store, &key, "work.released",
            json!({"status":"ready","attempt":1,"handoff_to":"person/avery","readiness_epoch":1}));
        store.connection.write().execute("UPDATE step_runs SET assignee='person/avery' WHERE subject=?1", [&key.source_id]).unwrap();
        let proof = qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().unwrap();
        assert!(proof.evidence().contains(&transfer));
        store.connection.write().execute("UPDATE step_runs SET assignee='person/other' WHERE subject=?1", [&key.source_id]).unwrap();
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());

        let (store, key) = fixture("person/avery");
        claim(&store, &key, "work.handoff-returned",
            json!({"status":"ready","attempt":1,"handoff_to":"person/other","readiness_epoch":1}));
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());
    }
}

#[cfg(test)]
mod bounded_authored_tests {
    use super::authored_tests::{fixture, claim};
    use super::*;

    #[test]
    fn admission_variables_are_historical_and_missing_ready_epoch_is_not_inferred() {
        let (store, key) = fixture("person/${ST_MISSION}");
        claim(&store, &key, "step-run.state", json!({"status":"ready","readiness_epoch":1}));
        store.connection.write().execute(
            "UPDATE mission_runs SET requester='person/other',current_generation_id='current-poison'", [],
        ).unwrap();
        assert!(qualify(&store.readers.get(), "person/fixture", &key, now_ms()).unwrap().is_some());
        let (store, key) = fixture("person/avery");
        claim(&store, &key, "work.released",
            json!({"attempt":1,"handoff_to":"person/avery","readiness_epoch":1}));
        claim(&store, &key, "step-run.state", json!({"status":"ready"}));
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());
    }

    #[test]
    fn oversized_opening_and_cardinality_fail_before_proof_hydration() {
        let (store, key) = fixture("person/avery");
        claim(&store, &key, "step-run.state",
            json!({"status":"ready","readiness_epoch":1,"reason":"x".repeat(MAX_PROOF_BYTES as usize + 1)}));
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());
        let (store, key) = fixture("person/avery");
        claim(&store, &key, "step-run.state", json!({"status":"ready","readiness_epoch":1}));
        for _ in 0..MAX_PROOF_CLAIMS {
            claim(&store, &key, "work.progress", json!({"attempt":1,"summary":"progress"}));
        }
        assert!(qualify(&store.readers.get(), "person/avery", &key, now_ms()).unwrap().is_none());
    }
}
