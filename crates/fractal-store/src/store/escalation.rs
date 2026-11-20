//! Rebuildable, incrementally maintained escalation clocks. Native receipts remain authority.
use super::*;

#[derive(Clone, Debug)]
pub struct EscalationEpisode {
    pub key: String,
    pub since: u128,
}

// One row per clock-bearing receipt, ordered by the shared canonical key. Inserting an
// out-of-order status changes only its own transition bit and the next status's bit.
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-h6232d25a79e0 owner=fractal_store::store::escalation TODO=chunk-or-reference-variable-columns */ "
CREATE TABLE IF NOT EXISTS local_escalation_clocks_v1 (
 store_index INTEGER PRIMARY KEY, subject TEXT NOT NULL, ordering BLOB NOT NULL,
 claim_id TEXT NOT NULL, state TEXT, at TEXT NOT NULL, pause_until TEXT,
 progress INTEGER NOT NULL, clock INTEGER NOT NULL, transition INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS local_escalation_status_v1 ON local_escalation_clocks_v1(subject,ordering) WHERE state IS NOT NULL;
CREATE INDEX IF NOT EXISTS local_escalation_entry_v1 ON local_escalation_clocks_v1(subject,ordering) WHERE transition=1;
CREATE INDEX IF NOT EXISTS local_escalation_progress_v1 ON local_escalation_clocks_v1(subject,ordering) WHERE progress=1;
CREATE INDEX IF NOT EXISTS local_escalation_clock_v1 ON local_escalation_clocks_v1(subject,ordering) WHERE clock=1;
CREATE TABLE IF NOT EXISTS local_escalation_pending_v1(store_index INTEGER PRIMARY KEY);
CREATE TRIGGER IF NOT EXISTS local_escalation_insert_v1 AFTER INSERT ON claims
WHEN (NEW.subject LIKE 'step-run/%' OR NEW.subject LIKE 'mission-run/%')
 AND (json_type(NEW.body,'$.fields.status')='text' OR NEW.kind='work.progress') BEGIN
 INSERT OR IGNORE INTO local_escalation_pending_v1 VALUES(NEW.store_index); END;
CREATE TRIGGER IF NOT EXISTS local_escalation_change_v1 AFTER UPDATE ON claims BEGIN
 INSERT OR IGNORE INTO local_escalation_pending_v1 SELECT NEW.store_index
 WHERE EXISTS(SELECT 1 FROM local_escalation_clocks_v1 WHERE store_index=OLD.store_index)
 OR ((NEW.subject LIKE 'step-run/%' OR NEW.subject LIKE 'mission-run/%')
 AND (json_type(NEW.body,'$.fields.status')='text' OR NEW.kind='work.progress')); END;
CREATE TRIGGER IF NOT EXISTS local_escalation_delete_v1 AFTER DELETE ON claims BEGIN
 INSERT OR IGNORE INTO local_escalation_pending_v1 SELECT OLD.store_index
 WHERE EXISTS(SELECT 1 FROM local_escalation_clocks_v1 WHERE store_index=OLD.store_index); END;
CREATE TRIGGER IF NOT EXISTS local_escalation_position_insert_v1 AFTER INSERT ON claim_positions BEGIN
 INSERT OR IGNORE INTO local_escalation_pending_v1 SELECT store_index FROM claims WHERE id=NEW.claim_id; END;
CREATE TRIGGER IF NOT EXISTS local_escalation_position_v1 AFTER UPDATE ON claim_positions
WHEN OLD.position<>NEW.position BEGIN
 INSERT OR IGNORE INTO local_escalation_pending_v1 SELECT store_index FROM claims WHERE id=NEW.claim_id; END;
";

pub(super) fn open(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h7bfb5c22361f owner=fractal_store::store::escalation TODO=trace-statement-and-value-representation */ SCHEMA)?;
    let initialized: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key='local_escalation_clocks_v1')", [], |row| row.get(0))?;
    if !initialized {
        rebuild(tx)?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-hc63df415703a owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES('local_escalation_clocks_v1','ready')", [])?;
    }
    flush(tx)
}

