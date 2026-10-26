//! The part of `store::projection_divergence` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use smallclaims::replication::ProjectionDivergence;
pub use fractal_store::store::projection_divergence::*;

pub const PROJECTION_DIVERGENCE_CONDITION: &str = "projection-divergence";

/// The member that records a divergence records it on its own daemon subject.
pub fn projection_divergence_subject(node: &str) -> String {
    format!("daemon/{node}")
}

/// What one check found and did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivergenceCheck {
    pub divergences: usize,
    pub opened: usize,
    pub recovered: usize,
}

/// One open divergence episode. Both members can record the same episode before either sees
/// the other's claim, so it can hold more than one failure; it ends when each is recovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenDivergence {
    pub failures: Vec<String>,
    pub members: Vec<String>,
}

pub trait ProjectionDivergenceStore {
    /// Open a fault for each divergence with no open fault, and end the open faults this node
    /// shares with a peer whose last comparison agrees. Reads the comparisons in memory and the
    /// open faults once; writes only when a fault opens or ends.
    fn check_projection_divergences(&self) -> Result<DivergenceCheck, St3Error>;
    fn append_projection_divergence(
            &self,
            local: &str,
            episode: &str,
            divergence: &ProjectionDivergence,
        ) -> Result<(), St3Error>;
    /// Open divergence episodes: every divergence failure on this member's and its peers' daemon
    /// subjects that no recovery names, however old. Each is a P1 bug, so these claims stay few.
    /// The read never stops at the newest of them, since an older episode that is still open
    /// would then look closed and open again as a second failure.
    fn open_projection_divergences(&self) -> Result<BTreeMap<String, OpenDivergence>>;
}

impl ProjectionDivergenceStore for Store {

    fn check_projection_divergences(&self) -> Result<DivergenceCheck, St3Error> {
        let local = self.origin().to_owned();
        let divergences = self.replication_divergences();
        let open = self.open_projection_divergences().map_err(internal)?;
        let mut check = DivergenceCheck {
            divergences: divergences.len(),
            ..DivergenceCheck::default()
        };
        for divergence in &divergences {
            let episode = episode(&local, divergence);
            if open.contains_key(&episode) {
                continue;
            }
            self.append_projection_divergence(&local, &episode, divergence)?;
            check.opened += 1;
        }
        for (episode, fault) in &open {
            let Some(peer) = fault
                .members
                .iter()
                .find(|member| **member != local)
                .filter(|_| fault.members.contains(&local))
            else {
                continue;
            };
            if !self.replication_projections_agree(peer) {
                continue;
            }
            for failure in &fault.failures {
                self.append_claim(&ClaimInput {
                    subject: projection_divergence_subject(&local),
                    kind: "operational.recovered".into(),
                    actor: Some("daemon/runtime".into()),
                    fields: BTreeMap::from([
                        ("episode".into(), json!(episode)),
                        ("failure".into(), json!(failure)),
                        (
                            "reason".into(),
                            json!(format!(
                                "{local} and {peer} project the same envelopes the same way again"
                            )),
                        ),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("{failure}:recovered")),
                })?;
            }
            check.recovered += 1;
        }
        *self.fractal.open_divergences.lock().unwrap() =
            Some((open.len() + check.opened - check.recovered) as u64);
        Ok(check)
    }

    fn append_projection_divergence(
            &self,
            local: &str,
            episode: &str,
            divergence: &ProjectionDivergence,
        ) -> Result<(), St3Error> {
        let peer = divergence.peer.as_str();
        let peer_build = divergence.peer_build.as_deref().unwrap_or("unknown");
        let reason = format!(
            "{local} and {peer} hold the same envelopes (inventory {inventory}) but project \
             {projection} differently: {local} {local_digest} on build {local_build}, {peer} \
             {peer_digest} on build {peer_build}. A projection is a pure function of the claims, \
             so this is a projection bug: reproduce it from these claims and fix the projection. \
             Nothing repairs it from a peer.",
            inventory = divergence.inventory_digest,
            projection = divergence.projection,
            local_digest = divergence.local_digest,
            local_build = divergence.local_build,
            peer_digest = divergence.peer_digest,
        );
        self.append_claim(&ClaimInput {
            subject: projection_divergence_subject(local),
            kind: "operational.failure".into(),
            actor: Some("daemon/runtime".into()),
            fields: BTreeMap::from([
                ("episode".into(), json!(episode)),
                ("condition".into(), json!(PROJECTION_DIVERGENCE_CONDITION)),
                ("reviewer".into(), json!("person/operator")),
                (
                    "title".into(),
                    json!(format!(
                        "P1 projection bug: {} differs between {local} and {peer}",
                        divergence.projection
                    )),
                ),
                ("reason".into(), json!(reason)),
                ("severity".into(), json!("error")),
                (
                    "targets".into(),
                    json!([
                        projection_divergence_subject(local),
                        projection_divergence_subject(peer)
                    ]),
                ),
                (
                    "reproduction".into(),
                    json!({
                        "projection": divergence.projection,
                        "members": [local, peer],
                        "inventory_digest": divergence.inventory_digest,
                        "writer_high_water": divergence.writer_high_water,
                        "digests": {local: divergence.local_digest, peer: divergence.peer_digest},
                        "builds": {local: divergence.local_build, peer: peer_build},
                        "since_unix_ms": divergence.since_unix_ms,
                        "compared_at_unix_ms": divergence.compared_at_unix_ms,
                    }),
                ),
            ]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("{episode}:{local}:{}", divergence.since_unix_ms)),
        })?;
        Ok(())
    }

    fn open_projection_divergences(&self) -> Result<BTreeMap<String, OpenDivergence>> {
        let mut nodes = self.compared_peers();
        nodes.push(self.origin().to_owned());
        let subjects = Value::from(
            nodes
                .iter()
                .map(|node| projection_divergence_subject(node))
                .collect::<Vec<_>>(),
        )
        .to_string();
        let connection = self.readers.get();
        let mut query = connection.prepare(
            "SELECT id, kind, json_extract(body,'$.fields.episode'),
                    json_extract(body,'$.fields.failure'),
                    json_extract(body,'$.fields.reproduction.members')
             FROM claims WHERE subject IN (SELECT value FROM json_each(?1))
               AND kind IN ('operational.failure','operational.recovered')
               AND json_extract(body,'$.fields.episode') LIKE ?2",
        )?;
        let rows = query
            .query_map(params![subjects, format!("{EPISODE_PREFIX}%")], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let recovered: BTreeSet<&str> = rows
            .iter()
            .filter(|(_, kind, ..)| kind == "operational.recovered")
            .filter_map(|(.., failure, _)| failure.as_deref())
            .collect();
        let mut open: BTreeMap<String, OpenDivergence> = BTreeMap::new();
        for (id, kind, episode, _, members) in &rows {
            if kind != "operational.failure" || recovered.contains(id.as_str()) {
                continue;
            }
            let fault = open.entry(episode.clone()).or_insert_with(|| OpenDivergence {
                failures: Vec::new(),
                members: members
                    .as_deref()
                    .and_then(|members| serde_json::from_str(members).ok())
                    .unwrap_or_default(),
            });
            fault.failures.push(id.clone());
        }
        for fault in open.values_mut() {
            fault.failures.sort();
        }
        Ok(open)
    }
}

