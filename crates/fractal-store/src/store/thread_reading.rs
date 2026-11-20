//! Owner-scoped observed-remove reminders. Immutable message receipts and thread lifecycle
//! never depend on this fold. Claims are authoritative; the existing subject/kind and
//! operation indexes serve narrow headers, so there is no new storage format or migration.
use super::*;
use fractal_schema::threads::{
    ThreadUnreadMark, ThreadUnreadReminder, UNREAD_CLEARED, UNREAD_MARKED,
};
use thread_metadata::{PersonAuthority, owned_creation};

pub enum Edit<'a> {
    Mark {
        anchor_message: &'a str,
        observed_marks: &'a [String],
    },
    Clear {
        observed_marks: &'a [String],
    },
}

/// Reauthorize inside append's transaction before any original-operation recovery.
/// The caller retains its operation key and exact observed set across retries.
pub fn apply_as(
    store: &Store,
    authority: PersonAuthority<'_>,
    person: &str,
    thread: &str,
    operation_key: &str,
    edit: Edit<'_>,
) -> Result<(ClaimRecord, bool), FractalError> {
    let (kind, anchor, observed) = match edit {
        Edit::Mark {
            anchor_message,
            observed_marks,
        } => (UNREAD_MARKED, Some(anchor_message), observed_marks),
        Edit::Clear { observed_marks } => (UNREAD_CLEARED, None, observed_marks),
    };
    let mut fields = BTreeMap::from([
        ("person".into(), json!(person)),
        ("observed_marks".into(), json!(observed)),
    ]);
    if let Some(anchor) = anchor {
        fields.insert("anchor_message".into(), json!(anchor));
    }
    let input = ClaimInput {
        subject: thread.into(),
        kind: kind.into(),
        actor: Some(person.into()),
        fields,
        evidence: vec![],
        expected_subject: None,
        idempotency_key: Some(operation_key.into()),
    };
    let authorize = |connection: &Connection| {
        authority.check(store, connection, person)?;
        store.thread_write_rollout(connection)?;
        let creation =
            owned_creation(connection, thread, person, false)?.ok_or_else(unavailable)?;
        if let Some(anchor) = anchor
            && !anchor_member(connection, &creation, person, thread, anchor, true)?
        {
            return Err(unavailable());
        }
        let headers = valid_headers(connection, person, thread, &creation)?;
        let valid_marks = headers
            .iter()
            .filter(|header| header.kind == UNREAD_MARKED && header.valid)
            .map(|header| header.id.as_str())
            .collect::<BTreeSet<_>>();
        for id in observed {
            if !valid_marks.contains(id.as_str()) {
                return Err(FractalError::new(
                    "invalid-thread-reminder-reference",
                    "observed marks must belong to this owner and thread",
                ));
            }
        }
        Ok(())
    };
    store.graph.ensure_principal_key(person)?;
    append_claim_with_commit_context(
        &store.graph,
        &input,
        None,
        None,
        None,
        None,
        ClaimCommitContext {
            authority: Some(&authorize),
            ..Default::default()
        },
    )
}

fn unavailable() -> FractalError {
    FractalError::new(
        "thread-unavailable",
        "the thread or reminder anchor is unavailable",
    )
}

pub fn reminder(
    store: &Store,
    person: &str,
    thread: &str,
) -> Result<Option<ThreadUnreadReminder>, FractalError> {
    store
        .graph
        .read_snapshot_unindexed(|| Ok(reminder_conn(&store.readers.get(), person, thread)))
        .map_err(internal)?
}

pub(super) fn reminder_conn(
    connection: &Connection,
    person: &str,
    thread: &str,
) -> Result<Option<ThreadUnreadReminder>, FractalError> {
    Ok(reminder_with_time_conn(connection, person, thread)?.map(|(reminder, _)| reminder))
}

