//! Authored subject context. Claims remain authoritative; notes never grant authority.
use super::*;
use smallclaims::store::canonical;
use fractal_schema::subject_notes::KIND;

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, Serialize)]
pub struct SubjectNote {
    pub id: String,
    pub target: String,
    pub author: String,
    pub text: String,
    pub audience: String,
    pub session: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: Option<String>,
    pub revision: String,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS subject_notes (
    note TEXT PRIMARY KEY,
    target TEXT NOT NULL,
    author TEXT NOT NULL,
    audience TEXT NOT NULL,
    session TEXT,
    claim_id TEXT NOT NULL,
    head_key BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS subject_notes_target ON subject_notes(target);
CREATE INDEX IF NOT EXISTS subject_notes_author ON subject_notes(author);
CREATE TABLE IF NOT EXISTS local_subject_note_pending (claim_id TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS local_subject_note_dirty (subject TEXT PRIMARY KEY);
CREATE TRIGGER IF NOT EXISTS subject_note_claim_insert AFTER INSERT ON claims
WHEN NEW.kind='note.set' BEGIN
    INSERT OR IGNORE INTO local_subject_note_pending VALUES(NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS subject_note_legacy_insert AFTER INSERT ON claims BEGIN
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims WHERE batch_id=NEW.batch_id AND store_index>NEW.store_index
        AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id)
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS subject_note_claim_delete AFTER DELETE ON claims BEGIN
    DELETE FROM local_subject_note_pending WHERE claim_id=OLD.id;
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT OLD.subject WHERE OLD.kind='note.set';
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims WHERE batch_id=OLD.batch_id AND store_index>OLD.store_index
        AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id)
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS subject_note_claim_update
AFTER UPDATE OF id,batch_id,subject,kind,accepted_at_unix_ms,store_index ON claims
WHEN OLD.id IS NOT NEW.id OR OLD.batch_id IS NOT NEW.batch_id
    OR OLD.subject IS NOT NEW.subject OR OLD.kind IS NOT NEW.kind
    OR OLD.accepted_at_unix_ms IS NOT NEW.accepted_at_unix_ms
    OR OLD.store_index IS NOT NEW.store_index BEGIN
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT OLD.subject WHERE OLD.kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=OLD.id);
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT NEW.subject WHERE NEW.kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=OLD.id);
    INSERT OR IGNORE INTO local_subject_note_pending
    SELECT NEW.id WHERE NEW.kind='note.set'
        AND (OLD.kind<>'note.set'
            OR EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=OLD.id));
    DELETE FROM local_subject_note_pending
    WHERE claim_id=OLD.id AND (OLD.id IS NOT NEW.id OR NEW.kind<>'note.set');
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims
    WHERE (OLD.batch_id IS NOT NEW.batch_id OR OLD.store_index IS NOT NEW.store_index)
        AND ((batch_id=OLD.batch_id AND store_index>OLD.store_index)
            OR (batch_id=NEW.batch_id AND store_index>NEW.store_index))
        AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=claims.id)
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS subject_note_record_insert AFTER INSERT ON replica_records
WHEN NEW.claim_id IS NOT NULL BEGIN
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims WHERE id=NEW.claim_id AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=NEW.claim_id)
        AND (NEW.state='repaired'
            OR (SELECT MIN(position) FROM replica_records WHERE claim_id=NEW.claim_id)
            IS NOT COALESCE(
                (SELECT MIN(position) FROM replica_records
                    WHERE claim_id=NEW.claim_id AND record_ref<>NEW.record_ref),
                (SELECT COUNT(*) FROM claims legacy_position
                    WHERE legacy_position.batch_id=claims.batch_id
                        AND legacy_position.store_index<claims.store_index)));
END;
CREATE TRIGGER IF NOT EXISTS subject_note_record_update
AFTER UPDATE OF position,claim_id,state,replacement_claim_id ON replica_records
WHEN OLD.position IS NOT NEW.position OR OLD.claim_id IS NOT NEW.claim_id
    OR OLD.state IS NOT NEW.state
    OR OLD.replacement_claim_id IS NOT NEW.replacement_claim_id BEGIN
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims WHERE id IN (OLD.claim_id,NEW.claim_id)
        AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=claims.id);
