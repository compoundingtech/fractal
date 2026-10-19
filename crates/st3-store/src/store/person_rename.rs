//! One-shot repair of current person routing. Historical actors and published missions stay put.
use super::*;
use crate::model::{PersonRenameReport, PersonRenameRequest};

impl Store {
    /// Reassign current open work and pending reviews in one writer transaction. The completion
    /// claim is the durable idempotency receipt, including after reopen or replication.
    pub fn rename_person(
        &self,
        input: &PersonRenameRequest,
    ) -> Result<PersonRenameReport, St3Error> {
        for person in [&input.old_person, &input.new_person] {
            validate_claim_subject(person)?;
            if !person.starts_with("person/") || person.matches('/').count() != 1 || person.contains('*') {
                return Err(St3Error::new("invalid-person-rename", "OLD and NEW must be complete person subjects"));
            }
        }
        if input.old_person == input.new_person || input.idempotency_key.trim().is_empty() {
            return Err(St3Error::new("invalid-person-rename", "a rename needs distinct people and a nonempty idempotency key"));
        }
        validate_claim_subject(&input.actor)?;
        self.connection.batched(|tx| {
            let receipt = tx.query_row(/* sqlite-large-value: allow-for-now lv-h0bb078bcf6e7 owner=st3_store::store::person_rename TODO=trace-statement-and-value-representation */ &canonical_sql(
                "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
                 FROM claims WHERE kind='person.rename.completed'
                 AND json_extract(body,'$.idempotency_key')=?1 ORDER BY CANONICAL_ASC(claims) LIMIT 1"),
                [&input.idempotency_key], claim_from_row).optional().map_err(internal)?;
            if let Some(receipt) = receipt {
                if receipt.actor.as_deref() != Some(input.actor.as_str())
                    || receipt.body["fields"]["old_person"] != input.old_person
                    || receipt.body["fields"]["new_person"] != input.new_person
                {
                    return Err(St3Error::new("idempotency-conflict", "this rename key already names another request"));
                }
                if input.actor != input.old_person && input.actor != input.new_person {
                    return Err(St3Error::new("person-rename-forbidden", "only OLD or NEW may repair their person identity"));
                }
                if winner_person_tx(tx, &input.old_person).map_err(internal)?
                    .is_some_and(|winner| winner != input.new_person)
                {
                    return Err(St3Error::new("person-rename-conflict", "this repair lost to another canonical completed repair of OLD"));
                }
                return serde_json::from_value(receipt.body["fields"]["report"].clone()).map_err(internal);
            }
            if input.actor != input.old_person && input.actor != input.new_person {
                return Err(St3Error::new("person-rename-forbidden", "only OLD or NEW may repair their person identity"));
            }
            let competing: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='person.rename.completed'
                 AND json_extract(body,'$.fields.new_person')<>?2)",
                params![input.old_person, input.new_person], |row| row.get(0)).map_err(internal)?;
            if competing {
                return Err(St3Error::new("person-rename-conflict", "OLD already has a completed repair to another person"));
            }
            if !known_person_tx(tx, &input.new_person).map_err(internal)? {
                return Err(St3Error::new("unknown-person", "NEW must already be known from stored person identity or routing evidence"));
            }
            let mut report = PersonRenameReport {
                old_person: input.old_person.clone(),
                new_person: input.new_person.clone(),
                reassigned_steps: Vec::new(),
                reassigned_reviews: Vec::new(),
                run_requesters: Vec::new(),
                missions_to_republish: Vec::new(),
                unchanged_attention_requests: Vec::new(),
                unchanged_fault_cards: Vec::new(),
                unchanged_revision_approvals: Vec::new(),
                unchanged_planning_launch_approvals: Vec::new(),
                live_run_references: BTreeMap::new(),
                published_mission_references: BTreeMap::new(),
                namespace_notice: "Client-derived custom/person namespaces are not automatically moved; no aliases or namespace migrations are created.".into(),
            };
            report.unchanged_attention_requests = unchanged_attention_requests_tx(tx, &input.old_person).map_err(internal)?;
            for fault in self.fault_snapshot(now_ms()).map_err(internal)? {
                if fault.item.person == input.old_person {
                    report.unchanged_fault_cards.push(fault.item.episode);
                }
            }
            for item in self.broken_gate_items(Some(&input.old_person), now_ms()).map_err(internal)? {
                report.unchanged_fault_cards.push(item.episode);
            }
            for item in self.mission_run_attention_items(Some(&input.old_person)).map_err(internal)? {
                match item.kind.as_str() {
                    "revision-approval" => report.unchanged_revision_approvals.push(item.subject),
                    "launch-approval" => report.unchanged_planning_launch_approvals.push(item.subject),
                    _ => {}
                }
            }
            let mut runs = tx.prepare(
                "SELECT m.id,m.requester,r.body FROM mission_runs m
                 JOIN run_generations g ON g.id=m.current_generation_id
                 JOIN mission_revisions r ON r.mission_id=m.mission_id AND r.revision=g.revision
                 WHERE m.status NOT IN ('completed','failed','cancelled') ORDER BY m.id").map_err(internal)?;
            for row in runs.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))).map_err(internal)? {
                let (run, requester, body) = row.map_err(internal)?;
                if !person_work::run_live(tx, &run, None, false).map_err(internal)? { continue; }
                let subject = format!("mission-run/{run}");
                let mut references = routing_references(&serde_json::from_str::<Value>(&body).map_err(internal)?, &input.old_person);
                if requester == input.old_person {
                    report.run_requesters.push(subject.clone());
                    references.push("/requester".into());
                }
                references.sort();
                if !references.is_empty() { report.live_run_references.insert(subject, references); }
            }
            let mut missions = tx.prepare(
                "SELECT r.body FROM mission_definitions d JOIN mission_revisions r
                 ON r.mission_id=d.mission_id AND r.revision=d.revision ORDER BY d.mission_id").map_err(internal)?;
            for body in missions.query_map([], |row| row.get::<_, String>(0)).map_err(internal)? {
                let mission: MissionSpec = serde_json::from_str(&body.map_err(internal)?).map_err(internal)?;
                let references = routing_references(&serde_json::to_value(&mission).map_err(internal)?, &input.old_person);
                if !references.is_empty() {
                    report.missions_to_republish.push(mission.subject.clone());
                    report.published_mission_references.insert(mission.subject, references);
                }
            }
            for subjects in [&mut report.unchanged_attention_requests, &mut report.unchanged_fault_cards,
                &mut report.unchanged_revision_approvals, &mut report.unchanged_planning_launch_approvals] {
                subjects.sort();
                subjects.dedup();
            }
            // The receipt fence above and both reassignments share this writer transaction:
            // another destination cannot observe or move only part of the repaired routing.
            report.reassigned_steps = person_work::reassign_person_steps_tx(tx, &self.origin, input)?;
            report.reassigned_reviews = human_review_reassignment::reassign_reviews_tx(tx, &self.origin, input)?;
            let mut remaining_steps = tx.prepare(
                "SELECT s.run_id,s.subject FROM step_runs s JOIN mission_runs m
                 ON m.id=s.run_id AND m.current_generation_id=s.generation_id
                 WHERE s.assignee=?1 AND m.status NOT IN ('completed','failed','cancelled')
                 ORDER BY s.run_id,s.subject").map_err(internal)?;
            for row in remaining_steps.query_map([&input.old_person], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(internal)? {
                let (run, step) = row.map_err(internal)?;
                if person_work::run_live(tx, &run, None, false).map_err(internal)? {
                    let segment = step.replace('~', "~0").replace('/', "~1");
                    report.live_run_references.entry(format!("mission-run/{run}")).or_default()
                        .push(format!("/step_runs/{segment}/assignee"));
                }
            }
            for references in report.live_run_references.values_mut() {
                references.sort();
                references.dedup();
            }
            append_claim_tx(tx, &self.origin, &input.old_person, "person.rename.completed", Some(&input.actor),
                &json!({"fields": {"old_person": input.old_person, "new_person": input.new_person, "report": report},
                    "idempotency_key": input.idempotency_key}), &[], None).map_err(claim_append_error)?;
            Ok(report)
        }).map_err(internal)?
    }
}