pub(super) fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-h10cb6e9549ef owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_escalation_clocks_v1", [])?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h6ba9d399312a owner=fractal_store::store::escalation TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO local_escalation_pending_v1 SELECT store_index FROM claims
        WHERE (subject LIKE 'step-run/%' OR subject LIKE 'mission-run/%')
        AND (json_type(body,'$.fields.status')='text' OR kind='work.progress')", [])?;
    flush(tx)
}

fn normalized_state<'a>(subject: &str, state: &'a str) -> &'a str {
    if subject.starts_with("step-run/") {
        crate::client_values::client_work_state(state)
    } else {
        match state { "standing" => "running", "blocked" => "waiting", other => other }
    }
}

fn repair_transition(tx: &Transaction<'_>, subject: &str, ordering: &[u8], include: bool) -> Result<()> {
    let query = if include {
        "SELECT store_index,ordering,state FROM local_escalation_clocks_v1
         WHERE subject=?1 AND state IS NOT NULL AND ordering>=?2 ORDER BY ordering LIMIT 1"
    } else {
        "SELECT store_index,ordering,state FROM local_escalation_clocks_v1
         WHERE subject=?1 AND state IS NOT NULL AND ordering>?2 ORDER BY ordering LIMIT 1"
    };
    let next: Option<(u64, Vec<u8>, String)> = tx.query_row(/* sqlite-large-value: allow-for-now lv-h68289383f438 owner=fractal_store::store::escalation TODO=trace-statement-and-value-representation */ query,
        params![subject, ordering], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    if let Some((index, key, state)) = next {
        let previous: Option<String> = tx.query_row("SELECT state FROM local_escalation_clocks_v1
            WHERE subject=?1 AND state IS NOT NULL AND ordering<?2 ORDER BY ordering DESC LIMIT 1",
            params![subject, key], |row| row.get(0)).optional()?;
        tx.execute(/* sqlite-large-value: allow-for-now lv-h26eb4a7344bf owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "UPDATE local_escalation_clocks_v1 SET transition=?2 WHERE store_index=?1",
            params![index, previous.as_deref() != Some(state.as_str())])?;
    }
    Ok(())
}

pub(super) fn flush(tx: &Transaction<'_>) -> Result<()> {
    loop {
        let indexes = tx.prepare_cached("SELECT store_index FROM local_escalation_pending_v1 ORDER BY store_index LIMIT 256")?
            .query_map([], |row| row.get::<_, u64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if indexes.is_empty() { return Ok(()); }
        for index in indexes {
            let old: Option<(String, Vec<u8>)> = tx.query_row("SELECT subject,ordering FROM local_escalation_clocks_v1 WHERE store_index=?1",
                [index], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
            tx.execute(/* sqlite-large-value: allow-for-now lv-h1789bc082c84 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_escalation_clocks_v1 WHERE store_index=?1", [index])?;
            if let Some((subject, key)) = old { repair_transition(tx, &subject, &key, false)?; }
            let claim = tx.query_row("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
                FROM claims WHERE store_index=?1", [index], claim_from_row).optional()?;
            if let Some(claim) = claim {
                let fields = &claim.body["fields"];
                let state = fields["status"].as_str().map(|state| normalized_state(&claim.subject, state));
                let progress = claim.kind == "work.progress";
                if (claim.subject.starts_with("step-run/") || claim.subject.starts_with("mission-run/")) && (state.is_some() || progress) {
                    let key = canonical::sortable_key(&canonical::claim_key(tx, &claim.id)?);
                    let pause = (progress && fields["clock"] == "paused").then(|| fields["wait_until_unix_ms"].as_u64()).flatten();
                    tx.execute(/* sqlite-large-value: allow-for-now lv-h322fd68b9411 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_escalation_clocks_v1(store_index,subject,ordering,claim_id,state,at,pause_until,progress,clock)
                        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![index, claim.subject, key, claim.id, state,
                        claim.accepted_at_unix_ms.to_string(), pause.map(|at| at.to_string()), progress, progress && fields["clock"].is_string()])?;
                    repair_transition(tx, &claim.subject, &key, true)?;
                    repair_transition(tx, &claim.subject, &key, false)?;
                }
            }
            tx.execute(/* sqlite-large-value: allow-for-now lv-h09dc9b16a789 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_escalation_pending_v1 WHERE store_index=?1", [index])?;
        }
    }
}

const ENTRY: &str = "SELECT claim_id,at,ordering,state FROM local_escalation_clocks_v1
    WHERE subject=?1 AND transition=1 ORDER BY ordering DESC LIMIT 1";
const PROGRESS: &str = "SELECT claim_id,at FROM local_escalation_clocks_v1
    WHERE subject=?1 AND progress=1 AND ordering>=?2 ORDER BY ordering DESC LIMIT 1";
const CLOCK: &str = "SELECT pause_until FROM local_escalation_clocks_v1
    WHERE subject=?1 AND clock=1 AND ordering>=?2 ORDER BY ordering DESC LIMIT 1";

impl Store {
    /// At most three indexed seeks, independent of history length. A read snapshot
    /// keeps entry and progress in the same episode even while another writer appends.
    pub fn escalation_episode(&self, subject: &str, state: &str, created: u128,
        progress: bool) -> Result<EscalationEpisode> {
        smallclaims::touched::note_read(|| subject.to_owned());
        let connection = self.readers.get();
        let tx = connection.unchecked_transaction()?;
        let entry: Option<(String, String, Vec<u8>, String)> = tx.query_row(/* sqlite-large-value: allow-for-now lv-hf2c216a372bd owner=fractal_store::store::escalation TODO=trace-statement-and-value-representation */ ENTRY, [subject],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?;
        let (mut episode, ordering, current) = match entry {
            Some((key, at, ordering, current)) => (EscalationEpisode { key, since: at.parse()? }, ordering, current),
            None => (EscalationEpisode { key: format!("created:{subject}"), since: created }, Vec::new(), String::new()),
        };
        if progress && current == state {
            let receipt: Option<(String, String)> = tx.query_row(/* sqlite-large-value: allow-for-now lv-he1933fe52e3b owner=fractal_store::store::escalation TODO=trace-statement-and-value-representation */ PROGRESS,
                params![subject, ordering], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
            if let Some((key, at)) = receipt {
                let at: u128 = at.parse()?;
                // Ordinary progress during a declared wait does not resume the clock.
                // Retain its deadline after automatic expiry, so the episode stays stable.
                let pause: Option<String> = tx.query_row(/* sqlite-large-value: allow-for-now lv-h5899ee723526 owner=fractal_store::store::escalation TODO=trace-statement-and-value-representation */ CLOCK,
                    params![subject, ordering], |row| row.get(0)).optional()?.flatten();
                episode = EscalationEpisode { key, since: at.max(pause.map(|pause| pause.parse::<u128>()).transpose()?.unwrap_or(at)) };
            }
        }
        Ok(episode)
    }

    // This restart index keeps terminal ladders scheduled after the active run roster drops
    // them. It contains no delivery authority: native message/ask receipts own deduplication.
    pub fn retain_escalation_run(&self, run: &str, origin: &str, keep: bool) -> Result<()> {
        let connection = self.connection.write();
        if keep {
            connection.execute(/* sqlite-large-value: allow-for-now lv-h0003f1ad06a3 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_escalation_runs_v1(run_id,origin) VALUES (?1,?2)", params![run, origin])?;
        } else {
            connection.execute(/* sqlite-large-value: allow-for-now lv-ha65878244411 owner=fractal_store::store::escalation TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_escalation_runs_v1 WHERE run_id=?1", [run])?;
        }
        Ok(())
    }

    pub fn escalation_run_ids(&self, origin: &str) -> Result<Vec<String>> {
        let connection = self.readers.get();
        let mut query = connection.prepare_cached("SELECT run_id FROM local_escalation_runs_v1 WHERE origin=?1 ORDER BY run_id")?;
        Ok(query.query_map([origin], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);
            CREATE TABLE batches(id TEXT PRIMARY KEY,origin TEXT,replica_sequence INTEGER);
            CREATE TABLE claims(store_index INTEGER PRIMARY KEY,id TEXT UNIQUE,batch_id TEXT,
                subject TEXT,kind TEXT,origin TEXT,actor TEXT,body TEXT,predecessors TEXT,accepted_at_unix_ms TEXT);
            CREATE INDEX claims_batch_index ON claims(batch_id,store_index);
            CREATE TABLE claim_positions(claim_id TEXT PRIMARY KEY,position INTEGER);
            CREATE TABLE replica_records(claim_id TEXT,position INTEGER);
            INSERT INTO batches VALUES('batch','writer',1);").unwrap();
        let tx = db.transaction().unwrap();
        open(&tx).unwrap();
        tx.commit().unwrap();
        db
    }

    fn insert(tx: &Transaction<'_>, index: u64, at: u64, state: &str, progress: bool) {
        tx.execute("INSERT INTO claims VALUES(?1,?2,'batch','step-run/test',?3,'writer',NULL,?4,'[]',?5)",
            params![index, format!("claim-{index}"), if progress { "work.progress" } else { "step-run.state" },
                json!({"fields":{"status":state}}).to_string(), at.to_string()]).unwrap();
        flush(tx).unwrap();
    }

    fn entry(tx: &Transaction<'_>) -> String {
        tx.query_row(ENTRY, ["step-run/test"], |row| row.get(0)).unwrap()
    }

    #[test]
    fn long_histories_use_bounded_seeks_and_same_state_renewals_keep_entry() {
        let mut db = database();
        let tx = db.transaction().unwrap();
        insert(&tx, 1, 1, "ready", false);
        for index in 2..=3000 { insert(&tx, index, index, "ready", true); }
        assert_eq!(entry(&tx), "claim-1");
        let latest: String = tx.query_row(PROGRESS, params!["step-run/test", Vec::<u8>::new()], |row| row.get(0)).unwrap();
        assert_eq!(latest, "claim-3000");
        for (query, index) in [(ENTRY, "local_escalation_entry_v1"), (PROGRESS, "local_escalation_progress_v1"), (CLOCK, "local_escalation_clock_v1")] {
            let mut statement = tx.prepare(&format!("EXPLAIN QUERY PLAN {query}")).unwrap();
            let args = vec![None::<String>; statement.parameter_count()];
            let plan = statement.query_map(rusqlite::params_from_iter(args), |row| row.get::<_, String>(3)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("; ");
            assert!(plan.contains(index), "{plan}");
            assert!(!plan.contains("SCAN") && !plan.contains("TEMP B-TREE"), "{plan}");
            assert!(query.ends_with("LIMIT 1"));
        }
    }

    #[test]
    fn out_of_order_status_insertion_correction_and_removal_repair_only_neighbors() {
        let mut db = database();
        let tx = db.transaction().unwrap();
        insert(&tx, 1, 10, "ready", false);
        insert(&tx, 2, 30, "ready", false);
        insert(&tx, 3, 20, "working", false);
        assert_eq!(entry(&tx), "claim-2");
        tx.execute("UPDATE claims SET body=?1 WHERE store_index=3", [json!({"fields":{"status":"ready"}}).to_string()]).unwrap();
        flush(&tx).unwrap();
        assert_eq!(entry(&tx), "claim-1");
        tx.execute("UPDATE claims SET body=?1 WHERE store_index=3", [json!({"fields":{"status":"working"}}).to_string()]).unwrap();
        flush(&tx).unwrap();
        assert_eq!(entry(&tx), "claim-2");
        tx.execute("DELETE FROM claims WHERE store_index=3", []).unwrap();
        flush(&tx).unwrap();
        assert_eq!(entry(&tx), "claim-1");
        rebuild(&tx).unwrap();
        assert_eq!(entry(&tx), "claim-1");
    }

    #[test]
    fn late_canonical_positions_reorder_tied_receipts_without_replaying_history() {
        let mut db = database();
        let tx = db.transaction().unwrap();
        insert(&tx, 1, 10, "ready", false);
        insert(&tx, 2, 10, "working", false);
        assert_eq!(entry(&tx), "claim-2");
        tx.execute("INSERT INTO claim_positions VALUES('claim-1',10)", []).unwrap();
        flush(&tx).unwrap();
        assert_eq!(entry(&tx), "claim-1");
        tx.execute("UPDATE claim_positions SET position=0 WHERE claim_id='claim-1'", []).unwrap();
        flush(&tx).unwrap();
        assert_eq!(entry(&tx), "claim-2");
    }
}