/// Presentation order uses an operation's earliest validated equivalent signed receipt,
/// independent of which equivalent receipt is currently canonical. No new clock is stored.
pub(super) fn reminder_with_time_conn(
    connection: &Connection,
    person: &str,
    thread: &str,
) -> Result<Option<(ThreadUnreadReminder, u128)>, FractalError> {
    let creation = owned_creation(connection, thread, person, false)?.ok_or_else(unavailable)?;
    let headers = valid_headers(connection, person, thread, &creation)?;
    let removed = headers
        .iter()
        .filter(|header| header.valid)
        .flat_map(|header| header.observed.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let active = headers
        .iter()
        .filter(|header| {
            header.valid && header.kind == UNREAD_MARKED && !removed.contains(header.id.as_str())
        })
        .collect::<Vec<_>>();
    let Some(first) = active.first() else {
        return Ok(None);
    };
    let time = first.own_time;
    let marks = active
        .into_iter()
        .filter_map(|header| {
            header.anchor.as_ref().map(|anchor| ThreadUnreadMark {
                mark_id: header.id.clone(),
                anchor_message_id: anchor.clone(),
            })
        })
        .collect::<Vec<_>>();
    let Some(first) = marks.first() else {
        return Ok(None);
    };
    Ok(Some((
        ThreadUnreadReminder {
            anchor_message_id: first.anchor_message_id.clone(),
            marks,
        },
        time,
    )))
}

fn operation_time(connection: &Connection, operation: &str) -> Result<u128, FractalError> {
    let sql = format!(
        "SELECT claims.accepted_at_unix_ms FROM claims INDEXED BY claims_operation_index
        JOIN operations ON operations.id=?1 AND operations.state='active'
        WHERE json_extract(claims.body,'$._operation.id')=?1
          AND json_extract(claims.body,'$._operation.id') IS NOT NULL
          AND json_extract(claims.body,'$._operation.request_digest')=operations.request_digest
          AND {UNREPAIRED_CLAIM}
        ORDER BY length(claims.accepted_at_unix_ms),claims.accepted_at_unix_ms,claims.id LIMIT 1"
    );
    let time: String = connection
        .query_row(/* sqlite-large-value: allow-for-now lv-hc5e92b33924f owner=fractal_store::store::thread_reading TODO=trace-statement-and-value-representation */ &sql, [operation], |row| row.get(0))
        .map_err(internal)?;
    time.parse().map_err(internal)
}

struct Header {
    id: String,
    kind: String,
    actor: Option<String>,
    person: String,
    anchor: Option<String>,
    observed: Vec<String>,
    valid: bool,
    own_time: u128,
}

fn reminder_headers_sql() -> String {
    canonical_sql(&format!("SELECT json_extract(claims.body,'$._operation.id'), claims.kind, claims.actor,
        json_extract(claims.body,'$.fields.person'), json_extract(claims.body,'$.fields.anchor_message'),
        json_extract(claims.body,'$.fields.observed_marks')
        FROM claims INDEXED BY claims_subject_kind_index
        WHERE claims.subject=?1 AND claims.kind IN ('thread.unread-marked','thread.unread-cleared')
          AND {UNREPAIRED_CLAIM}
          AND EXISTS(SELECT 1 FROM operations WHERE operations.id=json_extract(claims.body,'$._operation.id')
              AND operations.canonical_claim_id=claims.id AND operations.state='active')
        ORDER BY CANONICAL_DESC(claims)"))
}

/// Missing references stay inert until the exact signed dependencies arrive. This
/// avoids permanently rejecting legitimate reordered replicas. Invalid/cyclic/foreign
/// references never remove anything. Topological resolution avoids recursive history walks.
fn valid_headers(
    connection: &Connection,
    person: &str,
    thread: &str,
    creation: &ClaimRecord,
) -> Result<Vec<Header>, FractalError> {
    headers_with_pending(connection, person, thread, creation, false)
}

pub(super) fn private_anchor_intent_conn(
    connection: &Connection,
    person: &str,
    thread: &str,
    anchor: &str,
) -> Result<bool, FractalError> {
    let creation = owned_creation(connection, thread, person, false)?.ok_or_else(unavailable)?;
    let headers = headers_with_pending(connection, person, thread, &creation, true)?;
    let removed = headers
        .iter()
        .filter(|header| header.valid)
        .flat_map(|header| header.observed.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    Ok(headers.iter().any(|header| {
        header.valid
            && header.kind == UNREAD_MARKED
            && header.anchor.as_deref() == Some(anchor)
            && !removed.contains(header.id.as_str())
    }))
}

fn headers_with_pending(
    connection: &Connection,
    person: &str,
    thread: &str,
    creation: &ClaimRecord,
    allow_pending: bool,
) -> Result<Vec<Header>, FractalError> {
    let sql = reminder_headers_sql();
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h47f366444514 owner=fractal_store::store::thread_reading TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
    let rows = statement
        .query_map([thread], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(internal)?;
    let mut headers = Vec::new();
    for row in rows {
        let (id, kind, actor, owner, anchor, observed) = row.map_err(internal)?;
        let own_time = operation_time(connection, &id)?;
        headers.push(Header {
            own_time,
            id,
            kind,
            actor,
            person: owner,
            anchor,
            observed: serde_json::from_str(&observed).map_err(internal)?,
            valid: false,
        });
    }
    headers.sort_by(|left, right| {
        right
            .own_time
            .cmp(&left.own_time)
            .then_with(|| right.id.cmp(&left.id))
    });
    let indices = headers
        .iter()
        .enumerate()
        .filter(|(_, h)| h.kind == UNREAD_MARKED)
        .map(|(i, h)| (h.id.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let mut remaining = vec![0; headers.len()];
    let mut dependents = vec![Vec::new(); headers.len()];
    let mut ready = std::collections::VecDeque::new();
    for (index, header) in headers.iter().enumerate() {
        let owner = identities::resolve_reference_conn(connection, person)?;
        let owned = identities::resolve_reference_conn(connection, &header.person)? == owner
            && header
                .actor
                .as_deref()
                .map(|actor| identities::resolve_reference_conn(connection, actor))
                .transpose()?
                .as_deref()
                == Some(owner.as_str());
        let anchor_valid = match header.anchor.as_deref() {
            Some(anchor) if header.kind == UNREAD_MARKED => {
                anchor_member(connection, creation, person, thread, anchor, false)?
                    || (allow_pending && !connection.query_row("SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_kind_index WHERE subject=?1 AND kind='message.sent')",[anchor],|row|row.get::<_,bool>(0)).map_err(internal)?)
            }
            None => header.kind == UNREAD_CLEARED,
            _ => false,
        };
        if !owned || !anchor_valid {
            continue;
        }
        remaining[index] = header.observed.len();
        let mut complete = true;
        for id in &header.observed {
            if let Some(dependency) = indices.get(id) {
                dependents[*dependency].push(index);
            } else {
                complete = false;
            }
        }
        if !complete {
            remaining[index] += 1;
        }
        if remaining[index] == 0 {
            ready.push_back(index);
        }
    }
    while let Some(index) = ready.pop_front() {
        headers[index].valid = true;
        for dependent in &dependents[index] {
            remaining[*dependent] -= 1;
            if remaining[*dependent] == 0 {
                ready.push_back(*dependent);
            }
        }
    }
    Ok(headers)
}

/// Point lookup returns signed routing fields, never message content. SQLite still
/// parses the existing inline JSON; this does not claim a physical byte bound.
fn anchor_member(
    connection: &Connection,
    creation: &ClaimRecord,
    person: &str,
    thread: &str,
    anchor: &str,
    require_current: bool,
) -> Result<bool, FractalError> {
    // Historical admitted routing proves scope for retained intent only. It never
    // authorizes reading the body, sending, or creating a new mark at a retired anchor.
    let sql = canonical_sql(&format!("SELECT claims.actor, json_extract(claims.body,'$.fields.from'),
        json_extract(claims.body,'$.fields.to'), json_extract(claims.body,'$.fields.thread_id'), claims.batch_id
        FROM claims INDEXED BY claims_subject_kind_index
        WHERE claims.subject=?1 AND claims.kind='message.sent'
          AND (?2=0 OR ({UNREPAIRED_CLAIM}
            AND EXISTS(SELECT 1 FROM operations WHERE operations.id=json_extract(claims.body,'$._operation.id')
              AND operations.canonical_claim_id=claims.id AND operations.state='active')))
        ORDER BY CANONICAL_DESC(claims)"));
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-he08523974536 owner=fractal_store::store::thread_reading TODO=trace-statement-and-value-representation */ &sql).map_err(internal)?;
    let headers = statement
        .query_map(params![anchor, require_current], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(internal)?;
    let fields = &creation.body["fields"];
    let resolve = |id: &str| identities::resolve_reference_conn(connection, id);
    let person = resolve(person)?;
    let agent = resolve(fields["agent"].as_str().ok_or_else(unavailable)?)?;
    for header in headers {
        let (Some(actor), Some(from), Some(to), relation, batch) = header.map_err(internal)? else {
            continue;
        };
        let belongs = relation.as_deref().map_or_else(
            || fields["first_message"].as_str() == Some(anchor) && batch == creation.batch_id,
            |id| id == thread,
        );
        if !belongs {
            continue;
        }
        let (actor, from, to) = (resolve(&actor)?, resolve(&from)?, resolve(&to)?);
        if actor == from && ((from == person && to == agent) || (from == agent && to == person)) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PERSON: &str = "person/example";
    const AGENT: &str = "agent/example";
    const THREAD: &str = "thread/reminder";
    fn setup(store: &Store) {
        store.configure_thread_write_rollout([]);
        store.append_claim(&ClaimInput {
            subject:format!("daemon/{}",store.origin()), kind:"daemon.started".into(), actor:None,
            fields:serde_json::from_value(json!({"status":"running", "features":{"thread_read_privacy":fractal_schema::threads::READ_PRIVACY_VERSION}})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:None,
        }).unwrap();
    }
    fn create(store: &Store, thread: &str) {
        thread_metadata::apply(
            store,
            PERSON,
            thread,
            thread,
            thread_metadata::Edit::Create {
                agent: AGENT,
                title: "Review",
            },
        )
        .unwrap();
    }
    fn message(store: &Store, thread: &str, id: &str) {
        thread_delivery::append_reply(store, &ClaimInput {
            subject:id.into(), kind:"message.sent".into(), actor:Some(AGENT.into()),
            fields:serde_json::from_value(json!({"from":AGENT,"to":PERSON,"content":"Actual reply","status":"sent","thread_id":thread})).unwrap(),
            evidence:vec![], expected_subject:None, idempotency_key:Some(id.into()),
        }, PersonAuthority::TrustedLocal, None).unwrap();
    }
    fn mark(store: &Store, key: &str, anchor: &str, observed: &[String]) -> ClaimRecord {
        apply_as(
            store,
            PersonAuthority::TrustedLocal,
            PERSON,
            THREAD,
            key,
            Edit::Mark {
                anchor_message: anchor,
                observed_marks: observed,
            },
        )
        .unwrap()
        .0
    }
    fn clear(store: &Store, key: &str, observed: &[String]) {
        apply_as(
            store,
            PersonAuthority::TrustedLocal,
            PERSON,
            THREAD,
            key,
            Edit::Clear {
                observed_marks: observed,
            },
        )
        .unwrap();
    }
    fn pairing(store: &Store, grant: &str) -> ClaimRecord {
        store
            .append_client_operation_claim(&ClaimInput {
                subject: grant.into(),
                kind: "custom.client.pairing-completed".into(),
                actor: Some(PERSON.into()),
                fields: serde_json::from_value(json!({
                    "person_id":PERSON,"session_actor":"person/example/session/reminder-paired",
                    "device_id":"device/reminder-fixture","credential_hash":"test-not-a-secret",
                    "expires_at_unix_ms":now_ms() as u64+60_000,"scopes":["control.messages"]
                }))
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap()
    }

    fn replicate(from: &Store, to: &Store) {
        to.import_replication(from.origin(), &from.export_replication(0).unwrap())
            .unwrap();
    }

    #[test]
    fn observed_remove_keeps_concurrent_anchors_and_stale_clear_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("right.db");
        let left = Store::open_memory("left").unwrap();
        let right = Store::open(&path, "right").unwrap();
        setup(&left);
        setup(&right);
        create(&left, THREAD);
        message(&left, THREAD, "message/first");
        message(&left, THREAD, "message/second");
        thread_metadata::mark_read(
            &left,
            PersonAuthority::TrustedLocal,
            PERSON,
            THREAD,
            Some("message/second"),
            &["message/first".into(), "message/second".into()],
        )
        .unwrap();
        let before = thread_metadata::summary(&left, PERSON, THREAD).unwrap();
        thread_metadata::apply(
            &left,
            PERSON,
            THREAD,
            "close",
            thread_metadata::Edit::Close {
                revision: &before.revision,
            },
        )
        .unwrap();
        let closed = thread_metadata::summary(&left, PERSON, THREAD).unwrap();
        replicate(&left, &right);
        let first = mark(&left, "first", "message/first", &[]);
        let second = mark(&right, "second", "message/second", &[]);
        replicate(&right, &left);
        replicate(&left, &right);
        for store in [&left, &right] {
            let active = reminder(store, PERSON, THREAD).unwrap().unwrap();
            assert_eq!(active.marks.len(), 2);
            assert!(
                active
                    .marks
                    .iter()
                    .any(|m| m.mark_id == first.operation_id.clone().unwrap()
                        && m.anchor_message_id == "message/first")
            );
            assert!(
                active
                    .marks
                    .iter()
                    .any(|m| m.mark_id == second.operation_id.clone().unwrap()
                        && m.anchor_message_id == "message/second")
            );
        }
        clear(&left, "stale-clear", &[first.operation_id.clone().unwrap()]);
        replicate(&left, &right);
        let newest = mark(
            &right,
            "newer",
            "message/first",
            &[second.operation_id.clone().unwrap()],
        );
        clear(
            &left,
            "another-stale-clear",
            &[second.operation_id.clone().unwrap()],
        );
        replicate(&left, &right);
        replicate(&right, &left);
        drop(right);
        let right = Store::open(&path, "right").unwrap();
        for store in [&left, &right] {
            let summary = thread_metadata::summary(store, PERSON, THREAD).unwrap();
            assert_eq!(
                summary.unread_reminder.unwrap().marks,
                vec![ThreadUnreadMark {
                    mark_id: newest.operation_id.clone().unwrap(),
                    anchor_message_id: "message/first".into()
                }]
            );
            assert_eq!(summary.state, "closed");
            assert_eq!(summary.message_count, closed.message_count);
            assert_eq!(summary.unread, closed.unread);
            assert_eq!(summary.revision, closed.revision);
            assert_eq!(
                store
                    .claims_for("message/first", Some("message.read"))
                    .unwrap()
                    .len(),
                1
            );
        }
        clear(&left, "explicit-clear", &[newest.operation_id.unwrap()]);
        assert!(reminder(&left, PERSON, THREAD).unwrap().is_none());
    }

    #[test]
    fn exact_retry_rechecks_authority_and_rejects_foreign_anchor_owner_and_refs() {
        let store = Store::open_memory("local").unwrap();
        setup(&store);
        create(&store, THREAD);
        create(&store, "thread/other");
        message(&store, THREAD, "message/anchor");
        message(&store, "thread/other", "message/foreign");
        let first = mark(&store, "stable", "message/anchor", &[]);
        let mut forged_replica = first.clone();
        forged_replica.actor = Some("person/other".into());
        assert!(matches!(
            classify_replicated_claim_with_registry(&forged_replica, fractal_schema::registry()),
            Err(error) if error.code == "thread-reminder-forbidden"
        ));
        let retry = apply_as(
            &store,
            PersonAuthority::TrustedLocal,
            PERSON,
            THREAD,
            "stable",
            Edit::Mark {
                anchor_message: "message/anchor",
                observed_marks: &[],
            },
        )
        .unwrap();
        assert_eq!(retry.0.id, first.id);
        assert!(!retry.1);
        assert!(
            apply_as(
                &store,
                PersonAuthority::TrustedLocal,
                PERSON,
                THREAD,
                "stable",
                Edit::Clear {
                    observed_marks: &[]
                }
            )
            .is_err()
        );
        assert!(
            apply_as(
                &store,
                PersonAuthority::LocalWeb { expires_at: 0 },
                PERSON,
                THREAD,
                "stable",
                Edit::Mark {
                    anchor_message: "message/anchor",
                    observed_marks: &[]
                }
            )
            .is_err()
        );
        assert!(
            apply_as(
                &store,
                PersonAuthority::TrustedLocal,
                "person/other",
                THREAD,
                "foreign-owner",
                Edit::Mark {
                    anchor_message: "message/anchor",
                    observed_marks: &[]
                }
            )
            .is_err()
        );
        assert!(
            apply_as(
                &store,
                PersonAuthority::TrustedLocal,
                PERSON,
                THREAD,
                "foreign-anchor",
                Edit::Mark {
                    anchor_message: "message/foreign",
                    observed_marks: &[]
                }
            )
            .is_err()
        );
        let foreign = apply_as(
            &store,
            PersonAuthority::TrustedLocal,
            PERSON,
            "thread/other",
            "foreign",
            Edit::Mark {
                anchor_message: "message/foreign",
                observed_marks: &[],
            },
        )
        .unwrap()
        .0;
        let foreign_operation = foreign.operation_id.unwrap();
        for reference in [foreign_operation.clone(), "missing-mark".into()] {
            assert!(
                apply_as(
                    &store,
                    PersonAuthority::TrustedLocal,
                    PERSON,
                    THREAD,
                    "forged-reference",
                    Edit::Clear {
                        observed_marks: &[reference]
                    }
                )
                .is_err()
            );
        }
        for (key, kind, actor, fields) in [
            (
                "raw-foreign-ref",
                UNREAD_CLEARED,
                PERSON,
                json!({"person":PERSON,"observed_marks":[foreign_operation]}),
            ),
            (
                "raw-foreign-anchor",
                UNREAD_MARKED,
                PERSON,
                json!({"person":PERSON,"anchor_message":"message/foreign","observed_marks":[]}),
            ),
            (
                "raw-foreign-owner",
                UNREAD_MARKED,
                "person/other",
                json!({"person":"person/other","anchor_message":"message/anchor","observed_marks":[]}),
            ),
        ] {
            store
                .append_claim(&ClaimInput {
                    subject: THREAD.into(),
                    kind: kind.into(),
                    actor: Some(actor.into()),
                    fields: serde_json::from_value(fields).unwrap(),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: Some(key.into()),
                })
                .unwrap();
        }
        let active = reminder(&store, PERSON, THREAD).unwrap().unwrap();
        assert_eq!(active.marks.len(), 1);
        assert_eq!(active.marks[0].mark_id, first.operation_id.clone().unwrap());
        let replica = Store::open_memory("forgery-replica").unwrap();
        setup(&replica);
        replicate(&store, &replica);
        assert_eq!(reminder(&replica, PERSON, THREAD).unwrap().unwrap(), active);
        let spoof = ClaimInput {
            subject: THREAD.into(),
            kind: UNREAD_CLEARED.into(),
            actor: Some("person/other".into()),
            fields: serde_json::from_value(json!({"person":PERSON,"observed_marks":[first.id]}))
                .unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: Some("spoof".into()),
        };
        assert!(store.append_claim(&spoof).is_err());
        assert!(
            store
                .append_client_claim(&ClaimInput {
                    actor: Some(PERSON.into()),
                    ..spoof
                })
                .is_err()
        );
    }

    #[test]
    fn reordered_dependencies_stay_inert_then_activate_without_rejection() {
        let source = Store::open_memory("source").unwrap();
        let target = Store::open_memory("target").unwrap();
        setup(&source);
        setup(&target);
        create(&source, THREAD);
        message(&source, THREAD, "message/anchor");
        replicate(&source, &target);
        let marked = mark(&source, "mark", "message/anchor", &[]);
        // A signed reference can arrive while its named dependency is still absent.
        // Raw append exercises the same admitted claim/projection boundary as replay.
        target
            .append_claim(&ClaimInput {
                subject: THREAD.into(),
                kind: UNREAD_CLEARED.into(),
                actor: Some(PERSON.into()),
                fields: serde_json::from_value(
                    json!({"person":PERSON,"observed_marks":[marked.operation_id.unwrap()]}),
                )
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some("clear-before-mark".into()),
            })
            .unwrap();
        assert!(reminder(&target, PERSON, THREAD).unwrap().is_none());
        replicate(&source, &target);
        assert!(reminder(&target, PERSON, THREAD).unwrap().is_none());
        // The retained clear now resolves; a genuinely fresh concurrent mark survives it.
        mark(&target, "fresh", "message/anchor", &[]);
        assert_eq!(
            reminder(&target, PERSON, THREAD)
                .unwrap()
                .unwrap()
                .marks
                .len(),
            1
        );
    }
    #[test]
    fn same_operation_retried_on_two_members_keeps_observed_generation() {
        let left = Store::open_memory("retry-left").unwrap();
        let right = Store::open_memory("retry-right").unwrap();
        setup(&left);
        setup(&right);
        create(&left, THREAD);
        message(&left, THREAD, "message/anchor");
        replicate(&left, &right);
        let first = mark(&left, "same-key", "message/anchor", &[]);
        let second = mark(&right, "same-key", "message/anchor", &[]);
        assert_eq!(first.operation_id, second.operation_id);
        clear(&left, "observed-clear", &[first.operation_id.unwrap()]);
        replicate(&right, &left);
        replicate(&left, &right);
        assert!(reminder(&left, PERSON, THREAD).unwrap().is_none());
        assert!(reminder(&right, PERSON, THREAD).unwrap().is_none());
    }

    #[test]
    fn old_registry_holds_exact_signed_reminders_and_replays_after_restart() {
        use crate::store::fleet_admission_tests_shared_seats::{
            FLEET, admit, admitted, key, node, sync,
        };
        let anchor = key();
        let older_key = key();
        let current = node("current", Some(&anchor), Some(&anchor));
        admit(&current, "current", &anchor, "anchor", None);
        admit(&current, "older", &older_key, "invite", None);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("older.db");
        let older = Store::open(&path, "older").unwrap();
        older.bind_fleet(FLEET).unwrap();
        older.pin_fleet_anchor(anchor.public()).unwrap();
        older.set_member_key(Some(older_key.clone())).unwrap();
        let mut old_registry = fractal_schema::registry().clone();
        old_registry.claims.remove(UNREAD_MARKED).unwrap();
        old_registry.claims.remove(UNREAD_CLEARED).unwrap();
        older.set_claim_registry(old_registry);
        setup(&current);
        setup(&older);
        sync(&current, &older);
        sync(&older, &current);
        create(&current, THREAD);
        let paired = pairing(&current, "custom/client/pairing-reminder-backup");
        message(&current, THREAD, "message/anchor");
        sync(&current, &older);
        let marked = mark(&current, "signed-mark", "message/anchor", &[]);
        clear(
            &current,
            "signed-clear",
            &[marked.operation_id.clone().unwrap()],
        );
        let fresh = mark(&current, "signed-fresh", "message/anchor", &[]);
        let admission = sync(&current, &older);
        assert_eq!((admission.unknown, admission.invalid), (3, 0));
        assert!(!admitted(&older, &marked));
        assert!(!admitted(&older, &fresh));
        let held = older.replica_records(true).unwrap();
        assert_eq!(held.len(), 3);
        assert!(
            held.iter()
                .all(|record| record.error_code.as_deref() == Some("unknown-claim-kind"))
        );
        // Both independent namespace guards protect unknown records before schema decoding.
        assert!(fractal_schema::threads::is_reserved_subject(THREAD));
        assert!(fractal_schema::threads::is_reserved_kind(UNREAD_MARKED));
        assert!(reminder(&older, PERSON, THREAD).unwrap().is_none());
        let held_archive = directory.path().join("held.backup");
        older
            .write_backup(&mut std::fs::File::create(&held_archive).unwrap())
            .unwrap();
        let held_destination = directory.path().join("held-restored.db");
        let held_restored =
            crate::backup::restore(&held_archive, &held_destination, directory.path()).unwrap();
        assert!(
            !held_restored.projections_match,
            "upgraded registry intentionally admits retained unknown operations"
        );
        let restored_held = Store::open(&held_destination, &held_restored.writer).unwrap();
        assert_eq!(
            reminder(&restored_held, PERSON, THREAD).unwrap(),
            reminder(&current, PERSON, THREAD).unwrap()
        );
        drop(older);
        let upgraded = Store::open(&path, "older").unwrap();
        upgraded.set_member_key(Some(older_key)).unwrap();
        assert_eq!(upgraded.validate_replication_backlog().unwrap().unknown, 0);
        upgraded.project_replication_backlog().unwrap();
        assert!(upgraded.replica_records(true).unwrap().is_empty());
        assert_eq!(
            reminder(&upgraded, PERSON, THREAD).unwrap().unwrap().marks[0].mark_id,
            fresh.operation_id.unwrap()
        );
        assert_eq!(
            upgraded.claim_by_id(&marked.id).unwrap().unwrap().id,
            marked.id
        );
        let archive = directory.path().join("reminders.backup");
        current
            .write_backup(&mut std::fs::File::create(&archive).unwrap())
            .unwrap();
        let destination = directory.path().join("restored.db");
        let restored = crate::backup::restore(&archive, &destination, directory.path()).unwrap();
        assert!(restored.projections_match);
        let restored_store = Store::open(&destination, &restored.writer).unwrap();
        assert_eq!(
            reminder(&restored_store, PERSON, THREAD).unwrap(),
            reminder(&current, PERSON, THREAD).unwrap()
        );
        assert!(restored_store.client_operation_is_trusted(&paired).unwrap());
        assert_eq!(
            restored_store.claim_by_id(&paired.id).unwrap().unwrap().id,
            paired.id
        );
        assert!(upgraded.client_operation_is_trusted(&paired).unwrap());
    }

    #[test]
    fn reminder_header_work_is_independent_of_thread_message_history() {
        let store = Store::open_memory("bounded").unwrap();
        setup(&store);
        create(&store, THREAD);
        message(&store, THREAD, "message/anchor");
        mark(&store, "bounded-mark", "message/anchor", &[]);
        let cost = || {
            let connection = store.readers.get();
            let mut statement = connection.prepare(&reminder_headers_sql()).unwrap();
            let mut rows = statement.query([THREAD]).unwrap();
            while rows.next().unwrap().is_some() {}
            drop(rows);
            statement.get_status(rusqlite::StatementStatus::VmStep)
        };
        let before = cost();
        for n in 0..128 {
            message(&store, THREAD, &format!("message/history-{n}"));
        }
        assert!(reminder(&store, PERSON, THREAD).unwrap().is_some());
        let after = cost();
        assert!(
            after <= before + 20,
            "reminder header steps {before} -> {after}"
        );
        let connection = store.readers.get();
        let plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {}", reminder_headers_sql()))
            .unwrap()
            .query_map([THREAD], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ");
        assert!(plan.contains("claims_subject_kind_index"), "{plan}");
    }
    #[test]
    fn paired_revocation_refuses_unknown_outcome_retry_before_receipt_recovery() {
        let store = Store::open_memory("paired-retry").unwrap();
        setup(&store);
        create(&store, THREAD);
        message(&store, THREAD, "message/anchor");
        let grant = "custom/client/pairing-reminder-retry";
        let paired = pairing(&store, grant);
        let authority = || PersonAuthority::Paired {
            grant,
            completion: &paired.id,
            actor: "person/example/session/reminder-paired",
        };
        let edit = || Edit::Mark {
            anchor_message: "message/anchor",
            observed_marks: &[],
        };
        let first = apply_as(
            &store,
            authority(),
            PERSON,
            THREAD,
            "paired-unknown-outcome",
            edit(),
        )
        .unwrap();
        assert!(first.1);
        let retry = apply_as(
            &store,
            authority(),
            PERSON,
            THREAD,
            "paired-unknown-outcome",
            edit(),
        )
        .unwrap();
        assert!(!retry.1);
        assert_eq!(retry.0.id, first.0.id);
        store
            .append_client_operation_claim(&ClaimInput {
                subject: grant.into(),
                kind: "custom.client.pairing-revoked".into(),
                actor: Some(PERSON.into()),
                fields: BTreeMap::new(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
        assert_eq!(
            apply_as(
                &store,
                authority(),
                PERSON,
                THREAD,
                "paired-unknown-outcome",
                edit()
            )
            .unwrap_err()
            .code,
            "thread-authority-expired"
        );
        assert_eq!(
            store.claims_for(THREAD, Some(UNREAD_MARKED)).unwrap().len(),
            1
        );
    }
    #[test]
    fn unavailable_anchor_preserves_private_intent_and_can_be_explicitly_cleared() {
        let store = Store::open_memory("unavailable-anchor").unwrap();
        setup(&store);
        create(&store, THREAD);
        message(&store, THREAD, "message/anchor");
        let anchor = store
            .newest_claim_for_subject_kind("message/anchor", "message.sent")
            .unwrap()
            .unwrap();
        let marked = mark(&store, "before-repair", "message/anchor", &[]);
        // Existing repair-cache source seam, not a claim of repair authorization.
        store
            .connection
            .batched(|tx| -> anyhow::Result<()> {
                tx.execute(
                    "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",
                    [&anchor.id],
                )?;
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert!(
            thread_metadata::mixed_message_content(&store, PERSON, THREAD, &anchor.id).is_err()
        );
        assert_eq!(
            thread_metadata::summary(&store, PERSON, THREAD)
                .unwrap()
                .message_count,
            0
        );
        let retained = reminder(&store, PERSON, THREAD).unwrap().unwrap();
        assert_eq!(
            retained.marks[0].mark_id,
            marked.operation_id.clone().unwrap()
        );
        assert!(
            apply_as(
                &store,
                PersonAuthority::TrustedLocal,
                PERSON,
                THREAD,
                "after-repair",
                Edit::Mark {
                    anchor_message: "message/anchor",
                    observed_marks: &[]
                }
            )
            .is_err()
        );
        assert!(reminder(&store, "person/foreign", THREAD).is_err());
        clear(&store, "clear-unavailable", &[marked.operation_id.unwrap()]);
        assert!(reminder(&store, PERSON, THREAD).unwrap().is_none());
    }
    fn marked_page(
        store: &Store,
        cursor: Option<&str>,
        limit: usize,
    ) -> thread_metadata::SummaryPage {
        thread_metadata::list_context(
            store,
            PERSON,
            thread_metadata::ThreadContext::MarkedUnread,
            cursor,
            limit,
        )
        .unwrap()
    }
    fn timeline_ids(page: &thread_metadata::TimelinePage) -> Vec<String> {
        page.items
            .iter()
            .filter_map(|item| match item {
                thread_metadata::TimelineItem::Message(message) => Some(message.id.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn read_helpers_seek_middle_and_page_both_directions_without_acknowledging() {
        let store = Store::open_memory("seek").unwrap();
        setup(&store);
        create(&store, THREAD);
        for n in 1..=9 {
            store.set_write_clock_at(n * 100).unwrap();
            message(&store, THREAD, &format!("message/{n}"));
        }
        let around =
            thread_metadata::mixed_timeline(&store, PERSON, THREAD, None, Some("message/5"), 3)
                .unwrap();
        assert_eq!(
            timeline_ids(&around),
            ["message/6", "message/5", "message/4"]
        );
        assert_eq!(
            around.anchor.as_ref().unwrap().state,
            thread_metadata::TimelineAnchorState::Current
        );
        let newer = thread_metadata::mixed_timeline(
            &store,
            PERSON,
            THREAD,
            around.previous_cursor.as_deref(),
            None,
            3,
        )
        .unwrap();
        assert_eq!(
            timeline_ids(&newer),
            ["message/9", "message/8", "message/7"]
        );
        assert!(newer.previous_cursor.is_none());
        let older = thread_metadata::mixed_timeline(
            &store,
            PERSON,
            THREAD,
            around.next_cursor.as_deref(),
            None,
            3,
        )
        .unwrap();
        assert_eq!(
            timeline_ids(&older),
            ["message/3", "message/2", "message/1"]
        );
        assert!(older.next_cursor.is_none());
        let first = thread_metadata::mixed_timeline(&store, PERSON, THREAD, None, None, 3).unwrap();
        assert!(first.previous_cursor.is_none());
        assert!(first.anchor.is_none());
        let encoded = serde_json::to_value(&older).unwrap();
        assert!(encoded["next_cursor"].is_null());
        assert!(
            serde_json::to_value(&newer)
                .unwrap()
                .get("previous_cursor")
                .is_none()
        );
        assert!(
            thread_metadata::mixed_timeline(
                &store,
                PERSON,
                THREAD,
                around.next_cursor.as_deref(),
                Some("message/5"),
                3
            )
            .is_err()
        );
        for n in 1..=9 {
            assert!(
                store
                    .claims_for(&format!("message/{n}"), Some("message.read"))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn read_helpers_private_late_anchor_transitions_pending_current_unavailable() {
        let source = Store::open_memory("anchor-source").unwrap();
        let target = Store::open_memory("anchor-target").unwrap();
        setup(&source);
        setup(&target);
        create(&source, THREAD);
        create(&source, "thread/foreign");
        message(&source, "thread/foreign", "message/foreign");
        replicate(&source, &target);
        message(&source, THREAD, "message/late");
        let intent = target
            .append_claim(&ClaimInput {
                subject: THREAD.into(),
                kind: UNREAD_MARKED.into(),
                actor: Some(PERSON.into()),
                fields: serde_json::from_value(
                    json!({"person":PERSON,"anchor_message":"message/late","observed_marks":[]}),
                )
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some("late-intent".into()),
            })
            .unwrap();
        assert!(reminder(&target, PERSON, THREAD).unwrap().is_none());
        let pending =
            thread_metadata::mixed_timeline(&target, PERSON, THREAD, None, Some("message/late"), 5)
                .unwrap();
        assert!(pending.items.is_empty());
        assert_eq!(
            pending.anchor.unwrap().state,
            thread_metadata::TimelineAnchorState::Pending
        );
        assert!(
            thread_metadata::mixed_timeline(
                &target,
                "person/foreign",
                THREAD,
                None,
                Some("message/late"),
                5
            )
            .is_err()
        );
        target
            .append_claim(&ClaimInput {
                subject: THREAD.into(),
                kind: UNREAD_MARKED.into(),
                actor: Some(PERSON.into()),
                fields: serde_json::from_value(
                    json!({"person":PERSON,"anchor_message":"message/foreign","observed_marks":[]}),
                )
                .unwrap(),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: Some("foreign-intent".into()),
            })
            .unwrap();
        assert!(
            thread_metadata::mixed_timeline(
                &target,
                PERSON,
                THREAD,
                None,
                Some("message/foreign"),
                5
            )
            .is_err()
        );
        replicate(&source, &target);
        let current =
            thread_metadata::mixed_timeline(&target, PERSON, THREAD, None, Some("message/late"), 5)
                .unwrap();
        assert_eq!(
            current.anchor.unwrap().state,
            thread_metadata::TimelineAnchorState::Current
        );
        let anchor = target
            .newest_claim_for_subject_kind("message/late", "message.sent")
            .unwrap()
            .unwrap();
        target
            .connection
            .batched(|tx| -> anyhow::Result<()> {
                tx.execute(
                    "INSERT INTO projection_digest_repaired_claims(id) VALUES(?1)",
                    [&anchor.id],
                )?;
                Ok(())
            })
            .unwrap()
            .unwrap();
        let unavailable =
            thread_metadata::mixed_timeline(&target, PERSON, THREAD, None, Some("message/late"), 5)
                .unwrap();
        assert!(unavailable.items.is_empty());
        assert_eq!(
            unavailable.anchor.unwrap().state,
            thread_metadata::TimelineAnchorState::Unavailable
        );
        clear(&target, "clear-retired", &[intent.operation_id.unwrap()]);
        assert!(
            thread_metadata::mixed_timeline(&target, PERSON, THREAD, None, Some("message/late"), 5)
                .is_err()
        );
    }

    #[test]
    fn read_helpers_marked_context_finds_old_closed_thread_outside_other_first_pages() {
        let store = Store::open_memory("old-closed").unwrap();
        setup(&store);
        let old = "thread/z-ancient";
        create(&store, old);
        message(&store, old, "message/ancient");
        let revision = thread_metadata::summary(&store, PERSON, old)
            .unwrap()
            .revision;
        thread_metadata::apply(
            &store,
            PERSON,
            old,
            "close-old",
            thread_metadata::Edit::Close {
                revision: &revision,
            },
        )
        .unwrap();
        for n in 0..102 {
            let id = format!("thread/a-recent-{n:03}");
            create(&store, &id);
            let revision = thread_metadata::summary(&store, PERSON, &id)
                .unwrap()
                .revision;
            thread_metadata::apply(
                &store,
                PERSON,
                &id,
                &format!("close-{n}"),
                thread_metadata::Edit::Close {
                    revision: &revision,
                },
            )
            .unwrap();
        }
        apply_as(
            &store,
            PersonAuthority::TrustedLocal,
            PERSON,
            old,
            "mark-old",
            Edit::Mark {
                anchor_message: "message/ancient",
                observed_marks: &[],
            },
        )
        .unwrap();
        assert!(
            !thread_metadata::list_conversation(&store, PERSON, AGENT, None, 100)
                .unwrap()
                .items
                .iter()
                .any(|summary| summary.id == old)
        );
        assert!(
            !thread_metadata::list_context(
                &store,
                PERSON,
                thread_metadata::ThreadContext::RecentlyClosed,
                None,
                100
            )
            .unwrap()
            .items
            .iter()
            .any(|summary| summary.id == old)
        );
        let marked = marked_page(&store, None, 1);
        assert_eq!(marked.items.len(), 1);
        assert_eq!(marked.items[0].id, old);
        assert_eq!(marked.items[0].state, "closed");
        let foreign = thread_metadata::list_context(
            &store,
            "person/foreign",
            thread_metadata::ThreadContext::MarkedUnread,
            None,
            10,
        )
        .unwrap();
        assert!(foreign.items.is_empty());
    }

    #[test]
    fn read_helpers_equivalent_late_retry_does_not_refresh_marked_order() {
        let left = Store::open_memory("time-left").unwrap();
        let right = Store::open_memory("time-right").unwrap();
        setup(&left);
        setup(&right);
        create(&left, THREAD);
        create(&left, "thread/later");
        message(&left, THREAD, "message/old");
        message(&left, "thread/later", "message/later");
        replicate(&left, &right);
        // write_time clamps to this writer's newest accepted batch/checkpoint cut.
        // Pick a future base above both real setup histories, rather than backdating.
        let base = left
            .claims_page(None, None, 0, None, false, 100)
            .unwrap()
            .claims
            .iter()
            .chain(
                right
                    .claims_page(None, None, 0, None, false, 100)
                    .unwrap()
                    .claims
                    .iter(),
            )
            .map(|claim| claim.accepted_at_unix_ms)
            .max()
            .unwrap()
            + 100;
        left.set_write_clock_at(base + 100).unwrap();
        let first = mark(&left, "same-retried-op", "message/old", &[]);
        assert_eq!(first.accepted_at_unix_ms, base + 100);
        left.set_write_clock_at(base + 200).unwrap();
        apply_as(
            &left,
            PersonAuthority::TrustedLocal,
            PERSON,
            "thread/later",
            "different-later-op",
            Edit::Mark {
                anchor_message: "message/later",
                observed_marks: &[],
            },
        )
        .unwrap();
        right.set_write_clock_at(base + 300).unwrap();
        let retry = mark(&right, "same-retried-op", "message/old", &[]);
        assert_eq!(retry.accepted_at_unix_ms, base + 300);
        assert_eq!(first.operation_id, retry.operation_id);
        replicate(&right, &left);
        replicate(&left, &right);
        for store in [&left, &right] {
            assert_eq!(
                operation_time(&store.readers.get(), first.operation_id.as_deref().unwrap())
                    .unwrap(),
                first.accepted_at_unix_ms
            );
            let page = marked_page(store, None, 1);
            assert_eq!(page.items[0].id, "thread/later");
            let second = marked_page(store, page.next_cursor.as_deref(), 1);
            assert_eq!(second.items[0].id, THREAD);
            assert!(
                thread_metadata::list_context(
                    store,
                    PERSON,
                    thread_metadata::ThreadContext::RecentlyClosed,
                    page.next_cursor.as_deref(),
                    1
                )
                .is_err()
            );
        }
    }
    #[test]
    fn read_helpers_empty_candidate_pages_keep_continuation_to_older_active_mark() {
        let store = Store::open_memory("empty-candidates").unwrap();
        setup(&store);
        create(&store, THREAD);
        create(&store, "thread/old-active");
        message(&store, THREAD, "message/inactive");
        message(&store, "thread/old-active", "message/still-active");
        apply_as(
            &store,
            PersonAuthority::TrustedLocal,
            PERSON,
            "thread/old-active",
            "old-active",
            Edit::Mark {
                anchor_message: "message/still-active",
                observed_marks: &[],
            },
        )
        .unwrap();
        let mut retired = Vec::new();
        for n in 0..9 {
            retired.push(
                mark(&store, &format!("retired-{n}"), "message/inactive", &[])
                    .operation_id
                    .unwrap(),
            );
        }
        clear(&store, "clear-candidates", &retired);
        let first = marked_page(&store, None, 1);
        assert!(first.items.is_empty());
        assert!(first.next_cursor.is_some());
        let mut cursor = first.next_cursor;
        let mut found = Vec::new();
        for _ in 0..5 {
            let page = marked_page(&store, cursor.as_deref(), 1);
            found.extend(page.items.into_iter().map(|item| item.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(found, ["thread/old-active"]);
        assert!(cursor.is_none());
    }

    #[test]
    fn read_helpers_equal_time_duplicate_generations_page_each_thread_once() {
        let store = Store::open_memory("same-time").unwrap();
        setup(&store);
        create(&store, THREAD);
        create(&store, "thread/z-tie");
        message(&store, THREAD, "message/same-time");
        message(&store, "thread/z-tie", "message/tie-other");
        let time = store
            .claims_page(None, None, 0, None, false, 100)
            .unwrap()
            .claims
            .iter()
            .map(|claim| claim.accepted_at_unix_ms)
            .max()
            .unwrap()
            + 100;
        store.set_write_clock_at(time).unwrap();
        let mut ids = Vec::new();
        for n in 0..64 {
            let claim = mark(&store, &format!("same-time-{n}"), "message/same-time", &[]);
            assert_eq!(claim.accepted_at_unix_ms, time);
            ids.push(claim.operation_id.unwrap());
        }
        apply_as(
            &store,
            PersonAuthority::TrustedLocal,
            PERSON,
            "thread/z-tie",
            "same-time-other",
            Edit::Mark {
                anchor_message: "message/tie-other",
                observed_marks: &[],
            },
        )
        .unwrap();
        let reminder = reminder(&store, PERSON, THREAD).unwrap().unwrap();
        assert_eq!(reminder.marks.len(), 64);
        assert_eq!(reminder.marks[0].mark_id, ids.into_iter().max().unwrap());
        let first = marked_page(&store, None, 1);
        assert_eq!(first.items[0].id, THREAD);
        let second = marked_page(&store, first.next_cursor.as_deref(), 1);
        assert_eq!(second.items[0].id, "thread/z-tie");
        assert!(second.next_cursor.is_none());
    }
}
