//! One total order for shared claim folds. A claim's wire position is stored at admission in
//! `claim_positions`; arrival indexes recover only the relative position of legacy batches that
//! predate replica_records and claims admitted before positions were stored.
use super::*;

/// A claim's position in its batch is immutable input data. Admission stores it; for a claim
/// stored before that, the triggers here store the value the legacy expression gives just before
/// a removal could change it: deleting an earlier sibling renumbers the arrival count, and
/// deleting a wire record can raise the least record position. A later record of the same claim
/// only lowers the value, as the least position over its records always has, so the stored value
/// is a join and its order of arrival does not matter. Rows outlive their claim: the position of
/// a retired claim is part of its identity, not of the claim body that a trim removes.
pub const POSITION_SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-ha962eb0285cf owner=smallclaims::store::canonical TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS claim_positions (
    claim_id TEXT PRIMARY KEY,
    position INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS claim_positions_record_insert_v1 AFTER INSERT ON replica_records
WHEN NEW.claim_id IS NOT NULL BEGIN
    INSERT INTO claim_positions(claim_id, position)
    SELECT NEW.claim_id, MIN(position) FROM replica_records WHERE claim_id=NEW.claim_id
    ON CONFLICT(claim_id) DO UPDATE SET position=MIN(position, excluded.position);
END;
CREATE TRIGGER IF NOT EXISTS claim_positions_record_change_v1
BEFORE UPDATE OF claim_id, position ON replica_records
WHEN OLD.claim_id IS NOT NULL AND (OLD.claim_id IS NOT NEW.claim_id OR OLD.position<>NEW.position)
    AND NOT EXISTS (SELECT 1 FROM claim_positions WHERE claim_id=OLD.claim_id)
BEGIN
    INSERT OR IGNORE INTO claim_positions(claim_id, position)
    SELECT OLD.claim_id, MIN(position) FROM replica_records WHERE claim_id=OLD.claim_id;
END;
CREATE TRIGGER IF NOT EXISTS claim_positions_record_changed_v1
AFTER UPDATE OF claim_id, position ON replica_records
WHEN NEW.claim_id IS NOT NULL AND (OLD.claim_id IS NOT NEW.claim_id OR OLD.position<>NEW.position)
BEGIN
    INSERT INTO claim_positions(claim_id, position)
    SELECT NEW.claim_id, MIN(position) FROM replica_records WHERE claim_id=NEW.claim_id
    ON CONFLICT(claim_id) DO UPDATE SET position=MIN(position, excluded.position);
END;
CREATE TRIGGER IF NOT EXISTS claim_positions_record_delete_v1 BEFORE DELETE ON replica_records
WHEN OLD.claim_id IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM claim_positions WHERE claim_id=OLD.claim_id)
BEGIN
    INSERT OR IGNORE INTO claim_positions(claim_id, position)
    SELECT OLD.claim_id, MIN(position) FROM replica_records WHERE claim_id=OLD.claim_id;
END;
CREATE TRIGGER IF NOT EXISTS claim_positions_sibling_delete_v1 BEFORE DELETE ON claims
WHEN EXISTS (
    SELECT 1 FROM claims sibling INDEXED BY claims_batch_index
    WHERE sibling.batch_id=OLD.batch_id AND sibling.store_index>OLD.store_index
      AND NOT EXISTS (SELECT 1 FROM claim_positions WHERE claim_id=sibling.id)
) BEGIN
    INSERT OR IGNORE INTO claim_positions(claim_id, position)
    SELECT id, COALESCE((SELECT MIN(position) FROM replica_records WHERE claim_id=sibling.id), arrival)
    FROM (
        SELECT id, store_index, ROW_NUMBER() OVER (ORDER BY store_index) - 1 AS arrival
        FROM claims INDEXED BY claims_batch_index WHERE batch_id=OLD.batch_id
    ) sibling
    WHERE sibling.store_index>OLD.store_index;
END;
";