END;
CREATE TRIGGER IF NOT EXISTS subject_note_record_delete AFTER DELETE ON replica_records
WHEN OLD.claim_id IS NOT NULL BEGIN
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims WHERE id=OLD.claim_id AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=OLD.claim_id)
        AND (OLD.state='repaired'
            OR (SELECT MIN(position) FROM replica_records WHERE claim_id=OLD.claim_id)>OLD.position
            OR (NOT EXISTS (SELECT 1 FROM replica_records WHERE claim_id=OLD.claim_id)
                AND OLD.position IS NOT (SELECT COUNT(*) FROM claims legacy_position
                    WHERE legacy_position.batch_id=claims.batch_id
                        AND legacy_position.store_index<claims.store_index)));
END;
CREATE TRIGGER IF NOT EXISTS subject_note_batch_update
AFTER UPDATE OF origin,replica_sequence ON batches
WHEN OLD.origin IS NOT NEW.origin OR OLD.replica_sequence IS NOT NEW.replica_sequence BEGIN
    INSERT OR IGNORE INTO local_subject_note_dirty
    SELECT subject FROM claims WHERE batch_id=NEW.id AND kind='note.set'
        AND NOT EXISTS (SELECT 1 FROM local_subject_note_pending WHERE claim_id=claims.id);
END;
"#;

pub fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

pub fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("DELETE FROM subject_notes;
        DELETE FROM local_subject_note_pending;
        DELETE FROM local_subject_note_dirty;
        INSERT INTO local_subject_note_pending SELECT id FROM claims WHERE kind='note.set'
            AND NOT EXISTS (SELECT 1 FROM replica_records repaired
                WHERE repaired.claim_id=claims.id AND repaired.state='repaired');")?;
    flush(tx)
}

