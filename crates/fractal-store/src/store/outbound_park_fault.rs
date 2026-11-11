//! Outbound rejection alerts are repair evidence, never the exclusion authority.
//! Only explicit operational repair deletes the matching durable park episode.
use super::*;

const CONDITION: &str = "replication-outbound-parked";
pub(super) const REPAIR_CLASS: &str = "replication-outbound-park";

impl Store {
    /// Record an actionable graph fault before durably parking the exact target store's page.
    /// A crash between these commits leaves a retryable alert, not a false terminal success.
    pub fn park_outbound_with_failure(
        &self,
        target: &smallclaims::store::Store,
        peer: &str,
        hashes: &[String],
        code: &str,
    ) -> Result<(), FractalError> {
        if hashes.is_empty() { return Ok(()); }
        if !matches!(code,
            "replication-ingress/raw-oversize" | "replication-ingress/inflated-oversize"
            | "replication-ingress/malformed-deflate" | "replication-ingress/malformed-json") {
            return Err(FractalError::new("invalid-outbound-park", "only terminal ingress codes can park a page"));
        }
        if std::ptr::eq(target, &self.graph) {
            // The raw park transaction creates/reuses operational.failure through the runtime
            // and binds its non-null episode into each park. No second writer loan is taken.
            return target.park_outbound(peer, hashes, code, None).map_err(internal);
        }
        let scope = target.bound_shard().map_err(internal)?.unwrap_or_default();
        let origin = target.origin();
        let mut hashes = hashes.to_vec();
        hashes.sort();
        hashes.dedup();
        // Reuse the current destination/code episode even for rejected alert envelopes.
        let signature = canonical_hash(&(origin, &scope, peer, code)).map_err(internal)?;
        let subject = format!("daemon/{}", self.origin());
        let reviewer = self.trim_fault_reviewer();
        {
            // Lock order invariant: graph writer before shard writer, never the reverse.
            let mut writer = self.connection.write();
            let transaction = writer.transaction().map_err(internal)?;
            let open = transaction.query_row(
                "SELECT json_extract(f.body,'$.fields.episode') FROM claims f
                 WHERE f.subject=?1 AND f.kind='operational.failure'
                   AND json_extract(f.body,'$.fields.condition')=?2
                   AND json_extract(f.body,'$.fields.reproduction.signature')=?3
                   AND NOT EXISTS(SELECT 1 FROM claims r WHERE r.subject=f.subject
                     AND r.kind='operational.recovered'
                     AND json_extract(r.body,'$.fields.failure')=f.id)
                 ORDER BY f.store_index DESC LIMIT 1",
                params![subject, CONDITION, signature], |row| row.get::<_, String>(0),
            ).optional().map_err(internal)?;
            let episode = match open {
                Some(episode) => episode,
                None => {
                    let episode = format!("{CONDITION}/{signature}/{}", current_index(&transaction).map_err(internal)?);
                    append_claim_tx(&transaction, &self.origin, &subject, "operational.failure",
                        Some("daemon/runtime"), &json!({"fields": {
                            "episode": episode, "condition": CONDITION, "reviewer": reviewer,
                            "title": format!("outbound replication to {peer} parked after {code}"),
                            "reason": "The authenticated destination rejected the sent envelopes. Fix the rejection cause, then explicitly approve operational repair to retry this exact failure episode. Restart and wake do not unpark it.",
                            "severity": "error", "targets": [subject],
                            "reproduction": {"signature": signature, "origin": origin,
                                "scope": scope, "peer": peer, "hashes": hashes, "code": code}
                        }}), &[], None).map_err(internal)?;
                    episode
                }
            };
            transaction.commit().map_err(internal)?;
            // Keep the graph writer loan until shard persistence finishes, so manual repair
            // cannot recover the episode between the graph alert and its shard park.
            target.park_outbound(peer, &hashes, code, Some(&episode)).map_err(internal)
        }
    }
}

