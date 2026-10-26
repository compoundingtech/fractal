//! Range digests over the claims this store retains.
//!
//! Claims sync compares claim sets, not envelope histories or projection outputs. Two members can
//! retain different envelopes and trim different superseded claims and still hold the same live
//! inputs, so the comparison summarizes claim identities. A claim ID is the hash of its content and
//! batch, so the digest of IDs commits each claim's content too.
//!
//! Each claim falls in one leaf range by the first three hex digits of its ID (4,096 leaves), and
//! each leaf in one top range by its first six bits (64 tops). Every range keeps a count and the
//! 512-bit modular sum of its members' domain-separated SHA-512 hashes, so a range's digest is
//! independent of arrival order and changes in constant work per claim. A top is the sum of its
//! leaves, and the whole set is the sum of the tops. Comparing tops, then the leaves of differing
//! tops, then the IDs of differing leaves finds a difference without listing the set. An ID that
//! is not 64 lowercase hex digits (only test fixtures write one) is counted in a separate irregular
//! range so the sums stay complete.
//!
//! Plain-SQL triggers queue every claim insert and delete in `claim_range_pending`; a background
//! job folds the queue into the digests. The persistent schema calls no application function, so
//! an earlier build that writes claims still queues its changes. Claims stored before the triggers
//! existed join through a paced import whose cursor and bound live in `claim_range_import`. The
//! digests cover every retained claim exactly when both the import and the queue are empty.
//!
//! These are transport summaries. They do not authenticate claims; ordinary admission does.
use super::*;
use crate::replication::{
    CLAIM_RANGE_ANSWER_IDS, CLAIM_RANGE_QUERY_CLAIMS, CLAIM_RANGE_QUERY_LEAVES, CLAIM_RANGE_QUERY_TOPS,
    CLAIM_RANGE_VERSION, ClaimRangeComparison, ClaimRangeDigest, ClaimRangeEntry, ClaimRangeIds,
    ClaimRangeLeaves, ClaimRangeQuery, ClaimRangeSummary,
};
use sha2::Sha512;

