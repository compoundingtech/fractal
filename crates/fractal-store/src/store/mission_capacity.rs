//! A mission revision that declares a field an older build cannot read (`capacity` on the
//! mission or a step, the mission's run `authority`, or a `program` step) is published only once
//! every current member reads the field. A member whose build does not checks a replicated
//! revision by hashing the mission as it reads it, without the field, and quarantines the claim
//! (docs/st3/seat-lanes.md, "Mixed versions"). Each member advertises each field on its own
//! `daemon.started`, as the standby barrier reads `schedule_standby`.
use super::*;

/// The `daemon.started` feature of a build that reads mission and step capacity.
pub const FEATURE: &str = "mission_capacity";
/// The `daemon.started` feature of a build that reads a mission's run `authority`.
pub const AUTHORITY_FEATURE: &str = "mission_authority";
/// The `daemon.started` feature of a build that reads a step's `program`.
pub const PROGRAM_FEATURE: &str = "mission_program";

/// One mission field behind a fleet barrier: the feature that says a member reads it, the
/// refusal code, and what to do once every member is upgraded.
struct Barrier {
    feature: &'static str,
    code: &'static str,
    declares: fn(&MissionSpec) -> bool,
    later: &'static str,
}

const BARRIERS: [Barrier; 3] = [
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
    let refused = |reason: String| FractalError::new(*code, reason);
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
}