pub(super) fn plan(connection: &Connection, origin: &str, items: &mut Vec<OperationalRepairItem>) -> Result<()> {
    let mut query = connection.prepare(
        "SELECT f.id, f.subject, f.body FROM claims f
         WHERE f.kind='operational.failure' AND json_extract(f.body,'$.fields.condition')=?1
           AND f.origin=?2 AND f.subject='daemon/'||?2
           AND NOT EXISTS(SELECT 1 FROM claims r WHERE r.subject=f.subject
             AND r.kind='operational.recovered' AND json_extract(r.body,'$.fields.failure')=f.id)
         ORDER BY f.id",
    )?;
    let failures = query.query_map(params![CONDITION, origin], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (failure, subject, body) in failures {
        let body: Value = serde_json::from_str(&body)?;
        let fields = &body["fields"];
        let mut details = BTreeMap::from([
            ("failure".into(), json!(failure)),
            ("episode".into(), fields["episode"].clone()),
            ("reproduction".into(), fields["reproduction"].clone()),
        ]);
        // Keep every exact binding in the approval token, including the current failure ID.
        details.insert("failure_subject".into(), json!(subject));
        push_operational_repair_item(items, REPAIR_CLASS, &subject, vec![subject.clone()],
            "explicitly retry the outbound envelopes rejected in this current failure episode", details)?;
    }
    Ok(())
}

pub(super) fn apply(
    store: &Store,
    transaction: &Transaction<'_>,
    item: &OperationalRepairItem,
) -> Result<String, FractalError> {
    let required = |value: &Value| value.as_str().map(str::to_owned)
        .ok_or_else(|| FractalError::new("invalid-repair-plan", "outbound repair binding is missing"));
    let failure = required(&item.details["failure"])?;
    let episode = required(&item.details["episode"])?;
    let reproduction = &item.details["reproduction"];
    let origin = required(&reproduction["origin"])?;
    let scope = required(&reproduction["scope"])?;
    let peer = required(&reproduction["peer"])?;
    let code = required(&reproduction["code"])?;
    let remove = |connection: &Connection| -> Result<(), FractalError> {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h92069b925099 owner=fractal_store::store::outbound_park_fault TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM outbound_parks WHERE origin=?1 AND scope=?2 AND peer=?3 AND code=?4 AND episode=?5",
            params![origin, scope, peer, code, episode],
        ).map_err(internal)?;
        Ok(())
    };
    if scope.is_empty() {
        if origin != store.origin() {
            return Err(FractalError::new("invalid-repair-plan", "outbound park belongs to another origin"));
        }
        remove(transaction)?;
    } else {
        let shard = messages_shard::shard(store).ok_or_else(|| FractalError::new("shard-unavailable", "outbound park repair requires its configured shard"))?;
        if shard.origin() != origin || shard.bound_shard().map_err(internal)?.as_deref() != Some(scope.as_str()) {
            return Err(FractalError::new("shard-mismatch", "outbound park repair names a different shard or origin"));
        }
        // Lock order invariant: graph writer before shard writer, never the reverse.
        let mut writer = shard.connection.write();
        let shard_transaction = writer.transaction().map_err(internal)?;
        remove(&shard_transaction)?;
        shard_transaction.commit().map_err(internal)?;
        // A crash here leaves the same open graph failure and approval token. Reapplying the
        // bound deletion is harmless; no replacement episode can be removed by the old token.
    }
    let claim = append_claim_tx(transaction, &store.origin, &item.subject, "operational.recovered",
        Some("daemon/runtime"), &json!({"fields": {
            "episode": episode, "failure": failure,
            "reason": "explicit operational repair approved retry of this outbound park episode"
        }}), &[], None).map_err(internal)?;
    Ok(claim.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parked(store: &Store, peer: &str, hash: &str) -> bool {
        store.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM outbound_parks WHERE peer=?1 AND envelope_hash=?2)",
            params![peer, hash], |row| row.get(0),
        ).unwrap()
    }

    #[test]
    fn graph_park_alert_is_deduplicated_and_only_explicit_repair_unparks() {
        let store = Store::open_memory("alder").unwrap();
        let hashes = vec!["envelope-a".to_owned()];
        for _ in 0..2 {
            store.park_outbound_with_failure(&store, "birch", &hashes, "replication-ingress/raw-oversize").unwrap();
        }
        let plan = store.operational_repair_plan_with_legacy_starts_and_filter(&[], &[], &[REPAIR_CLASS.into()]).unwrap();
        assert_eq!(plan.items.len(), 1);
        assert!(parked(&store, "birch", &hashes[0]));
        let applied = store.apply_operational_repair(&plan.token).unwrap();
        assert_eq!(applied.applied, 1);
        assert!(!parked(&store, "birch", &hashes[0]));
        assert!(store.apply_operational_repair(&plan.token).unwrap().already_applied);
        store.park_outbound_with_failure(&store, "birch", &hashes, "replication-ingress/raw-oversize").unwrap();
        assert!(parked(&store, "birch", &hashes[0]));
        assert!(store.apply_operational_repair(&plan.token).unwrap().already_applied);
        assert!(parked(&store, "birch", &hashes[0]));
        let next = store.operational_repair_plan_with_legacy_starts_and_filter(&[], &[], &[REPAIR_CLASS.into()]).unwrap();
        assert_eq!(next.items.len(), 1);
        assert_ne!(next.token, plan.token);
    }

    #[test]
    fn additional_rejected_pages_reuse_open_episode_without_alert_recursion() {
        let store = Store::open_memory("alder").unwrap();
        for hash in ["first-page", "rejected-failure-alert", "third-page"] {
            store.park_outbound("birch", &[hash.into()], "replication-ingress/malformed-json", None).unwrap();
        }
        let plan = store.operational_repair_plan_with_legacy_starts_and_filter(&[], &[], &[REPAIR_CLASS.into()]).unwrap();
        assert_eq!(plan.items.len(), 1);
        let connection = store.readers.get();
        let (rows, episodes): (u64, u64) = connection.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT episode) FROM outbound_parks", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!((rows, episodes), (3, 1));
        drop(connection);
        store.apply_operational_repair(&plan.token).unwrap();
        assert!(!parked(&store, "birch", "rejected-failure-alert"));
    }

    #[test]
    fn shard_park_repair_is_destination_and_current_episode_bound() {
        let root = tempfile::tempdir().unwrap();
        let graph = Arc::new(Store::open(&root.path().join("claims.sqlite3"), "alder").unwrap());
        let shard = messages_shard::open(&graph, root.path()).unwrap();
        let hashes = vec!["shard-envelope".into()];
        graph.park_outbound_with_failure(&shard, "birch", &hashes, "replication-ingress/inflated-oversize").unwrap();
        shard.park_outbound("cedar", &hashes, "replication-ingress/inflated-oversize", Some("other-destination")).unwrap();
        let plan = graph.operational_repair_plan_with_legacy_starts_and_filter(&[], &[], &[REPAIR_CLASS.into()]).unwrap();
        assert_eq!(plan.items.len(), 1);
        graph.apply_operational_repair(&plan.token).unwrap();
        let rows: Vec<(String, String)> = shard.readers.get().prepare(
            "SELECT peer, episode FROM outbound_parks ORDER BY peer",
        ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(rows, vec![("cedar".into(), "other-destination".into())]);
        graph.park_outbound_with_failure(&shard, "birch", &hashes, "replication-ingress/inflated-oversize").unwrap();
        assert!(graph.apply_operational_repair(&plan.token).unwrap().already_applied);
        let remaining: u64 = shard.readers.get().query_row(
            "SELECT COUNT(*) FROM outbound_parks WHERE peer='birch'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn failed_raw_park_rolls_back_its_fault_and_exclusion_together() {
        let store = Store::open_memory("alder").unwrap();
        store.connection.write().execute_batch(
            "CREATE TRIGGER refuse_outbound_park BEFORE INSERT ON outbound_parks
             BEGIN SELECT RAISE(ABORT, 'park refused'); END;",
        ).unwrap();
        assert!(store.park_outbound("birch", &["envelope".into()], "replication-ingress/raw-oversize", None).is_err());
        assert!(!parked(&store, "birch", "envelope"));
        let plan = store.operational_repair_plan_with_legacy_starts_and_filter(&[], &[], &[REPAIR_CLASS.into()]).unwrap();
        assert!(plan.items.is_empty());
    }
}