/// A checkpoint proof copy orders its sealed set by the legacy expression over that set alone,
/// as every member holding the set computes it. Stored positions join every record a member
/// holds, including duplicate envelopes outside the set, so the copy must not read them.
/// Run on the copy before it deletes anything.
pub const PROOF_COPY_LEGACY_POSITIONS: &str = /* sqlite-large-value: allow-for-now lv-hd2150e929695 owner=smallclaims::store::canonical TODO=bound-or-reference-values-and-reclamation */ "
DROP TRIGGER IF EXISTS claim_positions_record_insert_v1;
DROP TRIGGER IF EXISTS claim_positions_record_change_v1;
DROP TRIGGER IF EXISTS claim_positions_record_changed_v1;
DROP TRIGGER IF EXISTS claim_positions_record_delete_v1;
DROP TRIGGER IF EXISTS claim_positions_sibling_delete_v1;
DELETE FROM claim_positions;
";

/// Store a replicated claim's wire position before its claim row, so the claim's own insert
/// triggers already order it by the position its record will carry.
pub fn record_position_tx(transaction: &Connection, claim_id: &str, position: u64) -> Result<()> {
    transaction
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h5cee4d10869d owner=smallclaims::store::canonical TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO claim_positions(claim_id, position) VALUES (?1, ?2)
             ON CONFLICT(claim_id) DO UPDATE SET position=MIN(position, excluded.position)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-h71a77b24fa37 owner=smallclaims::store::canonical TODO=trace-statement-and-value-representation */ params![claim_id, position])?;
    Ok(())
}

/// Store a local claim's position: the claims its batch already holds, which is where sealing
/// will place its record.
pub fn record_local_position_tx(
    transaction: &Connection,
    claim_id: &str,
    batch_id: &str,
) -> Result<()> {
    transaction
        .prepare_cached(
            /* sqlite-large-value: allow-for-now lv-h61b680cabd70 owner=smallclaims::store::canonical TODO=prepare-body-and-predecessor-values */ "INSERT INTO claim_positions(claim_id, position)
             SELECT ?1, COUNT(*) FROM claims INDEXED BY claims_batch_index WHERE batch_id=?2
             ON CONFLICT(claim_id) DO UPDATE SET position=MIN(position, excluded.position)",
        )?
        .execute(/* sqlite-large-value: allow-for-now lv-hfab2c82ae927 owner=smallclaims::store::canonical TODO=trace-statement-and-value-representation */ params![claim_id, batch_id])?;
    Ok(())
}

pub struct CanonicalOrder(pub bool);

impl std::fmt::Display for CanonicalOrder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&order_sql("claims", self.0))
    }
}

pub const CANONICAL_ORDER: CanonicalOrder = CanonicalOrder(false);
pub const CANONICAL_ORDER_DESC: CanonicalOrder = CanonicalOrder(true);

pub fn components(alias: &str) -> Vec<String> {
    vec![
        format!("length({alias}.accepted_at_unix_ms)"),
        format!("{alias}.accepted_at_unix_ms"),
        format!("(SELECT origin FROM batches WHERE id={alias}.batch_id)"),
        format!("(SELECT replica_sequence FROM batches WHERE id={alias}.batch_id)"),
        format!("{alias}.batch_id"),
        format!(
            "COALESCE((SELECT position FROM claim_positions WHERE claim_id={alias}.id), \
            (SELECT MIN(position) FROM replica_records WHERE claim_id={alias}.id), \
            (SELECT COUNT(*) FROM claims legacy_position WHERE legacy_position.batch_id={alias}.batch_id \
            AND legacy_position.store_index<{alias}.store_index))"
        ),
        format!("{alias}.id"),
    ]
}

