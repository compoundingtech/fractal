use super::*;
use crate::rollout::{Operation, Policy, Selection};

pub fn selection(connection: &Connection, subject: &str) -> Result<Option<Selection>, St3Error> {
    selection_at(connection, subject, None)
}

/// The selection at a client snapshot, or the live one when `at` is `None`.
fn selection_at(
    connection: &Connection,
    subject: &str,
    at: Option<u64>,
) -> Result<Option<Selection>, St3Error> {
    owned_sets::guard_member(connection, subject)?;
    let Some(row) = desired_row_at(connection, subject, at).map_err(internal)? else {
        return Ok(None);
    };
    let desired = match at {
        None => connection.query_row("SELECT subject,kind,body,member,owner_run,owner_generation,owner_step FROM desired WHERE subject=?1",
            [subject],desired_from_row).map_err(internal)?,
        Some(_) => {
            let desired = serde_json::from_str(&row.body).unwrap_or(Value::Null);
            DesiredSubject {
                accountable: fractal_schema::accountable::from_desired(&desired)
                    .map_err(internal)?
                    .map(str::to_owned),
                subject: subject.into(),
                kind: row.kind.clone(),
                desired,
                member: row.member.as_deref().and_then(|value| serde_json::from_str(value).ok()),
                owner_run: row.owner_run.clone(),
                owner_generation: row.owner_generation.clone(),
                owner_step: None,
            }
        }
    };
    for view in owned_sets::selected(connection, at)? {
        let members = owned_sets::effective_members(connection, &view, at)?;
        let Some((member, _, _)) = members.get(subject) else {
            continue;
        };
        // Retirement keeps the last authored seat policy even though its desired node is stop.
        let manual = if desired.kind == "stop" {
            owned_sets::manual_member_at(connection, member, at)?
        } else {
            crate::rollout::manual(&desired)
        };
        let policy = match view.receipt.rollout.clone() {
            Some(policy) => policy,
            None if manual => Policy::when_idle(30 * 60 * 1000, false),
            None => return Ok(None),
        };
        let actor = claim_by_id_tx(connection, &view.claim)
            .map_err(internal)?
            .and_then(|claim| claim.actor);
        let target = crate::rollout::target(&desired).map_err(internal)?;
        return Ok(Some(Selection {
            set: view.id,
            receipt: view.claim,
            source: view.receipt.source,
            policy,
            manual,
            desired_token: row.claim_id,
            target,
            desired,
            actor,
        }));
    }
    Ok(None)
}

pub fn operation(connection: &Connection, subject: &str) -> Result<Option<Operation>, St3Error> {
    let Some(selected) = selection(connection, subject)? else {
        return Ok(None);
    };
    operation_for_selection(connection, subject, &selected)
}

fn operation_for_selection(
    connection: &Connection,
    subject: &str,
    selected: &Selection,
) -> Result<Option<Operation>, St3Error> {
    operation_for_selection_at(connection, subject, selected, None)
}

