//! Immutable presented rosters, published by the existing shared list owner.

use std::sync::{Arc, OnceLock};
use serde::{Serialize, Serializer, ser::SerializeStruct as _};
use serde_json::Value;
use uuid::Uuid;
use super::published_list::{Publication, PublicationId};

const MAX_REVISION: u64 = (1 << 53) - 1;

pub struct NodeEpoch(std::sync::LazyLock<Uuid>);

impl Default for NodeEpoch {
    fn default() -> Self { Self(std::sync::LazyLock::new(Uuid::new_v4)) }
}

impl NodeEpoch {
    pub fn get(&self) -> Uuid { *self.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AgentsStatusWatermark {
    pub store_index: u64,
    pub local_frontier: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AgentsPublicationMetadata {
    #[serde(serialize_with = "serialize_epoch")]
    pub node_epoch: Uuid,
    pub revision: u64,
    pub status_watermark: AgentsStatusWatermark,
    pub materialized_at_ms: u64,
}

fn serialize_epoch<S: Serializer>(epoch: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(epoch)
}

/// Rows and metadata become visible together in the existing PublishedList's Arc.
pub struct PresentedAgents {
    metadata: AgentsPublicationMetadata,
    rows: Vec<Value>,
    order: Vec<String>,
    encoded: OnceLock<Arc<[u8]>>,
}

pub type AgentsPublication = Publication<PresentedAgents>;

impl PresentedAgents {
    pub fn new(
        epoch: Uuid,
        identity: PublicationId,
        watermark: AgentsStatusWatermark,
        materialized_at_ms: u64,
        rows: Vec<Value>,
        order: Vec<String>,
    ) -> Self {
        // The list generation fences resets; integer exhaustion rolls to another epoch before
        // a revision could exceed the exact integer range shared by JSON clients.
        let rollover = (identity.sequence - 1) / MAX_REVISION;
        let node_epoch = Uuid::from_u128(epoch.as_u128()
            ^ (u128::from(identity.generation) << 64) ^ u128::from(rollover));
        Self {
            metadata: AgentsPublicationMetadata {
                node_epoch,
                revision: (identity.sequence - 1) % MAX_REVISION + 1,
                status_watermark: watermark,
                materialized_at_ms,
            },
            rows,
            order,
            encoded: OnceLock::new(),
        }
    }
}

impl AgentsPublication {
    pub fn metadata(&self) -> &AgentsPublicationMetadata { &self.rows.metadata }
    pub fn rows(&self) -> &[Value] { &self.rows.rows }
    pub fn order(&self) -> &[String] { &self.rows.order }
    pub fn encoded(&self) -> &Arc<[u8]> {
        self.rows.encoded.get_or_init(|| serde_json::to_vec(self)
            .expect("publication JSON values serialize").into())
    }
}

impl Serialize for AgentsPublication {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_struct("AgentsPublication", 4)?;
        object.serialize_field("publication", self.metadata())?;
        object.serialize_field("items", self.rows())?;
        object.serialize_field("order", self.order())?;
        object.serialize_field("has_more", &false)?;
        object.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::published_list::PublishedList;
    use serde_json::json;

    fn prepared(epoch: Uuid, identity: PublicationId, name: &str) -> AgentsPublication {
        Publication {
            cut: 1,
            published_at_unix_ms: 1,
            valid_until_unix_ms: None,
            rows: PresentedAgents::new(epoch, identity,
                AgentsStatusWatermark { store_index: 1, local_frontier: 2 }, 1,
                vec![json!({"id": name})], vec![name.to_owned()]),
        }
    }

    #[test]
    fn agents_publication_revision_exhaustion_starts_a_distinct_epoch() {
        let epoch = Uuid::new_v4();
        let last = prepared(epoch, PublicationId { list: 1, generation: 0, sequence: MAX_REVISION }, "agent/example");
        let next = prepared(epoch, PublicationId { list: 1, generation: 0, sequence: MAX_REVISION + 1 }, "agent/example");
        assert_eq!(last.metadata().revision, MAX_REVISION);
        assert_eq!(next.metadata().revision, 1);
        assert_ne!(last.metadata().node_epoch, next.metadata().node_epoch);
        let reset = prepared(epoch, PublicationId { list: 1, generation: 1, sequence: 1 }, "agent/example");
        assert_ne!(reset.metadata().node_epoch, next.metadata().node_epoch);
    }

    #[test]
    fn agents_publication_shared_owner_discards_late_old_completions() {
        let list = PublishedList::default();
        let epoch = Uuid::new_v4();
        list.start();
        let old = list.begin_materialization().unwrap();
        let new = list.begin_materialization().unwrap();
        assert_eq!(list.publish_identified(new, |id| prepared(epoch, id, "agent/new")), Some(false));
        let current = list.newest().unwrap();
        list.publish_identified(old, |id| prepared(epoch, id, "agent/old"));
        assert!(Arc::ptr_eq(&current, &list.newest().unwrap()));
        assert_eq!(current.order(), ["agent/new"]);
        let reset = list.begin_materialization().unwrap();
        list.forget();
        assert!(list.publish_identified(reset, |id| prepared(epoch, id, "agent/reset")).is_none());
    }

    #[test]
    fn agents_publication_identical_completion_fences_older_work_and_reuses_encoding() {
        let list = PublishedList::default();
        let epoch = Uuid::new_v4();
        list.start();
        list.publish_identified(list.begin_materialization().unwrap(),
            |id| prepared(epoch, id, "agent/example"));
        let current = list.newest().unwrap();
        let encoding = Arc::clone(current.encoded());
        let old = list.begin_materialization().unwrap();
        let same = list.begin_materialization().unwrap();
        assert!(list.retain_materialization(same, &current));
        list.publish_identified(old, |id| prepared(epoch, id, "agent/old"));
        assert!(Arc::ptr_eq(&current, &list.newest().unwrap()));
        assert!(Arc::ptr_eq(&encoding, current.encoded()));
        assert_eq!(current.metadata().revision, 1);
    }
}
