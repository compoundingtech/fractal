use super::*;
use fractal_schema::notice_owner::Declaration;
use smallclaims::fleet::MemberState;

/// An explicit desired binding, not an acknowledgement or historical authorization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Binding {
    pub declaration_claim: String,
    pub subject: String,
    pub member: String,
    pub incarnation: String,
    pub member_key: String,
}

fn unavailable(reason: impl std::fmt::Display) -> FractalError {
    FractalError::new("notices-unavailable", format!("notice owner unavailable: {reason}"))
}

fn retained(connection: &Connection) -> Result<BTreeSet<String>, FractalError> {
    connection.prepare_cached(
        "SELECT subject FROM desired WHERE kind='notice-owner' OR (subject GLOB 'notice-owner/*' AND kind<>'stop')
         UNION SELECT subject_hint FROM replica_records
         WHERE subject_hint GLOB 'notice-owner/*' AND state IN ('unknown','invalid')",
    ).map_err(internal)?.query_map([], |row| row.get(0)).map_err(internal)?
        .collect::<rusqlite::Result<_>>().map_err(internal)
}

fn available(connection: &Connection, subject: &str) -> Result<(), FractalError> {
    let selected = current_desired_row(connection, subject).map_err(unavailable)?;
    if !desired_conflicts_at(connection, subject,
        selected.as_ref().map(|row| row.claim_id.as_str()), None).map_err(internal)?.is_empty() {
        return Err(unavailable("competing desired declarations"));
    }
    if let Some(reason) = has_unknown_claim_at(connection, subject, None).map_err(internal)? {
        return Err(unavailable(reason));
    }
    let blocked: Option<String> = connection.query_row(
        "SELECT unavailable FROM identity_heads WHERE identity=COALESCE(
             (SELECT identity FROM identity_names WHERE name=?1),?1)",
        [subject], |row| row.get(0),
    ).optional().map_err(internal)?.flatten();
    if let Some(reason) = blocked { return Err(unavailable(reason)); }
    Ok(())
}

fn member_key(connection: &Connection, declaration: &Declaration) -> Result<String, FractalError> {
    let membership = smallclaims::store::fleet_membership_tx(connection).map_err(internal)?;
    match membership.state(declaration.member.trim_start_matches("host/")) {
        MemberState::Current(member) if member.admitted_claim == declaration.incarnation => {
            Ok(member.member_key.clone())
        }
        MemberState::Current(_) => Err(unavailable("admission incarnation mismatch")),
        MemberState::Conflicted(_) => Err(unavailable("conflicted member")),
        MemberState::Ended(_) | MemberState::LegacyRemoved(_) => Err(unavailable("ended member")),
        MemberState::NotMember => Err(unavailable("not an admitted member")),
    }
}

pub fn resolve(connection: &Connection) -> Result<Binding, FractalError> {
    let subjects = retained(connection)?;
    if subjects.len() != 1 { return Err(unavailable("expected exactly one retained declaration")); }
    let subject = subjects.into_iter().next().expect("one declaration");
    available(connection, &subject)?;
    let row = current_desired_row(connection, &subject).map_err(unavailable)?
        .filter(|row| row.kind == "notice-owner" && row.member.is_none() && row.owner_run.is_none()
            && row.owner_generation.is_none())
        .ok_or_else(|| unavailable("missing or ended declaration"))?;
    let desired: Value = serde_json::from_str(&row.body).map_err(unavailable)?;
    let declaration = Declaration::from_desired(&desired).map_err(|error| unavailable(error.message))?;
    let member_key = member_key(connection, &declaration)?;
    Ok(Binding { declaration_claim: row.claim_id, subject, member: declaration.member,
        incarnation: declaration.incarnation, member_key })
}

/// Validate the proposed singleton without electing or replacing another retained holder.
pub fn validate_apply(connection: &Connection, intent: &NormalizedIntent) -> Result<(), FractalError> {
    let candidates = intent.subjects.values().filter(|desired| desired.kind == "notice-owner")
        .collect::<Vec<_>>();
    if candidates.is_empty() { return Ok(()); }
    let mut subjects = retained(connection)?;
    subjects.retain(|subject| !intent.subjects.get(subject).is_some_and(|desired| desired.kind == "stop"));
    subjects.extend(candidates.iter().map(|desired| desired.subject.clone()));
    if subjects.len() != 1 || candidates.len() != 1 {
        return Err(unavailable("a second retained declaration is forbidden"));
    }
    for desired in candidates {
        // Graph apply's normal expected-head check permits an explicit causal repair.
        // Resolution remains unavailable until that repair is actually committed.
        if !desired.subject.starts_with("notice-owner/") || desired.member.is_some()
            || desired.owner_run.is_some() || desired.owner_generation.is_some() || desired.owner_step.is_some() {
            return Err(unavailable("declaration must be a standalone root"));
        }
        let declaration = Declaration::from_desired(&desired.desired)
            .map_err(|error| unavailable(error.message))?;
        member_key(connection, &declaration)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "notice_owner_tests.rs"]
mod tests;