pub fn flush(tx: &Transaction<'_>) -> Result<()> {
    let dirty = tx.prepare("SELECT subject FROM local_subject_note_dirty ORDER BY subject")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for person in dirty {
        tx.execute("DELETE FROM subject_notes WHERE note=?1", [&person])?;
        let latest = tx.query_row(&canonical_sql("SELECT id FROM claims WHERE subject=?1 AND kind='note.set'
            AND NOT EXISTS (SELECT 1 FROM replica_records repaired
                WHERE repaired.claim_id=claims.id AND repaired.state='repaired')
            ORDER BY CANONICAL_DESC(claims) LIMIT 1"), [&person], |row| row.get::<_, String>(0)).optional()?;
        if let Some(id) = latest { project(tx, &person, &id)?; }
    }
    let pending = tx.prepare("SELECT c.subject,c.id FROM local_subject_note_pending p JOIN claims c ON c.id=p.claim_id
        WHERE NOT EXISTS (SELECT 1 FROM replica_records repaired
            WHERE repaired.claim_id=c.id AND repaired.state='repaired')
        ORDER BY p.claim_id")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (person, id) in pending { project(tx, &person, &id)?; }
    tx.execute_batch("DELETE FROM local_subject_note_dirty; DELETE FROM local_subject_note_pending;")?;
    Ok(())
}

fn project(tx: &Transaction<'_>, note: &str, id: &str) -> Result<()> {
    let key = canonical::sortable_key(&canonical::claim_key(tx, id)?);
    let body: String = tx.query_row("SELECT body FROM claims WHERE id=?1", [id], |row| row.get(0))?;
    let body: Value = serde_json::from_str(&body)?;
    let fields = &body["fields"];
    let author = fractal_schema::subject_notes::author(note).map_err(anyhow::Error::new)?;
    tx.execute("INSERT INTO subject_notes(note,target,author,audience,session,claim_id,head_key) VALUES(?1,?2,?3,?4,?5,?6,?7)
        ON CONFLICT(note) DO UPDATE SET target=excluded.target,author=excluded.author,audience=excluded.audience,
        session=excluded.session,claim_id=excluded.claim_id,head_key=excluded.head_key
        WHERE excluded.head_key>subject_notes.head_key",
        params![note, fields["target"].as_str().context("note target missing")?, author,
            fields["audience"].as_str().context("note audience missing")?, fields["session"].as_str(), id, key])?;
    Ok(())
}

/// A pinned membership is necessary, not proof of undiscovered legacy peers. Operators must
/// fence legacy replication before enabling notes; reject every observed unfenced writer too.
pub fn validate_publication(tx: &Transaction<'_>, origin: &str) -> Result<(), St3Error> {
    fn refused(message: impl Into<String>) -> St3Error {
        St3Error::new("subject-note-feature-barrier", message)
    }
    // A newly staged local admission/removal is not in the signed membership fold yet.
    // Do not publish against yesterday's members while the replication writer seals it.
    let pending: bool = tx.query_row(&format!(
        "SELECT EXISTS(SELECT 1 FROM claims c JOIN batches b ON b.id=c.batch_id
         LEFT JOIN replica_envelopes e ON e.batch_id=b.id
         WHERE c.origin=?1 AND c.kind IN ({})
           AND NOT EXISTS(SELECT 1 FROM replica_envelope_signatures s
             WHERE s.writer=b.origin AND s.sequence=b.replica_sequence
               AND s.envelope_hash=e.envelope_hash))",
        smallclaims::store::FLEET_CLAIM_KINDS,
    ), [origin], |row| row.get(0)).map_err(internal)?;
    if pending {
        return Err(refused("pending local fleet membership must be signed before publishing subject notes"));
    }
    let membership = fleet_membership_tx(tx).map_err(internal)?;
    if membership.anchor().is_none()
        || !matches!(membership.state(origin), crate::fleet::MemberState::Current(_))
    {
        return Err(refused("subject notes require anchored membership and an active local member; fence legacy peers from replication before enabling notes"));
    }
    for member in membership.incarnations().filter(|member| member.end.is_none()) {
        if !matches!(membership.state(&member.name), crate::fleet::MemberState::Current(_)) {
            return Err(refused("conflicted fleet membership cannot enable subject notes"));
        }
        let supported: Option<bool> = tx.query_row(&canonical_sql(
            "SELECT json_extract(claims.body,?4)=?5
             FROM claims JOIN batches ON batches.id=claims.batch_id
             WHERE claims.subject=?1 AND claims.kind='daemon.started' AND claims.origin=?2
                 AND batches.replica_sequence>=?3 ORDER BY CANONICAL_DESC(claims) LIMIT 1"),
            params![format!("daemon/{}", member.name), member.name, member.start,
                format!("$.fields.features.{}", fractal_schema::subject_notes::FEATURE),
                fractal_schema::subject_notes::FEATURE_VERSION], |row| row.get(0))
            .optional().map_err(internal)?.flatten();
        if supported != Some(true) {
            return Err(refused(format!("host/{} has not advertised features.{}={}; upgrade or fence it before enabling notes",
                member.name, fractal_schema::subject_notes::FEATURE, fractal_schema::subject_notes::FEATURE_VERSION)));
        }
    }
    let fenced = membership.legacy_removed_names().collect::<BTreeSet<_>>();
    // Seek past each writer prefix instead of scanning all retained batches/envelopes.
    // Checkpoint tombstones still witness legacy writers after their payloads are trimmed.
    for (table, column) in [
        ("replica_envelopes", "writer"),
        ("batches", "origin"),
        ("checkpoint_envelopes", "writer"),
    ] {
        let mut statement = tx.prepare(&format!(
            "SELECT {column} FROM {table} WHERE {column}>?1 ORDER BY {column} LIMIT 1"
        )).map_err(internal)?;
        let mut previous = String::new();
        while let Some(writer) = statement.query_row([&previous], |row| row.get::<_, String>(0))
            .optional().map_err(internal)?
        {
            if !membership.is_keyed_writer(&writer) && !fenced.contains(&writer) {
                return Err(refused(format!("legacy writer `{writer}` remains unfenced; feature advertisements do not establish complete legacy discovery")));
            }
            previous = writer;
        }
    }
    Ok(())
}

