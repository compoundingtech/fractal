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
    Failed,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
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
        self.phase == Phase::Completed
    }
    pub fn next_deadline_ms(&self) -> Option<u128> {
        DEADLINES
            .iter()
            .find(|(_, age)| !self.resolved() && self.age < *age)
            .map(|(delay, _)| self.first_pending_ms.saturating_add(*delay))
    }
    /// Time is supplied by the caller, including after recovery; never read a wall clock.
    pub fn reduce(&mut self, event: Event, now_ms: u128) -> Result<()> {
        anyhow::ensure!(!self.resolved(), "refresh already resolved");
        match event {
            Event::Tick => {}
            Event::Target(desired_token, digest) => {
                anyhow::ensure!(self.phase == Phase::Pending, "refresh target is sealed");
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
                if self.resolved() {
                    self.outcome = Some("replaced".into());
                }
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
fn load(connection: &Connection, seat: &str, incumbent: &str) -> Result<Option<Episode>> {
    let body = connection
        .query_row(
            &canonical_sql(
                "SELECT body FROM claims WHERE subject=?1 AND kind='runtime.refresh'
         AND json_extract(body,'$.fields.episode.incumbent')=?2
         ORDER BY CANONICAL_DESC(claims) LIMIT 1",
            ),
            params![seat, incumbent],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    body.map(|body| {
        let value: Value = serde_json::from_str(&body)?;
        Ok(serde_json::from_value(value["fields"]["episode"].clone())?)
    })
    .transpose()
}
fn persist(connection: &Transaction<'_>, origin: &str, episode: &Episode) -> Result<()> {
    append_claim_tx(
        connection,
        origin,
        &episode.seat,
        "runtime.refresh",
        None,
        &json!({"fields": {"episode": episode}}),
        &[],
        None,
    )?;
    Ok(())
}
impl Store {
    pub fn refresh_episode(&self, seat: &str, incumbent: &str) -> Result<Option<Episode>> {
        load(&self.readers.get(), seat, incumbent)
    }
    /// A second operation for the same incumbent is refused, even after resolution.
    pub fn create_refresh(&self, episode: &Episode) -> Result<()> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        anyhow::ensure!(
            load(&transaction, &episode.seat, &episode.incumbent)?.is_none(),
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
        event: Event,
        now_ms: u128,
    ) -> Result<Episode> {
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let mut episode = load(&transaction, seat, incumbent)?
            .ok_or_else(|| anyhow::anyhow!("refresh episode missing"))?;
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
        assert_eq!(jump.next_deadline_ms(), None);
    }
    #[test]
    fn refresh_coalesces_converges_and_seals() {
        let mut e = episode();
        e.reduce(Event::Target("next".into(), "latest".into()), 2000)
            .unwrap();
        assert_eq!(e.first_pending_ms, 1000);
        assert_eq!(e.initial_target, "new");
        assert_eq!(e.target, "latest");
        let mut sealed = e.clone();
        sealed.reduce(Event::Seal(7), 2000).unwrap();
        assert!(
            sealed
                .reduce(Event::Target("next".into(), "old".into()), 2000)
                .is_err()
        );
        for phase in [Phase::Stopping, Phase::Starting, Phase::Verifying] {
            sealed.reduce(Event::Advance(phase), 2000).unwrap();
        }
        let mut completed = sealed.clone();
        completed
            .reduce(Event::Advance(Phase::Completed), 2000)
            .unwrap();
        assert!(completed.resolved());
        sealed.reduce(Event::Fail("blocked".into()), 2000).unwrap();
        sealed.reduce(Event::Tick, 1000 + 86400 * 1000).unwrap();
        assert_eq!(sealed.age, Age::Escalated);
        e.reduce(Event::Target("next".into(), "old".into()), 3000)
            .unwrap();
        assert!(e.resolved());
        assert_eq!(e.outcome.as_deref(), Some("converged"));
        assert!(e.sealed_epoch.is_none());
    }
    #[test]
    fn refresh_reopens_and_refuses_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let before = {
            let store = Store::open(&path, "test").unwrap();
            store.create_refresh(&episode()).unwrap();
            assert!(store.create_refresh(&episode()).is_err());
            store
                .reduce_refresh("agent/seat", "incumbent", Event::Seal(7), 1000)
                .unwrap();
            store
                .reduce_refresh("agent/seat", "incumbent", Event::Tick, 1000 + 86400 * 1000)
                .unwrap()
        };
        let store = Store::open(&path, "test").unwrap();
        assert_eq!(
            store.refresh_episode("agent/seat", "incumbent").unwrap(),
            Some(before)
        );
        assert!(store.create_refresh(&episode()).is_err());
        let after = store
            .reduce_refresh("agent/seat", "incumbent", Event::Tick, 1000 + 86400 * 1000)
            .unwrap();
        assert_eq!(after.effects.len(), 3);
    }
}
