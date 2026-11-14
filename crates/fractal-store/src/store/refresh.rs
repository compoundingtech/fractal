//! Durable refresh policy only: effects are an ordered outbox, never runtime actuation.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Age {
    #[default]
    Pending,
    Attention,
    InterruptEligible,
    Escalated,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Pending,
    Sealing,
    Stopping,
    Starting,
    Verifying,
    Completed,
    Superseded,
    Failed,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Episode {
    pub id: String,
    pub seat: String,
    pub incumbent: String,
    pub desired_token: String,
    pub initial_target: String,
    pub target: String,
    pub incumbent_digest: String,
    pub first_pending_ms: u128,
    pub age: Age,
    pub phase: Phase,
    /// Durable, ordered effect identities: `(id, age)` is the downstream deduplication key.
    pub effects: Vec<Age>,
    pub sealed_epoch: Option<u64>,
    pub outcome: Option<String>,
}
pub enum Event {
    Tick,
    Target(String, String),
    Seal(u64),
    Advance(Phase),
    Fail(String),
}
const DEADLINES: [(u128, Age); 3] = [
    (30 * 60 * 1000, Age::Attention),
    (4 * 60 * 60 * 1000, Age::InterruptEligible),
    (24 * 60 * 60 * 1000, Age::Escalated),
];
impl Episode {
    pub fn resolved(&self) -> bool {
        matches!(self.phase, Phase::Completed | Phase::Superseded)
    }
    /// Time is supplied by the caller, including after recovery; never read a wall clock.
    pub fn reduce(&mut self, event: Event, now_ms: u128) -> Result<()> {
        anyhow::ensure!(!self.resolved(), "refresh already resolved");
        match event {
            Event::Tick => {}
            Event::Target(desired_token, digest) => {
                if self.sealed_epoch.is_some() {
                    if self.target != digest || self.desired_token != desired_token {
                        self.phase = Phase::Superseded;
                        self.outcome = Some("superseded".into());
                    }
                    return Ok(());
                }
                self.desired_token = desired_token;
                self.target = digest;
                if self.target == self.incumbent_digest {
                    self.phase = Phase::Completed;
                    self.outcome = Some("converged".into());
                    return Ok(());
                }
            }
            Event::Seal(epoch) => {
                anyhow::ensure!(self.phase == Phase::Pending, "refresh already sealed");
                self.sealed_epoch = Some(epoch);
                self.phase = Phase::Sealing;
            }
            Event::Advance(phase) => {
                anyhow::ensure!(
                    matches!(
                        (self.phase, phase),
                        (Phase::Sealing, Phase::Stopping)
                            | (Phase::Stopping, Phase::Starting)
                            | (Phase::Starting, Phase::Verifying)
                            | (Phase::Verifying, Phase::Completed)
                    ),
                    "invalid refresh phase transition"
                );
                self.phase = phase;
                self.outcome = (phase == Phase::Completed).then(|| "replaced".into());
            }
            Event::Fail(reason) => {
                self.phase = Phase::Failed;
                self.outcome = Some(reason);
            }
        }
        if !self.resolved() {
            for (delay, age) in DEADLINES {
                if now_ms >= self.first_pending_ms.saturating_add(delay) && self.age < age {
                    self.effects.push(age);
                    self.age = age;
                }
            }
        }
        Ok(())
    }
}
fn load(db: &Connection, seat: &str, incumbent: &str) -> Result<Option<Episode>> {
    let sql = canonical_sql(
        "SELECT body FROM claims WHERE subject=?1 AND kind='runtime.refresh'
         AND json_extract(body,'$.fields.episode.incumbent')=?2
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
    );
    let body = db.query_row(/* sqlite-large-value: allow-for-now lv-h56a3a258cc3e owner=fractal_store::store::refresh TODO=trace-statement-and-value-representation */ &sql, params![seat, incumbent], |r| r.get::<_, String>(0)).optional()?;
    body.map(|body| {
        let mut value: Value = serde_json::from_str(&body)?;
        anyhow::ensure!(value["fields"]["version"] == 1, "unsupported refresh version");
        Ok(serde_json::from_value(value["fields"]["episode"].take())?)
    })
    .transpose()
}
fn persist(tx: &Transaction<'_>, origin: &str, e: &Episode) -> Result<ClaimRecord> {
    let body = json!({"fields": {"version": 1, "episode": e}});
    append_claim_tx(tx, origin, &e.seat, "runtime.refresh", None, &body, &[], None)
}
impl Store {
    pub fn refresh_episode(&self, seat: &str, incumbent: &str) -> Result<Option<Episode>> {
        load(&self.readers.get(), seat, incumbent)
    }
    /// A superseded sealed target permits a fresh operation for the same incumbent.
    pub fn create_refresh(&self, episode: &Episode) -> Result<()> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        anyhow::ensure!(
            load(&transaction, &episode.seat, &episode.incumbent)?
                .is_none_or(|old| old.phase == Phase::Superseded && old.id != episode.id),
            "refresh episode already exists for incumbent"
        );
        anyhow::ensure!(
            episode.phase == Phase::Pending
                && episode.age == Age::Pending
                && episode.effects.is_empty()
                && episode.target != episode.incumbent_digest,
            "refresh must begin pending with drift"
        );
        persist(&transaction, &self.origin, episode)?;
        transaction.commit()?;
        Ok(())
    }
    pub fn reduce_refresh(
        &self,
        seat: &str,
        incumbent: &str,
        operation: &str,
        event: Event,
        now_ms: u128,
    ) -> Result<Episode> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let mut episode = load(&transaction, seat, incumbent)?
            .ok_or_else(|| anyhow::anyhow!("refresh episode missing"))?;
        anyhow::ensure!(episode.id == operation, "refresh operation changed");
        episode.reduce(event, now_ms)?;
        persist(&transaction, &self.origin, &episode)?;
        transaction.commit()?;
        Ok(episode)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn episode() -> Episode {
        Episode {
            id: "operation".into(),
            seat: "agent/seat".into(),
            incumbent: "incumbent".into(),
            desired_token: "desired".into(),
            incumbent_digest: "old".into(),
            initial_target: "new".into(),
            target: "new".into(),
            first_pending_ms: 1000,
            ..Episode::default()
        }
    }
    #[test]
    fn refresh_boundaries_and_jump() {
        let mut e = episode();
        for (seconds, age) in [
            (1799, Age::Pending),
            (1800, Age::Attention),
            (14399, Age::Attention),
            (14400, Age::InterruptEligible),
            (86399, Age::InterruptEligible),
            (86400, Age::Escalated),
        ] {
            e.reduce(Event::Tick, 1000 + seconds * 1000).unwrap();
            assert_eq!(e.age, age);
        }
        let expected = vec![Age::Attention, Age::InterruptEligible, Age::Escalated];
        assert_eq!(e.effects, expected);
        let mut jump = episode();
        jump.reduce(Event::Tick, 1000 + 86400 * 1000).unwrap();
        jump.reduce(Event::Tick, 1000 + 86400 * 1000).unwrap();
        assert_eq!(jump.effects, expected);
    }
    #[test]
    fn refresh_coalesces_converges_and_seals() {
        let mut e = episode();
        e.reduce(Event::Target("next".into(), "latest".into()), 2000).unwrap();
        assert_eq!(e.first_pending_ms, 1000);
        assert_eq!(e.initial_target, "new");
        assert_eq!(e.target, "latest");
        let mut sealed = e.clone();
        sealed.reduce(Event::Seal(7), 2000).unwrap();
        for phase in [Phase::Stopping, Phase::Starting, Phase::Verifying] {
            sealed.reduce(Event::Advance(phase), 2000).unwrap();
        }
        let mut completed = sealed.clone();
        completed.reduce(Event::Advance(Phase::Completed), 2000).unwrap();
        assert!(completed.resolved());
        sealed.reduce(Event::Fail("blocked".into()), 2000).unwrap();
        sealed.reduce(Event::Tick, 1000 + 86400 * 1000).unwrap();
        assert_eq!(sealed.age, Age::Escalated);
        e.reduce(Event::Target("next".into(), "old".into()), 3000).unwrap();
        assert!(e.resolved());
        assert!(e.sealed_epoch.is_none());
    }
    #[test]
    fn refresh_reopens_and_refuses_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let change =
            |s: &Store, id, event| s.reduce_refresh("agent/seat", "incumbent", id, event, 86401000);
        let read = |s: &Store| s.refresh_episode("agent/seat", "incumbent");
        let before = {
            let store = Store::open(&path, "test").unwrap();
            store.create_refresh(&episode()).unwrap();
            assert!(store.create_refresh(&episode()).is_err());
            change(&store, "operation", Event::Seal(7)).unwrap();
            let aged = change(&store, "operation", Event::Tick).unwrap();
            assert_eq!(aged.effects.len(), 3);
            change(&store, "operation", Event::Target("latest".into(), "latest".into())).unwrap()
        };
        let store = Store::open(&path, "test").unwrap();
        assert_eq!(read(&store).unwrap(), Some(before));
        assert!(store.create_refresh(&episode()).is_err());
        let mut next = episode();
        next.id = "next-operation".into();
        store.create_refresh(&next).unwrap();
        change(&store, &next.id, Event::Seal(8)).unwrap();
        assert!(change(&store, "operation", Event::Advance(Phase::Stopping)).is_err());
        let mut connection = store.connection.write();
        let tx = connection.transaction().unwrap();
        let future = json!({"fields": {"version": 2, "episode": next}});
        let seat = &next.seat;
        append_claim_tx(&tx, "test", seat, "runtime.refresh", None, &future, &[], None).unwrap();
        tx.commit().unwrap();
        drop(connection);
        assert!(read(&store).is_err());
        assert!(change(&store, &next.id, Event::Tick).is_err());
    }
}
