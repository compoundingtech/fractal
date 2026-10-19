//! Validation for public direct-message registrations on existing observer declarations.
use super::*;

pub fn validate_actor(
    intent: &NormalizedIntent,
    actor: Option<&str>,
) -> Result<(), St3Error> {
    if intent.direct_message_registrations.is_empty() {
        return Ok(());
    }
    let actor = actor.ok_or_else(|| {
        St3Error::new(
            "missing-publication-actor",
            "message subscription registration needs an actor",
        )
    })?;
    if !(actor.starts_with("person/") || actor.starts_with("agent/"))
        || st3_schema::registry().validate_subject(actor).is_err()
    {
        return Err(St3Error::new(
            "invalid-registration-actor",
            "message subscription registration needs a person or agent actor",
        ));
    }
    for subject in &intent.direct_message_registrations {
        let spec = crate::graph::subscription_spec(&intent.subjects[subject].desired).unwrap();
        if spec.to != actor {
            return Err(St3Error::new(
                "foreign-registration-recipient",
                "a public message subscription must be published by its recipient",
            ));
        }
    }
    Ok(())
}

pub fn validate(
    connection: &Connection,
    intent: &NormalizedIntent,
    index: u64,
) -> Result<(), St3Error> {
    for subject in &intent.direct_message_registrations {
        let desired = &intent.subjects[subject];
        let subscription = crate::graph::subscription_spec(&desired.desired).ok_or_else(|| {
            St3Error::new(
                "invalid-message-registration",
                "message registration needs a subscription",
            )
        })?;
        let existing = connection
            .query_row(
                "SELECT kind, body FROM desired WHERE subject=?1",
                [subject],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(internal)?;
        if let Some((kind, body)) = existing {
            let body: Value = serde_json::from_str(&body).map_err(internal)?;
            if kind != desired.kind
                || crate::graph::subscription_spec(&body).as_ref() != Some(&subscription)
            {
                return Err(St3Error::new(
                    "message-registration-conflict",
                    "an existing subscription cannot be replaced or retargeted by public registration",
                ));
            }
        }
        let declared = |subject: &str| -> Result<Option<(String, Value)>, St3Error> {
            connection.query_row(
                "SELECT desired.kind, desired.body FROM desired JOIN claims ON claims.id=desired.claim_id
                 WHERE desired.subject=?1 AND claims.store_index<=?2",
                params![subject, index], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .optional().map_err(internal)?
                .map(|(kind, body)| serde_json::from_str(&body).map(|body| (kind, body)).map_err(internal)).transpose()
        };
        let observer = declared(&subscription.observer)?
            .filter(|(kind, _)| kind == "observer")
            .and_then(|(_, body)| crate::graph::observer_spec(&body));
        if observer.as_ref().is_none_or(|observer| observer.stopped) {
            return Err(St3Error::new(
                "missing-registration-observer",
                format!(
                    "subscription `{subject}` needs an already-declared live observer `{}`",
                    subscription.observer
                ),
            ));
        }
        if observer
            .as_ref()
            .is_some_and(|observer| observer.provider != "github.repository")
        {
            return Err(St3Error::new(
                "invalid-registration-observer",
                "main Performance registration needs a github.repository observer",
            ));
        }
        if declared(&subscription.to)?.is_none_or(|(kind, _)| kind != "agent")
            || declaration_stopped_tx(connection, &subscription.to)
                .map_err(internal)?
                .is_some()
        {
            return Err(St3Error::new(
                "missing-registration-recipient",
                format!(
                    "subscription `{subject}` needs an already-declared agent `{}`",
                    subscription.to
                ),
            ));
        }
    }
    Ok(())
}

/// Publication and replicated projection share the original-author check. The public API
/// reparses KDL; a replicated main-field registration is classified from its durable spec.
pub fn validate_publisher(
    connection: &Connection,
    intent: &NormalizedIntent,
    actor: Option<&str>,
) -> Result<(), St3Error> {
    validate_actor(intent, actor)?;
    for subject in &intent.direct_message_registrations {
        let existing_author: Option<Option<String>> = connection.query_row(
            "SELECT claims.actor FROM desired JOIN claims ON claims.id=desired.claim_id WHERE desired.subject=?1",
            [subject], |row| row.get(0)).optional().map_err(internal)?;
        if existing_author.is_some_and(|author| author.as_deref() != actor) {
            return Err(St3Error::new(
                "message-registration-author",
                "only the original author can re-register a public subscription",
            ));
        }
    }
    Ok(())
}

pub fn validate_replicated(
    connection: &Transaction<'_>,
    claim: &ClaimRecord,
    desired: &DesiredSubject,
) -> Result<(), St3Error> {
    if desired.owner_run.is_some() || desired.kind != "subscription" {
        return Ok(());
    }
    let Some(spec) = crate::graph::subscription_spec(&desired.desired) else {
        return Ok(());
    };
    if !spec
        .fields
        .iter()
        .any(|field| field == crate::resource::github_workflows::PERFORMANCE_FAILURES_FIELD)
        || spec.watch.is_some()
    {
        return Ok(());
    }
    if spec.delivery != "message"
        || spec.fields != [crate::resource::github_workflows::PERFORMANCE_FAILURES_FIELD]
        || spec.batch_every_ms.is_some()
        || spec.condition.is_some()
        || spec.owner_message
        || spec.stopped
    {
        return Err(St3Error::new(
            "invalid-message-registration",
            "unmanaged main Performance registrations must be simple direct messages",
        ));
    }
    let intent = NormalizedIntent {
        direct_message_registrations: BTreeSet::from([desired.subject.clone()]),
        schema: "st3.v1".into(),
        source_hash: String::new(),
        subjects: BTreeMap::from([(desired.subject.clone(), desired.clone())]),
        missions: BTreeMap::new(),
        mission_provenance: BTreeMap::new(),
        mission_runs: BTreeMap::new(),
        planning_sessions: BTreeMap::new(),
        resource_refreshes: Vec::new(),
        replica_repairs: Vec::new(),
        document_refs: BTreeSet::new(),
        deprecated_syntax: BTreeSet::new(),
        normalized: Value::Null,
    };
    validate_publisher(connection, &intent, claim.actor.as_deref())?;
    validate(
        connection,
        &intent,
        current_index_tx(connection).map_err(internal)?,
    )
}

#[cfg(any(test, feature = "test-support"))]
pub const PERFORMANCE_REGISTRATION: &str = r#"version 2
subscription "fixture/main-performance-p0" {
  observer "observer/github/compoundingtech/smalltalk"
  on "main_performance_failures"
  to "agent/fleet/fixture-project/speed"
  delivery "message"
}
"#;

#[cfg(any(test, feature = "test-support"))]
pub const PERFORMANCE_DECLARATIONS: &str = r#"version 2
resource "github/compoundingtech/smalltalk" { kind "vcs.repository" }
observer "github/compoundingtech/smalltalk" {
  resource "resource/github/compoundingtech/smalltalk"
  provider "github.repository"
  locator "compoundingtech/smalltalk"
  field "issues"
}
agent "fleet/fixture-project/speed" { workspace "."; command "true" }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_performance_failures_registration_does_not_allow_other_top_level_runtime() {
        for source in [
            "version 2\nobserver \"other\" { resource \"resource/repo\"; provider \"github.repository\"; locator \"owner/repo\"; field \"issues\" }",
            "version 2\nexec \"other\" { command \"true\" }",
            "version 2\nlane \"other\" { entries \"example\" }",
            "version 2\nsubscription \"other\" { observer \"observer/existing\"; delivery \"mission\" { mission \"example\"; resource \"source\"; workspace \"/tmp\" } }",
            "version 2\nsubscription \"other\" { observer \"observer/existing\"; to \"agent/existing\"; on \"issues\"; delivery \"message\" { every \"30m\" } }",
        ] {
            assert_eq!(
                crate::graph::parse_intent(source, "node").unwrap_err().code,
                "runtime-outside-mission"
            );
        }
        // Names belong in the deployment fixture, not a parser allowlist.
        assert!(
            crate::graph::parse_intent(
                &PERFORMANCE_REGISTRATION.replace(
                    "on \"main_performance_failures\"",
                    "on \"main_performance_failures\"; on \"issues\""
                ),
                "node"
            )
            .is_err()
        );
        for field in ["issues", "pull_requests"] {
            assert!(
                crate::graph::parse_intent(
                    &PERFORMANCE_REGISTRATION.replace("main_performance_failures", field),
                    "node"
                )
                .is_err()
            );
        }
        let generic = PERFORMANCE_REGISTRATION
            .replace("fixture/main-performance-p0", "another/registration")
            .replace(
                "observer/github/compoundingtech/smalltalk",
                "observer/already-declared",
            )
            .replace(
                "agent/fleet/fixture-project/speed",
                "agent/another/recipient",
            );
        assert!(crate::graph::parse_intent(&generic, "node").is_ok());
    }
}
