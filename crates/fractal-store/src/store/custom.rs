//! Registered custom projections. The signed claim log is authority; these tables are caches.
use super::*;
use serde::{Deserialize, Serialize};
use fractal_schema::custom::{self as schema, Authority, Basis, Expr, Manifest, Predicate, Select};

/// Whole-history admission limits: decision snapshots are refused, never truncated.
pub const DECISION_INPUTS_MAX_CLAIMS: u64 = 10_000;
pub const DECISION_INPUTS_MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;
/// Work budget includes repaired rows, which do not consume the effective history limits.
/// Excessive repaired history is refused with the same typed error, never silently skipped.
const DECISION_INPUTS_MAX_SCANNED_CLAIMS: u64 = 2 * DECISION_INPUTS_MAX_CLAIMS;

// Explicit UPSERTs in triggers keep an outer admission UPSERT from overriding IGNORE.
const SCHEMA: &str = /* sqlite-large-value: allow-for-now lv-hd9bc2fa21a4c owner=fractal_store::store::custom TODO=chunk-or-reference-variable-columns */ r#"
CREATE TABLE IF NOT EXISTS custom_registrations (
 subject TEXT PRIMARY KEY, kind TEXT NOT NULL, version INTEGER NOT NULL,
 prefix TEXT NOT NULL, hash TEXT NOT NULL, manifest TEXT NOT NULL,
 claim_id TEXT NOT NULL, state TEXT NOT NULL, error TEXT
);
CREATE INDEX IF NOT EXISTS custom_registrations_kind ON custom_registrations(kind,version);
CREATE TABLE IF NOT EXISTS custom_sources (
 subject TEXT PRIMARY KEY, registration TEXT NOT NULL, kind TEXT NOT NULL,
 revision TEXT NOT NULL, state TEXT NOT NULL, body TEXT NOT NULL,
 person TEXT, episode TEXT, active INTEGER NOT NULL DEFAULT 0,
 requested_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS custom_sources_kind ON custom_sources(kind,subject);
CREATE INDEX IF NOT EXISTS custom_sources_kind_version ON custom_sources(kind,json_extract(body,'$.schema_version'),subject);
CREATE INDEX IF NOT EXISTS custom_sources_version ON custom_sources(json_extract(body,'$.schema_version'),subject);
CREATE INDEX IF NOT EXISTS custom_sources_attention ON custom_sources(person,subject) WHERE active=1;
CREATE TABLE IF NOT EXISTS custom_dependencies (
 source TEXT NOT NULL, dependency TEXT NOT NULL, kinds TEXT NOT NULL, revision TEXT NOT NULL,
 PRIMARY KEY(source,dependency,kinds)
);
CREATE INDEX IF NOT EXISTS custom_dependencies_input ON custom_dependencies(dependency,source);
CREATE TABLE IF NOT EXISTS local_custom_dirty (subject TEXT PRIMARY KEY);
-- Conflict updates intentionally wake the connection-local work notifier even when
-- another connection already queued the subject; DO NOTHING would lose that hint.
-- Map canonical-order changes to everything flush can consume, not the whole log.
-- Registry subjects trigger prefix expansion; dependencies and document bindings
-- must reach their consumers even when the changed claim is not custom/.
-- Every use names the claim IDs, so each branch must look its claims up by ID. The unary
-- plus keeps the planner from walking every doc.bound claim through claims_kind_index.
CREATE VIEW IF NOT EXISTS custom_claim_invalidations AS
 SELECT id,subject FROM claims
 WHERE (subject LIKE 'custom/%' AND subject NOT LIKE 'custom/client/%')
 OR EXISTS(SELECT 1 FROM custom_dependencies WHERE source=claims.subject)
 UNION ALL
 SELECT claims.id,custom_dependencies.source FROM claims JOIN custom_dependencies
 ON custom_dependencies.dependency IN (claims.subject,claims.id)
 UNION ALL
 SELECT binding.id,registration.subject FROM claims AS binding JOIN claims AS registration
 ON registration.kind='custom.st3-kinds.registered'
 AND substr(json_extract(registration.body,'$.fields.document'),1,length(binding.subject)+1)=binding.subject||'@'
 WHERE +binding.kind='doc.bound';
CREATE TRIGGER IF NOT EXISTS custom_document_insert AFTER INSERT ON documents BEGIN
 INSERT INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency=NEW.name ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM claims WHERE kind='custom.st3-kinds.registered' AND json_extract(body,'$.fields.document')=NEW.name||'@'||NEW.hash ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_document_delete AFTER DELETE ON documents BEGIN
 INSERT INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency=OLD.name ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM claims WHERE kind='custom.st3-kinds.registered' AND json_extract(body,'$.fields.document')=OLD.name||'@'||OLD.hash ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_document_update AFTER UPDATE OF name,hash ON documents
WHEN OLD.name IS NOT NEW.name OR OLD.hash IS NOT NEW.hash BEGIN
 INSERT INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency IN (OLD.name,NEW.name) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM claims WHERE kind='custom.st3-kinds.registered' AND json_extract(body,'$.fields.document') IN (OLD.name||'@'||OLD.hash,NEW.name||'@'||NEW.hash) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_claim_insert AFTER INSERT ON claims BEGIN
 INSERT INTO local_custom_dirty SELECT NEW.subject WHERE NEW.subject LIKE 'custom/%' AND NEW.subject NOT LIKE 'custom/client/%' ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE id IN (SELECT id FROM claims WHERE batch_id=NEW.batch_id AND store_index>NEW.store_index AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id)) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency IN (NEW.subject,NEW.id) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM claims WHERE NEW.kind='doc.bound' AND kind='custom.st3-kinds.registered' AND substr(json_extract(body,'$.fields.document'),1,length(NEW.subject)+1)=NEW.subject||'@' ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_claim_delete AFTER DELETE ON claims BEGIN
 INSERT INTO local_custom_dirty SELECT OLD.subject WHERE OLD.subject LIKE 'custom/%' AND OLD.subject NOT LIKE 'custom/client/%' ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE id IN (SELECT id FROM claims WHERE batch_id=OLD.batch_id AND store_index>OLD.store_index AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id)) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency IN (OLD.subject,OLD.id) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_claim_update AFTER UPDATE ON claims
WHEN OLD.id IS NOT NEW.id OR OLD.subject IS NOT NEW.subject OR OLD.kind IS NOT NEW.kind
 OR OLD.body IS NOT NEW.body OR OLD.actor IS NOT NEW.actor OR OLD.origin IS NOT NEW.origin
 OR OLD.predecessors IS NOT NEW.predecessors OR OLD.accepted_at_unix_ms IS NOT NEW.accepted_at_unix_ms
 OR OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index BEGIN
 INSERT INTO local_custom_dirty SELECT OLD.subject WHERE OLD.subject LIKE 'custom/%' AND OLD.subject NOT LIKE 'custom/client/%' ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT NEW.subject WHERE NEW.subject LIKE 'custom/%' AND NEW.subject NOT LIKE 'custom/client/%' ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency IN (OLD.subject,NEW.subject,OLD.id,NEW.id) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE (OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index) AND id IN (SELECT id FROM claims WHERE ((batch_id=OLD.batch_id AND store_index>OLD.store_index) OR (batch_id=NEW.batch_id AND store_index>NEW.store_index)) AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id)) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_batch_update AFTER UPDATE OF origin,replica_sequence ON batches
