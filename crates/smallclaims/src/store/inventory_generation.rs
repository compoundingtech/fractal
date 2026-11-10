//! Mutation markers distinguish append-only inventory growth from changes to its prefix.
use super::*;

const KEY: &str = "replication_inventory_generation";

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    // Appends are found by rowid. A deletion, replacement of an identity, or insertion below
    // the existing tail invalidates that shortcut. Tombstones also change payload availability.
    for (table, event, condition) in [
        (
            "replica_envelopes",
            "INSERT",
            "WHEN NEW.rowid < (SELECT MAX(rowid) FROM replica_envelopes)",
        ),
        (
            "replica_envelopes",
            "UPDATE",
            "WHEN OLD.rowid IS NOT NEW.rowid OR OLD.writer IS NOT NEW.writer OR OLD.sequence IS NOT NEW.sequence OR OLD.envelope_hash IS NOT NEW.envelope_hash",
        ),
        ("checkpoint_envelopes", "DELETE", ""),
        ("checkpoint_envelopes", "INSERT", ""),
        (
            "checkpoint_envelopes",
            "UPDATE",
            "WHEN OLD.writer IS NOT NEW.writer OR OLD.sequence IS NOT NEW.sequence OR OLD.envelope_hash IS NOT NEW.envelope_hash",
        ),
        // Where the retired ledger is the authority, the inventory lists its identities.
        ("retired_envelopes", "DELETE", ""),
    ] {
        create(connection, &format!("{table}_inventory_{event}"), table, event, condition)?;
    }
    // A trim deletes the payloads of envelopes its checkpoint lists, and on a member whose
    // retired ledger is the authority it retires them first. The inventory lists those
    // identities either way, payload or not, so neither changes what it lists: only the payload
    // a peer could be sent, which export already skips once it is gone. The first versions
    // counted every such row, so every chunk of a trim rebuilt the replication snapshot from
    // nothing. An older build recreates them beside these: it counts every row again, as it did.
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hd0fb382d248c owner=smallclaims::store::inventory_generation TODO=chunk-or-reference-exact-wire-values */ "DROP TRIGGER IF EXISTS replica_envelopes_inventory_DELETE;
         DROP TRIGGER IF EXISTS retired_envelopes_inventory_INSERT;",
    )?;
    let authoritative = format!("EXISTS(SELECT 1 FROM meta WHERE key='{}' AND value='1')", super::retired::AUTHORITY);
    let old_retired = super::retired::identity_sql("OLD.envelope_hash");
    let new_text = super::retired_facts::identity_text_sql("NEW.envelope_hash");
    create(
        connection,
        "replica_envelopes_inventory_DELETE_v2",
        "replica_envelopes",
        "DELETE",
        &format!(
            "WHEN NOT EXISTS(SELECT 1 FROM checkpoint_envelopes AS listed
                 WHERE listed.writer=OLD.writer AND listed.sequence=OLD.sequence AND listed.envelope_hash=OLD.envelope_hash)
             AND NOT ({authoritative} AND EXISTS(SELECT 1 FROM retired_envelopes AS retired
                 WHERE retired.writer=OLD.writer AND retired.sequence=OLD.sequence AND retired.envelope_hash={old_retired}))"
        ),
    )?;
    // Envelope rows have no AUTOINCREMENT, so deleting the newest row lets the next insert reuse
    // its rowid. A snapshot finds additions by `rowid > max_envelope_rowid` and would never list
    // that envelope, so deleting the tail counts even when the identity stays listed. A trim
    // rarely deletes the newest row: only an old envelope a peer sent late sits there.
    create(
        connection,
        "replica_envelopes_inventory_DELETE_TAIL",
        "replica_envelopes",
        "DELETE",
        "WHEN OLD.rowid > COALESCE((SELECT MAX(rowid) FROM replica_envelopes), 0)",
    )?;
    create(
        connection,
        "retired_envelopes_inventory_INSERT_v2",
        "retired_envelopes",
        "INSERT",
        &format!(
            "WHEN NOT EXISTS(SELECT 1 FROM replica_envelopes AS held
                 WHERE held.writer=NEW.writer AND held.sequence=NEW.sequence AND held.envelope_hash={new_text})
             AND NOT EXISTS(SELECT 1 FROM checkpoint_envelopes AS listed
                 WHERE listed.writer=NEW.writer AND listed.sequence=NEW.sequence AND listed.envelope_hash={new_text})"
        ),
    )
}

fn create(connection: &Connection, name: &str, table: &str, event: &str, condition: &str) -> Result<()> {
    connection.execute_batch(&format!(
        /* sqlite-large-value: allow-for-now lv-h2d51ac92d293 owner=smallclaims::store::inventory_generation TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER IF NOT EXISTS {name} AFTER {event} ON {table}
         {condition} BEGIN
             INSERT INTO meta(key,value) VALUES('{KEY}','1')
             ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1;
         END;"
    ))?;
    Ok(())
}

/// Mark the inventory changed for a reason no table trigger sees.
pub(super) fn bump(connection: &Connection) -> Result<()> {
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-ha017db4fd2e3 owner=smallclaims::store::inventory_generation TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,'1')
         ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1",
        [KEY],
    )?;
    Ok(())
}

pub(super) fn current(connection: &Connection) -> Result<i64> {
    connection
        .prepare_cached(
            "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1),0)",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-hd60ba3ca89ff owner=smallclaims::store::inventory_generation TODO=trace-statement-and-value-representation */ [KEY], |row| row.get(0))
        .map_err(Into::into)
}