pub fn order_sql(alias: &str, descending: bool) -> String {
    let direction = if descending { " DESC" } else { "" };
    components(alias)
        .into_iter()
        .map(|part| format!("{part}{direction}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn after_sql(left: &str, right: &str) -> String {
    format!(
        "({}) > ({})",
        components(left).join(", "),
        components(right).join(", ")
    )
}

pub fn position_sql(alias: &str) -> String {
    components(alias).remove(5)
}

pub fn key_from_record(
    claim: &ClaimRecord,
    writer: String,
    sequence: u64,
    position: u64,
) -> ClaimKey {
    (
        claim.accepted_at_unix_ms,
        writer,
        sequence,
        claim.batch_id.clone(),
        position,
        claim.id.clone(),
    )
}

/// Expand explicit canonical ordering markers without introducing joins or ambiguous column
/// names in the caller's SQL. Aliases are fixed source identifiers, never user input.
pub fn canonical_sql(query: &str) -> String {
    let mut query = query.to_owned();
    for alias in ["claims", "request", "created", "sent", "resolution"] {
        for (marker, descending) in [("CANONICAL_ASC", false), ("CANONICAL_DESC", true)] {
            let marker = format!("{marker}({alias})");
            if query.contains(&marker) {
                query = query.replace(&marker, &order_sql(alias, descending));
            }
        }
    }
    query
}

pub type ClaimKey = (u128, String, u64, String, u64, String);

/// SQLite BLOB ordering equivalent to ClaimKey's total order. Fixed-width numbers sort
/// numerically; escaped zeroes and a double-zero terminator preserve string prefix ordering.
pub fn sortable_key(key: &ClaimKey) -> Vec<u8> {
    fn string(output: &mut Vec<u8>, value: &str) {
        for byte in value.bytes() {
            output.push(byte);
            if byte == 0 {
                output.push(255);
            }
        }
        output.extend_from_slice(&[0, 0]);
    }
    let mut output = key.0.to_be_bytes().to_vec();
    string(&mut output, &key.1);
    output.extend_from_slice(&key.2.to_be_bytes());
    string(&mut output, &key.3);
    output.extend_from_slice(&key.4.to_be_bytes());
    string(&mut output, &key.5);
    output
}

pub fn claim_key(connection: &Connection, id: &str) -> Result<ClaimKey> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let position = position_sql("claims");
        format!("SELECT claims.accepted_at_unix_ms, batches.origin, batches.replica_sequence,
            claims.batch_id, {position}, claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
            WHERE claims.id=?1")
    });
    let (time, writer, sequence, batch, position, id) =
        connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h548175136201 owner=smallclaims::store::canonical TODO=trace-statement-and-value-representation */ &QUERY)?.query_row(/* sqlite-large-value: allow-for-now lv-h6f4cb2726891 owner=smallclaims::store::canonical TODO=trace-statement-and-value-representation */ [id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
    Ok((time.parse()?, writer, sequence, batch, position, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: &str = "CREATE TABLE batches(id TEXT, origin TEXT, replica_sequence INTEGER);
         CREATE TABLE claims(id TEXT, batch_id TEXT, accepted_at_unix_ms TEXT, store_index INTEGER);
         CREATE INDEX claims_batch_index ON claims(batch_id, store_index);
         CREATE TABLE replica_records(claim_id TEXT, position INTEGER);
         CREATE INDEX replica_records_claim ON replica_records(claim_id, position);";

    fn keys(connection: &Connection, ids: &[&str]) -> Vec<ClaimKey> {
        ids.iter().map(|id| claim_key(connection, id).unwrap()).collect()
    }

    #[test]
    fn claim_keys_preserve_numeric_time_and_record_or_legacy_position_order() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(TABLES).unwrap();
        connection.execute_batch(POSITION_SCHEMA).unwrap();
        connection.execute_batch(
            "INSERT INTO batches VALUES ('batch', 'writer', 2);
             INSERT INTO claims VALUES ('a', 'batch', '10', 3), ('b', 'batch', '9', 2), ('c', 'batch', '10', 1);
             INSERT INTO replica_records VALUES ('b', 8), ('b', 4);"
        ).unwrap();
        let mut keys = keys(&connection, &["a", "b", "c"]);
        keys.sort();
        assert_eq!(
            keys,
            vec![
                (9, "writer".into(), 2, "batch".into(), 4, "b".into()),
                (10, "writer".into(), 2, "batch".into(), 0, "c".into()),
                (10, "writer".into(), 2, "batch".into(), 2, "a".into()),
            ]
        );
    }

    /// Claims stored before positions were: `local` has no wire record, so its position is
    /// its arrival count; `wire` has two records of one claim.
    fn legacy_store(triggers: bool) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(TABLES).unwrap();
        connection.execute_batch(
            "INSERT INTO batches VALUES ('local', 'writer', 1), ('wire', 'peer', 1);
             INSERT INTO claims VALUES ('a', 'local', '10', 1), ('b', 'local', '10', 2),
                 ('c', 'local', '10', 3), ('d', 'wire', '10', 4), ('e', 'wire', '10', 5);
             INSERT INTO replica_records VALUES ('d', 8), ('d', 4), ('e', 5);",
        ).unwrap();
        if triggers {
            connection.execute_batch(POSITION_SCHEMA).unwrap();
        } else {
            connection
                .execute_batch("CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY, position INTEGER NOT NULL) WITHOUT ROWID;")
                .unwrap();
        }
        connection
    }

    fn remove_earlier_inputs(connection: &Connection) {
        connection
            .execute_batch(
                "DELETE FROM claims WHERE id='a';
                 DELETE FROM replica_records WHERE claim_id='d' AND position=4;",
            )
            .unwrap();
    }

    #[test]
    fn removing_an_earlier_sibling_or_record_never_renumbers_a_survivor() {
        let survivors = ["b", "c", "d", "e"];
        let connection = legacy_store(true);
        let before = keys(&connection, &survivors);
        assert_eq!(
            before.iter().map(|key| key.4).collect::<Vec<_>>(),
            [1, 2, 4, 5]
        );
        remove_earlier_inputs(&connection);
        assert_eq!(keys(&connection, &survivors), before);
        // Removing the survivors' own inputs later changes nothing either.
        connection
            .execute_batch("DELETE FROM claims WHERE id='b'; DELETE FROM replica_records;")
            .unwrap();
        assert_eq!(keys(&connection, &["c", "d", "e"]), before[1..]);

        // Negative control: without the stored positions the same removals renumber.
        let control = legacy_store(false);
        remove_earlier_inputs(&control);
        assert_eq!(
            keys(&control, &survivors)
                .iter()
                .map(|key| key.4)
                .collect::<Vec<_>>(),
            [0, 1, 8, 5]
        );
    }

    #[test]
    fn stored_positions_join_records_in_any_order() {
        // Admission's own position, then two later records of the same claim, in each order.
        let steps = [
            "admitted",
            "INSERT INTO replica_records VALUES ('x', 3)",
            "INSERT INTO replica_records VALUES ('x', 7)",
        ];
        let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        for order in orders {
            let connection = Connection::open_in_memory().unwrap();
            connection.execute_batch(TABLES).unwrap();
            connection.execute_batch(POSITION_SCHEMA).unwrap();
            for step in order.map(|index| steps[index]) {
                if step == "admitted" {
                    record_position_tx(&connection, "x", 5).unwrap();
                } else {
                    connection.execute_batch(step).unwrap();
                }
            }
            let position: u64 = connection
                .query_row(
                    "SELECT position FROM claim_positions WHERE claim_id='x'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(position, 3, "{order:?}");
        }
    }

    #[test]
    fn a_local_claim_takes_the_position_sealing_gives_it() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(TABLES).unwrap();
        connection.execute_batch(POSITION_SCHEMA).unwrap();
        connection
            .execute_batch("INSERT INTO batches VALUES ('local', 'writer', 1);")
            .unwrap();
        for (index, id) in ["a", "b", "c"].into_iter().enumerate() {
            record_local_position_tx(&connection, id, "local").unwrap();
            connection
                .execute(
                    "INSERT INTO claims VALUES (?1, 'local', '10', ?2)",
                    params![id, index as i64 + 1],
                )
                .unwrap();
        }
        let positions = keys(&connection, &["a", "b", "c"])
            .iter()
            .map(|key| key.4)
            .collect::<Vec<_>>();
        assert_eq!(positions, [0, 1, 2]);
        // Sealing records the same positions; deleting the first claim renumbers nothing.
        connection
            .execute_batch(
                "INSERT INTO replica_records VALUES ('a', 0), ('b', 1), ('c', 2);
                 DELETE FROM claims WHERE id='a';",
            )
            .unwrap();
        assert_eq!(
            keys(&connection, &["b", "c"])
                .iter()
                .map(|key| key.4)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }
}
