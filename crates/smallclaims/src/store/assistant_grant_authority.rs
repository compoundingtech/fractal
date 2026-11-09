//! Incremental, authenticated external-grant authority. Raw claims remain evidence only.
use super::*;
use std::time::{Duration, Instant};

pub const GRANTED: &str = "custom.client.assistant.v1.granted";
pub const POLICY: &str = "custom.client.assistant.v1.policy";
pub const REVOKED: &str = "custom.client.assistant.v1.revoked";
const VERSION: &str = "1";
const KINDS: [&str; 3] = [GRANTED, POLICY, REVOKED];
pub const CLAIMS_PER_SLICE: usize = 64;
pub const MEMBERSHIP_KEYS_PER_REFRESH: usize = 16;
const SCOPES: &[&str] = &[
    "read.assistant.summary", "read.assistant.attention", "read.assistant.missions",
    "read.assistant.agents", "read.assistant.conversation", "read.assistant.terminal",
    "write.assistant.messages", "write.assistant.custom-reply", "write.assistant.answers",
];

pub fn principal(fields: &Value) -> Option<&str> {
    match fields.get("principal") {
        None => Some("assistant/chatgpt"),
        Some(value) => value.as_str().filter(|value| matches!(*value, "assistant/chatgpt" | "assistant/mcp")),
    }
}
fn profile(fields: &Value) -> Option<&str> {
    match fields.get("profile") {
        None => Some("assistant"),
        Some(value) => value.as_str().filter(|value| matches!(*value, "assistant" | "service")),
    }
}
fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
fn digest_only(fields: &Value) -> bool {
    fields.get("credential").is_none() && fields.get("code").is_none()
        && ["credential_hash", "code_hash"].iter().all(|key| fields.get(*key).is_none_or(digest))
}
fn identity(claim: &ClaimRecord) -> bool {
    let fields = &claim.body["fields"];
    let Some(id) = fields["grant_id"].as_str() else { return false; };
    let suffix = id.strip_prefix("assistant-grant/").unwrap_or(id);
    fields.is_object() && principal(fields).is_some() && profile(fields).is_some()
        && suffix.len() == 24 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        && claim.subject.strip_prefix("custom/client/assistant-v1-") == Some(suffix)
        && claim.actor.as_deref().is_some_and(|actor| fields["issuer"].as_str() == Some(actor))
}
fn policy_fields(claim: &ClaimRecord) -> bool {
    let fields = &claim.body["fields"];
    identity(claim) && digest_only(fields)
        && fields["session_actor"].as_str().and_then(|actor|
            principal(fields).and_then(|principal| actor.strip_prefix(principal)))
            .and_then(|suffix| suffix.strip_prefix("/session/")) == fields["grant_id"].as_str()
        && matches!(fields["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow"))
        && fields["disclosure_consent"] == true
        && fields["expires_at_unix_ms"].as_u64().is_some()
        && fields["scopes"].as_array().is_some_and(|scopes|
            scopes.iter().all(|scope| scope.as_str().is_some_and(|scope| SCOPES.contains(&scope))))
}
fn base_valid(claim: &ClaimRecord) -> bool {
    identity(claim) && digest_only(&claim.body["fields"])
        && digest(&claim.body["fields"]["credential_hash"])
}
/// Shared validation for admission projection and consumers. The reference oracle
/// deliberately does not call this function.
pub fn structurally_valid(grant: &ClaimRecord, policy: &ClaimRecord) -> bool {
    base_valid(grant) && policy_fields(policy) && grant.subject == policy.subject
        && grant.origin == policy.origin && grant.actor == policy.actor
        && principal(&grant.body["fields"]) == principal(&policy.body["fields"])
        && ["grant_id", "session_actor", "issuer", "profile"].iter()
            .all(|key| grant.body["fields"][*key] == policy.body["fields"][*key])
}
pub fn transition_valid(grant: &ClaimRecord, transition: &ClaimRecord) -> bool {
    if transition.kind == POLICY { return structurally_valid(grant, transition); }
    transition.kind == REVOKED && base_valid(grant) && identity(transition)
        && transition.origin == grant.origin && transition.actor == grant.actor
        && transition.subject == grant.subject
        && transition.body["fields"]["grant_id"] == grant.body["fields"]["grant_id"]
        && transition.body["fields"]["issuer"] == grant.body["fields"]["issuer"]
        && principal(&transition.body["fields"]) == principal(&grant.body["fields"])
        && profile(&transition.body["fields"]) == profile(&grant.body["fields"])
}

pub fn expiry_key(value: u64) -> String { format!("{value:020}") }