fn known_person_tx(connection: &Connection, person: &str) -> Result<bool> {
    let known: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM claims
         WHERE kind NOT IN ('person.rename.completed','gate.reviewer-reassigned','work.person-reassigned')
         AND (actor=?1 OR subject=?1 OR json_extract(body,'$.fields.reviewer')=?1
           OR json_extract(body,'$.fields.person')=?1 OR json_extract(body,'$.fields.owner')=?1
           OR json_extract(body,'$.fields.requester')=?1 OR json_extract(body,'$.fields.assignee')=?1))
         OR EXISTS(SELECT 1 FROM mission_runs WHERE requester=?1)
         OR EXISTS(SELECT 1 FROM planning_sessions WHERE requester=?1)",
        [person], |row| row.get(0))?;
    if known { return Ok(true); }
    let mut missions = connection.prepare("SELECT body FROM mission_revisions ORDER BY mission_id,revision")?;
    for body in missions.query_map([], |row| row.get::<_, String>(0))? {
        if !routing_references(&serde_json::from_str::<Value>(&body?)?, person).is_empty() { return Ok(true); }
    }
    Ok(false)
}

fn unchanged_attention_requests_tx(connection: &Connection, person: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT request.subject FROM claims request
         WHERE request.kind='attention.requested' AND json_extract(request.body,'$.fields.reviewer')=?1
         AND NOT EXISTS(SELECT 1 FROM claims resolved WHERE resolved.subject=request.subject
           AND resolved.kind='attention.resolved') ORDER BY request.subject")?;
    let subjects = statement.query_map([person], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(subjects)
}

/// Match structured routes, never prose, goals, historical actors or arbitrary input strings.
/// Recursion covers retained nested missions and every loop round/branch/gate without rewriting them.
pub fn routing_references(value: &Value, person: &str) -> Vec<String> {
    fn visit(value: &Value, person: &str, path: &str, route: bool, found: &mut Vec<String>) {
        match value {
            Value::String(subject) if route && subject == person => found.push(path.to_owned()),
            Value::Object(fields) => for (name, child) in fields {
                if name == "inputs" { continue; }
                let segment = name.replace('~', "~0").replace('/', "~1");
                let route = matches!(name.as_str(), "assignee" | "assigned_to" | "available_to"
                    | "agent" | "agents" | "reviewer" | "revision_reviewer" | "revision_owners" | "report_to");
                visit(child, person, &format!("{path}/{segment}"), route, found);
            },
            Value::Array(values) => for (index, child) in values.iter().enumerate() {
                visit(child, person, &format!("{path}/{index}"), route, found);
            },
            _ => {}
        }
    }
    let mut found = Vec::new();
    visit(value, person, "", false, &mut found);
    found.sort();
    found
}

/// Both work and human-review reducers must choose the same replicated receipt winner.
/// Before any receipt arrives, existing episode fences still govern provisional assignments.
/// Receipt arrival invalidates affected run trees so canonical replay removes a losing route.
pub(super) fn accepted_reassignment_sql(alias: &str, previous: &str, destination: &str) -> String {
    let old = format!("json_extract({alias}.body,'$.fields.{previous}')");
    let new = format!("json_extract({alias}.body,'$.fields.{destination}')");
    format!(
        "(NOT EXISTS(SELECT 1 FROM claims rename_receipt
            WHERE rename_receipt.kind='person.rename.completed' AND rename_receipt.subject={old})
          OR {new}=(SELECT json_extract(rename_receipt.body,'$.fields.new_person')
            FROM claims rename_receipt
            WHERE rename_receipt.kind='person.rename.completed' AND rename_receipt.subject={old}
            ORDER BY {} LIMIT 1))",
        canonical::order_sql("rename_receipt", false),
    )
}

pub fn winner_person_tx(connection: &Connection, old: &str) -> Result<Option<String>> {
    Ok(connection.query_row(
        /* sqlite-large-value: allow-for-now lv-h3fc98290ad14 owner=st3_store::store::person_rename TODO=trace-statement-and-value-representation */
        &canonical_sql("SELECT json_extract(body,'$.fields.new_person') FROM claims
            WHERE kind='person.rename.completed' AND subject=?1
            ORDER BY CANONICAL_ASC(claims) LIMIT 1"),
        [old], |row| row.get(0)).optional()?)
}

