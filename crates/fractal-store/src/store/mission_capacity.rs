//! A mission revision that declares a field an older build cannot read (`capacity` on the
//! mission or a step, the mission's run `authority`, or a `program` step) is published only once
//! every current member reads the field. A member whose build does not checks a replicated
//! revision by hashing the mission as it reads it, without the field, and quarantines the claim
//! (docs/st3/seat-lanes.md, "Mixed versions"). Each member advertises each field on its own
//! `daemon.started`, as the standby barrier reads `schedule_standby`.
use super::*;

/// Transactionally maintained admission membership: population and the strictest
/// pinned limit are indexed, rather than re-decoding every active definition.
/// Invalid source remains writable for downstream diagnostics, but projects to
/// the refusing sentinel (-1). Each CASE must validate before calling JSON
/// accessors: SQLite also rejects otherwise valid JSON beyond its depth limit.
pub(super) fn install_run_capacity_schema(connection: &Connection) -> Result<(), FractalError> {
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='mission_run_capacity_v1_bootstrapped' AND value='1')
           AND (SELECT COUNT(*) FROM sqlite_schema WHERE type='table'
                AND name IN ('mission_run_capacity_v1','mission_run_capacity_totals_v1'))=2",
        [], |row| row.get(0),
    ).map_err(internal)?;
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h2e817e64343d owner=fractal_store::store::mission_capacity TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS mission_run_capacity_v1 (
             run_id TEXT PRIMARY KEY, mission_id TEXT NOT NULL, revision TEXT NOT NULL, run_limit INTEGER
         );
         CREATE INDEX IF NOT EXISTS mission_run_capacity_limit_v1
         ON mission_run_capacity_v1(mission_id,run_limit,run_id);
         CREATE INDEX IF NOT EXISTS mission_run_capacity_revision_v1
         ON mission_run_capacity_v1(mission_id,revision);
         CREATE TABLE IF NOT EXISTS mission_run_capacity_totals_v1 (
             mission_id TEXT PRIMARY KEY, active_count INTEGER NOT NULL
         );
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_add_v1
         AFTER INSERT ON mission_run_capacity_v1 BEGIN
             INSERT INTO mission_run_capacity_totals_v1 VALUES(NEW.mission_id,1)
             ON CONFLICT(mission_id) DO UPDATE SET active_count=active_count+1;
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_remove_v1
         AFTER DELETE ON mission_run_capacity_v1 BEGIN
             UPDATE mission_run_capacity_totals_v1 SET active_count=active_count-1
             WHERE mission_id=OLD.mission_id;
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_insert_v1
         AFTER INSERT ON mission_runs BEGIN
             INSERT INTO mission_run_capacity_v1
             SELECT NEW.id,NEW.mission_id,g.revision,
                    CASE WHEN json_valid(d.body) IS NOT 1 THEN -1
                         WHEN json_type(d.body)<>'object' THEN -1
                         WHEN json_type(d.body,'$.max_active_runs') IS NULL THEN 1
                         WHEN json_type(d.body,'$.max_active_runs')='null' THEN NULL
                         WHEN json_type(d.body,'$.max_active_runs')='integer'
                           AND json_extract(d.body,'$.max_active_runs') BETWEEN 0 AND 4294967295
                           THEN json_extract(d.body,'$.max_active_runs') ELSE -1 END
             FROM run_generations g LEFT JOIN mission_revisions d
               ON d.mission_id=NEW.mission_id AND d.revision=g.revision
             WHERE g.id=NEW.current_generation_id
               AND NEW.status IN ('running','standing','blocked')
               AND NEW.phase NOT IN ('final-cancelled','cleanup-cancelled');
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_update_v1
         AFTER UPDATE OF status,phase,current_generation_id,mission_id ON mission_runs BEGIN
             DELETE FROM mission_run_capacity_v1 WHERE run_id=OLD.id;
             INSERT INTO mission_run_capacity_v1
             SELECT NEW.id,NEW.mission_id,g.revision,
                    CASE WHEN json_valid(d.body) IS NOT 1 THEN -1
                         WHEN json_type(d.body)<>'object' THEN -1
                         WHEN json_type(d.body,'$.max_active_runs') IS NULL THEN 1
                         WHEN json_type(d.body,'$.max_active_runs')='null' THEN NULL
                         WHEN json_type(d.body,'$.max_active_runs')='integer'
                           AND json_extract(d.body,'$.max_active_runs') BETWEEN 0 AND 4294967295
                           THEN json_extract(d.body,'$.max_active_runs') ELSE -1 END
             FROM run_generations g LEFT JOIN mission_revisions d
               ON d.mission_id=NEW.mission_id AND d.revision=g.revision
             WHERE g.id=NEW.current_generation_id
               AND NEW.status IN ('running','standing','blocked')
               AND NEW.phase NOT IN ('final-cancelled','cleanup-cancelled');
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_delete_v1
         AFTER DELETE ON mission_runs BEGIN
             DELETE FROM mission_run_capacity_v1 WHERE run_id=OLD.id;
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_generation_v1
         AFTER INSERT ON run_generations BEGIN
             INSERT OR IGNORE INTO mission_run_capacity_v1
             SELECT r.id,r.mission_id,NEW.revision,
                    CASE WHEN json_valid(d.body) IS NOT 1 THEN -1
                         WHEN json_type(d.body)<>'object' THEN -1
                         WHEN json_type(d.body,'$.max_active_runs') IS NULL THEN 1
                         WHEN json_type(d.body,'$.max_active_runs')='null' THEN NULL
                         WHEN json_type(d.body,'$.max_active_runs')='integer'
                           AND json_extract(d.body,'$.max_active_runs') BETWEEN 0 AND 4294967295
                           THEN json_extract(d.body,'$.max_active_runs') ELSE -1 END
             FROM mission_runs r LEFT JOIN mission_revisions d
               ON d.mission_id=r.mission_id AND d.revision=NEW.revision
             WHERE r.id=NEW.run_id AND r.current_generation_id=NEW.id
               AND r.status IN ('running','standing','blocked')
               AND r.phase NOT IN ('final-cancelled','cleanup-cancelled');
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_generation_delete_v1
         AFTER DELETE ON run_generations BEGIN
             DELETE FROM mission_run_capacity_v1 WHERE run_id=OLD.run_id
               AND EXISTS(SELECT 1 FROM mission_runs r WHERE r.id=OLD.run_id AND r.current_generation_id=OLD.id);
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_generation_update_v1
         AFTER UPDATE OF revision ON run_generations BEGIN
             UPDATE mission_runs SET current_generation_id=current_generation_id
             WHERE id=NEW.run_id AND current_generation_id=NEW.id;
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_revision_insert_v1
         AFTER INSERT ON mission_revisions BEGIN
             UPDATE mission_run_capacity_v1 SET run_limit=
                 CASE WHEN json_valid(NEW.body) IS NOT 1 THEN -1
                      WHEN json_type(NEW.body)<>'object' THEN -1
                      WHEN json_type(NEW.body,'$.max_active_runs') IS NULL THEN 1
                      WHEN json_type(NEW.body,'$.max_active_runs')='null' THEN NULL
                      WHEN json_type(NEW.body,'$.max_active_runs')='integer'
                        AND json_extract(NEW.body,'$.max_active_runs') BETWEEN 0 AND 4294967295
                        THEN json_extract(NEW.body,'$.max_active_runs') ELSE -1 END
             WHERE mission_id=NEW.mission_id AND revision=NEW.revision;
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_revision_update_v1
         AFTER UPDATE OF body ON mission_revisions BEGIN
             UPDATE mission_run_capacity_v1 SET run_limit=
                 CASE WHEN json_valid(NEW.body) IS NOT 1 THEN -1
                      WHEN json_type(NEW.body)<>'object' THEN -1
                      WHEN json_type(NEW.body,'$.max_active_runs') IS NULL THEN 1
                      WHEN json_type(NEW.body,'$.max_active_runs')='null' THEN NULL
                      WHEN json_type(NEW.body,'$.max_active_runs')='integer'
                        AND json_extract(NEW.body,'$.max_active_runs') BETWEEN 0 AND 4294967295
                        THEN json_extract(NEW.body,'$.max_active_runs') ELSE -1 END
             WHERE mission_id=NEW.mission_id AND revision=NEW.revision;
         END;
         CREATE TRIGGER IF NOT EXISTS mission_run_capacity_revision_delete_v1
         AFTER DELETE ON mission_revisions BEGIN
             UPDATE mission_run_capacity_v1 SET run_limit=-1
             WHERE mission_id=OLD.mission_id AND revision=OLD.revision;
         END;"
    ).map_err(internal)?;
    if !installed {
        let bootstrap = |connection: &Connection| -> Result<(), FractalError> {
            connection.execute_batch(
                /* sqlite-large-value: allow-for-now lv-hca0d0145f4ce owner=fractal_store::store::mission_capacity TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM mission_run_capacity_v1;
                 DELETE FROM mission_run_capacity_totals_v1;
                 INSERT INTO mission_run_capacity_v1
                 SELECT r.id,r.mission_id,g.revision,
                        CASE WHEN json_valid(d.body) IS NOT 1 THEN -1
                             WHEN json_type(d.body)<>'object' THEN -1
                             WHEN json_type(d.body,'$.max_active_runs') IS NULL THEN 1
                             WHEN json_type(d.body,'$.max_active_runs')='null' THEN NULL
                             WHEN json_type(d.body,'$.max_active_runs')='integer'
                               AND json_extract(d.body,'$.max_active_runs') BETWEEN 0 AND 4294967295
                               THEN json_extract(d.body,'$.max_active_runs') ELSE -1 END
                 FROM mission_runs r JOIN run_generations g ON g.id=r.current_generation_id
                 LEFT JOIN mission_revisions d ON d.mission_id=r.mission_id AND d.revision=g.revision
                 WHERE r.status IN ('running','standing','blocked')
                   AND r.phase NOT IN ('final-cancelled','cleanup-cancelled');
                 INSERT INTO meta(key,value) VALUES('mission_run_capacity_v1_bootstrapped','1')
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value;"
            ).map_err(internal)
        };
        if connection.is_autocommit() {
            let transaction = connection.unchecked_transaction().map_err(internal)?;
            bootstrap(&transaction)?;
            transaction.commit().map_err(internal)?;
        } else {
            bootstrap(connection)?;
        }
    }
    Ok(())
}

/// The `daemon.started` feature of a build that reads mission and step capacity.
pub const FEATURE: &str = "mission_capacity";
/// The `daemon.started` feature of a build that reads a mission's run `authority`.
pub const AUTHORITY_FEATURE: &str = "mission_authority";
/// The `daemon.started` feature of a build that reads a step's `program`.
pub const PROGRAM_FEATURE: &str = "mission_program";
/// The `daemon.started` feature of a build that reads finite execution resource lanes.
pub const RESOURCE_LANE_FEATURE: &str = "mission_resource_lane";

/// One mission field behind a fleet barrier: the feature that says a member reads it, the
/// refusal code, and what to do once every member is upgraded.
struct Barrier {
    feature: &'static str,
    code: &'static str,
    declares: fn(&MissionSpec) -> bool,
    later: &'static str,
}

const BARRIERS: [Barrier; 4] = [
    Barrier {
        feature: FEATURE,
        code: "mission-capacity-unavailable",
        declares: declares_capacity,
        later: "declare capacity on a mission or step",
    },
    Barrier {
        feature: AUTHORITY_FEATURE,
        code: "mission-authority-unavailable",
        declares: |mission| !mission.authority.is_empty(),
        later: "declare authority on a mission",
    },
    Barrier {
        feature: PROGRAM_FEATURE,
        code: "mission-program-unavailable",
        declares: |mission| !mission.programs().is_empty(),
        later: "declare a program step",
    },
    Barrier {
        feature: RESOURCE_LANE_FEATURE,
        code: "mission-resource-lane-unavailable",
        declares: declares_resource_lane,
        later: "declare a finite execution resource lane",
    },
];

const WRITER_TABLES: [(&str, &str); 3] = [
    ("replica_envelopes", "writer"), ("batches", "origin"), ("checkpoint_envelopes", "writer"),
];
fn writer_seek(table: &str, column: &str) -> String {
    format!("SELECT {column} FROM {table} WHERE {column}>?1 ORDER BY {column} LIMIT 1")
}

fn declares_capacity(mission: &MissionSpec) -> bool {
    mission.capacity.is_some()
        || mission.steps.values().any(|step| {
            step.capacity.is_some() || step.nested_mission.as_deref().is_some_and(declares_capacity)
        })
}

fn declares_resource_lane(mission: &MissionSpec) -> bool {
    mission.steps.values().any(|step| {
        step.resource_lane.is_some()
            || step.nested_mission.as_deref().is_some_and(declares_resource_lane)
            || step.loop_spec.as_ref().is_some_and(|spec| {
                [Some(&spec.round), spec.on_keep.as_ref(), spec.on_discard.as_ref()]
                    .into_iter().flatten().any(|branch| declares_resource_lane(branch))
            })
    })
}

/// Refuses each new mission revision in `intent` that declares a field behind a barrier while a
/// current member has not advertised its feature. A revision already published passes, and so
/// does a host with no other member.
pub(super) fn validate_publication(
    connection: &Connection,
    origin: &str,
    intent: &NormalizedIntent,
) -> Result<(), FractalError> {
    for mission in intent.missions.values()
        .filter(|mission| BARRIERS.iter().any(|barrier| (barrier.declares)(mission)))
    {
        let published: Option<String> = connection
            .query_row(
                "SELECT revision FROM mission_definitions WHERE mission_id=?1",
                [&mission.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(internal)?;
        if published.as_deref() != Some(mission.revision.as_str()) {
            validate_mission(connection, origin, mission)?;
        }
    }
    Ok(())
}

pub(super) fn validate_mission(
    connection: &Connection,
    origin: &str,
    mission: &MissionSpec,
) -> Result<(), FractalError> {
    for barrier in BARRIERS.iter().filter(|barrier| (barrier.declares)(mission)) {
        validate_barrier(connection, origin, barrier)?;
    }
    Ok(())
}

fn validate_barrier(connection: &Connection, origin: &str, barrier: &Barrier) -> Result<(), FractalError> {
    let Barrier { feature, code, later, .. } = barrier;
    let refused = |reason: String| FractalError::new(code, reason);
    let membership = fleet_membership_tx(connection).map_err(internal)?;
    for member in membership.incarnations().filter(|member| member.end.is_none()) {
        // This host runs this build.
        if member.name == origin {
            continue;
        }
        if !matches!(membership.state(&member.name), crate::fleet::MemberState::Current(_)) {
            return Err(refused(format!(
                "the fleet membership of host/{} is ambiguous, so its build cannot be checked; {later} once it is settled",
                member.name
            )));
        }
        let features: Option<Option<String>> = connection
            .query_row(
                /* sqlite-large-value: allow-for-now lv-h88ff6fefeb9f owner=fractal_store::store::mission_capacity TODO=trace-statement-and-value-representation */ &canonical_sql(
                    "SELECT json_extract(claims.body,'$.fields.features')
                     FROM claims JOIN batches ON batches.id=claims.batch_id
                     WHERE claims.subject=?1 AND claims.kind='daemon.started' AND claims.origin=?2
                       AND batches.replica_sequence>=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1",
                ),
                params![format!("daemon/{}", member.name), member.name, member.start],
                |row| row.get(0),
            )
            .optional()
            .map_err(internal)?;
        let advertised = features.flatten()
            .and_then(|features| serde_json::from_str::<Value>(&features).ok())
            .is_some_and(|features| features.get(*feature) == Some(&json!(1)));
        if !advertised {
            return Err(refused(format!(
                "host/{} runs an older build that would refuse this revision (it has not advertised {feature}=1); {later} once every member is upgraded",
                member.name
            )));
        }
    }
    // A writer outside the membership has no advertisement to read.
    let fenced = membership.legacy_removed_names().collect::<BTreeSet<_>>();
    for (table, column) in WRITER_TABLES {
        let mut previous = String::new();
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h55ab9de796f8 owner=fractal_store::store::mission_capacity TODO=trace-statement-and-value-representation */ &writer_seek(table, column)).map_err(internal)?;
        while let Some(writer) = statement
            .query_row(/* sqlite-large-value: allow-for-now lv-hfc667e6ac72f owner=fractal_store::store::mission_capacity TODO=trace-statement-and-value-representation */ [&previous], |row| row.get::<_, String>(0))
            .optional()
            .map_err(internal)?
        {
            if writer != origin && !membership.is_keyed_writer(&writer) && !fenced.contains(&writer) {
                return Err(refused(format!(
                    "`{writer}` replicates to this host outside the fleet membership, so its build cannot be checked; {later} once it is a member or fenced"
                )));
            }
            previous = writer;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::MemberKey;
    use crate::store::directive_notes::tests_shared_delivery::{FLEET, claim, enable, sync};

    const LANED: &str = r#"version 2
mission "laned" state="ready" {
  goal "Work."
  capacity cores=2 memory="4G"
  step "work" {
    assigned-to "agent/example/worker"
    goal "Work."
    capacity memory="8G"
  }
}
"#;
    const PLAIN: &str = r#"version 2
mission "plain" state="ready" {
  goal "Work."
  step "work" {
    assigned-to "agent/example/worker"
    goal "Work."
  }
}
"#;

    fn publish(store: &Store, source: &str, key: &str) -> Result<Vec<String>, FractalError> {
        let intent = crate::graph::parse_intent(source, &store.origin)?;
        let planned = store.mission(&intent, IntentInput { kdl: source.into(), source_name: None })?;
        store.apply_as(&intent, &planned.subject_tokens, key, Some("person/avery"))?;
        Ok(planned.blockers)
    }

    fn started(store: &Store, reads_capacity: bool) {
        claim(store, &format!("daemon/{}", store.origin), "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: u8::from(reads_capacity)}}));
    }

    /// `alder` with `birch` admitted, synced and started on a build that reads no barrier field.
    fn two_members() -> (Store, Store) {
        let store = Store::open_memory("alder").unwrap();
        let anchor = enable(&store);
        let peer_key = Arc::new(MemberKey::generate().unwrap().0);
        claim(&store, "host/birch", "fleet.member-admitted", None,
            json!({"fleet_id": FLEET, "member_key": peer_key.public(), "via": "invite", "sponsor": "host/alder", "mode": "listening"}));
        store.replication_snapshot().unwrap();
        let peer = Store::open_memory("birch").unwrap();
        peer.bind_fleet(FLEET).unwrap();
        peer.pin_fleet_anchor(anchor.public()).unwrap();
        peer.set_member_key(Some(peer_key.clone())).unwrap();
        sync(&store, &peer);
        started(&peer, false);
        sync(&peer, &store);
        (store, peer)
    }

    #[test]
    fn capacity_projection_preserves_invalid_source_and_refuses_admission() {
        let store = Store::open_memory("fixture").unwrap();
        publish(&store, PLAIN, "capacity-fixture").unwrap();
        let mut requested = store.mission_spec("plain", None).unwrap().unwrap();
        // An unlimited new revision must still respect a pinned source's limit.
        requested.max_active_runs = None;
        let connection = store.connection.lock().unwrap();
        let (revision, state, claim_id, created_index): (String, String, String, i64) = connection
            .query_row(
                "SELECT revision,state,claim_id,created_index FROM mission_revisions WHERE mission_id='plain'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        connection.execute(
            "INSERT INTO mission_runs(id,mission_id,initial_revision,current_generation_id,root_revision,root_run_id,workspace,requester,inputs,mode,status,phase,created_at_unix_ms,updated_at_unix_ms)
             VALUES('capacity-run','plain',?1,'capacity-generation',?1,'capacity-run','/workspace','person/example','{}','normal','running','active','1','1')",
            [&revision],
        ).unwrap();
        let insert_generation = || {
            connection.execute(
                "INSERT INTO run_generations(id,run_id,revision,status,actor,reason,created_at_unix_ms,updated_at_unix_ms)
                 VALUES('capacity-generation','capacity-run',?1,'running','person/example','fixture','1','1')",
                [&revision],
            ).unwrap();
        };
        insert_generation();
        let assert_capacity = |expected: Option<i64>| {
            let actual: Option<i64> = connection.query_row(
                "SELECT run_limit FROM mission_run_capacity_v1 WHERE run_id='capacity-run'",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(actual, expected);
            let count: i64 = connection.query_row(
                "SELECT active_count FROM mission_run_capacity_totals_v1 WHERE mission_id='plain'",
                [], |row| row.get(0),
            ).unwrap();
            assert_eq!(count, 1);
            match expected {
                Some(-1) => assert_eq!(
                    enforce_mission_run_capacity(&connection, &requested).unwrap_err().code,
                    "missing-mission-revision",
                ),
                Some(0 | 1) => assert_eq!(
                    enforce_mission_run_capacity(&connection, &requested).unwrap_err().code,
                    "mission-run-capacity",
                ),
                _ => enforce_mission_run_capacity(&connection, &requested).unwrap(),
            }
        };
        let too_deep = format!("{{\"nested\":{}0{}}}", "[".repeat(1100), "]".repeat(1100));
        let cases = [
            ("{", Some(-1)),
            (too_deep.as_str(), Some(-1)),
            ("null", Some(-1)),
            ("[]", Some(-1)),
            ("1", Some(-1)),
            (r#""source""#, Some(-1)),
            (r#"{"max_active_runs":"2"}"#, Some(-1)),
            (r#"{"max_active_runs":true}"#, Some(-1)),
            (r#"{"max_active_runs":[]}"#, Some(-1)),
            (r#"{"max_active_runs":{}}"#, Some(-1)),
            (r#"{"max_active_runs":1.5}"#, Some(-1)),
            (r#"{"max_active_runs":-1}"#, Some(-1)),
            (r#"{"max_active_runs":4294967296}"#, Some(-1)),
            ("{}", Some(1)),
            (r#"{"max_active_runs":0}"#, Some(0)),
            (r#"{"max_active_runs":2}"#, Some(2)),
            (r#"{"max_active_runs":4294967295}"#, Some(4294967295)),
            (r#"{"max_active_runs":null}"#, None),
        ];
        for (body, expected) in cases {
            // Preserve canonical bad source for downstream readers to diagnose.
            connection.execute(
                "UPDATE mission_revisions SET body=?1 WHERE mission_id='plain' AND revision=?2",
                params![body, revision],
            ).unwrap();
            assert_capacity(expected);
            connection.execute(
                "UPDATE mission_runs SET phase=phase WHERE id='capacity-run'", [],
            ).unwrap();
            assert_capacity(expected);
            connection.execute(
                "UPDATE run_generations SET revision=revision WHERE id='capacity-generation'", [],
            ).unwrap();
            assert_capacity(expected);
            connection.execute(
                "DELETE FROM run_generations WHERE id='capacity-generation'", [],
            ).unwrap();
            insert_generation();
            assert_capacity(expected);
            connection.execute(
                "DELETE FROM mission_revisions WHERE mission_id='plain' AND revision=?1", [&revision],
            ).unwrap();
            assert_capacity(Some(-1));
            connection.execute(
                "INSERT INTO mission_revisions(mission_id,revision,state,body,claim_id,created_index)
                 VALUES('plain',?1,?2,?3,?4,?5)",
                params![revision, state, body, claim_id, created_index],
            ).unwrap();
            assert_capacity(expected);
            connection.execute(
                "DELETE FROM meta WHERE key='mission_run_capacity_v1_bootstrapped'", [],
            ).unwrap();
            install_run_capacity_schema(&connection).unwrap();
            assert_capacity(expected);
            let persisted: String = connection.query_row(
                "SELECT body FROM mission_revisions WHERE mission_id='plain' AND revision=?1",
                [&revision], |row| row.get(0),
            ).unwrap();
            assert_eq!(persisted, body);
        }
        connection.execute(
            "DELETE FROM mission_revisions WHERE mission_id='plain' AND revision=?1", [&revision],
        ).unwrap();
        connection.execute(
            "DELETE FROM meta WHERE key='mission_run_capacity_v1_bootstrapped'", [],
        ).unwrap();
        install_run_capacity_schema(&connection).unwrap();
        assert_capacity(Some(-1));
        let changes = connection.total_changes();
        install_run_capacity_schema(&connection).unwrap();
        assert_eq!(connection.total_changes(), changes);
        assert_capacity(Some(-1));
    }

    #[test]
    fn a_revision_that_declares_capacity_waits_until_every_member_reads_it() {
        // A host alone publishes at once, with or without a fleet.
        publish(&Store::open_memory("alder").unwrap(), LANED, "alone").unwrap();
        let anchored = Store::open_memory("alder").unwrap();
        enable(&anchored);
        publish(&anchored, LANED, "anchored-alone").unwrap();

        let (store, peer) = two_members();

        // A member on an older build would quarantine the revision: the plan shows why and the
        // publication is refused. A mission without capacity is unaffected.
        let intent = crate::graph::parse_intent(LANED, "alder").unwrap();
        let planned = store.mission(&intent, IntentInput { kdl: LANED.into(), source_name: None }).unwrap();
        assert!(planned.blockers.iter().any(|blocker| blocker.starts_with("mission-capacity-unavailable: host/birch runs an older build")),
            "{:?}", planned.blockers);
        let refusal = publish(&store, LANED, "older").unwrap_err();
        assert_eq!(refusal.code, "mission-capacity-unavailable");
        assert!(refusal.message.contains("declare capacity on a mission or step once every member is upgraded"), "{}", refusal.message);
        assert!(store.mission_spec("laned", None).unwrap().is_none());
        publish(&store, PLAIN, "plain").unwrap();
        // Another origin cannot speak for the member.
        started(&store, true);
        claim(&store, "daemon/birch", "daemon.started", None, json!({"status": "running", "features": {FEATURE: 1}}));
        assert_eq!(publish(&store, LANED, "spoken-for").unwrap_err().code, "mission-capacity-unavailable");
        // A direct append is held to the same rule.
        let mission = intent.missions.values().next().unwrap();
        let append = store.append_claim(&ClaimInput {
            subject: "mission/laned".into(), kind: "mission.published".into(), actor: Some("person/avery".into()),
            fields: BTreeMap::from([("body".into(), serde_json::to_value(mission).unwrap())]), evidence: Vec::new(),
            expected_subject: None, idempotency_key: None,
        });
        assert!(append.unwrap_err().to_string().contains("mission-capacity-unavailable"));

        // Once the member upgrades, the revision publishes.
        started(&peer, true);
        sync(&peer, &store);
        assert!(publish(&store, LANED, "upgraded").unwrap().is_empty());
        assert_eq!(store.mission_spec("laned", None).unwrap().unwrap().capacity, mission.capacity);
    }

    #[test]
    fn a_revision_that_declares_run_authority_waits_until_every_member_reads_it() {
        const AUTHORIZED: &str = r#"version 2
mission "authorized" state="ready" {
  goal "Work."
  authority { write "resource/example/*"; }
  step "work" { agentless; }
}
"#;
        publish(&Store::open_memory("alder").unwrap(), AUTHORIZED, "alone").unwrap();
        let (store, peer) = two_members();
        // Reading capacity is not reading authority.
        claim(&peer, "daemon/birch", "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: 1}}));
        sync(&peer, &store);
        let refusal = publish(&store, AUTHORIZED, "older").unwrap_err();
        assert_eq!(refusal.code, "mission-authority-unavailable");
        assert!(refusal.message.contains("has not advertised mission_authority=1); declare authority on a mission once every member is upgraded"),
            "{}", refusal.message);
        assert!(store.mission_spec("authorized", None).unwrap().is_none());
        claim(&peer, "daemon/birch", "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: 1, AUTHORITY_FEATURE: 1}}));
        sync(&peer, &store);
        assert!(publish(&store, AUTHORIZED, "upgraded").unwrap().is_empty());
        assert_eq!(store.mission_spec("authorized", None).unwrap().unwrap().authority.len(), 1);
    }

    #[test]
    fn a_revision_that_declares_a_program_step_waits_until_every_member_reads_it() {
        let bundle = b"export default {}";
        let source = format!(
            "version 2\nmission \"programmed\" state=\"ready\" {{\n  goal \"Work.\"\n  step \"run\" {{ program \"/bundles/land.mjs\" sha256=\"{}\"; }}\n}}\n",
            hex::encode(Sha256::digest(bundle))
        );
        let alone = Store::open_memory("alder").unwrap();
        alone.put_blob(bundle).unwrap();
        publish(&alone, &source, "alone").unwrap();
        let (store, peer) = two_members();
        store.put_blob(bundle).unwrap();
        // Reading capacity and authority is not reading programs.
        claim(&peer, "daemon/birch", "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: 1, AUTHORITY_FEATURE: 1}}));
        sync(&peer, &store);
        let refusal = publish(&store, &source, "older").unwrap_err();
        assert_eq!(refusal.code, "mission-program-unavailable");
        assert!(refusal.message.contains("has not advertised mission_program=1); declare a program step once every member is upgraded"),
            "{}", refusal.message);
        assert!(store.mission_spec("programmed", None).unwrap().is_none());
        claim(&peer, "daemon/birch", "daemon.started", None,
            json!({"status": "running", "features": {FEATURE: 1, AUTHORITY_FEATURE: 1, PROGRAM_FEATURE: 1}}));
        sync(&peer, &store);
        assert!(publish(&store, &source, "upgraded").unwrap().is_empty());
        assert_eq!(store.mission_spec("programmed", None).unwrap().unwrap().programs().len(), 1);
    }

    #[test]
    fn resource_lane_publication_waits_for_reader_capability() {
        const SOURCE: &str = r#"version 2
mission "resource-laned" state="ready" {
  goal "Verify."
  step "verify" {
    agentless
    lane "verification" host="alder"
    exec "verify" { command "true"; }
  }
}
"#;
        let (store, peer) = two_members();
        let error = publish(&store, SOURCE, "old-reader").unwrap_err();
        assert_eq!(error.code, "mission-resource-lane-unavailable");
        assert!(store.mission_spec("resource-laned", None).unwrap().is_none());
        publish(&store, PLAIN, "unaffected-history").unwrap();
        claim(&peer, "daemon/birch", "daemon.started", None,
            json!({"status": "running", "features": {RESOURCE_LANE_FEATURE: 1}}));
        sync(&peer, &store);
        assert!(publish(&store, SOURCE, "lane-reader").unwrap().is_empty());
        assert!(store.mission_spec("resource-laned", None).unwrap().unwrap().steps["verify"].resource_lane.is_some());
    }
}