WHEN OLD.origin IS NOT NEW.origin OR OLD.replica_sequence IS NOT NEW.replica_sequence BEGIN
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE id IN (SELECT id FROM claims WHERE batch_id=NEW.id) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_record_insert AFTER INSERT ON replica_records WHEN NEW.claim_id IS NOT NULL BEGIN
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE id=NEW.claim_id ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_record_update AFTER UPDATE OF position,claim_id,state ON replica_records
WHEN OLD.position IS NOT NEW.position OR OLD.claim_id IS NOT NEW.claim_id OR OLD.state IS NOT NEW.state BEGIN
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE id IN (OLD.claim_id,NEW.claim_id) ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
CREATE TRIGGER IF NOT EXISTS custom_record_delete AFTER DELETE ON replica_records BEGIN
 INSERT INTO local_custom_dirty SELECT subject FROM custom_claim_invalidations WHERE id=OLD.claim_id ON CONFLICT(subject) DO UPDATE SET subject=excluded.subject;
END;
"#;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationRequest {
    pub manifest: Manifest,
    pub actor: String,
}

/// Complete, authority-validated decision inputs from one database snapshot.
/// Vector position is canonical interpretation order; store indexes are arrival cursors only.
#[derive(Debug, Deserialize, Serialize)]
pub struct DecisionInputs {
    pub claims: Vec<ClaimRecord>,
    pub basis: Value,
}
pub fn typed(e: fractal_schema::ValidationError) -> FractalError {
    FractalError::new(e.code, e.message)
}
pub fn fail(message: impl Into<String>) -> FractalError {
    FractalError::new("invalid-custom-claim", message)
}
pub fn fields(claim: &ClaimRecord) -> BTreeMap<String, Value> {
    claim
        .body
        .get("fields")
        .and_then(Value::as_object)
        .map(|v| v.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}
pub fn claims(connection: &Connection, subject: &str) -> Result<Vec<ClaimRecord>> {
    connection.prepare(/* sqlite-large-value: allow-for-now lv-hd5f90ba66291 owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ &canonical_sql(
        "SELECT claims.id,claims.store_index,claims.batch_id,claims.subject,claims.kind,claims.origin,claims.actor,claims.body,claims.predecessors,claims.accepted_at_unix_ms
         FROM claims JOIN batches ON batches.id=claims.batch_id WHERE claims.subject=?1
         AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
         ORDER BY CANONICAL_ASC(claims)"))?
        .query_map([subject],claim_from_row)?.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn decision_inputs_preflight(
    connection: &Connection,
    subject: &str,
    max_claims: u64,
    max_body_bytes: u64,
) -> Result<()> {
    // No aggregate or sort: SQLite yields each subject row before visiting the next.
    // Keep repaired rows in the cursor so even a repaired-only history has bounded work.
    // CASE avoids measuring excluded bodies. Canonical ordering is applied only after
    // admission, by claims(), in the caller's same read transaction.
    let mut statement = connection.prepare(
        "SELECT CASE WHEN NOT EXISTS(
             SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired'
         ) THEN length(CAST(claims.body AS BLOB)) END
         FROM claims CROSS JOIN batches ON batches.id=claims.batch_id
         WHERE claims.subject=?1 LIMIT ?2",
    )?;
    let mut rows = statement.query(params![subject, DECISION_INPUTS_MAX_SCANNED_CLAIMS + 1])?;
    let mut scanned = 0;
    let mut claim_count = 0;
    let mut body_bytes = 0_u64;
    while let Some(row) = rows.next()? {
        scanned += 1;
        let bytes: Option<u64> = row.get(0)?;
        if let Some(bytes) = bytes {
            claim_count += 1;
            body_bytes = body_bytes.saturating_add(bytes);
        }
        if scanned > DECISION_INPUTS_MAX_SCANNED_CLAIMS
            || claim_count > max_claims || body_bytes > max_body_bytes
        {
            return Err(FractalError::new(
                "decision-inputs-too-large",
                format!(
                    "decision history exceeds its admission budget after scanning {scanned} claims \
                     ({claim_count} effective claims and {body_bytes} body bytes so far); \
                     limits are {max_claims} effective claims, {max_body_bytes} body bytes, \
                     and {DECISION_INPUTS_MAX_SCANNED_CLAIMS} scanned claims including repaired history"
                ),
            ).into());
        }
    }
    Ok(())
}
pub fn registration(
    connection: &Connection,
    subject: &str,
) -> Result<Option<(String, Manifest, String)>> {
    let row: Option<(String,String,String)> = connection.query_row(
        "SELECT hash,manifest,state FROM custom_registrations WHERE substr(?1,1,length(prefix))=prefix ORDER BY subject LIMIT 1",[subject],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    row.map(|(hash, m, state)| Ok((hash, serde_json::from_str(&m)?, state)))
        .transpose()
}
pub fn create_schema(connection: &Connection) -> Result<()> {
    // Upgrade installed definitions once without changing the schema cookie on reopen.
    for (kind, name, end) in [
        ("VIEW", "custom_claim_invalidations", ";"),
        ("TRIGGER", "custom_document_insert", "\nEND;"),
        ("TRIGGER", "custom_document_delete", "\nEND;"),
        ("TRIGGER", "custom_document_update", "\nEND;"),
        ("TRIGGER", "custom_claim_insert", "\nEND;"),
        ("TRIGGER", "custom_claim_delete", "\nEND;"),
        ("TRIGGER", "custom_claim_update", "\nEND;"),
        ("TRIGGER", "custom_batch_update", "\nEND;"),
        ("TRIGGER", "custom_record_insert", "\nEND;"),
        ("TRIGGER", "custom_record_update", "\nEND;"),
        ("TRIGGER", "custom_record_delete", "\nEND;"),
    ] {
        let marker = format!("CREATE {kind} IF NOT EXISTS {name}");
        let start = SCHEMA.find(&marker).context("missing custom schema definition")?;
        let length = SCHEMA[start..].find(end).context("unterminated custom schema definition")?;
        let expected = SCHEMA[start..start + length + end.len()].replace("IF NOT EXISTS ", "");
        let installed: Option<String> = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type=lower(?1) AND name=?2",
            [kind, name], |row| row.get(0),
        ).optional()?;
        let tokens = |sql: &str| sql.trim().trim_end_matches(';').split_whitespace().collect::<Vec<_>>().join(" ");
        if installed.as_deref().is_some_and(|sql| tokens(sql) != tokens(&expected)) {
            connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h50daecb5c0e9 owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ &format!("DROP {kind} {name}"))?;
        }
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h3f6849b90a68 owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ SCHEMA)?;
    notice_authority::create_invalidation_triggers(connection)
}
pub fn open(tx: &Transaction<'_>) -> Result<()> {
    capture_work(tx)?;
    if tx
        .query_row(
            "SELECT value FROM meta WHERE key='custom_projections_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .as_deref()
        != Some("2")
    {
        rebuild(tx)
    } else {
        flush_pending(tx)
    }
}
pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(/* sqlite-large-value: allow-for-now lv-hcc772813cbfc owner=fractal_store::store::custom TODO=prepare-body-and-predecessor-values */ "DELETE FROM custom_registrations; DELETE FROM custom_sources; DELETE FROM custom_dependencies; DELETE FROM local_custom_dirty; INSERT INTO local_custom_dirty SELECT DISTINCT subject FROM claims WHERE subject LIKE 'custom/%' AND subject NOT LIKE 'custom/client/%';")?;
    flush_pending(tx)?;
    tx.execute(
        /* sqlite-large-value: allow-for-now lv-ha275af346170 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('custom_projections_version','2')",
        [],
    )?;
    Ok(())
}
pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    if !transaction_work::custom_pending(tx) { return Ok(()); }
    flush_pending(tx)
}

// The preparation fence uses these same exact catalog definitions. Keep notifier
// installation and eligibility together: a trusted name alone never admits SQL.
pub(super) fn work_notifier_definitions() -> &'static BTreeMap<String, String> {
    static DEFINITIONS: std::sync::LazyLock<BTreeMap<String, String>> = std::sync::LazyLock::new(|| {
        let mut definitions = BTreeMap::new();
        for table in ["local_custom_dirty", "custom_registrations", "custom_sources",
            "custom_dependencies", "identity_names", "identity_heads"] {
            for action in ["INSERT", "UPDATE", "DELETE"] {
                let name = format!("custom_work_{table}_{action}");
                definitions.insert(name.clone(), format!(
                    /* sqlite-large-value: allow-for-now lv-hb43fa7777c18 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */
                    "CREATE TRIGGER {name} AFTER {action} ON main.{table} BEGIN SELECT custom_work_pending(); END"
                ));
            }
        }
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let rows = match action { "INSERT" => &["NEW"][..], "DELETE" => &["OLD"][..], _ => &["OLD", "NEW"][..] };
            let relevant = rows.iter().map(|row| format!(
            "({row}.kind_hint IN ('agent.renamed','mission.renamed','agent.retired','intent.desired','mission.published')
              OR {row}.kind_hint LIKE 'custom.%' OR {row}.kind_hint IS NULL)"
            )).collect::<Vec<_>>().join(" OR ");
            let name = format!("custom_work_lifecycle_{action}");
            definitions.insert(name.clone(), format!(
                /* sqlite-large-value: allow-for-now lv-hc38ba81818b8 owner=fractal_store::store::custom TODO=chunk-or-reference-exact-wire-values */
                "CREATE TRIGGER {name} AFTER {action} ON main.replica_records WHEN {relevant} BEGIN SELECT custom_work_pending(); END"
            ));
        }
        definitions
    });
    &DEFINITIONS
}

