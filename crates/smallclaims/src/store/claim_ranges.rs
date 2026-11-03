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
//! The retired claim IDs (`retired_claim_ids`) are summarized the same way in a second namespace,
//! hashed by their claim ID text, with their own queue and import. Members that trim locally
//! compare and walk them beside the retained claims, and record the retired IDs a peer holds
//! that they lack.
//!
//! These are transport summaries. They do not authenticate claims; ordinary admission does.
use super::*;
use crate::replication::{
    CLAIM_RANGE_ANSWER_IDS, CLAIM_RANGE_QUERY_CLAIMS, CLAIM_RANGE_QUERY_LEAVES, CLAIM_RANGE_QUERY_TOPS,
    CLAIM_RANGE_VERSION, ClaimRangeComparison, ClaimRangeDigest, ClaimRangeEntry, ClaimRangeIds,
    ClaimRangeLeaves, ClaimRangeQuery, ClaimRangeSummary,
};
use sha2::Sha512;

/// The namespace of claims this store retains.
pub const RETAINED: i64 = 0;
/// The namespace of claim IDs this store retired (`retired_claim_ids`), summarized the same way so
/// members that trim locally can find the retired IDs a peer holds and this one lacks.
pub const RETIRED: i64 = 1;
pub const TOP_RANGES: u32 = 64;
pub const LEAF_RANGES: u32 = 4096;
const LEAVES_PER_TOP: u32 = LEAF_RANGES / TOP_RANGES;
const PAGE: i64 = 4096;

pub const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-ha32af1cdbaa4 owner=smallclaims::store::claim_ranges TODO=chunk-or-reference-variable-columns */ "
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
END;
-- Retired IDs keep their stored form (`retired::identity_sql`), in untyped columns so no affinity
-- converts them. The import walks them in key order; an ID at or below its cursor, or any ID once
-- it is done, arrives by trigger instead.
CREATE TABLE IF NOT EXISTS retired_range_import (
    id INTEGER PRIMARY KEY CHECK(id=1),
    cursor
);
CREATE TABLE IF NOT EXISTS retired_range_pending (
    change INTEGER PRIMARY KEY,
    id NOT NULL,
    sign INTEGER NOT NULL CHECK(sign IN (1, -1))
);
CREATE TRIGGER IF NOT EXISTS retired_range_insert_v1 AFTER INSERT ON retired_claim_ids
WHEN NOT EXISTS (SELECT 1 FROM retired_range_import WHERE cursor IS NULL OR NEW.id>cursor)
BEGIN
    INSERT INTO retired_range_pending(id, sign) VALUES (NEW.id, 1);
END;
CREATE TRIGGER IF NOT EXISTS retired_range_delete_v1 AFTER DELETE ON retired_claim_ids
WHEN NOT EXISTS (SELECT 1 FROM retired_range_import WHERE cursor IS NULL OR OLD.id>cursor)
BEGIN
    INSERT INTO retired_range_pending(id, sign) VALUES (OLD.id, -1);
END;";