/// The namespace of claims this store retains. Retired claim IDs will be a separate namespace.
pub const RETAINED: i64 = 0;
pub const TOP_RANGES: u32 = 64;
pub const LEAF_RANGES: u32 = 4096;
const LEAVES_PER_TOP: u32 = LEAF_RANGES / TOP_RANGES;
const PAGE: i64 = 4096;

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-ha87d0dffb01d owner=smallclaims::store::claim_ranges TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS claim_range_digests (
    namespace INTEGER NOT NULL,
    level INTEGER NOT NULL CHECK(level IN (1, 2)),
    range INTEGER NOT NULL,
    count INTEGER NOT NULL CHECK(count>0),
    accumulator BLOB NOT NULL CHECK(length(accumulator)=64),
    PRIMARY KEY(namespace, level, range)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS claim_range_import (
    id INTEGER PRIMARY KEY CHECK(id=1),
    cursor INTEGER NOT NULL,
    bound INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS claim_range_pending (
    change INTEGER PRIMARY KEY,
    id TEXT NOT NULL,
    sign INTEGER NOT NULL CHECK(sign IN (1, -1))
);
CREATE TRIGGER IF NOT EXISTS claim_range_insert_v1 AFTER INSERT ON claims BEGIN
    INSERT INTO claim_range_pending(id, sign) VALUES (NEW.id, 1);
END;
-- A claim the import has not reached was never added, so its delete subtracts nothing.
CREATE TRIGGER IF NOT EXISTS claim_range_delete_v1 AFTER DELETE ON claims
WHEN NOT EXISTS (SELECT 1 FROM claim_range_import
                 WHERE OLD.store_index>cursor AND OLD.store_index<=bound)
BEGIN
    INSERT INTO claim_range_pending(id, sign) VALUES (OLD.id, -1);
END;";

/// Create the schema and, once per store, start the import over the claims that predate it. Every
/// claim up to today's highest index waits for the import, and later ones arrive by trigger:
/// AUTOINCREMENT never reissues an index, so the two sets never overlap.
pub fn install(connection: &Connection) -> Result<()> {
    // A savepoint nests inside a fresh store's schema transaction and stands alone otherwise, so
    // the triggers and the import bound commit together.
    connection.execute_batch(
        &format!(
            /* sqlite-large-value: allow-for-now lv-h87a7ee032fc0 owner=smallclaims::store::claim_ranges TODO=prepare-body-and-predecessor-values */ "SAVEPOINT claim_ranges_install;
             {SCHEMA}
             INSERT INTO claim_range_import(id, cursor, bound)
             SELECT 1, 0, (SELECT MAX(store_index) FROM claims)
             WHERE NOT EXISTS (SELECT 1 FROM meta WHERE key='claim_ranges_installed')
               AND EXISTS (SELECT 1 FROM claims);
             INSERT OR IGNORE INTO meta(key, value) VALUES ('claim_ranges_installed', '1');
             RELEASE claim_ranges_install;"
        ),
    )?;
    Ok(())
}

/// The leaf range of a claim ID. Irregular IDs share leaf 4096 (top 64).
pub fn leaf(id: &str) -> u32 {
    let regular = id.len() == 64 && id.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if regular { u32::from_str_radix(&id[..3], 16).expect("three hex digits") } else { LEAF_RANGES }
}

fn member_hash(namespace: i64, id: &str) -> [u8; 64] {
    let mut hash = Sha512::new();
    hash.update(b"st3-claim-range-v1\0");
    hash.update(namespace.to_be_bytes());
    hash.update((id.len() as u64).to_be_bytes());
    hash.update(id.as_bytes());
    hash.finalize().into()
}

/// Signed changes to ranges, summed in memory and written once per range.
#[derive(Default)]
struct Delta(BTreeMap<(i64, u32), (i64, [u8; 64])>);

impl Delta {
    fn change(&mut self, id: &str, sign: i64) {
        let hash = member_hash(RETAINED, id);
        let leaf = leaf(id);
        for key in [(1, leaf / LEAVES_PER_TOP), (2, leaf)] {
            let (count, sum) = self.0.entry(key).or_insert((0, [0; 64]));
            *count += sign;
            projection_digest::add(sum, &hash, sign < 0);
        }
    }

    fn apply(self, connection: &Connection) -> Result<()> {
        let mut read = connection.prepare_cached(
            "SELECT count, accumulator FROM claim_range_digests
             WHERE namespace=?1 AND level=?2 AND range=?3",
        )?;
        let mut write = connection.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h49fe314134f8 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO claim_range_digests(namespace, level, range, count, accumulator)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(namespace, level, range) DO UPDATE SET
                 count=excluded.count, accumulator=excluded.accumulator",
        )?;
        let mut remove = connection.prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h7d12df891863 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_range_digests WHERE namespace=?1 AND level=?2 AND range=?3",
        )?;
        for ((level, range), (change, sum)) in self.0 {
            let stored: Option<(i64, Vec<u8>)> = read
                .query_row(/* sqlite-large-value: allow-for-now lv-h121d37c4c0a8 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ params![RETAINED, level, range], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            let (count, mut accumulator) = match stored {
                Some((count, bytes)) => (count, digest_bytes(bytes)?),
                None => (0, [0; 64]),
            };
            let count = count + change;
            anyhow::ensure!(count >= 0, "claim range {level}/{range} would hold {count} claims");
            projection_digest::add(&mut accumulator, &sum, false);
            if count == 0 {
                remove.execute(/* sqlite-large-value: allow-for-now lv-h1d30d73585b7 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ params![RETAINED, level, range])?;
            } else {
                write.execute(/* sqlite-large-value: allow-for-now lv-hd8f1bc4be2f9 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ params![RETAINED, level, range, count, accumulator.as_slice()])?;
            }
        }
        Ok(())
    }
}

fn digest_bytes(bytes: Vec<u8>) -> Result<[u8; 64]> {
    bytes.try_into().map_err(|_| anyhow::anyhow!("invalid claim range digest"))
}

/// One range's population and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRange {
    pub range: u32,
    pub count: u64,
    pub digest: [u8; 64],
}

fn read_ranges(connection: &Connection, level: i64, from: u32, to: u32) -> Result<Vec<ClaimRange>> {
    let rows: Vec<(u32, u64, Vec<u8>)> = connection
        .prepare_cached(
            "SELECT range, count, accumulator FROM claim_range_digests
             WHERE namespace=?1 AND level=?2 AND range>=?3 AND range<?4 ORDER BY range",
        )?
        .query_map(params![RETAINED, level, from, to], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(range, count, digest)| Ok(ClaimRange { range, count, digest: digest_bytes(digest)? }))
        .collect()
}

/// Every non-empty top range, including the irregular range 64.
pub fn tops(connection: &Connection) -> Result<Vec<ClaimRange>> {
    read_ranges(connection, 1, 0, TOP_RANGES + 1)
}

/// The non-empty leaves of one top range. Top 64 holds only the irregular leaf 4096.
pub fn leaves(connection: &Connection, top: u32) -> Result<Vec<ClaimRange>> {
    read_ranges(connection, 2, top * LEAVES_PER_TOP, (top + 1) * LEAVES_PER_TOP)
}

/// The count and digest of every retained claim: the sum of the tops.
pub fn whole(connection: &Connection) -> Result<(u64, [u8; 64])> {
    let mut count = 0;
    let mut digest = [0; 64];
    for top in tops(connection)? {
        count += top.count;
        projection_digest::add(&mut digest, &top.digest, false);
    }
    Ok((count, digest))
}

/// The retained claim IDs in one leaf range, in ID order.
pub fn ids_in_leaf(connection: &Connection, leaf: u32) -> Result<Vec<String>> {
    let ids: Vec<String> = if leaf >= LEAF_RANGES {
        // Only fixtures write irregular IDs; a real store's irregular range is empty.
        connection
            .prepare("SELECT id FROM claims ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    } else {
        // 'g' sorts after every hex digit, so it bounds the last leaf.
        let to = if leaf + 1 == LEAF_RANGES { "g".into() } else { format!("{:03x}", leaf + 1) };
        connection
            .prepare_cached("SELECT id FROM claims WHERE id>=?1 AND id<?2 ORDER BY id")?
            .query_map(params![format!("{leaf:03x}"), to], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    Ok(ids.into_iter().filter(|id| self::leaf(id) == leaf).collect())
}

/// Whether the digests cover exactly the retained claims: the import is done and nothing waits.
pub fn complete(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM claim_range_import)
            AND NOT EXISTS(SELECT 1 FROM claim_range_pending)",
        [],
        |row| row.get(0),
    )?)
}

/// What one page of work folded into the digests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClaimRangePage {
    pub changes: usize,
    pub done: bool,
}

/// Fold one page into the digests: queued changes first, then claims that predate the triggers.
/// Each page commits with its cursor or its removal from the queue.
fn fold_page(connection: &Connection) -> Result<ClaimRangePage> {
    let pending: Vec<(i64, String, i64)> = connection
        .prepare_cached("SELECT change, id, sign FROM claim_range_pending ORDER BY change LIMIT ?1")?
        .query_map([PAGE], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some((through, _, _)) = pending.last() {
        let mut delta = Delta::default();
        for (_, id, sign) in &pending {
            delta.change(id, *sign);
        }
        delta.apply(connection)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-haef91a621227 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_range_pending WHERE change<=?1", [through])?;
        return Ok(ClaimRangePage { changes: pending.len(), done: false });
    }
    let Some((cursor, bound)): Option<(i64, i64)> = connection
        .query_row("SELECT cursor, bound FROM claim_range_import", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?
    else {
        return Ok(ClaimRangePage { changes: 0, done: true });
    };
    let claims: Vec<(i64, String)> = connection
        .prepare_cached(
            "SELECT store_index, id FROM claims WHERE store_index>?1 AND store_index<=?2
             ORDER BY store_index LIMIT ?3",
        )?
        .query_map(params![cursor, bound, PAGE], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut delta = Delta::default();
    for (_, id) in &claims {
        delta.change(id, 1);
    }
    delta.apply(connection)?;
    match claims.last() {
        Some((through, _)) if *through < bound => {
            connection.execute(/* sqlite-large-value: allow-for-now lv-hcaaa5dcfcb19 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "UPDATE claim_range_import SET cursor=?1", [through])?;
        }
        _ => {
            connection.execute(/* sqlite-large-value: allow-for-now lv-hfe42d9c80b3d owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_range_import", [])?;
        }
    }
    Ok(ClaimRangePage { changes: claims.len(), done: false })
}

impl Store {
    /// Fold one bounded page into the claim range digests on a background writer loan, and return
    /// it with the loan's hold time.
    pub fn fold_claim_ranges(&self) -> Result<(ClaimRangePage, std::time::Duration)> {
        let mut connection = self.connection.write_background();
        let started = std::time::Instant::now();
        let transaction = connection.transaction()?;
        let page = fold_page(&transaction)?;
        transaction.commit()?;
        Ok((page, started.elapsed()))
    }

    /// Fold until nothing waits, resting `retired::import_rest` outside the loan between pages.
    pub fn fold_claim_ranges_paced(&self, mut rest: impl FnMut(std::time::Duration)) -> Result<()> {
        loop {
            let (page, held) = self.fold_claim_ranges()?;
            if page.done {
                return Ok(());
            }
            rest(retired::import_rest(held));
        }
    }

    /// Whether the claim range digests cover exactly the retained claims.
    pub fn claim_ranges_complete(&self) -> Result<bool> {
        complete(&self.readers.get())
    }

    /// What this node tells a peer about the claims it retains: the whole set's count and digest,
    /// read with the index's completeness in one read transaction, or none while it folds.
    pub fn claim_range_summary(&self) -> Result<ClaimRangeSummary> {
        self.claim_range_summary_answering(None)
    }

    /// The summary, with answers to a peer's query from the same read. An index still folding
    /// answers nothing, since its ranges do not yet match its claims. Each answer is bounded by
    /// the `CLAIM_RANGE_*` limits whatever the query asks. The irregular range is never listed:
    /// only fixtures write such IDs, and listing it reads every claim.
    pub fn claim_range_summary_answering(
        &self,
        query: Option<&ClaimRangeQuery>,
    ) -> Result<ClaimRangeSummary> {
        let connection = self.readers.get();
        let read = connection.unchecked_transaction()?;
        let mut summary = ClaimRangeSummary { version: CLAIM_RANGE_VERSION, ..Default::default() };
        if !complete(&read)? {
            return Ok(summary);
        }
        let (count, digest) = whole(&read)?;
        summary.retained = Some(ClaimRangeDigest { count, digest: hex::encode(digest) });
        let Some(query) = query else {
            return Ok(summary);
        };
        if query.tops {
            summary.tops = Some(tops(&read)?.iter().map(entry).collect());
        }
        for &top in query.leaves_of.iter().take(CLAIM_RANGE_QUERY_TOPS) {
            if top <= TOP_RANGES {
                let leaves = leaves(&read, top)?.iter().map(entry).collect();
                summary.leaves.push(ClaimRangeLeaves { top, leaves });
            }
        }
        let mut listed = 0;
        for &leaf in query.ids_of.iter().take(CLAIM_RANGE_QUERY_LEAVES) {
            if listed >= CLAIM_RANGE_ANSWER_IDS {
                break;
            }
            if leaf < LEAF_RANGES {
                let ids = ids_in_leaf(&read, leaf)?;
                listed += ids.len();
                summary.ids.push(ClaimRangeIds { leaf, ids });
            }
        }
        Ok(summary)
    }

    /// Compare a peer's claim-range summary, and the answers in it, with this node's index.
    /// `None` when they cannot be compared: another protocol version, or either index still
    /// folding. Only ranges the peer holds claims
    /// in are named, since only those can hold a claim this node lacks; a claim this node has
    /// retired is never named missing.
    pub fn compare_claim_ranges(
        &self,
        remote: &ClaimRangeSummary,
    ) -> Result<Option<ClaimRangeComparison>> {
        let Some(theirs) = remote.retained.as_ref().filter(|_| remote.version == CLAIM_RANGE_VERSION)
        else {
            return Ok(None);
        };
        let connection = self.readers.get();
        let read = connection.unchecked_transaction()?;
        if !complete(&read)? {
            return Ok(None);
        }
        let (count, digest) = whole(&read)?;
        let ours = hex::encode(digest);
        let mut comparison =
            ClaimRangeComparison { pair: format!("{ours}/{}", theirs.digest), ..Default::default() };
        if count == theirs.count && ours == theirs.digest {
            comparison.same = true;
            return Ok(Some(comparison));
        }
        if let Some(remote_tops) = &remote.tops {
            comparison.tops = differing(&tops(&read)?, remote_tops);
        }
        for group in remote.leaves.iter().take(CLAIM_RANGE_QUERY_TOPS) {
            if group.top <= TOP_RANGES {
                comparison.leaves.extend(differing(&leaves(&read, group.top)?, &group.leaves));
            }
        }
        let mut examined = 0;
        for group in remote.ids.iter().take(CLAIM_RANGE_QUERY_LEAVES) {
            if group.leaf >= LEAF_RANGES || examined >= CLAIM_RANGE_ANSWER_IDS {
                continue;
            }
            let local: BTreeSet<String> = ids_in_leaf(&read, group.leaf)?.into_iter().collect();
            for id in &group.ids {
                examined += 1;
                if leaf(id) == group.leaf
                    && !local.contains(id)
                    && !retired::claim_retired(&read, id)?
                {
                    comparison.missing.push(id.clone());
                }
            }
        }
        Ok(Some(comparison))
    }

    /// The envelopes that carry these claims, for a peer that found it lacks them; at most
    /// `CLAIM_RANGE_QUERY_CLAIMS` claims are looked up.
    pub fn envelopes_carrying_claims(&self, claims: &[String]) -> Result<Vec<ReplicaEnvelopeId>> {
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT envelopes.writer, envelopes.sequence, envelopes.envelope_hash
             FROM claims JOIN replica_envelopes AS envelopes ON envelopes.batch_id=claims.batch_id
             WHERE claims.id=?1",
        )?;
        let mut found = BTreeSet::new();
        for id in claims.iter().take(CLAIM_RANGE_QUERY_CLAIMS) {
            let rows = statement.query_map([id], |row| {
                Ok(ReplicaEnvelopeId { writer: row.get(0)?, sequence: row.get(1)?, hash: row.get(2)? })
            })?;
            for row in rows {
                found.insert(row?);
            }
        }
        Ok(found.into_iter().collect())
    }
}

fn entry(range: &ClaimRange) -> ClaimRangeEntry {
    ClaimRangeEntry { range: range.range, count: range.count, digest: hex::encode(range.digest) }
}

/// The ranges the peer holds claims in whose population or digest differs from this node's.
fn differing(local: &[ClaimRange], remote: &[ClaimRangeEntry]) -> Vec<u32> {
    let local: BTreeMap<u32, (u64, String)> =
        local.iter().map(|range| (range.range, (range.count, hex::encode(range.digest)))).collect();
    remote
        .iter()
        .filter(|range| local.get(&range.range) != Some(&(range.count, range.digest.clone())))
        .map(|range| range.range)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::runtime::Plain;
    use std::sync::Arc;

    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE claims(store_index INTEGER PRIMARY KEY AUTOINCREMENT,
                     id TEXT NOT NULL UNIQUE);",
            )
            .unwrap();
        connection
    }

    fn id(n: u32) -> String {
        hex::encode(Sha256::digest(n.to_be_bytes()))
    }

    fn insert(connection: &Connection, ids: impl IntoIterator<Item = String>) {
        let transaction = connection.unchecked_transaction().unwrap();
        for id in ids {
            transaction.execute("INSERT INTO claims(id) VALUES(?1)", [id]).unwrap();
        }
        transaction.commit().unwrap();
    }

    fn fold_all(connection: &Connection) -> usize {
        let mut pages = 0;
        while !fold_page(connection).unwrap().done {
            pages += 1;
            assert!(pages < 100);
        }
        assert!(complete(connection).unwrap());
        pages
    }

    /// Every range recomputed from the claims table.
    fn recomputed(connection: &Connection) -> Vec<(i64, u32, u64, [u8; 64])> {
        let ids: Vec<String> = connection
            .prepare("SELECT id FROM claims")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let mut ranges = BTreeMap::<(i64, u32), (u64, [u8; 64])>::new();
        for id in ids {
            for key in [(1, leaf(&id) / LEAVES_PER_TOP), (2, leaf(&id))] {
                let (count, sum) = ranges.entry(key).or_insert((0, [0; 64]));
                *count += 1;
                projection_digest::add(sum, &member_hash(RETAINED, &id), false);
            }
        }
        ranges.into_iter().map(|((level, range), (count, sum))| (level, range, count, sum)).collect()
    }

    fn maintained(connection: &Connection) -> Vec<(i64, u32, u64, [u8; 64])> {
        let mut ranges = Vec::new();
        for top in tops(connection).unwrap() {
            for leaf in leaves(connection, top.range).unwrap() {
                ranges.push((2, leaf.range, leaf.count, leaf.digest));
            }
            ranges.push((1, top.range, top.count, top.digest));
        }
        ranges.sort_by_key(|range| (range.0, range.1));
        ranges
    }

    #[test]
    fn the_digests_are_the_claim_set_whatever_the_arrival_order_or_deletes() {
        let ids: Vec<String> = (0..6000).map(id).chain(["fixture-claim".to_string()]).collect();
        let forward = fixture();
        install(&forward).unwrap();
        insert(&forward, ids.iter().cloned());
        let reverse = fixture();
        install(&reverse).unwrap();
        insert(&reverse, ids.iter().rev().cloned());
        assert!(!complete(&forward).unwrap(), "queued changes are not yet in the digests");
        assert!(fold_all(&forward) > 1);
        fold_all(&reverse);
        assert_eq!(maintained(&forward), recomputed(&forward));
        assert_eq!(maintained(&forward), maintained(&reverse));
        assert_eq!(whole(&forward).unwrap().0, 6001);
        // A trim deletes some claims; the digests follow, and emptied ranges disappear.
        forward
            .execute("DELETE FROM claims WHERE store_index%3=0 OR id='fixture-claim'", [])
            .unwrap();
        fold_all(&forward);
        assert_eq!(maintained(&forward), recomputed(&forward));
        assert_ne!(whole(&forward).unwrap(), whole(&reverse).unwrap());
        // A leaf lists exactly its members, so a differing leaf names the differing claims.
        let leaf = super::leaf(&id(7));
        let mut members: Vec<String> =
            ids.iter().filter(|id| super::leaf(id) == leaf).cloned().collect();
        members.sort();
        assert_eq!(ids_in_leaf(&reverse, leaf).unwrap(), members);
        assert_eq!(ids_in_leaf(&reverse, LEAF_RANGES).unwrap(), vec!["fixture-claim".to_string()]);
    }

    #[test]
    fn claims_from_before_the_triggers_join_by_import_while_inserts_and_deletes_continue() {
        let connection = fixture();
        insert(&connection, (0..10_000).map(id));
        install(&connection).unwrap();
        // Reinstalling, as every open does, neither restarts nor duplicates the import.
        install(&connection).unwrap();
        assert!(!complete(&connection).unwrap());
        assert!(fold_page(&connection).unwrap().changes > 0);
        // Deletes on both sides of the cursor and new claims arrive mid-import.
        connection
            .execute("DELETE FROM claims WHERE store_index IN (1, 9000, 9999)", [])
            .unwrap();
        insert(&connection, (10_000..10_050).map(id));
        connection.execute("DELETE FROM claims WHERE store_index=10010", []).unwrap();
        fold_all(&connection);
        assert_eq!(maintained(&connection), recomputed(&connection));
        // After the import, every delete subtracts.
        connection.execute("DELETE FROM claims WHERE store_index<=500", []).unwrap();
        fold_all(&connection);
        assert_eq!(maintained(&connection), recomputed(&connection));
    }

    #[test]
    fn a_store_queues_its_claims_from_the_first_and_folds_them_in_the_background() {
        let store = Store::open_memory("writer", Arc::new(Plain)).unwrap();
        for n in 0..20 {
            store
                .append_claim(&crate::claim::ClaimInput {
                    subject: format!("note/{n}"),
                    kind: "example.note".into(),
                    actor: None,
                    fields: BTreeMap::new(),
                    evidence: vec![],
                    expected_subject: None,
                    idempotency_key: None,
                })
                .unwrap();
        }
        assert!(!store.claim_ranges_complete().unwrap());
        let mut rests = 0;
        store.fold_claim_ranges_paced(|_| rests += 1).unwrap();
        assert_eq!(rests, 1);
        assert!(store.claim_ranges_complete().unwrap());
        let connection = store.readers.get();
        assert_eq!(maintained(&connection), recomputed(&connection));
        assert_eq!(whole(&connection).unwrap().0, 20);
    }
}