fn capture_work(connection: &Connection) -> Result<()> {
    transaction_work::install(connection)?;
    // Lease renewals do not change declaration liveness or wake this finalizer.
    for definition in work_notifier_definitions().values() {
        let body = definition.strip_prefix(/* sqlite-large-value: allow-for-now lv-h248251de0d26 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER ").expect("native notifier definition");
        connection.execute_batch(&format!(/* sqlite-large-value: allow-for-now lv-h4711b40b27f6 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "CREATE TEMP TRIGGER IF NOT EXISTS {body}"))?;
    }
    Ok(())
}

fn flush_pending(tx: &Transaction<'_>) -> Result<()> {
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM local_custom_dirty)", [], |r| {
        r.get::<_, bool>(0)
    })? {
        return Ok(());
    }
    let registry_dirty: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM local_custom_dirty WHERE subject LIKE 'custom/st3-kinds/%')",
        [],
        |r| r.get(0),
    )?;
    if registry_dirty {
        let before = registration_rows(tx)?;
        refresh_registrations(tx)?;
        let after = registration_rows(tx)?;
        let changed = before
            .iter()
            .chain(&after)
            .filter(|(id, _)| before.get(*id) != after.get(*id))
            .map(|(_, (prefix, _))| prefix.clone())
            .collect::<BTreeSet<_>>();
        for prefix in changed {
            tx.execute(/* sqlite-large-value: allow-for-now lv-h75147b2df5b4 owner=fractal_store::store::custom TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO local_custom_dirty SELECT DISTINCT subject FROM claims WHERE subject>=?1 AND subject<?2",params![prefix,prefix_end(&prefix)])?;
        }
    }
    // Registry refresh can add source subjects to the dirty set. Follow their mandatory
    // consumers only after prefix expansion, including on registration/document removal.
    tx.execute(/* sqlite-large-value: allow-for-now lv-h234719a19887 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_custom_dirty SELECT source FROM custom_dependencies WHERE dependency IN (SELECT subject FROM local_custom_dirty)", [])?;
    let dirty=tx.prepare("SELECT subject FROM local_custom_dirty WHERE subject LIKE 'custom/%' AND subject NOT LIKE 'custom/client/%' AND subject NOT LIKE 'custom/st3-kinds/%' ORDER BY subject")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    // Build dependency edges before checking cycles, so arrival order cannot choose health.
    for _ in 0..2 {
        for subject in &dirty {
            tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h8053f53278fe owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ "SAVEPOINT custom_source")?;
            let result = refresh_source(tx, subject);
            if result.is_err() {
                tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h4718566161fa owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ "ROLLBACK TO custom_source; RELEASE custom_source")?;
                result?;
            } else {
                tx.execute_batch(/* sqlite-large-value: allow-for-now lv-h54522921c58d owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ "RELEASE custom_source")?;
            }
        }
    }
    tx.execute(/* sqlite-large-value: allow-for-now lv-h31356a8318b1 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM local_custom_dirty", [])?;
    Ok(())
}
fn prefix_end(prefix: &str) -> String {
    // Validated prefixes end in '/', whose ASCII successor bounds the subject index.
    format!("{}0", prefix.strip_suffix('/').unwrap_or(prefix))
}
fn registration_rows(connection: &Connection) -> Result<BTreeMap<String, (String, Value)>> {
    connection
        .prepare(
            "SELECT subject,prefix,hash,manifest,claim_id,state,error FROM custom_registrations",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, String>(1)?,
                    json!([
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?
                    ]),
                ),
            ))
        })?
        .collect::<rusqlite::Result<_>>()
        .map_err(Into::into)
}
fn refresh_registrations(tx: &Transaction<'_>) -> Result<()> {
    tx.execute(/* sqlite-large-value: allow-for-now lv-hd0a55e8fc5ef owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM custom_registrations", [])?;
    let subjects=tx.prepare("SELECT DISTINCT subject FROM claims WHERE kind='custom.st3-kinds.registered' ORDER BY subject")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for subject in subjects {
        let records = claims(tx, &subject)?;
        let mut registrations = BTreeMap::new();
        for c in records.iter().filter(|c| c.kind == schema::REGISTERED) {
            if !c
                .actor
                .as_deref()
                .is_some_and(|a| a.starts_with("agent/") || a.starts_with("person/"))
            {
                continue;
            }
            let f = fields(c);
            let Some(reference) = f.get("document").and_then(Value::as_str) else {
                continue;
            };
            let Ok((name, hash)) = split_document_ref(reference) else {
                continue;
            };
            let bytes:Option<Vec<u8>>=tx.query_row("SELECT b.bytes FROM documents d JOIN blobs b ON b.hash=d.hash WHERE d.name=?1 AND d.hash=?2",params![name,hash],|r|r.get(0)).optional()?;
            let Some(bytes) = bytes else { continue };
            if bytes.len() > schema::MAX_BYTES || hex::encode(Sha256::digest(&bytes)) != hash {
                continue;
            }
            let Ok(m) = serde_json::from_slice::<Manifest>(&bytes) else {
                continue;
            };
            if m.validate().is_err() || m.subject() != subject {
                continue;
            }
            if fractal_schema::notice::validate_registration(&m, hash).is_err() {
                continue;
            }
            registrations.entry(hash.to_owned()).or_insert((m, c));
        }
        if let Some((hash, (m, c))) = registrations.first_key_value() {
            let state = if registrations.len() == 1 {
                "ready"
            } else {
                "conflict"
            };
            tx.execute(
                /* sqlite-large-value: allow-for-now lv-hf1c863d6a2ad owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO custom_registrations VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    subject,
                    m.kind,
                    m.version,
                    m.subject_prefix,
                    hash,
                    canonical_serialized_json_text(m)?,
                    c.id,
                    state,
                    if state == "conflict" {
                        Some("immutable registration has competing hashes")
                    } else {
                        None
                    }
                ],
            )?;
        }
    }
    // Overlapping replicated registrations cannot silently capture each other's subjects.
    tx.execute(/* sqlite-large-value: allow-for-now lv-hd1a5520c907a owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "UPDATE custom_registrations SET state='conflict',error='overlapping registration prefixes' WHERE EXISTS(SELECT 1 FROM custom_registrations other WHERE other.subject<>custom_registrations.subject AND (substr(other.prefix,1,length(custom_registrations.prefix))=custom_registrations.prefix OR substr(custom_registrations.prefix,1,length(other.prefix))=other.prefix))",[])?;
    Ok(())
}

/// Local writes use this before appending. Replicated facts are validated again in the fold,
/// including facts received before the immutable manifest or their creation claim.
pub fn prepare(
    tx: &Transaction<'_>,
    origin: &str,
    input: &ClaimInput,
) -> Result<Option<BTreeMap<String, Value>>, FractalError> {
    if input.subject.starts_with(schema::REGISTRY_PREFIX) || input.kind == schema::REGISTERED {
        return Err(FractalError::new(
            "custom-registration-only",
            "use schema register to publish a registration",
        ));
    }
    let reserved = fractal_schema::notice::is_reserved_subject(&input.subject)
        || fractal_schema::notice::is_reserved_kind(&input.kind);
    if reserved && !fractal_schema::notice::is_reserved_subject(&input.subject) {
        return Err(fail("notices unavailable: reserved claim kind outside notice prefix"));
    }
    if !input.subject.starts_with("custom/") {
        return Ok(None);
    }
    flush(tx).map_err(internal)?;
    let Some((hash, m, state)) = registration(tx, &input.subject).map_err(internal)? else {
        if reserved {
            return Err(fail("notices unavailable: pinned registration is not available"));
        }
        if input.fields.contains_key("_registration") {
            return Err(fail("registration is not available"));
        }
        return Ok(None);
    };
    if state != "ready" {
        return Err(fail("registration is conflicting"));
    }
    let mut f = input.fields.clone();
    if f.get("_registration")
        .is_some_and(|v| v.as_str() != Some(&hash))
    {
        return Err(fail("claim pins another registration"));
    }
    f.insert("_registration".into(), json!(hash));
    let history = claims(tx, &input.subject).map_err(internal)?;
    let creation = history.iter().find(|c| {
        c.kind == m.creation_kind && fields(c).get("_registration") == Some(&json!(hash))
    });
    if input.subject.starts_with("custom/attention/source/") && creation.is_some() {
        return Err(fail("notices unavailable: producer input is immutable"));
    }
    let position = if input.subject.starts_with(fractal_schema::notice::SNAPSHOT_PREFIX) {
        Some(notice_authority::Position::next(tx, origin).map_err(internal)?)
    } else { None };
    validate_fact(tx, &input.subject, &m, &input.kind, input.actor.as_deref(), &f, creation, position)
        .map_err(|e| fail(e.to_string()))?;
    Ok(Some(f))
}
#[allow(clippy::too_many_arguments)]
fn validate_fact(
    connection: &Connection,
    subject: &str,
    m: &Manifest,
    kind: &str,
    actor: Option<&str>,
    f: &BTreeMap<String, Value>,
    creation: Option<&ClaimRecord>,
    position: Option<notice_authority::Position<'_>>,
) -> Result<()> {
    let spec = m
        .claims
        .get(kind)
        .ok_or_else(|| anyhow::anyhow!("claim kind is not registered for this subject"))?;
    schema::validate_fields(spec, f)?;
    // Only the notice runtime's snapshots are typed facts a daemon writes.
    let actor = actor
        .filter(|a| a.starts_with("agent/") || a.starts_with("person/")
            || (*a == fractal_schema::notice::RUNTIME_ACTOR && fractal_schema::notice::is_reserved_subject(subject)))
        .ok_or_else(|| anyhow::anyhow!("typed claim needs a concrete actor"))?;
    fractal_schema::registry().validate_subject(actor)?;
    notice_authority::validate(connection, subject, m, kind, actor, f, creation, position)?;
    match spec.authority {
        Authority::Creator => {
            if let Some(c) = creation {
                anyhow::ensure!(
                    c.actor.as_deref() == Some(actor),
                    "only the creator may append creation facts"
                );
            }
        }
        Authority::Owner => {
            anyhow::ensure!(
                creation.and_then(|c| c.actor.as_deref()) == Some(actor),
                "only the request owner may write this claim"
            );
        }
        Authority::Recipient => {
            let attention = m
                .attention
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("recipient is undeclared"))?;
            let recipient = creation
                .and_then(|c| {
                    c.body
                        .get("fields")
                        .and_then(|f| f.get(&attention.recipient_field))
                })
                .and_then(Value::as_str);
            anyhow::ensure!(
                recipient == Some(actor),
                "only the addressed person may write this claim"
            );
        }
    }
    for (name, spec) in &spec.fields {
        if let Some(v) = f.get(name).and_then(Value::as_str) {
            if spec.document {
                validate_documents_connection(connection, v)?;
            }
            if spec.claim {
                anyhow::ensure!(
                    claim_by_id_tx(connection, v)?.is_some(),
                    "claim reference is not available"
                );
            }
        }
    }
    if let Some(v) = f.get("_basis") {
        let basis: Vec<Basis> = serde_json::from_value(v.clone())?;
        anyhow::ensure!(
            !basis.is_empty() && basis.len() <= 256,
            "basis needs 1..256 dependencies"
        );
        for b in basis {
            fractal_schema::registry().validate_subject(&b.subject)?;
            anyhow::ensure!(
                !b.kinds.is_empty()
                    && b.kinds.len() <= 32
                    && b.kinds
                        .iter()
                        .all(|k| fractal_schema::is_custom_claim_kind(k) && k != kind)
                    && b.revision.len() == 64,
                "invalid basis filter or revision"
            );
        }
    }
    Ok(())
}
fn validate_documents_connection(connection: &Connection, reference: &str) -> Result<()> {
    let (name, hash) = split_document_ref(reference)?;
    let found:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM documents d JOIN blobs b ON b.hash=d.hash WHERE d.name=?1 AND d.hash=?2)",params![name,hash],|r|r.get(0))?;
    anyhow::ensure!(found, "document reference is not available");
    Ok(())
}
pub fn eval(expr: &Expr, slots: &BTreeMap<String, ClaimRecord>) -> Value {
    match expr {
        Expr::Field { slot, field } => slots
            .get(slot)
            .and_then(|c| c.body.get("fields"))
            .and_then(|v| v.get(field))
            .cloned()
            .unwrap_or(Value::Null),
        Expr::Actor { slot } => slots
            .get(slot)
            .and_then(|c| c.actor.clone())
            .map(Value::String)
            .unwrap_or(Value::Null),
        Expr::ClaimId { slot } => slots.get(slot).map(|c| json!(c.id)).unwrap_or(Value::Null),
        Expr::Constant { value } => value.clone(),
    }
}
fn predicate(p: &Predicate, slots: &BTreeMap<String, ClaimRecord>) -> bool {
    match p {
        Predicate::Exists { slot } => slots.contains_key(slot),
        Predicate::Eq { left, right } => {
            let a = eval(left, slots);
            let b = eval(right, slots);
            !a.is_null() && !b.is_null() && a == b
        }
        Predicate::All { args } => args.iter().all(|p| predicate(p, slots)),
        Predicate::Any { args } => args.iter().any(|p| predicate(p, slots)),
        Predicate::Not { arg } => !predicate(arg, slots),
    }
}
pub fn slots(m: &Manifest, history: &[ClaimRecord]) -> BTreeMap<String, ClaimRecord> {
    m.slots
        .iter()
        .filter_map(|(name, s)| {
            let iter = history.iter().filter(|c| c.kind == s.kind);
            let c = match s.select {
                Select::First => iter.into_iter().next(),
                Select::Last => iter.into_iter().next_back(),
            };
            c.map(|c| (name.clone(), c.clone()))
        })
        .collect()
}
fn basis_revision(connection: &Connection, subject: &str, kinds: &[String]) -> Result<String> {
    let history = claims(connection, subject)?;
    basis_revision_from_history(subject, kinds, &history)
}
fn basis_revision_from_history(
    subject: &str,
    kinds: &[String],
    history: &[ClaimRecord],
) -> Result<String> {
    let mut kinds = kinds.to_vec();
    kinds.sort();
    kinds.dedup();
    canonical_hash(&(
        subject,
        &kinds,
        history
            .iter()
            .filter(|c| kinds.contains(&c.kind))
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
    ))
}
fn cycle(connection: &Connection, source: &str, dependency: &str) -> Result<bool> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![dependency.to_owned()];
    while let Some(subject) = pending.pop() {
        if subject == source {
            return Ok(true);
        }
        if !visited.insert(subject.clone()) {
            continue;
        }
        anyhow::ensure!(visited.len() <= 256, "dependency traversal exceeds bound");
        pending.extend(
            connection
                .prepare("SELECT dependency FROM custom_dependencies WHERE source=?1")?
                .query_map([&subject], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    Ok(false)
}
fn refresh_source(tx: &Transaction<'_>, subject: &str) -> Result<()> {
    let history = claims(tx, subject)?;
    tx.execute(/* sqlite-large-value: allow-for-now lv-h5f040aa46115 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM custom_dependencies WHERE source=?1", [subject])?;
    notice_authority::record_dependencies(tx, subject, &history)?;
    let Some((hash, m, registration_state)) = registration(tx, subject)? else {
        if fractal_schema::notice::is_reserved_subject(subject) {
            let body = json!({"id":subject,"kind":"custom-subject","state":"unavailable","error":"notices unavailable: pinned registration is missing or mismatched"});
            tx.execute(/* sqlite-large-value: allow-for-now lv-h50b120964acf owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO custom_sources VALUES(?1,'','','','unavailable',?2,NULL,NULL,0,'0')",
                params![subject, canonical_json_text(&body)?])?;
            return Ok(());
        }
        if let Some(c) = history
            .iter()
            .find(|c| fields(c).contains_key("_registration"))
        {
            let hash = fields(c)["_registration"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let revision =
                canonical_hash(&history.iter().map(|c| c.id.as_str()).collect::<Vec<_>>())?;
            let body = json!({"id":subject,"kind":"custom-subject","registered_kind":"","registration":hash,"revision":revision,"state":"pending-dependencies","fields":{},"updated_at":timestamp(c.accepted_at_unix_ms)});
            tx.execute(/* sqlite-large-value: allow-for-now lv-hc048cabe68ca owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO custom_sources VALUES(?1,?2,'',?3,'pending-dependencies',?4,NULL,NULL,0,?5)",params![subject,hash,revision,canonical_json_text(&body)?,c.accepted_at_unix_ms.to_string()])?;
            return Ok(());
        }
        tx.execute(/* sqlite-large-value: allow-for-now lv-hb864ed6dba28 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM custom_sources WHERE subject=?1", [subject])?;
        return Ok(());
    };
    let pinned = history
        .iter()
        .filter(|c| fields(c).get("_registration") == Some(&json!(hash)))
        .cloned()
        .collect::<Vec<_>>();
    let creation = pinned.iter().find(|c| c.kind == m.creation_kind);
    let reserved = fractal_schema::notice::is_reserved_subject(subject);
    let mut error = if reserved && pinned.len() != history.len() {
        Some("notices unavailable: fact registration pin is missing or mismatched".to_owned())
    } else if subject.starts_with(fractal_schema::notice::SOURCE_PREFIX)
        && pinned.iter().filter(|claim| claim.kind == m.creation_kind).count() != 1 {
        Some("notices unavailable: missing or duplicate source creation".to_owned())
    } else { None };
    let mut valid = Vec::new();
    for c in &pinned {
        let position = if subject.starts_with(fractal_schema::notice::SNAPSHOT_PREFIX) {
            notice_authority::Position::written(tx, c).map(Some)
        } else { Ok(None) };
        match position.and_then(|position| validate_fact(tx, subject, &m, &c.kind, c.actor.as_deref(), &fields(c), creation, position)) {
            Ok(()) => valid.push(c.clone()),
            Err(e) => {
                error = Some(e.to_string());
            }
        }
    }
    let slots = slots(&m, &valid);
    let revision = canonical_hash(&(
        hash.as_str(),
        pinned.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
    ))?;
    let mut state = if registration_state != "ready" {
        "conflict"
    } else if error
        .as_ref()
        .is_some_and(|e| !e.contains("reference is not available"))
        && creation.is_some()
    {
        "invalid"
    } else if creation.is_none() || error.is_some() {
        "pending-dependencies"
    } else {
        "ready"
    };
    let mut basis = Vec::new();
    for c in slots.values() {
        if let Some(v) = fields(c).get("_basis")
            && let Ok(b) = serde_json::from_value::<Vec<Basis>>(v.clone())
        {
            basis.extend(b);
        }
    }
    if basis.len() > 256 {
        state = "invalid";
        error = Some("selected slots exceed 256 basis dependencies".into());
        basis.clear();
    }
    for b in &basis {
        tx.execute(
            /* sqlite-large-value: allow-for-now lv-hcae6ed773291 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO custom_dependencies VALUES(?1,?2,?3,?4)",
            params![
                subject,
                b.subject,
                canonical_serialized_json_text(&b.kinds)?,
                b.revision
            ],
        )?;
    }
    for b in &basis {
        if b.subject == subject
            && pinned
                .iter()
                .any(|c| b.kinds.contains(&c.kind) && fields(c).contains_key("_basis"))
        {
            state = "invalid";
            error = Some("same-source basis must select raw input kinds".into());
        }
        match if b.subject == subject {
            Ok(false)
        } else {
            cycle(tx, subject, &b.subject)
        } {
            Ok(false) => {}
            Ok(true) => {
                state = "invalid";
                error = Some("cyclic derived state basis".into());
            }
            Err(e) => {
                state = "invalid";
                error = Some(e.to_string());
            }
        }
        if state == "ready" && basis_revision(tx, &b.subject, &b.kinds)? != b.revision {
            state = "stale";
        }
    }
    // Document arrival retries only sources that actually reference that immutable document.
    for c in &pinned {
        let Some(specification) = m.claims.get(&c.kind) else {
            continue;
        };
        for (name, spec) in &specification.fields {
            if spec.claim
                && let Some(reference) = fields(c).get(name).and_then(Value::as_str)
            {
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-h44ecd869d0d2 owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO custom_dependencies VALUES(?1,?2,'[]','')",
                    params![subject, reference],
                )?;
            }
            if spec.document
                && let Some(reference) = fields(c).get(name).and_then(Value::as_str)
                && let Ok((name, _)) = split_document_ref(reference)
            {
                tx.execute(
                    /* sqlite-large-value: allow-for-now lv-h44ac934d417e owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO custom_dependencies VALUES(?1,?2,'[]','')",
                    params![subject, name],
                )?;
            }
        }
    }
    let mut replies: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for c in &pinned {
        if let (Some(revision), Some(digest)) = (
            c.body
                .pointer("/_custom_reply/revision")
                .and_then(Value::as_str),
            c.body
                .pointer("/_custom_reply/digest")
                .and_then(Value::as_str),
        ) {
            replies
                .entry(revision.into())
                .or_default()
                .entry(digest.into())
                .or_default()
                .push(c.id.clone());
        }
    }
    let reply_conflicts = replies
        .into_iter()
        .filter(|(_, variants)| variants.len() > 1)
        .map(|(revision, variants)| {
            let mut ids = variants.into_values().flatten().collect::<Vec<_>>();
            ids.sort();
            json!({"revision":revision,"claims":ids})
        })
        .collect::<Vec<_>>();
    if state == "ready" && !reply_conflicts.is_empty() {
        state = "conflict";
    }
    let output = m
        .fields
        .iter()
        .map(|(n, e)| (n.clone(), eval(e, &slots)))
        .collect::<BTreeMap<_, _>>();
    let mut body = json!({"kind":"custom-subject","id":subject,"registered_kind":m.kind,"schema_version":m.version,"registration":hash,"revision":revision,"state":state,"owner":creation.and_then(|c|c.actor.as_deref()),"fields":output,"provenance":slots.iter().map(|(n,c)|(n.clone(),json!({"claim_id":c.id,"actor":c.actor}))).collect::<BTreeMap<_,_>>(),"input_count":pinned.len()});
    if !reply_conflicts.is_empty() {
        body["reply_conflicts"] = json!(reply_conflicts);
    }
    if let Some(e) = error {
        body["error"] = json!(e);
    }
    let mut person = None;
    let mut episode = None;
    let mut active = false;
    if let Some(a) = &m.attention {
        person = creation
            .and_then(|c| c.body.get("fields"))
            .and_then(|f| f.get(&a.recipient_field))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let title = eval(&a.title, &slots);
        let detail = eval(&a.detail, &slots);
        let ep = eval(&a.episode, &slots);
        if state == "ready" && predicate(&a.when, &slots) {
            if !title
                .as_str()
                .is_some_and(|s| !s.is_empty() && s.len() <= 512)
                || !detail.as_str().is_some_and(|s| s.len() <= 8192)
                || !ep.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256)
                || person.is_none()
            {
                state = "invalid";
                body["state"] = json!(state);
                body["error"] = json!("attention output type/size is invalid");
            } else {
                active = true;
                episode = ep.as_str().map(str::to_owned);
            }
        }
        body["attention"] = json!({"title":title,"detail":detail,"episode":ep,"recipient":person,"active":active,"reply":a.reply});
    }
    if serde_json::to_vec(&body)?.len() > schema::MAX_BYTES {
        active = false;
        state = "invalid";
        body = json!({"kind":"custom-subject","id":subject,"registered_kind":m.kind,"schema_version":m.version,"registration":hash,"revision":revision,"state":state,"error":"projected row exceeds 64 KiB"});
    }
    let requested_at = creation.map_or(0, |c| c.accepted_at_unix_ms).to_string();
    body["updated_at"] = json!(timestamp(requested_at.parse().unwrap_or(0)));
    tx.execute(/* sqlite-large-value: allow-for-now lv-hebf7bf39d8bb owner=fractal_store::store::custom TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO custom_sources VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(subject) DO UPDATE SET registration=excluded.registration,kind=excluded.kind,revision=excluded.revision,state=excluded.state,body=excluded.body,person=excluded.person,episode=excluded.episode,active=excluded.active,requested_at=excluded.requested_at",params![subject,hash,m.kind,revision,state,canonical_json_text(&body)?,person,episode,active,requested_at])?;
    Ok(())
}

impl Store {
    pub fn register_custom_kind(&self, request: &RegistrationRequest) -> Result<Value, FractalError> {
        request.manifest.validate().map_err(typed)?;
        validate_actor(&request.actor)?;
        if !request.actor.starts_with("agent/") && !request.actor.starts_with("person/") {
            return Err(fail("registration needs a concrete actor"));
        }
        let m = &request.manifest;
        let bytes = serde_json::to_vec(m).map_err(internal)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        fractal_schema::notice::validate_registration(m, &hash).map_err(typed)?;
        let name = format!("doc/custom-kinds/{}.v{}/{}", m.kind, m.version, hash);
        self.put_document_as(
            &name,
            &bytes,
            &None,
            &format!("custom-manifest:{hash}"),
            Some(&request.actor),
        )?;
        self.connection.batched(|tx|->Result<Value,FractalError>{
            flush(tx).map_err(internal)?;
            let existing:Option<(String,String)>=tx.query_row("SELECT hash,state FROM custom_registrations WHERE subject=?1",[m.subject()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(internal)?;
            if let Some((prior,state))=existing {
                if prior==hash&&state=="ready" {return Ok(json!({"id":m.subject(),"registration":hash,"state":state}));}
                return Err(fail("kind/version is immutable; publish a new version"));
            }
            let overlapping:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM custom_registrations WHERE substr(prefix,1,length(?1))=?1 OR substr(?1,1,length(prefix))=prefix)",[&m.subject_prefix],|r|r.get(0)).map_err(internal)?;
            let upper = prefix_end(&m.subject_prefix);
            let populated:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM claims WHERE subject>=?1 AND subject<?2)",params![m.subject_prefix,upper],|r|r.get(0)).map_err(internal)?;
            if overlapping||populated {return Err(fail("subject prefix overlaps a registration or existing untyped facts"));}
            smallclaims::store::principals::rules_gate_tx(tx,&self.origin,&request.actor,schema::REGISTERED,&m.subject()).map_err(internal)?;
            append_claim_tx(tx,&self.origin,&m.subject(),schema::REGISTERED,Some(&request.actor),&json!({"fields":{"document":format!("{name}@{hash}")}}),&[],None).map_err(internal)?;
            flush(tx).map_err(internal)?;
            Ok(json!({"id":m.subject(),"registration":hash,"state":"ready"}))
        }).map_err(internal)?
    }
    pub fn custom_registrations(&self) -> Result<Vec<Value>> {
        self.readers.get().prepare("SELECT subject,hash,manifest,state,error FROM custom_registrations ORDER BY subject")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?)))?.map(|r|{let(s,h,m,state,error)=r?;Ok(json!({"id":s,"registration":h,"manifest":serde_json::from_str::<Value>(&m)?,"state":state,"error":error}))}).collect()
    }
    pub fn custom_subject(&self, subject: &str) -> Result<Option<Value>> {
        if fractal_schema::notice::is_reserved_subject(subject) {
            anyhow::bail!("notices unavailable: reserved raw notice rows require the native notice reader");
        }
        source(&self.readers.get(), subject)
    }
    /// Which of these subjects are registered custom sources, in one read.
    pub fn registered_custom_subjects(&self, subjects: &[String]) -> Result<BTreeSet<String>> {
        if subjects.is_empty() {
            return Ok(BTreeSet::new());
        }
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject, body FROM custom_sources
             WHERE subject IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = statement.query_map([serde_json::to_string(subjects)?], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut registered = BTreeSet::new();
        for row in rows {
            let (subject, body) = row?;
            // `custom_subject` reports a source whose body does not parse as unregistered.
            if serde_json::from_str::<Value>(&body).is_ok() {
                registered.insert(subject);
            }
        }
        Ok(registered)
    }
    pub fn custom_subjects(
        &self,
        kind: Option<&str>,
        version: Option<u32>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let connection = self.readers.get();
        let query = match (kind, version) {
            (Some(_), Some(_)) => {
                "SELECT subject,body FROM custom_sources WHERE kind=?1 AND json_extract(body,'$.schema_version')=?2 AND subject>?3 ORDER BY subject"
            }
            (Some(_), None) => {
                "SELECT subject,body FROM custom_sources WHERE kind=?1 AND subject>?3 ORDER BY subject"
            }
            (None, Some(_)) => {
                "SELECT subject,body FROM custom_sources WHERE json_extract(body,'$.schema_version')=?2 AND subject>?3 ORDER BY subject"
            }
            (None, None) => {
                "SELECT subject,body FROM custom_sources WHERE subject>?3 ORDER BY subject"
            }
        };
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hef0985e2aacf owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ query)?;
        let rows = statement.query_map(params![kind, version, after.unwrap_or("")], |row| {
            let subject: String = row.get(0)?;
            if fractal_schema::notice::is_reserved_subject(&subject) {
                Ok(None)
            } else {
                row.get::<_, String>(1).map(Some)
            }
        })?;
        // Apply the shared reservation predicate before the bounded page, including roots
        // and ancestors. Streaming avoids parsing reserved bodies or reading beyond the page.
        rows.filter_map(|row| row.transpose()).take(limit.min(501))
            .map(|text| Ok(serde_json::from_str(&text?)?)).collect()
    }
    pub fn custom_basis_revision(&self, subject: &str, kinds: &[String]) -> Result<String> {
        basis_revision(&self.readers.get(), subject, kinds)
    }
    /// Read the same repaired-filtered canonical inputs as the basis, never raw log pages.
    /// Invalid or unavailable authority rejects the entire read rather than granting approval.
    pub fn decision_inputs(&self, subject: &str) -> Result<DecisionInputs> {
        self.decision_inputs_with_limits(
            subject,
            DECISION_INPUTS_MAX_CLAIMS,
            DECISION_INPUTS_MAX_BODY_BYTES,
        )
    }

    fn decision_inputs_with_limits(
        &self,
        subject: &str,
        max_claims: u64,
        max_body_bytes: u64,
    ) -> Result<DecisionInputs> {
        let version = if subject.starts_with("custom/decision/tree/v1/") {
            1
        } else if subject.starts_with("custom/decision/tree/v2/") {
            2
        } else {
            anyhow::bail!("expected a decision.tree v1 or v2 subject");
        };
        let connection = self.readers.get();
        let snapshot = connection.unchecked_transaction()?;
        let (hash, manifest, state) = registration(&snapshot, subject)?
            .ok_or_else(|| anyhow::anyhow!("decision registration is unavailable"))?;
        anyhow::ensure!(
            state == "ready" && manifest.kind == "decision.tree" && manifest.version == u32::from(version),
            "decision registration is not ready or does not match the subject"
        );
        let answered = format!("custom.decision.tree.v{version}.answered");
        anyhow::ensure!(
            manifest.creation_kind == format!("custom.decision.tree.v{version}.opened")
                && manifest.attention.as_ref()
                    .is_some_and(|attention| attention.recipient_field == "recipient")
                && manifest.claims.get(&answered)
                    .is_some_and(|spec| matches!(spec.authority, Authority::Recipient)),
            "decision registration must authorize answers only from the addressed recipient"
        );
        let source_status: Option<(String, Option<String>)> = snapshot.query_row(
            "SELECT state,person FROM custom_sources WHERE subject=?1",
            [subject], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let (source_state, source_recipient) = source_status
            .ok_or_else(|| anyhow::anyhow!("decision source is unavailable"))?;
        // Stale derived status must not prevent its importer from reading fresh raw inputs.
        anyhow::ensure!(matches!(source_state.as_str(), "ready" | "stale"),
            "decision source authority is not ready");
        let source_body = source(&snapshot, subject)?
            .ok_or_else(|| anyhow::anyhow!("decision source is unavailable"))?;
        decision_inputs_preflight(&snapshot, subject, max_claims, max_body_bytes)?;
        let history = claims(&snapshot, subject)?;
        anyhow::ensure!(
            history.iter().all(|claim| fields(claim).get("_registration") == Some(&json!(hash))),
            "decision history contains mismatched registration pins"
        );
        let creation = history.iter().find(|claim| claim.kind == manifest.creation_kind)
            .ok_or_else(|| anyhow::anyhow!("decision creation is unavailable"))?;
        let creation_fields = fields(creation);
        let seat = creation_fields.get("seat").and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("decision creation has no seat"))?;
        let seat_path = seat.strip_prefix("agent/")
            .ok_or_else(|| anyhow::anyhow!("decision seat must be an agent"))?;
        anyhow::ensure!(
            creation.actor.as_deref() == Some(seat)
                && (subject == format!("custom/decision/tree/v{version}/{seat_path}")
                    || (version == 2 && subject.strip_prefix("custom/decision/tree/v2/legacy/")
                        .is_some_and(|spelling| !spelling.is_empty()))),
            "decision creation actor, seat, and subject identity must match"
        );
        let recipient = creation_fields.get("recipient").and_then(Value::as_str)
            .filter(|recipient| recipient.starts_with("person/"))
            .ok_or_else(|| anyhow::anyhow!("decision recipient must be a person"))?;
        fractal_schema::registry().validate_subject(recipient)?;
        anyhow::ensure!(
            source_recipient.as_deref() == Some(recipient)
                && source_body.pointer("/fields/recipient").and_then(Value::as_str) == Some(recipient),
            "decision source recipient must match the creation recipient"
        );
        for claim in &history {
            validate_fact(&snapshot, subject, &manifest, &claim.kind,
                claim.actor.as_deref(), &fields(claim), Some(creation), None)?;
        }
        let kinds = schema::decisions::input_kinds(version);
        let revision = basis_revision_from_history(subject, &kinds, &history)?;
        let claims = history.into_iter().filter(|claim| kinds.contains(&claim.kind)).collect();
        snapshot.commit()?;
        Ok(DecisionInputs { claims, basis: json!({"subject":subject,"kinds":kinds,"revision":revision}) })
    }
    pub fn custom_attention_items(
        &self,
        person: Option<&str>,
    ) -> Result<Vec<AttentionItemView>> {
        let connection = self.readers.get();
        let query = if person.is_some() {
            "SELECT subject,body,requested_at FROM custom_sources WHERE active=1 AND person=?1 ORDER BY subject"
        } else {
            "SELECT subject,body,requested_at FROM custom_sources WHERE active=1 AND ?1 IS NULL ORDER BY subject"
        };
        let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h45d8564d30b6 owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */ query)?;
        let rows = statement.query_map([person], |row| {
            let subject: String = row.get(0)?;
            if fractal_schema::notice::is_reserved_subject(&subject) {
                Ok(None)
            } else {
                Ok(Some((row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            }
        })?.filter_map(|row| row.transpose()).collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(text, at)| {
                attention_item(&serde_json::from_str(&text)?, at.parse().unwrap_or(0))
            })
            .collect()
    }
}
/// Metadata of the canonical, registration-pinned creation that owns the custom projection.
/// Replies and repaired claims cannot change the source publisher.
pub(super) fn attention_creation_owners(
    connection: &Connection,
    subjects: &BTreeSet<String>,
) -> Result<BTreeMap<String, (String, Option<String>)>> {
    if subjects.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut registrations = connection.prepare(
        "SELECT requested.value, hash, manifest, state
         FROM json_each(?1) requested JOIN custom_registrations registration
           ON registration.subject=(SELECT subject FROM custom_registrations
             WHERE substr(requested.value,1,length(prefix))=prefix ORDER BY subject LIMIT 1)",
    )?;
    let rows = registrations.query_map([serde_json::to_string(subjects)?], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?))
    })?;
    let mut pinned = Vec::new();
    for row in rows {
        let (subject, hash, manifest, state) = row?;
        // Decode even an unready registration, as the keyed reader does.
        let manifest: Manifest = serde_json::from_str(&manifest)?;
        if state == "ready" {
            pinned.push(json!({"subject": subject, "kind": manifest.creation_kind, "hash": hash}));
        }
    }
    let mut statement = connection.prepare(
        /* sqlite-large-value: allow-for-now lv-hf5c43701d222 owner=fractal_store::store::custom TODO=trace-statement-and-value-representation */
        &canonical_sql(
            "SELECT claims.subject, claims.origin, claims.actor FROM json_each(?1) requested
             JOIN claims ON claims.id=(
               SELECT claims.id FROM claims JOIN batches ON batches.id=claims.batch_id
               WHERE claims.subject=json_extract(requested.value,'$.subject')
                 AND claims.kind=json_extract(requested.value,'$.kind')
                 AND json_extract(claims.body,'$.fields._registration')=json_extract(requested.value,'$.hash')
                 AND NOT EXISTS(SELECT 1 FROM replica_records WHERE claim_id=claims.id AND state='repaired')
               ORDER BY CANONICAL_ASC(claims) LIMIT 1)",
        ),
    )?;
    let owners = statement.query_map([serde_json::to_string(&pinned)?], |row| {
        Ok((row.get(0)?, (row.get(1)?, row.get(2)?)))
    })?.collect::<rusqlite::Result<_>>()?;
    Ok(owners)
}