/// Create the schema and, once per store, start the import over the claims that predate it. Every
/// claim up to today's highest index waits for the import, and later ones arrive by trigger:
/// AUTOINCREMENT never reissues an index, so the two sets never overlap.
pub fn install(connection: &Connection) -> Result<()> {
    // A savepoint nests inside a fresh store's schema transaction and stands alone otherwise, so
    // the triggers and the import bound commit together.
    connection.execute_batch(
        &format!(
            /* sqlite-large-value: allow-for-now lv-hed3d278d3e86 owner=smallclaims::store::claim_ranges TODO=prepare-body-and-predecessor-values */ "SAVEPOINT claim_ranges_install;
             {SCHEMA}
             INSERT INTO claim_range_import(id, cursor, bound)
             SELECT 1, 0, (SELECT MAX(store_index) FROM claims)
             WHERE NOT EXISTS (SELECT 1 FROM meta WHERE key='claim_ranges_installed')
               AND EXISTS (SELECT 1 FROM claims);
             INSERT OR IGNORE INTO meta(key, value) VALUES ('claim_ranges_installed', '1');
             INSERT INTO retired_range_import(id, cursor)
             SELECT 1, NULL
             WHERE NOT EXISTS (SELECT 1 FROM meta WHERE key='retired_ranges_installed')
               AND EXISTS (SELECT 1 FROM retired_claim_ids);
             INSERT OR IGNORE INTO meta(key, value) VALUES ('retired_ranges_installed', '1');
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
struct Delta(BTreeMap<(i64, i64, u32), (i64, [u8; 64])>);

impl Delta {
    fn change(&mut self, namespace: i64, id: &str, sign: i64) {
        let hash = member_hash(namespace, id);
        let leaf = leaf(id);
        for (level, range) in [(1, leaf / LEAVES_PER_TOP), (2, leaf)] {
            let key = (namespace, level, range);
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
        for ((namespace, level, range), (change, sum)) in self.0 {
            let stored: Option<(i64, Vec<u8>)> = read
                .query_row(/* sqlite-large-value: allow-for-now lv-h9f843f0fa9a7 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ params![namespace, level, range], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            let (count, mut accumulator) = match stored {
                Some((count, bytes)) => (count, digest_bytes(bytes)?),
                None => (0, [0; 64]),
            };
            let count = count + change;
            anyhow::ensure!(count >= 0, "claim range {level}/{range} would hold {count} claims");
            projection_digest::add(&mut accumulator, &sum, false);
            if count == 0 {
                remove.execute(/* sqlite-large-value: allow-for-now lv-h8d36210a9cc0 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ params![namespace, level, range])?;
            } else {
                write.execute(/* sqlite-large-value: allow-for-now lv-h6d739a028d62 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ params![namespace, level, range, count, accumulator.as_slice()])?;
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

fn read_ranges(
    connection: &Connection,
    namespace: i64,
    level: i64,
    from: u32,
    to: u32,
) -> Result<Vec<ClaimRange>> {
    let rows: Vec<(u32, u64, Vec<u8>)> = connection
        .prepare_cached(
            "SELECT range, count, accumulator FROM claim_range_digests
             WHERE namespace=?1 AND level=?2 AND range>=?3 AND range<?4 ORDER BY range",
        )?
        .query_map(params![namespace, level, from, to], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(range, count, digest)| Ok(ClaimRange { range, count, digest: digest_bytes(digest)? }))
        .collect()
}

/// Every non-empty top range of a namespace, including the irregular range 64.
pub fn tops(connection: &Connection, namespace: i64) -> Result<Vec<ClaimRange>> {
    read_ranges(connection, namespace, 1, 0, TOP_RANGES + 1)
}

/// The non-empty leaves of one top range. Top 64 holds only the irregular leaf 4096.
pub fn leaves(connection: &Connection, namespace: i64, top: u32) -> Result<Vec<ClaimRange>> {
    read_ranges(connection, namespace, 2, top * LEAVES_PER_TOP, (top + 1) * LEAVES_PER_TOP)
}

/// The count and digest of a whole namespace: the sum of the tops.
pub fn whole(connection: &Connection, namespace: i64) -> Result<(u64, [u8; 64])> {
    let mut count = 0;
    let mut digest = [0; 64];
    for top in tops(connection, namespace)? {
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

/// Whether the retired digests cover exactly `retired_claim_ids`, by the same rule.
pub fn retired_complete(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM retired_range_import)
            AND NOT EXISTS(SELECT 1 FROM retired_range_pending)",
        [],
        |row| row.get(0),
    )?)
}

/// The retired claim IDs in one leaf range, in ID order, as the text claim IDs have everywhere.
/// A regular ID is stored as its 32 bytes, so a leaf is the blobs whose first 12 bits are its
/// number.
pub fn retired_ids_in_leaf(connection: &Connection, leaf: u32) -> Result<Vec<String>> {
    let ids: Vec<String> = if leaf >= LEAF_RANGES {
        connection
            .prepare(/* sqlite-large-value: allow-for-now lv-haeded61c6421 owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ &format!(
                "SELECT {} FROM retired_claim_ids WHERE typeof(id)<>'blob' ORDER BY id",
                retired_facts::identity_text_sql("id")
            ))?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    } else {
        let bound = |leaf: u32| vec![(leaf >> 4) as u8, ((leaf & 0xf) << 4) as u8];
        let from = bound(leaf);
        // Both bounds are bound, so the scan stays inside the leaf: SQLite cannot bound an index
        // range by an OR term. A blob sorts after every TEXT, so the last leaf runs to the end.
        if leaf + 1 < LEAF_RANGES {
            connection
                .prepare_cached(
                    "SELECT lower(hex(id)) FROM retired_claim_ids
                     WHERE typeof(id)='blob' AND id>=?1 AND id<?2 ORDER BY id",
                )?
                .query_map(params![from, bound(leaf + 1)], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            connection
                .prepare_cached(
                    "SELECT lower(hex(id)) FROM retired_claim_ids
                     WHERE typeof(id)='blob' AND id>=?1 ORDER BY id",
                )?
                .query_map(params![from], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        }
    };
    Ok(ids.into_iter().filter(|id| self::leaf(id) == leaf).collect())
}

/// A stored retired identity as its claim ID text.
fn retired_text(value: rusqlite::types::Value) -> Result<String> {
    Ok(match value {
        rusqlite::types::Value::Blob(bytes) => hex::encode(bytes),
        rusqlite::types::Value::Text(text) => text,
        other => anyhow::bail!("invalid retired claim ID {other:?}"),
    })
}

/// Fold one page of the retired namespace: queued changes first, then the import.
fn fold_retired_page(connection: &Connection) -> Result<ClaimRangePage> {
    let pending: Vec<(i64, rusqlite::types::Value, i64)> = connection
        .prepare_cached("SELECT change, id, sign FROM retired_range_pending ORDER BY change LIMIT ?1")?
        .query_map([PAGE], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some((through, _, _)) = pending.last() {
        let through = *through;
        let mut delta = Delta::default();
        for (_, id, sign) in pending.iter().cloned() {
            delta.change(RETIRED, &retired_text(id)?, sign);
        }
        delta.apply(connection)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h7e0e784a9c75 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM retired_range_pending WHERE change<=?1", [through])?;
        return Ok(ClaimRangePage { changes: pending.len(), done: false });
    }
    let Some(cursor): Option<rusqlite::types::Value> = connection
        .query_row("SELECT cursor FROM retired_range_import", [], |row| row.get(0))
        .optional()?
    else {
        return Ok(ClaimRangePage { changes: 0, done: true });
    };
    let ids: Vec<rusqlite::types::Value> = connection
        .prepare_cached(
            "SELECT id FROM retired_claim_ids WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT ?2",
        )?
        .query_map(params![cursor, PAGE], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut delta = Delta::default();
    for id in ids.iter().cloned() {
        delta.change(RETIRED, &retired_text(id)?, 1);
    }
    delta.apply(connection)?;
    match ids.last() {
        Some(through) if ids.len() as i64 == PAGE => {
            connection.execute(/* sqlite-large-value: allow-for-now lv-hbab7fa303eb7 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "UPDATE retired_range_import SET cursor=?1", [through])?;
        }
        _ => {
            connection.execute(/* sqlite-large-value: allow-for-now lv-ha3b0673506b4 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM retired_range_import", [])?;
        }
    }
    Ok(ClaimRangePage { changes: ids.len(), done: false })
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
            delta.change(RETAINED, id, *sign);
        }
        delta.apply(connection)?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-haef91a621227 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM claim_range_pending WHERE change<=?1", [through])?;
        return Ok(ClaimRangePage { changes: pending.len(), done: false });
    }
    let Some((cursor, bound)): Option<(i64, i64)> = connection
        .query_row("SELECT cursor, bound FROM claim_range_import", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?
    else {
        return fold_retired_page(connection);
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
        delta.change(RETAINED, id, 1);
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
    /// only fixtures write such IDs, and listing it reads every claim. A member that trims
    /// locally adds the same summary of its retired claim IDs.
    pub fn claim_range_summary_answering(
        &self,
        query: Option<&ClaimRangeQuery>,
    ) -> Result<ClaimRangeSummary> {
        let connection = self.readers.get();
        let read = connection.unchecked_transaction()?;
        let mut summary = answer(&read, RETAINED, query)?;
        if retired::authoritative(&read)? && retired_complete(&read)? {
            let query = query.and_then(|query| query.retired.as_deref());
            summary.retired = Some(Box::new(answer(&read, RETIRED, query)?));
        }
        Ok(summary)
    }

    /// Compare a peer's claim-range summary, and the answers in it, with this node's index.
    /// `None` when they cannot be compared: another protocol version, or either index still
    /// folding. Only ranges the peer holds claims
    /// in are named, since only those can hold a claim this node lacks; a claim this node has
    /// retired is never named missing. When both members trim locally their retired IDs are
    /// compared too, and the retired IDs the peer listed that this node lacked are recorded as
    /// retired here, so it never admits what the peer deleted.
    pub fn compare_claim_ranges(
        &self,
        remote: &ClaimRangeSummary,
    ) -> Result<Option<ClaimRangeComparison>> {
        if remote.version != CLAIM_RANGE_VERSION {
            return Ok(None);
        }
        let (mut comparison, retired) = {
            let connection = self.readers.get();
            let read = connection.unchecked_transaction()?;
            if !complete(&read)? {
                return Ok(None);
            }
            let Some(comparison) = compare(&read, RETAINED, remote)? else {
                return Ok(None);
            };
            let retired = match remote.retired.as_deref() {
                Some(remote) if retired::authoritative(&read)? => compare(&read, RETIRED, remote)?,
                _ => None,
            };
            (comparison, retired)
        };
        if let Some(retired) = retired {
            self.retire_learned_claims(&retired.missing)?;
            comparison.retired = Some(Box::new(retired));
        }
        Ok(Some(comparison))
    }

    /// Record claim IDs a peer retired as retired here too, while this member trims locally.
    fn retire_learned_claims(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        if retired::authoritative(&transaction)? {
            let mut insert = transaction.prepare_cached(&format!(
                /* sqlite-large-value: allow-for-now lv-h7c16e8b65c63 owner=smallclaims::store::claim_ranges TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO retired_claim_ids(id) VALUES ({})",
                retired::identity_sql("?1")
            ))?;
            for id in ids {
                insert.execute(/* sqlite-large-value: allow-for-now lv-hf13d52544a0d owner=smallclaims::store::claim_ranges TODO=trace-statement-and-value-representation */ [id])?;
            }
        }
        transaction.commit()?;
        Ok(())
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
        // A claim this member retired is not served, even while a copy is still held.
        let authoritative = retired::authoritative(&connection)?;
        let mut found = BTreeSet::new();
        for id in claims.iter().take(CLAIM_RANGE_QUERY_CLAIMS) {
            if authoritative && retired::claim_retired(&connection, id)? {
                continue;
            }
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

/// One namespace's IDs in a leaf range.
fn ids_in(connection: &Connection, namespace: i64, leaf: u32) -> Result<Vec<String>> {
    if namespace == RETIRED { retired_ids_in_leaf(connection, leaf) } else { ids_in_leaf(connection, leaf) }
}

fn namespace_complete(connection: &Connection, namespace: i64) -> Result<bool> {
    if namespace == RETIRED { retired_complete(connection) } else { complete(connection) }
}

/// One namespace's summary, answering `query`, or only the version while its index folds.
fn answer(read: &Connection, namespace: i64, query: Option<&ClaimRangeQuery>) -> Result<ClaimRangeSummary> {
    let mut summary = ClaimRangeSummary { version: CLAIM_RANGE_VERSION, ..Default::default() };
    if !namespace_complete(read, namespace)? {
        return Ok(summary);
    }
    let (count, digest) = whole(read, namespace)?;
    summary.retained = Some(ClaimRangeDigest { count, digest: hex::encode(digest) });
    let Some(query) = query else {
        return Ok(summary);
    };
    if query.tops {
        summary.tops = Some(tops(read, namespace)?.iter().map(entry).collect());
    }
    for &top in query.leaves_of.iter().take(CLAIM_RANGE_QUERY_TOPS) {
        if top <= TOP_RANGES {
            let leaves = leaves(read, namespace, top)?.iter().map(entry).collect();
            summary.leaves.push(ClaimRangeLeaves { top, leaves });
        }
    }
    let mut listed = 0;
    for &leaf in query.ids_of.iter().take(CLAIM_RANGE_QUERY_LEAVES) {
        if listed >= CLAIM_RANGE_ANSWER_IDS {
            break;
        }
        if leaf < LEAF_RANGES {
            let ids = ids_in(read, namespace, leaf)?;
            listed += ids.len();
            summary.ids.push(ClaimRangeIds { leaf, ids });
        }
    }
    Ok(summary)
}

/// Compare one namespace of a peer's summary with this node's index. In the retained namespace
/// a claim this node retired is never missing, and the index must be complete. The retired
/// namespace is compared while it folds too: recording what a walk finds queues a fold, and the
/// walk must not stall behind it. Its digests then cover a subset of `retired_claim_ids`, never
/// more (but for the judging when it became the authority), so a range that matches the peer's
/// holds everything the peer's does, and one that does not is only listed; the IDs it lacks
/// are read from the table itself.
fn compare(read: &Connection, namespace: i64, remote: &ClaimRangeSummary) -> Result<Option<ClaimRangeComparison>> {
    let Some(theirs) = remote.retained.as_ref() else {
        return Ok(None);
    };
    if namespace == RETAINED && !complete(read)? {
        return Ok(None);
    }
    let (count, digest) = whole(read, namespace)?;
    let ours = hex::encode(digest);
    let mut comparison =
        ClaimRangeComparison { pair: format!("{ours}/{}", theirs.digest), ..Default::default() };
    if count == theirs.count && ours == theirs.digest {
        comparison.same = true;
        return Ok(Some(comparison));
    }
    if let Some(remote_tops) = &remote.tops {
        comparison.tops = differing(&tops(read, namespace)?, remote_tops);
    }
    for group in remote.leaves.iter().take(CLAIM_RANGE_QUERY_TOPS) {
        if group.top <= TOP_RANGES {
            comparison.leaves.extend(differing(&leaves(read, namespace, group.top)?, &group.leaves));
        }
    }
    let mut examined = 0;
    for group in remote.ids.iter().take(CLAIM_RANGE_QUERY_LEAVES) {
        if group.leaf >= LEAF_RANGES || examined >= CLAIM_RANGE_ANSWER_IDS {
            continue;
        }
        let local: BTreeSet<String> = ids_in(read, namespace, group.leaf)?.into_iter().collect();
        for id in &group.ids {
            examined += 1;
            if leaf(id) == group.leaf
                && !local.contains(id)
                && (namespace == RETIRED || !retired::claim_retired(read, id)?)
            {
                comparison.missing.push(id.clone());
            }
        }
    }
    Ok(Some(comparison))
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
                     id TEXT NOT NULL UNIQUE);
                 CREATE TABLE retired_claim_ids(id BLOB NOT NULL PRIMARY KEY) WITHOUT ROWID;",
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
        recomputed_from(connection, "SELECT id FROM claims", RETAINED)
    }

    fn retire(connection: &Connection, ids: impl IntoIterator<Item = String>) {
        let transaction = connection.unchecked_transaction().unwrap();
        for id in ids {
            transaction
                .execute(&format!("INSERT INTO retired_claim_ids(id) VALUES({})", retired::identity_sql("?1")), [id])
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    fn recomputed_from(connection: &Connection, query: &str, namespace: i64) -> Vec<(i64, u32, u64, [u8; 64])> {
        let ids: Vec<String> = connection
            .prepare(query)
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
                projection_digest::add(sum, &member_hash(namespace, &id), false);
            }
        }
        ranges.into_iter().map(|((level, range), (count, sum))| (level, range, count, sum)).collect()
    }

    fn maintained(connection: &Connection) -> Vec<(i64, u32, u64, [u8; 64])> {
        maintained_in(connection, RETAINED)
    }

    fn maintained_in(connection: &Connection, namespace: i64) -> Vec<(i64, u32, u64, [u8; 64])> {
        let mut ranges = Vec::new();
        for top in tops(connection, namespace).unwrap() {
            for leaf in leaves(connection, namespace, top.range).unwrap() {
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
        assert_eq!(whole(&forward, RETAINED).unwrap().0, 6001);
        // A trim deletes some claims; the digests follow, and emptied ranges disappear.
        forward
            .execute("DELETE FROM claims WHERE store_index%3=0 OR id='fixture-claim'", [])
            .unwrap();
        fold_all(&forward);
        assert_eq!(maintained(&forward), recomputed(&forward));
        assert_ne!(whole(&forward, RETAINED).unwrap(), whole(&reverse, RETAINED).unwrap());
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
    fn retired_ids_have_their_own_digests_by_their_claim_id_text() {
        let connection = fixture();
        let retired_query = format!("SELECT {} FROM retired_claim_ids", retired_facts::identity_text_sql("id"));
        // Before the triggers: joined by the import, in key order.
        retire(&connection, (0..6000).map(id).chain(["fixture-retired".to_string()]));
        install(&connection).unwrap();
        install(&connection).unwrap();
        assert!(!retired_complete(&connection).unwrap());
        // Retained claims fold first, then the retired namespace page by page.
        let mut pages = 0;
        while !retired_complete(&connection).unwrap() {
            assert!(!fold_page(&connection).unwrap().done);
            pages += 1;
            if pages == 1 {
                // On both sides of the cursor while the import runs: each counted once.
                let mut sorted: Vec<String> = (0..6000).map(id).collect();
                sorted.sort();
                retire(&connection, (6000..6100).map(id));
                connection
                    .execute(
                        &format!("DELETE FROM retired_claim_ids WHERE id IN ({}, {})", retired::identity_sql("?1"), retired::identity_sql("?2")),
                        params![sorted[0], sorted[5999]],
                    )
                    .unwrap();
            }
            assert!(pages < 100);
        }
        assert!(pages > 2, "the import is paged");
        assert!(fold_page(&connection).unwrap().done);
        assert_eq!(maintained_in(&connection, RETIRED), recomputed_from(&connection, &retired_query, RETIRED));
        assert_eq!(whole(&connection, RETIRED).unwrap().0, 6000 + 100 - 2 + 1);
        assert_eq!(whole(&connection, RETAINED).unwrap().0, 0, "the namespaces stay apart");
        // After the import every change arrives by trigger.
        retire(&connection, ["another-fixture".to_string(), id(7000)]);
        fold_all(&connection);
        assert_eq!(maintained_in(&connection, RETIRED), recomputed_from(&connection, &retired_query, RETIRED));

        // A leaf lists its members as claim ID text, including the first and last leaves.
        let all: Vec<String> = recomputed_ids(&connection, &retired_query);
        for leaf in [0, super::leaf(&id(7)), LEAF_RANGES - 1] {
            let mut members: Vec<String> = all.iter().filter(|id| super::leaf(id) == leaf).cloned().collect();
            members.sort();
            assert_eq!(retired_ids_in_leaf(&connection, leaf).unwrap(), members, "leaf {leaf}");
        }
        assert_eq!(
            retired_ids_in_leaf(&connection, LEAF_RANGES).unwrap(),
            ["another-fixture".to_string(), "fixture-retired".to_string()]
        );
    }

    fn recomputed_ids(connection: &Connection, query: &str) -> Vec<String> {
        connection
            .prepare(query)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
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
        assert_eq!(whole(&connection, RETAINED).unwrap().0, 20);
    }
}