pub fn install(connection: &Connection, local_origin: &str) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hd3babb5b6b9a owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-variable-columns */
        "CREATE TABLE IF NOT EXISTS assistant_grant_authority (
            claim_id TEXT NOT NULL, base_claim_id TEXT NOT NULL,
            grant_subject TEXT NOT NULL, kind TEXT NOT NULL, origin TEXT NOT NULL,
            actor TEXT NOT NULL, principal TEXT NOT NULL, profile TEXT NOT NULL,
            expires_at TEXT NOT NULL, store_index INTEGER NOT NULL,
            credential_hash TEXT, record TEXT NOT NULL,
            PRIMARY KEY(claim_id,base_claim_id));
         CREATE INDEX IF NOT EXISTS assistant_grant_authority_transition_index
            ON assistant_grant_authority(grant_subject,kind,origin,actor,base_claim_id,store_index DESC);
         CREATE INDEX IF NOT EXISTS assistant_grant_authority_latest_index
            ON assistant_grant_authority(grant_subject,kind,store_index DESC);
         CREATE INDEX IF NOT EXISTS assistant_grant_authority_credential_index
            ON assistant_grant_authority(credential_hash,store_index DESC) WHERE kind='custom.client.assistant.v1.granted';
         CREATE TABLE IF NOT EXISTS assistant_grant_effective (
            claim_id TEXT PRIMARY KEY, grant_subject TEXT NOT NULL, principal TEXT NOT NULL,
            owner TEXT NOT NULL, profile TEXT NOT NULL, expires_at TEXT NOT NULL,
            revoked INTEGER NOT NULL, grant_record TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS assistant_grant_effective_active_index
            ON assistant_grant_effective(principal,owner,revoked,expires_at,profile);
         CREATE INDEX IF NOT EXISTS assistant_grant_effective_subject_index
            ON assistant_grant_effective(grant_subject);
         CREATE TABLE IF NOT EXISTS assistant_grant_pending (
            claim_id TEXT PRIMARY KEY, grant_subject TEXT NOT NULL, kind TEXT NOT NULL,
            origin TEXT NOT NULL, actor TEXT NOT NULL, sequence INTEGER NOT NULL,
            authenticated INTEGER NOT NULL, record TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_subject_index
            ON assistant_grant_pending(grant_subject,authenticated,origin,actor,kind);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_writer_index
            ON assistant_grant_pending(origin,authenticated,sequence,claim_id);
         CREATE TABLE IF NOT EXISTS assistant_grant_dirty (claim_id TEXT PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS assistant_grant_auth_cursor (
            claim_id TEXT PRIMARY KEY, position INTEGER NOT NULL, record_rowid INTEGER NOT NULL, member_key TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS assistant_grant_membership_dirty (
            writer TEXT PRIMARY KEY, sequence INTEGER NOT NULL, claim_id TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS assistant_grant_membership_state (
            id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, applied_revision INTEGER NOT NULL);
         INSERT OR IGNORE INTO assistant_grant_membership_state VALUES(1,1,0);
         CREATE TABLE IF NOT EXISTS assistant_grant_local_appends (claim_id TEXT PRIMARY KEY);
         CREATE TRIGGER IF NOT EXISTS assistant_grant_dirty_insert AFTER INSERT ON claims
         WHEN NEW.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked')
         BEGIN INSERT OR IGNORE INTO assistant_grant_dirty VALUES(NEW.id); END;
         DROP TRIGGER IF EXISTS assistant_grant_dirty_signature_insert;
         DROP TRIGGER IF EXISTS assistant_grant_dirty_signature_insert_v2;
         CREATE TRIGGER IF NOT EXISTS assistant_grant_dirty_signature_insert_v3
         AFTER INSERT ON replica_envelope_signatures
         WHEN EXISTS(SELECT 1 FROM assistant_grant_membership_windows
              WHERE writer=NEW.writer AND member_key=NEW.member_key
                AND NEW.sequence>=start AND (end IS NULL OR NEW.sequence<=end))
         BEGIN
            INSERT OR IGNORE INTO assistant_grant_dirty(claim_id)
            SELECT records.claim_id FROM replica_records AS records
            JOIN claims ON claims.id=records.claim_id
            WHERE records.writer=NEW.writer AND records.sequence=NEW.sequence
              AND records.envelope_hash=NEW.envelope_hash
              AND claims.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked');
            DELETE FROM assistant_grant_auth_cursor WHERE claim_id IN (
              SELECT claim_id FROM replica_records
              WHERE writer=NEW.writer AND sequence=NEW.sequence AND envelope_hash=NEW.envelope_hash);
         END;
         CREATE TABLE IF NOT EXISTS assistant_grant_membership_windows (
            writer TEXT NOT NULL, member_key TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER,
            PRIMARY KEY(writer,member_key));"
    )?;
    install_membership_evidence_triggers(connection)?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h3f117dab1bdd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_local_origin',?1)", [local_origin])?;
    Ok(())
}

/// Independent from the inherited fleet-generation counter: an irrelevant
/// signature may invalidate that legacy cache, but never this projection.
fn install_membership_evidence_triggers(connection: &Connection) -> Result<()> {
    let bump = "UPDATE assistant_grant_membership_state SET revision=revision+1 WHERE id=1;";
    for (event, row) in [("INSERT", "NEW"), ("DELETE", "OLD")] {
        connection.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_claim_{event}
             AFTER {event} ON claims WHEN {row}.kind IN ({FLEET_CLAIM_KINDS})
             BEGIN {bump} END;
             CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_envelope_{event}
             AFTER {event} ON replica_envelopes
             WHEN EXISTS(SELECT 1 FROM claims WHERE batch_id={row}.batch_id
                         AND kind IN ({FLEET_CLAIM_KINDS}))
             BEGIN {bump} END;
             CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_anchor_{event}
             AFTER {event} ON meta WHEN {row}.key IN ('fleet_anchor_key','fleet_id')
             BEGIN {bump} END;"
        ))?;
    }
    connection.execute_batch(&format!(
        "CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_claim_update
         AFTER UPDATE OF kind,body,subject,batch_id ON claims
         WHEN OLD.kind IN ({FLEET_CLAIM_KINDS}) OR NEW.kind IN ({FLEET_CLAIM_KINDS})
         BEGIN {bump} END;
         CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_envelope_update
         AFTER UPDATE OF writer,sequence,batch_id,envelope_hash,receipt_state ON replica_envelopes
         WHEN EXISTS(SELECT 1 FROM claims WHERE batch_id IN (OLD.batch_id,NEW.batch_id)
                     AND kind IN ({FLEET_CLAIM_KINDS}))
         BEGIN {bump} END;
         CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_anchor_update
         AFTER UPDATE OF value ON meta
         WHEN NEW.key IN ('fleet_anchor_key','fleet_id') AND NEW.value IS NOT OLD.value
         BEGIN {bump} END;
         CREATE TRIGGER IF NOT EXISTS assistant_grant_membership_signature_insert
         AFTER INSERT ON replica_envelope_signatures
         WHEN EXISTS(SELECT 1 FROM assistant_grant_membership_windows
              WHERE writer=NEW.writer AND member_key=NEW.member_key
                AND NEW.sequence>=start AND (end IS NULL OR NEW.sequence<=end))
          AND EXISTS(SELECT 1 FROM replica_envelopes AS envelopes
              JOIN claims ON claims.batch_id=envelopes.batch_id
              WHERE envelopes.writer=NEW.writer AND envelopes.sequence=NEW.sequence
                AND envelopes.envelope_hash=NEW.envelope_hash
                AND claims.kind IN ({FLEET_CLAIM_KINDS}))
         BEGIN {bump} END;"
    ))?;
    Ok(())
}

pub fn ready(connection: &Connection) -> Result<(), FractalError> {
    let complete: bool = connection.query_row(
        "SELECT COALESCE((SELECT value FROM meta WHERE key='assistant_grant_authority_version'),'')=?1",
        [VERSION], |row| row.get(0),
    ).map_err(internal)?;
    if !complete {
        return Err(FractalError::new("assistant-grant-authority-not-ready", "external-grant authority backfill is in progress; retry after it completes"));
    }
    Ok(())
}
/// Durable maintenance work is independent of initial-backfill readiness.
pub fn maintenance_pending(connection: &Connection) -> Result<bool, FractalError> {
    let pending: bool = connection.query_row(
        "SELECT COALESCE((SELECT value FROM meta WHERE key='assistant_grant_authority_version'),'')<>?1
            OR EXISTS(SELECT 1 FROM assistant_grant_dirty)
            OR EXISTS(SELECT 1 FROM assistant_grant_membership_dirty)
            OR EXISTS(SELECT 1 FROM assistant_grant_membership_state WHERE revision<>applied_revision)",
        [VERSION], |row| row.get(0),
    ).map_err(internal)?;
    Ok(pending)
}
fn subject_pending(connection: &Connection, subject: &str) -> Result<bool, FractalError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty AS dirty
                JOIN claims ON claims.id=dirty.claim_id WHERE claims.subject=?1)
            OR EXISTS(SELECT 1 FROM assistant_grant_auth_cursor AS cursor
                JOIN claims ON claims.id=cursor.claim_id WHERE claims.subject=?1)
            OR EXISTS(SELECT 1 FROM assistant_grant_pending
                WHERE grant_subject=?1 AND authenticated=0)",
        [subject], |row| row.get(0),
    ).map_err(internal)
}
/// A pending proof or unauthenticated revoke fences only its grant subject.
pub fn subject_ready(connection: &Connection, subject: &str) -> Result<(), FractalError> {
    ready(connection)?;
    if subject_pending(connection, subject)? {
        return Err(FractalError::new("assistant-grant-subject-not-ready", "external-grant subject authentication is in progress"));
    }
    Ok(())
}
fn decode(record: String) -> Result<ClaimRecord, FractalError> {
    serde_json::from_str(&record).map_err(internal)
}
pub const TRANSITION_QUERY: &str = "SELECT record FROM assistant_grant_authority
    WHERE grant_subject=?1 AND kind=?2 AND origin=?3 AND actor=?4 AND base_claim_id=?5
    ORDER BY store_index DESC LIMIT 1";