/// One canonical custom source, shared by the collection reader and keyed maintenance.
pub fn attention_item(body: &Value, at: u128) -> Result<AttentionItemView> {
    let a = &body["attention"];
    Ok(AttentionItemView {
        deadline_unix_ms: None,
        default_answer: None,
        episode: a["episode"].as_str().unwrap_or_default().into(),
        priority: "normal".into(),
        kind: format!(
            "custom.{}",
            body["registered_kind"].as_str().unwrap_or_default()
        ),
        review_mode: None,
        subject: body["id"].as_str().unwrap_or_default().into(),
        person: a["recipient"].as_str().unwrap_or_default().into(),
        requester_id: body["owner"].as_str().map(str::to_owned),
        launch_id: None,
        variant_id: None,
        message_id: None, answers: Vec::new(),
        prompt_call: None,
        conversation: body["owner"]
            .as_str()
            .and_then(super::attention_snapshot::agent),
        title: a["title"].as_str().unwrap_or_default().into(),
        detail: a["detail"].as_str().unwrap_or_default().into(),
        request: None,
        mission: None,
        mission_run: None,
        step: None,
        targets: vec![],
        requested_at_unix_ms: at,
        actions: vec![attention_action(
            "reply",
            &[
                "st",
                "subject",
                "reply",
                body["id"].as_str().unwrap_or_default(),
                "--fields-file",
                "REPLY.json",
                "--registration",
                body["registration"].as_str().unwrap_or_default(),
                "--revision",
                body["revision"].as_str().unwrap_or_default(),
                "--episode",
                a["episode"].as_str().unwrap_or_default(),
                "--idempotency-key",
                "REPLY-KEY",
                "--as",
                a["recipient"].as_str().unwrap_or_default(),
            ],
        )],
    })
}

pub fn source(connection: &Connection, subject: &str) -> Result<Option<Value>> {
    let text: Option<String> = connection
        .query_row(
            "SELECT body FROM custom_sources WHERE subject=?1",
            [subject],
            |r| r.get(0),
        )
        .optional()?;
    text.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}

fn timestamp(at: u128) -> String {
    chrono::DateTime::from_timestamp_millis(i64::try_from(at).unwrap_or(0))
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_delivery;