/// The operation `selected` has, from the claims accepted at or before `at`.
fn operation_for_selection_at(
    connection: &Connection,
    subject: &str,
    selected: &Selection,
    at: Option<u64>,
) -> Result<Option<Operation>, St3Error> {
    smallclaims::touched::note_read(|| subject.to_owned());
    let mut query = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h7e7d5bb183d4 owner=fractal_store::store::rollouts TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
         FROM claims WHERE subject=?1 AND store_index<=?2 AND kind IN ('runtime.action.requested','runtime.action.succeeded','runtime.action.failed','runtime.action.deadline-reached')
         ORDER BY CANONICAL_ASC(claims)" )).map_err(internal)?;
    let claims = query
        .query_map(params![subject, at.unwrap_or(i64::MAX as u64)], claim_from_row)
        .map_err(internal)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?;
    let owner = selected
        .desired
        .member
        .as_ref()
        .map(|m| m.host.clone())
        .or_else(|| {
            selected_actual_source_at(connection, subject, at, None)
                .ok()
                .and_then(|v| v.1)
        });
    let request = claims.iter().rev().find(|claim| {
        claim.body.pointer("/fields/action").and_then(Value::as_str) == Some("rollout")
            && claim.kind == "runtime.action.requested"
            && claim.actor.is_some()
            && owner.as_deref() == Some(claim.origin.as_str())
    });
    let Some(request) = request else {
        return Ok(None);
    };
    let Some(data) = request.body.pointer("/fields/rollout") else {
        return Ok(None);
    };
    let Ok(mut operation) = serde_json::from_value::<Operation>(data.clone()) else {
        return Ok(None);
    };
    operation.id = request.id.clone();
    operation.requested_by = request.actor.clone();
    // The initial request can carry a positively stopped predecessor after supersession.
    for claim in claims.iter().filter(|c| {
        c.origin == request.origin
            && c.actor == request.actor
            && c.body.pointer("/fields/operation").and_then(Value::as_str)
                == Some(request.id.as_str())
    }) {
        if let Some(phase) = claim
            .body
            .pointer("/fields/operation_status")
            .and_then(Value::as_str)
        {
            if phase == "drain-ack" {
                operation.drain_ack = Some(claim.accepted_at_unix_ms);
                continue;
            }
            if phase == "start-attempted" {
                if let Some(token) = claim
                    .body
                    .pointer("/fields/rollout/desired_token")
                    .and_then(Value::as_str)
                {
                    operation.desired_token = token.into();
                }
                operation.start_attempted = true;
                continue;
            }
            operation.phase = if phase == "failed-replacement" {
                "failed"
            } else {
                phase
            }
            .into();
            operation.phase_at_unix_ms = claim.accepted_at_unix_ms;
            if let Some(state) = claim.body.pointer("/fields/rollout") {
                if let Some(token) = state["desired_token"].as_str() {
                    operation.desired_token = token.into();
                }
                operation.native_session_id = state["native_session_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or(operation.native_session_id);
                operation.native_account = state["native_account"]
                    .as_str()
                    .map(str::to_owned)
                    .or(operation.native_account);
                operation.native_path = state["native_path"]
                    .as_str()
                    .map(str::to_owned)
                    .or(operation.native_path);
                operation.replacement_incarnation = state["replacement_incarnation"]
                    .as_str()
                    .map(str::to_owned)
                    .or(operation.replacement_incarnation);
                operation.forced |= state["forced"].as_bool().unwrap_or(false);
                operation.blocking = state["blocking"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                operation.reason = state["reason"].as_str().map(str::to_owned);
            }
        }
    }
    if selected.set == operation.set
        && selected.target == operation.target
        && (selected.policy != operation.publication_policy
            || selected.manual != operation.publication_manual)
        && matches!(operation.phase.as_str(), "running" | "retired")
    {
        return Ok(None);
    }
    if selected.set != operation.set
        || selected.target != operation.target
        || (selected.policy != operation.publication_policy
            || selected.manual != operation.publication_manual)
    {
        operation.phase = "superseded".into();
    }
    Ok(Some(operation))
}

/// A pending person ask remains attached to the still-running retiring seat until
/// its existing work finishes or the original incarnation actually ends.
pub fn retiring_ask_live(
    connection: &Connection,
    ask: &ClaimRecord,
) -> Result<bool, St3Error> {
    let Some(subject) = ask.actor.as_deref() else {
        return Ok(false);
    };
    let retiring: bool = connection.query_row(
        "SELECT kind='stop' FROM desired WHERE subject=?1", [subject], |row| row.get(0),
    ).optional().map_err(internal)?.unwrap_or(false);
    if !retiring { return Ok(false); }
    let Some(selected) = selection(connection, subject)? else {
        return Ok(false);
    };
    if selected.desired.kind != "stop" {
        return Ok(false);
    }
    let fields = &ask.body["fields"];
    let Some(step) = fields["origin_step"].as_str() else {
        return Ok(false);
    };
    let Some(attempt) = fields["origin_attempt"].as_u64() else {
        return Ok(false);
    };
    let actual = latest_actual(connection, subject)
        .map_err(internal)?
        .unwrap_or(Value::Null);
    let Some(incarnation) = actual["incarnation_id"]
        .as_str()
        .filter(|_| actual["status"] == "running")
    else {
        return Ok(false);
    };
    if let Some(operation) = operation(connection, subject)? {
        return Ok(operation.holds_seat()
            && operation.old_incarnation == incarnation
            && operation.allowed_work.get(step).copied().map(u64::from) == Some(attempt));
    }
    // Publication can precede the owner's first reconciliation pass.
    connection
        .query_row(
            "SELECT status='waiting-person' AND attempt=?2 FROM step_runs WHERE subject=?1",
            params![step, attempt],
            |row| row.get(0),
        )
        .optional()
        .map(|result| result.unwrap_or(false))
        .map_err(internal)
}

pub fn intake_held(
    connection: &Connection,
    subject: &str,
    step: &str,
) -> Result<bool, St3Error> {
    let Some(operation) = operation(connection, subject)?.filter(|o| o.holds_intake()) else {
        return Ok(false);
    };
    let attempt: Option<u32> = connection
        .query_row(
            "SELECT attempt FROM step_runs WHERE subject=?1",
            [step],
            |r| r.get(0),
        )
        .optional()
        .map_err(internal)?;
    Ok(operation.allowed_work.get(step).copied() != attempt || attempt.is_none())
}

pub fn message_allowed(
    connection: &Connection,
    message: &MessageView,
) -> Result<bool, St3Error> {
    if message.status != "sent" {
        return Ok(true);
    }
    let hold = intake_hold(connection, &message.to)?;
    message_allowed_under(connection, hold.as_ref(), message)
}

pub fn intake_hold(connection: &Connection, subject: &str) -> Result<Option<Operation>, St3Error> {
    Ok(operation(connection, subject)?.filter(|o| o.holds_intake()))
}

/// `hold` is the recipient's `intake_hold`, read once by a caller checking many messages.
pub fn message_allowed_under(
    connection: &Connection,
    hold: Option<&Operation>,
    message: &MessageView,
) -> Result<bool, St3Error> {
    let Some(operation) = hold else {
        return Ok(true);
    };
    if message.status != "sent" {
        return Ok(true);
    }
    if message.from == "daemon/runtime"
        && crate::silent::message_tag_value(message.tags.iter().map(String::as_str), "work:")
            .is_some_and(|value| {
            let mut parts = value.rsplitn(4, '@');
            let _incarnation = parts.next();
            let _epoch = parts.next();
            let attempt = parts.next().and_then(|s| s.parse::<u32>().ok());
            parts
                .next()
                .is_some_and(|step| operation.allowed_work.get(step).copied() == attempt)
            })
    {
        return Ok(true);
    }
    let mut parent = message.in_reply_to.clone();
    let mut visited = BTreeSet::new();
    while let Some(subject) = parent {
        if !visited.insert(subject.clone()) || visited.len() > 64 {
            break;
        }
        let index: Option<u64> = connection
            .query_row(
                "SELECT MIN(store_index) FROM claims WHERE subject=?1",
                [&subject],
                |row| row.get(0),
            )
            .map_err(internal)?;
        let Some(index) = index else {
            break;
        };
        let prior = message_view_tx(connection, &subject, index).map_err(internal)?;
        let request_index: u64 = connection
            .query_row(
                "SELECT store_index FROM claims WHERE id=?1",
                [&operation.id],
                |row| row.get(0),
            )
            .map_err(internal)?;
        if index < request_index
            && (prior.from == message.to || (prior.to == message.to && prior.status != "sent"))
        {
            return Ok(true);
        }
        parent = prior.in_reply_to;
    }
    Ok(false)
}

impl Store {
    pub fn rollout_message_allowed(&self, message: &MessageView) -> Result<bool, St3Error> {
        message_allowed(&self.readers.get(), message)
    }
    pub fn rollout_selection(&self, subject: &str) -> Result<Option<Selection>, St3Error> {
        selection(&self.readers.get(), subject)
    }
    /// `rollout_selection` at a client snapshot.
    pub(crate) fn rollout_selection_at(
        &self,
        subject: &str,
        at: Option<u64>,
    ) -> Result<Option<Selection>, St3Error> {
        selection_at(&self.readers.get(), subject, at)
    }
    /// The rollout of the declaration selected at a client snapshot, from claims accepted by then.
    pub(crate) fn rollout_at(
        &self,
        subject: &str,
        at: Option<u64>,
    ) -> Result<Option<Operation>, St3Error> {
        if at.is_none() {
            return self.rollout(subject);
        }
        let connection = self.readers.get();
        let Some(selected) = selection_at(&connection, subject, at)? else {
            return Ok(None);
        };
        operation_for_selection_at(&connection, subject, &selected, at)
    }
    pub fn rollout(&self, subject: &str) -> Result<Option<Operation>, St3Error> {
        self.rollout_and_selection(subject)
            .map(|(operation, _)| operation)
    }
    /// A status read uses the publication that selected its operation, including when that
    /// operation is absent or superseded, rather than selecting the declaration a second time.
    pub fn rollout_and_selection(
        &self,
        subject: &str,
    ) -> Result<(Option<Operation>, Option<Selection>), St3Error> {
        let connection = self.readers.get();
        let selected = selection(&connection, subject)?;
        let mut operation = selected
            .as_ref()
            .map(|selected| operation_for_selection(&connection, subject, selected))
            .transpose()?
            .flatten();
        if let Some(operation) = operation.as_mut().filter(|o| o.phase == "draining") {
            operation.blocking =
                crate::rollout::blockers(self, subject, operation).map_err(internal)?;
        }
        Ok((operation, selected))
    }
}