pub fn transition(connection: &Connection, granted: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
    subject_ready(connection, &granted.subject)?;
    transition_unchecked(connection, granted, kind).map_err(internal)
}
fn transition_unchecked(connection: &Connection, granted: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>> {
    let record: Option<String> = connection.query_row(/* sqlite-large-value: allow-for-now lv-h80fb9d4725e0 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ TRANSITION_QUERY,
        params![granted.subject, kind, granted.origin, granted.actor, granted.id], |row| row.get(0)).optional()?;
    record.map(|record| serde_json::from_str(&record).map_err(Into::into)).transpose()
}
pub fn latest(connection: &Connection, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
    subject_ready(connection, subject)?;
    let record: Option<String> = connection.query_row(
        "SELECT record FROM assistant_grant_authority WHERE grant_subject=?1 AND kind=?2 ORDER BY store_index DESC LIMIT 1",
        params![subject,kind], |row| row.get(0)).optional().map_err(internal)?;
    record.map(decode).transpose()
}
pub const CANDIDATES_QUERY: &str = "SELECT grant_record FROM assistant_grant_effective
    WHERE ?3 IS NOT NULL AND principal=?1 AND owner=?2 AND revoked=0 AND expires_at>?3
      AND (?4 IS NULL OR claim_id<>?4) AND (?5 IS NULL OR profile<>?5)
    UNION ALL
    SELECT grant_record FROM assistant_grant_effective
    WHERE ?3 IS NULL AND principal=?1 AND owner=?2
      AND (?4 IS NULL OR claim_id<>?4) AND (?5 IS NULL OR profile<>?5)";
/// Authenticated effective snapshots for conflict admission, not bearer access.
/// A pending revoke must not erase an opposite-profile conflict witness before
/// it authenticates. Bearer and transition readers independently fence subjects.
pub fn candidates(connection: &Connection, principal: &str, owner: &str, expiry_cut: Option<u64>, excluded: Option<&str>, ignored_profile: Option<&str>) -> Result<Vec<ClaimRecord>, FractalError> {
    ready(connection)?;
    let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h556bdadaf8cd owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ CANDIDATES_QUERY).map_err(internal)?;
    let records = statement.query_map(params![principal,owner,expiry_cut.map(expiry_key),excluded,ignored_profile], |row| row.get::<_,String>(0))
        .map_err(internal)?;
    records.map(|record| decode(record.map_err(internal)?)).collect()
}
pub fn by_credential(connection: &Connection, digest: &str) -> Result<Option<ClaimRecord>, FractalError> {
    ready(connection)?;
    let record: Option<String> = connection.query_row(
        "SELECT record FROM assistant_grant_authority WHERE credential_hash=?1 AND kind='custom.client.assistant.v1.granted' ORDER BY store_index DESC LIMIT 1",
        [digest], |row| row.get(0)).optional().map_err(internal)?;
    let grant = record.map(decode).transpose()?;
    match grant {
        Some(grant) if !subject_pending(connection, &grant.subject)? => Ok(Some(grant)),
        _ => Ok(None),
    }
}
pub fn contains(connection: &Connection, claim_id: &str) -> Result<bool, FractalError> {
    ready(connection)?;
    let subject: Option<String> = connection.query_row(
        "SELECT grant_subject FROM assistant_grant_authority WHERE claim_id=?1 LIMIT 1",
        [claim_id], |row| row.get(0),
    ).optional().map_err(internal)?;
    match subject {
        Some(subject) => Ok(!subject_pending(connection, &subject)?),
        None => Ok(false),
    }
}

fn insert_authority(connection: &Connection, claim: &ClaimRecord, base: &ClaimRecord) -> Result<bool> {
    let fields = &claim.body["fields"];
    Ok(connection.execute(
        /* sqlite-large-value: allow-for-now lv-h60572e3ac6a5 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */
        "INSERT OR IGNORE INTO assistant_grant_authority
         (claim_id,base_claim_id,grant_subject,kind,origin,actor,principal,profile,expires_at,store_index,credential_hash,record)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![claim.id,base.id,claim.subject,claim.kind,claim.origin,claim.actor,principal(fields),profile(fields),
            expiry_key(fields["expires_at_unix_ms"].as_u64().unwrap_or(0)),claim.store_index,
            if claim.kind==GRANTED { fields["credential_hash"].as_str() } else { None },serde_json::to_string(claim)?],
    )? != 0)
}
/// The only writer of effective rows. Rebuilding a subject uses authenticated
/// authority alone, including an earlier-arriving revoke, in the same transaction.
pub fn rebuild_subject(connection: &Connection, subject: &str) -> Result<()> {
    connection.execute(/* sqlite-large-value: allow-for-now lv-h47619972ac92 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_effective WHERE grant_subject=?1", [subject])?;
    let mut statement = connection.prepare_cached("SELECT record FROM assistant_grant_authority WHERE grant_subject=?1 AND kind=?2")?;
    let records = statement.query_map(params![subject,GRANTED], |row| row.get::<_,String>(0))?;
    for record in records {
        let record = record?;
        let grant: ClaimRecord = serde_json::from_str(&record)?;
        let policy_claim = transition_unchecked(connection,&grant,POLICY)?;
        let policy = policy_claim.as_ref().unwrap_or(&grant);
        if !structurally_valid(&grant,policy) { continue; }
        let fields = &policy.body["fields"];
        connection.execute(/* sqlite-large-value: allow-for-now lv-h2909dd1a0b9c owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_effective(claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![grant.id,grant.subject,principal(fields),grant.actor,profile(fields),
                expiry_key(fields["expires_at_unix_ms"].as_u64().unwrap_or(0)),
                transition_unchecked(connection,&grant,REVOKED)?.is_some(),record])?;
    }
    Ok(())
}
fn attach_claim(connection: &Connection, claim: &ClaimRecord) -> Result<()> {
    let mut changed = false;
    if claim.kind == GRANTED {
        // A malformed base stays pending until a valid issuer policy repairs it.
        // Otherwise a newer malformed copy could mask a valid bearer lookup.
        if structurally_valid(claim,claim) {
            changed |= insert_authority(connection,claim,claim)?;
        }
        // Only this base's issuer prefix can attach to it. In particular, an
        // authenticated wrong-actor history cannot enlarge later admission work.
        let mut statement = connection.prepare_cached(
            "SELECT record FROM assistant_grant_pending WHERE grant_subject=?1
             AND authenticated=1 AND origin=?2 AND actor=?3 AND kind<>?4")?;
        let records = statement.query_map(params![claim.subject,claim.origin,claim.actor,GRANTED], |row| row.get::<_,String>(0))?;
        for record in records {
            let transition: ClaimRecord = serde_json::from_str(&record?)?;
            if transition_valid(claim,&transition) {
                if transition.kind == POLICY {
                    changed |= insert_authority(connection,claim,claim)?;
                }
                changed |= insert_authority(connection,&transition,claim)?;
            }
        }
    } else {
        let mut statement = connection.prepare_cached(
            "SELECT record FROM assistant_grant_pending WHERE grant_subject=?1
             AND authenticated=1 AND kind=?2 AND origin=?3 AND actor=?4")?;
        let records = statement.query_map(params![claim.subject,GRANTED,claim.origin,claim.actor], |row| row.get::<_,String>(0))?;
        for record in records {
            let grant: ClaimRecord = serde_json::from_str(&record?)?;
            if transition_valid(&grant,claim) {
                if claim.kind == POLICY {
                    changed |= insert_authority(connection,&grant,&grant)?;
                }
                changed |= insert_authority(connection,claim,&grant)?;
            }
        }
    }
    if changed { rebuild_subject(connection,&claim.subject)?; }
    Ok(())
}
fn local_origin(connection: &Connection) -> Result<String> {
    Ok(connection.query_row("SELECT value FROM meta WHERE key='assistant_grant_authority_local_origin'", [], |row| row.get(0))?)
}
/// Admission pays structural/authentication cost once. Invalid rows never enter
/// the authority or the pending-authentication writer/window index.
pub fn refresh_claim(connection: &Connection, id: &str) -> Result<()> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(||
        format!("SELECT {CLAIM_COLUMNS} FROM claims WHERE id=?1"));
    let claim = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h1665a4a8cfdf owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ &QUERY)?.query_row(/* sqlite-large-value: allow-for-now lv-h5520ddeb5d20 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ [id], claim_from_row).optional()?;
    let Some(claim) = claim else { return Ok(()); };
    if !KINDS.contains(&claim.kind.as_str()) { return Ok(()); }
    let valid = if claim.kind==GRANTED { base_valid(&claim) }
        else if claim.kind==POLICY { policy_fields(&claim) } else { identity(&claim) };
    if valid {
        let sequence: u64 = connection.query_row("SELECT replica_sequence FROM batches WHERE id=?1", [&claim.batch_id], |row| row.get(0))?;
        // Only the local append transaction can create this durable admission
        // fact. Origin text and the absence of a receipt prove nothing: legacy
        // imports can also have neither envelopes nor replica records.
        let local_append: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_local_appends WHERE claim_id=?1)",
            [&claim.id], |row| row.get(0),
        )?;
        let admitted: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_grant_pending WHERE claim_id=?1 AND authenticated=1)",
            [&claim.id], |row| row.get(0),
        )?;
        let authentication = if admitted || (local_append && claim.origin == local_origin(connection)?) {
            Some(true)
        } else {
            authenticated_claim(connection,&claim.id)?
        };
        let authenticated = authentication == Some(true);
        let changed = connection.execute(/* sqlite-large-value: allow-for-now lv-h9cf86ad98d1e owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_pending(claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(claim_id) DO UPDATE SET authenticated=excluded.authenticated
            WHERE assistant_grant_pending.authenticated<excluded.authenticated",
            params![claim.id,claim.subject,claim.kind,claim.origin,claim.actor,sequence,authenticated,serde_json::to_string(&claim)?])? != 0;
        if changed && authenticated { attach_claim(connection,&claim)?; }
        if authentication.is_none() {
            connection.execute("INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [id])?;
            return Ok(());
        }
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h3221720b83b7 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1", [id])?;
    Ok(())
}
/// One record and at most sixteen membership keys per refresh. Nonmember
/// signatures are never enumerated: every signature lookup supplies its full PK.
/// Persist progress even through missing/invalid evidence, so a later valid key
/// or record cannot be hidden behind an arbitrary number of earlier candidates.
fn authenticated_claim(connection: &Connection, id: &str) -> Result<Option<bool>> {
    let Some(fleet_id) = fleet_meta(connection,"assistant_grant_authority_fleet_id")? else {
        return Ok(Some(false));
    };
    // Never first-authenticate against a partially applied or stale membership
    // snapshot. Already admitted claims and durable local appends bypass this
    // path; only this pending subject remains fenced until maintenance catches up.
    let membership_ready: bool = connection.query_row(
        "SELECT revision=applied_revision FROM assistant_grant_membership_state WHERE id=1",
        [], |row| row.get(0),
    )?;
    if !membership_ready { return Ok(None); }
    let shard = fleet_meta(connection,"shard_id")?;
    let cursor: Option<(i64,i64,String)> = connection.query_row(
        "SELECT position,record_rowid,member_key FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let initial = cursor.is_none();
    let (position_cursor,rowid_cursor,key_cursor) = cursor.unwrap_or((i64::MIN,i64::MIN,String::new()));
    let record: Option<(i64,i64,String,u64,String,bool)> = connection.query_row(
        "SELECT records.position,records.rowid,records.writer,records.sequence,records.envelope_hash,
            records.state='valid' AND claims.origin=records.writer
            AND EXISTS(SELECT 1 FROM replica_envelopes AS envelopes
                WHERE envelopes.writer=records.writer AND envelopes.sequence=records.sequence
                  AND envelopes.envelope_hash=records.envelope_hash
                  AND envelopes.receipt_state IN ('validated','degraded'))
         FROM replica_records AS records INDEXED BY replica_records_claim
         JOIN claims ON claims.id=records.claim_id
         WHERE records.claim_id=?1 AND (records.position,records.rowid)>=(?2,?3)
           AND (?5 OR (records.position,records.rowid)<>(?2,?3) OR ?4<>'')
         ORDER BY records.position,records.rowid LIMIT 1",
        params![id,position_cursor,rowid_cursor,key_cursor,initial],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
    let Some((position,record_rowid,writer,sequence,hash,eligible)) = record else {
        connection.execute("DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
        return Ok(Some(false));
    };
    // A nonempty key resumes this record; an empty key advances past it.
    // The index seek skips at most the one completed record at the cursor.
    let mut next_key = String::new();
    if eligible {
        let mut statement = connection.prepare_cached(AUTHENTICATION_KEYS_QUERY)?;
        let keys = statement.query_map(
            params![writer,sequence,hash,if (position,record_rowid)==(position_cursor,rowid_cursor) { key_cursor.as_str() } else { "" },MEMBERSHIP_KEYS_PER_REFRESH],
            |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key,signature) in &keys {
            if signature.as_deref().is_some_and(|signature|
                VerifiedEnvelopeSignature::verify(shard.as_deref(),&fleet_id,&writer,sequence,&hash,key,signature).is_some()) {
                connection.execute("DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
                return Ok(Some(true));
            }
        }
        if keys.len()==MEMBERSHIP_KEYS_PER_REFRESH {
            next_key = keys.into_iter().last().expect("full key page").0;
        }
    }
    if next_key.is_empty() {
        let more: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records INDEXED BY replica_records_claim
             WHERE claim_id=?1 AND (position,rowid)>(?2,?3))",
            params![id,position,record_rowid], |row| row.get(0))?;
        if !more {
            connection.execute("DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
            return Ok(Some(false));
        }
    }
    connection.execute(
        "INSERT INTO assistant_grant_auth_cursor VALUES(?1,?2,?3,?4)
         ON CONFLICT(claim_id) DO UPDATE SET position=excluded.position,
            record_rowid=excluded.record_rowid,member_key=excluded.member_key",
        params![id,position,record_rowid,next_key])?;
    Ok(None)
}
pub const AUTHENTICATION_KEYS_QUERY: &str =
    "SELECT windows.member_key,signatures.signature
     FROM assistant_grant_membership_windows AS windows
     LEFT JOIN replica_envelope_signatures AS signatures
       ON signatures.writer=windows.writer AND signatures.sequence=?2
      AND signatures.envelope_hash=?3 AND signatures.member_key=windows.member_key
      AND ?2>=windows.start AND (windows.end IS NULL OR ?2<=windows.end)
     WHERE windows.writer=?1 AND windows.member_key>?4
     ORDER BY windows.member_key LIMIT ?5";
/// Refresh only this envelope's grant records, after receipt/signature state has
/// been written, and before its transaction is committed.
pub fn refresh_envelope(connection: &Connection, writer: &str, sequence: u64, hash: &str) -> Result<()> {
    let ids = {
        let mut statement = connection.prepare_cached("SELECT records.claim_id FROM replica_records AS records JOIN claims ON claims.id=records.claim_id
            WHERE records.writer=?1 AND records.sequence=?2 AND records.envelope_hash=?3
              AND claims.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked')")?;
        statement.query_map(params![writer,sequence,hash], |row| row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    // A replicated same-origin claim does NOT acquire the local-write exception.
    for id in ids {
        // Signatures may predate their replica record and therefore could not
        // invalidate a claim cursor when inserted. New evidence must restart
        // its claim even when its indexed position precedes that cursor.
        connection.execute("DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&id])?;
        refresh_claim(connection,&id)?;
    }
    Ok(())
}
fn membership_revision(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT revision FROM assistant_grant_membership_state WHERE id=1", [], |row| row.get(0),
    )?)
}

struct MembershipUpdate {
    revision: i64,
    fleet_id: Option<String>,
    // Only changed/deleted keys, never a replacement of the complete snapshot.
    windows: Vec<(String, String, Option<(u64, Option<u64>)>)>,
    complete: bool,
}

impl MembershipUpdate {
    fn apply(&self, connection: &Connection) -> Result<usize> {
        for (writer,key,window) in &self.windows {
            if let Some((start,end)) = window {
                connection.execute(
                    "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,?3,?4)
                     ON CONFLICT(writer,member_key) DO UPDATE SET start=excluded.start,end=excluded.end",
                    params![writer,key,start,end],
                )?;
            } else {
                connection.execute(
                    "DELETE FROM assistant_grant_membership_windows WHERE writer=?1 AND member_key=?2",
                    params![writer,key],
                )?;
            }
            connection.execute(
                "INSERT OR REPLACE INTO assistant_grant_membership_dirty
                 SELECT ?1,0,'' WHERE EXISTS(
                   SELECT 1 FROM assistant_grant_pending INDEXED BY assistant_grant_pending_writer_index
                   WHERE origin=?1 AND authenticated=0)", [writer],
            )?;
        }
        if self.complete {
            if let Some(fleet_id) = &self.fleet_id {
                connection.execute(
                    "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_fleet_id',?1)",
                    [fleet_id],
                )?;
            } else {
                connection.execute("DELETE FROM meta WHERE key='assistant_grant_authority_fleet_id'", [])?;
            }
            connection.execute(
                "UPDATE assistant_grant_membership_state SET applied_revision=?1 WHERE id=1",
                [self.revision],
            )?;
        }
        Ok(self.windows.len())
    }
}
/// Resumable, idempotent versioned backfill. Each page uses claims_kind_index;
/// unrelated claims are never visited. Cursor and projection changes commit
/// together in the caller's transaction. Old-build appends and signature-only
/// arrivals are captured by persistent dirty triggers even after completion.
pub fn backfill_slice(connection: &Connection, budget: Duration) -> Result<bool> {
    backfill_slice_with_limit(connection, budget, CLAIMS_PER_SLICE)
}

fn backfill_slice_with_limit(connection: &Connection, budget: Duration, mut remaining: usize) -> Result<bool> {
    let started = Instant::now();
    if remaining==0 { return Ok(false); }
    // Include membership-driven retries in the same fixed item budget. Each
    // empty writer task also consumes a unit, bounding sparse writer queues.
    while remaining>0 {
        let task: Option<(String,u64,String)> = connection.query_row(
            "SELECT writer,sequence,claim_id FROM assistant_grant_membership_dirty ORDER BY writer LIMIT 1",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let Some((writer,sequence,id)) = task else { break; };
        let pending: Option<(u64,String)> = connection.query_row(
            "SELECT sequence,claim_id FROM assistant_grant_pending INDEXED BY assistant_grant_pending_writer_index
             WHERE origin=?1 AND authenticated=0 AND (sequence,claim_id)>(?2,?3)
             ORDER BY sequence,claim_id LIMIT 1", params![writer,sequence,id],
            |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((sequence,id)) = pending {
            connection.execute("DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&id])?;
            refresh_claim(connection,&id)?;
            connection.execute(
                "UPDATE assistant_grant_membership_dirty SET sequence=?2,claim_id=?3 WHERE writer=?1",
                params![writer,sequence,id])?;
        } else {
            connection.execute("DELETE FROM assistant_grant_membership_dirty WHERE writer=?1", [writer])?;
        }
        remaining -= 1;
        if remaining==0 || started.elapsed()>=budget { return Ok(false); }
    }
    // A completed version only drains the old-build dirty queue. Readiness also
    // includes that queue, but must never cause a completed history rescan.
    if fleet_meta(connection,"assistant_grant_authority_version")?.as_deref() != Some(VERSION) {
        for kind in KINDS {
            let key = format!("assistant_grant_authority_cursor/{kind}");
            loop {
                let cursor: u64 = connection.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",[&key],|row|row.get(0)).optional()?.unwrap_or(0);
                let rows = {
                    let mut statement = connection.prepare_cached("SELECT id,store_index FROM claims INDEXED BY claims_kind_index WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT 64")?;
                    statement.query_map(params![kind,cursor],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
                };
                if rows.is_empty() { break; }
                for (id,index) in rows {
                    refresh_claim(connection,&id)?;
                    remaining -= 1;
                    connection.execute(/* sqlite-large-value: allow-for-now lv-h3f49c0e579cd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",params![key,index.to_string()])?;
                    if remaining==0 || started.elapsed()>=budget { return Ok(false); }
                }
            }
        }
    }
    // Visit each selected dirty claim once. An unfinished authentication page
    // keeps its dirty row for a later transaction, not a loop in this one.
    let rows = {
        let mut statement = connection.prepare_cached("SELECT claim_id FROM assistant_grant_dirty ORDER BY claim_id LIMIT ?1")?;
        statement.query_map([remaining],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for id in rows {
        refresh_claim(connection,&id)?;
        remaining -= 1;
        if remaining==0 || started.elapsed()>=budget { break; }
    }
    let dirty: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty)
             OR EXISTS(SELECT 1 FROM assistant_grant_membership_dirty)", [], |row| row.get(0))?;
    if dirty { return Ok(false); }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h9b70e51e754e owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_version',?1)",[VERSION])?;
    Ok(true)
}

impl Store {
    /// Relevant membership evidence only; unlike fleet_generation, arbitrary
    /// nonmember signatures do not advance this projection-owned revision.
    pub fn assistant_grant_membership_revision(&self) -> Result<i64> {
        if let Some(authority) = self.fleet_authority.get() {
            return authority.assistant_grant_membership_revision();
        }
        membership_revision(&self.readers.get())
    }

    // The membership fold and snapshot comparison run on readers, before the
    // writer is acquired. A writer transaction applies at most 64 changed keys.
    fn prepare_assistant_grant_membership(&self) -> Result<Option<MembershipUpdate>> {
        let revision = self.assistant_grant_membership_revision()?;
        let connection = self.readers.get();
        let applied: i64 = connection.query_row(
            "SELECT applied_revision FROM assistant_grant_membership_state WHERE id=1",
            [], |row| row.get(0),
        )?;
        if revision==applied { return Ok(None); }
        let membership = self.fleet_membership_sealed()?;
        let fleet_id = if let Some(authority) = self.fleet_authority.get() {
            authority.fleet_id()?
        } else {
            fleet_meta(&connection, "fleet_id")?
        };
        let desired: BTreeMap<_,_> = membership.incarnations().map(|incarnation| (
            (incarnation.name.clone(),incarnation.member_key.clone()),
            (incarnation.start,incarnation.end),
        )).collect();
        let current: BTreeMap<_,_> = connection.prepare(
            "SELECT writer,member_key,start,end FROM assistant_grant_membership_windows",
        )?.query_map([], |row| Ok(((row.get::<_,String>(0)?,row.get::<_,String>(1)?),
                                  (row.get::<_,u64>(2)?,row.get::<_,Option<u64>>(3)?))))?
            .collect::<rusqlite::Result<_>>()?;
        let mut windows = Vec::with_capacity(CLAIMS_PER_SLICE);
        let mut complete = true;
        for (identity,window) in &desired {
            if current.get(identity)==Some(window) { continue; }
            if windows.len()==CLAIMS_PER_SLICE { complete=false; break; }
            windows.push((identity.0.clone(),identity.1.clone(),Some(*window)));
        }
        if complete {
            for identity in current.keys() {
                if desired.contains_key(identity) { continue; }
                if windows.len()==CLAIMS_PER_SLICE { complete=false; break; }
                windows.push((identity.0.clone(),identity.1.clone(),None));
            }
        }
        Ok(Some(MembershipUpdate { revision, fleet_id, windows, complete }))
    }

    /// Background work only; receipt transactions never fold fleet history.
    pub fn advance_assistant_grant_authority_backfill(&self) -> Result<bool> {
        let prepared = self.prepare_assistant_grant_membership()?;
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        self.sync_assistant_grant_membership(&transaction)?;
        let used = if let Some(update) = &prepared {
            if membership_revision(&transaction)? != update.revision {
                return Ok(false);
            }
            update.apply(&transaction)?
        } else { 0 };
        let done = backfill_slice_with_limit(
            &transaction, Duration::from_millis(100), CLAIMS_PER_SLICE-used,
        )?;
        let membership_ready: bool = transaction.query_row(
            "SELECT revision=applied_revision FROM assistant_grant_membership_state WHERE id=1",
            [], |row| row.get(0),
        )?;
        transaction.commit()?;
        Ok(done && membership_ready)
    }

    /// Shards copy only the authoritative relevant-evidence counter here. This
    /// constant-size synchronization is safe in a receipt transaction; the
    /// membership snapshot itself is prepared off-writer by maintenance.
    pub(super) fn sync_assistant_grant_membership(&self, connection: &Connection) -> Result<()> {
        if let Some(authority) = self.fleet_authority.get() {
            connection.execute(
                "UPDATE assistant_grant_membership_state SET revision=?1 WHERE id=1",
                [authority.assistant_grant_membership_revision()?],
            )?;
        }
        Ok(())
    }
}
