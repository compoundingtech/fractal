//! Incremental, authenticated external-grant authority. Raw claims remain evidence only.
use super::*;
use std::time::{Duration, Instant};

pub const GRANTED: &str = "custom.client.assistant.v1.granted";
pub const POLICY: &str = "custom.client.assistant.v1.policy";
pub const REVOKED: &str = "custom.client.assistant.v1.revoked";
const VERSION: &str = "1";
/// Any authority DDL change (tables, indexes or triggers) must bump this version
/// and provide any required migration; update the versioned DDL digest as well.
const SCHEMA_VERSION: &str = "1";
#[cfg(test)]
pub(super) const SCHEMA_DDL_FINGERPRINT: (&str, &str) =
    ("1", "1bee597c13dc75e97e29c66e14c129baa93d3c048bd92e9b2e58e6c4e618846f");
const KINDS: [&str; 3] = [GRANTED, POLICY, REVOKED];
pub const CLAIMS_PER_SLICE: usize = 64;
pub const MEMBERSHIP_KEYS_PER_REFRESH: usize = 16;
const SCOPES: &[&str] = &[
    "read.assistant.summary", "read.assistant.attention", "read.assistant.missions",
    "read.assistant.agents", "read.assistant.conversation", "read.assistant.terminal",
    "write.assistant.messages", "write.assistant.custom-reply", "write.assistant.answers",
];
const SERVICE_SCOPES: &[&str] = &["read.claims", "read.work", "read.documents"];

fn service_principal(value: &str) -> bool {
    let mut parts = value.split('/');
    let component = |part: Option<&str>| part.is_some_and(|part| !part.is_empty()
        && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)));
    parts.next() == Some("plugin") && component(parts.next())
        && component(parts.next()) && parts.next().is_none()
}

/// Projection identity includes legacy Service-on-assistant conflict witnesses.
/// API bearer authentication must separately require a plugin principal for Service.
pub fn principal(fields: &Value) -> Option<&str> {
    match fields.get("principal") {
        None => Some("assistant/chatgpt"),
        Some(value) => value.as_str().filter(|value|
            matches!(*value, "assistant/chatgpt" | "assistant/mcp")
                || (profile(fields) == Some("service") && service_principal(value))),
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
    // Historical Service-on-assistant rows remain conservative conflict evidence
    // under their original assistant policy, never Service bearer credentials.
    let service = principal(fields).is_some_and(service_principal);
    let scopes = if service { SERVICE_SCOPES } else { SCOPES };
    identity(claim) && digest_only(fields)
        && fields["session_actor"].as_str().and_then(|actor|
            principal(fields).and_then(|principal| actor.strip_prefix(principal)))
            .and_then(|suffix| suffix.strip_prefix("/session/")) == fields["grant_id"].as_str()
        && (service || (matches!(fields["minimal_read_mode"].as_str(), Some("approval-once" | "caller-bound-allow"))
            && fields["disclosure_consent"] == true))
        && fields["expires_at_unix_ms"].as_u64().is_some()
        && fields["scopes"].as_array().is_some_and(|values|
            values.iter().all(|scope| scope.as_str().is_some_and(|scope| scopes.contains(&scope))))
}
fn base_valid(claim: &ClaimRecord) -> bool {
    identity(claim) && digest_only(&claim.body["fields"])
        && digest(&claim.body["fields"]["credential_hash"])
}
/// Shared wire validation for admission, consumers and the raw-history oracle.
/// Authentication and projection folding are independently implemented by tests.
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
    // Schema installation is distinct from the resumable authority backfill.
    // Replacing an unchanged trigger invalidates durable consumer bindings, and
    // even replacing the same meta value needlessly writes the database header.
    if fleet_meta(connection, "assistant_grant_authority_schema_version")?.as_deref() == Some(SCHEMA_VERSION) {
        migrate_legacy_state(connection)?;
        if fleet_meta(connection, "assistant_grant_authority_local_origin")?.as_deref() != Some(local_origin) {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-hbbdcd60c3dad owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */
                "UPDATE meta SET value=?1 WHERE key='assistant_grant_authority_local_origin'",
                [local_origin],
            )?;
        }
        return Ok(());
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hff2d1c2b03b3 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ "SAVEPOINT assistant_grant_upgrade")?;
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-hcda5071d3120 owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-variable-columns */
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
         CREATE INDEX IF NOT EXISTS assistant_grant_authority_management_revoke_index
            ON assistant_grant_authority(grant_subject,kind,origin,actor,principal,profile,
                json_extract(record,'$.body.fields.grant_id'),json_extract(record,'$.body.fields.issuer'),store_index DESC);
         CREATE INDEX IF NOT EXISTS assistant_grant_authority_management_policy_index
            ON assistant_grant_authority(grant_subject,kind,origin,actor,principal,profile,
                json_extract(record,'$.body.fields.grant_id'),json_extract(record,'$.body.fields.issuer'),
                json_extract(record,'$.body.fields.profile'),json_extract(record,'$.body.fields.session_actor'),store_index DESC);
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
            authenticated INTEGER NOT NULL, record TEXT NOT NULL,
            auth_member_key TEXT, auth_sequence INTEGER,
            auth_envelope_hash TEXT, auth_record_position INTEGER);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_subject_index
            ON assistant_grant_pending(grant_subject,authenticated,origin,actor,kind);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_writer_index
            ON assistant_grant_pending(origin,authenticated,sequence,claim_id);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_evidence_index
            ON assistant_grant_pending(origin,auth_member_key,auth_sequence,claim_id);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_signature_index
            ON assistant_grant_pending(origin,auth_sequence,auth_envelope_hash,auth_member_key,claim_id);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_envelope_index
            ON assistant_grant_pending(origin,auth_sequence,auth_envelope_hash,claim_id);
         CREATE TABLE IF NOT EXISTS assistant_grant_proof_invalidations (
            id INTEGER PRIMARY KEY, writer TEXT NOT NULL, sequence INTEGER NOT NULL,
            envelope_hash TEXT NOT NULL, member_key TEXT NOT NULL,
            claim_id TEXT NOT NULL DEFAULT '');
         CREATE INDEX IF NOT EXISTS assistant_grant_proof_invalidations_identity_index
            ON assistant_grant_proof_invalidations(writer,sequence,envelope_hash,member_key);
         CREATE TABLE IF NOT EXISTS assistant_grant_proof_rechecks (claim_id TEXT PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS assistant_grant_dirty (claim_id TEXT PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS assistant_grant_auth_cursor (
            claim_id TEXT PRIMARY KEY, position INTEGER NOT NULL, record_rowid INTEGER NOT NULL,
            member_key TEXT NOT NULL, membership_start INTEGER NOT NULL DEFAULT -1);
         CREATE TABLE IF NOT EXISTS assistant_grant_additions (
            id INTEGER PRIMARY KEY, writer TEXT NOT NULL, member_key TEXT NOT NULL,
            lower_sequence INTEGER NOT NULL, upper_sequence INTEGER NOT NULL,
            sequence INTEGER NOT NULL DEFAULT -1, envelope_hash TEXT NOT NULL DEFAULT '',
            position INTEGER NOT NULL DEFAULT -1, claim_id TEXT NOT NULL DEFAULT '',
            active_claim TEXT NOT NULL DEFAULT '');
         CREATE TABLE IF NOT EXISTS assistant_grant_auth_work (
            writer TEXT NOT NULL, claim_id TEXT NOT NULL, PRIMARY KEY(writer,claim_id));
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_unauthenticated_index
            ON assistant_grant_pending(origin,claim_id) WHERE authenticated=0;
         CREATE INDEX IF NOT EXISTS assistant_grant_auth_work_claim_index
            ON assistant_grant_auth_work(claim_id);
         DROP INDEX IF EXISTS replica_records_grant_proof_index;
         CREATE INDEX IF NOT EXISTS replica_records_writer_grant_proof_index
            ON replica_records(writer,claim_id,sequence,envelope_hash,position);
         CREATE TABLE IF NOT EXISTS assistant_grant_retractions (
            id INTEGER PRIMARY KEY, writer TEXT NOT NULL, member_key TEXT NOT NULL,
            lower_sequence INTEGER NOT NULL, upper_sequence INTEGER NOT NULL,
            sequence INTEGER NOT NULL DEFAULT -1, claim_id TEXT NOT NULL DEFAULT '');
         CREATE INDEX IF NOT EXISTS assistant_grant_retractions_writer_key_index
            ON assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence);
         CREATE TABLE IF NOT EXISTS assistant_grant_recompute (claim_id TEXT PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS assistant_grant_attachments (
            claim_id TEXT NOT NULL, candidate_kind TEXT NOT NULL,
            grant_subject TEXT NOT NULL, origin TEXT NOT NULL, actor TEXT NOT NULL,
            after_claim_id TEXT NOT NULL, upper_claim_id TEXT NOT NULL,
            PRIMARY KEY(claim_id,candidate_kind));
         CREATE INDEX IF NOT EXISTS assistant_grant_attachments_subject_index
            ON assistant_grant_attachments(grant_subject);
         CREATE INDEX IF NOT EXISTS assistant_grant_pending_attachment_index
            ON assistant_grant_pending(grant_subject,authenticated,origin,actor,kind,claim_id);
         CREATE TABLE IF NOT EXISTS assistant_grant_attachment_witnesses (
            base_claim_id TEXT PRIMARY KEY, grant_subject TEXT NOT NULL,
            principal TEXT NOT NULL, owner TEXT NOT NULL, profile TEXT NOT NULL,
            expires_at TEXT NOT NULL, revoked INTEGER NOT NULL, grant_record TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS assistant_grant_attachment_witnesses_subject_index
            ON assistant_grant_attachment_witnesses(grant_subject);
         CREATE INDEX IF NOT EXISTS assistant_grant_attachment_witnesses_active_index
            ON assistant_grant_attachment_witnesses(principal,owner,revoked,expires_at,profile);
         CREATE INDEX IF NOT EXISTS assistant_grant_authority_base_index
            ON assistant_grant_authority(base_claim_id,kind,store_index DESC);
         CREATE TABLE IF NOT EXISTS assistant_grant_membership_state (
            id INTEGER PRIMARY KEY CHECK(id=1), applied_generation INTEGER NOT NULL);
         INSERT OR IGNORE INTO assistant_grant_membership_state VALUES(1,-1);
         CREATE TABLE IF NOT EXISTS assistant_grant_local_appends (claim_id TEXT PRIMARY KEY);
         CREATE TRIGGER IF NOT EXISTS assistant_grant_dirty_insert AFTER INSERT ON claims
         WHEN NEW.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked')
         BEGIN INSERT OR IGNORE INTO assistant_grant_dirty VALUES(NEW.id); END;
         CREATE TRIGGER IF NOT EXISTS assistant_grant_dirty_delete AFTER DELETE ON claims
         WHEN EXISTS(SELECT 1 FROM assistant_grant_pending WHERE claim_id=OLD.id)
         BEGIN INSERT OR IGNORE INTO assistant_grant_dirty VALUES(OLD.id); END;
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
            PRIMARY KEY(writer,member_key));
         CREATE INDEX IF NOT EXISTS assistant_grant_membership_windows_seek
            ON assistant_grant_membership_windows(writer,start,member_key);"
    )?;
    install_proof_invalidation_triggers(connection)?;
    install_auth_work(connection)?;
    migrate_legacy_state(connection)?;
    install_queue_owners(connection)?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h3f117dab1bdd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_local_origin',?1)", [local_origin])?;
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h31877443d557 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */
        "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_schema_version',?1)",
        [SCHEMA_VERSION],
    )?;
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h57db182d0631 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ "RELEASE assistant_grant_upgrade")?;
    Ok(())
}

// Older builds may leave legacy work even after this schema version was
// installed. Detect it read-only; unchanged reopens never recreate DDL or write.
fn migrate_legacy_state(connection: &Connection) -> Result<()> {
    let has_start: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('assistant_grant_auth_cursor') WHERE name='membership_start')",
        [], |row| row.get(0),
    )?;
    let old_additions: bool = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM pragma_table_info('assistant_grant_additions') WHERE name='claim_id')",
        [], |row| row.get(0),
    )?;
    let installed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='assistant_grant_auth_work_installed')",
        [], |row| row.get(0),
    )?;
    let legacy: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='assistant_grant_membership_dirty')",
        [], |row| row.get(0),
    )?;
    if has_start && !old_additions && installed && !legacy {
        return Ok(());
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-hbad5073042d8 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ "SAVEPOINT assistant_grant_legacy_upgrade")?;
    if !has_start {
        // The unreleased key-only cursor cannot describe the new window order.
        // Requeue every interrupted claim rather than trusting its stale position.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-h7b614d59afc6 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty SELECT claim_id FROM assistant_grant_auth_cursor;
             DELETE FROM assistant_grant_auth_cursor;
             ALTER TABLE assistant_grant_auth_cursor ADD COLUMN membership_start INTEGER NOT NULL DEFAULT -1;"
        )?;
    }
    if old_additions {
        // Retain key/range obligations, but replace history-wide progress with
        // a fresh grant-work cursor. No committed obligation is discarded.
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hed00daccc56e owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "ALTER TABLE assistant_grant_additions ADD COLUMN claim_id TEXT NOT NULL DEFAULT '';
             ALTER TABLE assistant_grant_additions ADD COLUMN active_claim TEXT NOT NULL DEFAULT '';
             UPDATE assistant_grant_additions SET sequence=-1,envelope_hash='',position=-1;"
        )?;
    }
    if !installed {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hda7f8f715d53 owner=smallclaims::store::assistant_grant_authority TODO=prepare-body-and-predecessor-values */ "INSERT OR IGNORE INTO assistant_grant_auth_work
               SELECT origin,claim_id FROM assistant_grant_pending
               INDEXED BY assistant_grant_pending_unauthenticated_index WHERE authenticated=0;
             INSERT OR IGNORE INTO assistant_grant_auth_work
               SELECT claims.origin,cursor.claim_id FROM assistant_grant_auth_cursor AS cursor
               JOIN claims ON claims.id=cursor.claim_id;
             INSERT OR IGNORE INTO assistant_grant_auth_work
               SELECT pending.origin,work.claim_id FROM assistant_grant_proof_rechecks AS work
               JOIN assistant_grant_pending AS pending ON pending.claim_id=work.claim_id;
             INSERT INTO meta(key,value) VALUES('assistant_grant_auth_work_installed','1');"
        )?;
    }
    if legacy {
        connection.execute_batch(
            /* sqlite-large-value: allow-for-now lv-hda9098901f71 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks
             SELECT pending.claim_id FROM assistant_grant_membership_dirty AS work
             JOIN assistant_grant_pending AS pending INDEXED BY assistant_grant_pending_writer_index
               ON pending.origin=work.writer AND pending.authenticated=0
             WHERE (pending.sequence,pending.claim_id)>(work.sequence,work.claim_id);
             DROP TABLE assistant_grant_membership_dirty;"
        )?;
    }
    connection.execute_batch(/* sqlite-large-value: allow-for-now lv-h2bc6720e295e owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ "RELEASE assistant_grant_legacy_upgrade")?;
    Ok(())
}

// Owner admission order is local maintenance state, never an attacker-controlled
// content hash. Queue rows keep their existing schemas and inner page cursors.
fn install_queue_owners(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        /* sqlite-large-value: allow-for-now lv-h8523b0e4eedf owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-variable-columns */ "CREATE TABLE IF NOT EXISTS assistant_grant_queue_sequences (
           queue TEXT PRIMARY KEY, sequence INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS assistant_grant_queue_owners (
           queue TEXT NOT NULL, owner TEXT NOT NULL, subowner TEXT NOT NULL,
           sequence INTEGER NOT NULL, PRIMARY KEY(queue,owner,subowner),
           UNIQUE(queue,sequence));"
    )?;
    let installed = fleet_meta(connection,"assistant_grant_queue_owners_installed")?.is_some();
    for (queue,table,owner,subowner) in [
        ("dirty","assistant_grant_dirty","claim_id","''"),
        ("recheck","assistant_grant_proof_rechecks","claim_id","''"),
        ("invalidation","assistant_grant_proof_invalidations","id","''"),
        ("retraction","assistant_grant_retractions","id","''"),
        ("addition","assistant_grant_additions","id","''"),
        ("attachment","assistant_grant_attachments","claim_id","candidate_kind"),
        ("witness","assistant_grant_attachment_witnesses","base_claim_id","''"),
    ] {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h1b952236b766 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_queue_sequences VALUES(?1,0)", [queue],
        )?;
        let new_subowner = if subowner=="''" { "''".to_owned() } else { format!("NEW.{subowner}") };
        let old_subowner = if subowner=="''" { "''".to_owned() } else { format!("OLD.{subowner}") };
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h76abc2bf6838 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "CREATE TRIGGER IF NOT EXISTS {table}_owner_insert AFTER INSERT ON {table}
             WHEN NOT EXISTS(SELECT 1 FROM assistant_grant_queue_owners
               WHERE queue='{queue}' AND owner=CAST(NEW.{owner} AS TEXT) AND subowner={new_subowner})
             BEGIN
               UPDATE assistant_grant_queue_sequences SET sequence=sequence+1 WHERE queue='{queue}';
               INSERT INTO assistant_grant_queue_owners
                 SELECT '{queue}',CAST(NEW.{owner} AS TEXT),{new_subowner},sequence
                 FROM assistant_grant_queue_sequences WHERE queue='{queue}';
             END;
             CREATE TRIGGER IF NOT EXISTS {table}_owner_delete AFTER DELETE ON {table}
             BEGIN
               DELETE FROM assistant_grant_queue_owners WHERE queue='{queue}'
                 AND owner=CAST(OLD.{owner} AS TEXT) AND subowner={old_subowner};
             END;"
        ))?;
        if !installed {
            // Only the existing work set is indexed once on upgrade; raw claim
            // history and already-completed authority are never replayed.
            connection.execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-hffdfae208d46 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_queue_owners
                   SELECT '{queue}',CAST({owner} AS TEXT),{subowner},
                     ROW_NUMBER() OVER(ORDER BY {owner},{subowner})
                   FROM {table};
                 UPDATE assistant_grant_queue_sequences SET sequence=COALESCE(
                   (SELECT MAX(sequence) FROM assistant_grant_queue_owners WHERE queue='{queue}'),0)
                   WHERE queue='{queue}';"
            ))?;
        }
    }
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h4f48d30d177b owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key IN ('assistant_grant_dirty_cursor','assistant_grant_attachment_witness_cursor')",
        [],
    )?;
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-hdb46a61cfda0 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO meta(key,value) VALUES('assistant_grant_queue_owners_installed','1')", [],
    )?;
    Ok(())
}

fn queue_owner(connection: &Connection, queue: &str) -> Result<Option<(String,String)>> {
    let cursor_key = format!("assistant_grant_owner_cursor/{queue}");
    let upper_key = format!("assistant_grant_owner_upper/{queue}");
    let after = fleet_meta(connection,&cursor_key)?.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0);
    let upper = fleet_meta(connection,&upper_key)?.and_then(|value| value.parse::<i64>().ok()).unwrap_or(0);
    let seek = |after: i64, upper: i64| -> Result<Option<(String,String,i64)>> {
        Ok(connection.query_row(
            "SELECT owner,subowner,sequence FROM assistant_grant_queue_owners
             WHERE queue=?1 AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT 1",
            params![queue,after,upper], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?)
    };
    let candidate = match seek(after,upper)? {
        Some(candidate) => Some(candidate),
        None => {
            let upper: i64 = connection.query_row(
                "SELECT sequence FROM assistant_grant_queue_sequences WHERE queue=?1",
                [queue], |row| row.get(0),
            )?;
            let candidate = seek(0,upper)?;
            if candidate.is_some() {
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-h3f49c0e579cd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![upper_key,upper],
                )?;
            }
            candidate
        }
    };
    let Some((owner,subowner,sequence)) = candidate else { return Ok(None); };
    // The fixed round frontier excludes all later arrivals, including arrivals
    // above the current cursor. An unfinished owner resumes next round.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8760a7d75590 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", params![cursor_key,sequence],
    )?;
    Ok(Some((owner,subowner)))
}

fn install_auth_work(connection: &Connection) -> Result<()> {
    // The outer statement's conflict policy can override a trigger's OR IGNORE.
    // Work is set membership, so explicitly avoid inserting an existing claim;
    // this remains idempotent under cursor/pending UPSERTs as well as plain writes.
    for table in ["assistant_grant_pending","assistant_grant_auth_cursor","assistant_grant_proof_rechecks"] {
        for event in ["INSERT","UPDATE","DELETE"] {
            let row = if event=="DELETE" { "OLD" } else { "NEW" };
            connection.execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-h8579ec8dafea owner=smallclaims::store::assistant_grant_authority TODO=prepare-body-and-predecessor-values */ "DROP TRIGGER IF EXISTS {table}_auth_work_{event};
                 CREATE TRIGGER {table}_auth_work_{event} AFTER {event} ON {table} BEGIN
                   INSERT INTO assistant_grant_auth_work
                   SELECT COALESCE((SELECT origin FROM assistant_grant_pending WHERE claim_id={row}.claim_id),
                                   (SELECT origin FROM claims WHERE id={row}.claim_id)),{row}.claim_id
                   WHERE (EXISTS(SELECT 1 FROM assistant_grant_pending WHERE claim_id={row}.claim_id AND authenticated=0)
                      OR EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id={row}.claim_id)
                      OR EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id={row}.claim_id))
                     AND NOT EXISTS(SELECT 1 FROM assistant_grant_auth_work WHERE claim_id={row}.claim_id)
                     AND (EXISTS(SELECT 1 FROM assistant_grant_pending WHERE claim_id={row}.claim_id)
                       OR EXISTS(SELECT 1 FROM claims WHERE id={row}.claim_id));
                   DELETE FROM assistant_grant_auth_work WHERE claim_id={row}.claim_id
                     AND NOT EXISTS(SELECT 1 FROM assistant_grant_pending WHERE claim_id={row}.claim_id AND authenticated=0)
                     AND NOT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id={row}.claim_id)
                     AND NOT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id={row}.claim_id);
                 END;"
            ))?;
        }
    }
    Ok(())
}

// Destructive evidence changes are independent of membership invalidation.
// Triggers enqueue a proof identity in constant work; indexed maintenance
// visits dependent claims one at a time instead of expanding them in a trigger.
fn install_proof_invalidation_triggers(connection: &Connection) -> Result<()> {
    for (table,stem,key,index,changes) in [
        ("replica_envelope_signatures","signature","OLD.member_key",
         "assistant_grant_pending_signature_index",
         "OLD.writer IS NOT NEW.writer OR OLD.sequence IS NOT NEW.sequence OR OLD.envelope_hash IS NOT NEW.envelope_hash OR OLD.member_key IS NOT NEW.member_key OR OLD.signature IS NOT NEW.signature"),
        ("replica_envelopes","envelope","''",
         "assistant_grant_pending_envelope_index",
         "OLD.writer IS NOT NEW.writer OR OLD.sequence IS NOT NEW.sequence OR OLD.envelope_hash IS NOT NEW.envelope_hash OR (OLD.receipt_state IN ('validated','degraded') AND NEW.receipt_state NOT IN ('validated','degraded'))"),
    ] {
        let key_filter = if stem=="signature" { "AND auth_member_key=OLD.member_key" } else { "" };
        let dependent = format!(
            "SELECT 1 FROM assistant_grant_pending INDEXED BY {index}
             WHERE origin=OLD.writer AND auth_sequence=OLD.sequence
               AND auth_envelope_hash=OLD.envelope_hash {key_filter}"
        );
        for event in ["DELETE","UPDATE"] {
            let changed = if event=="UPDATE" { format!("({changes}) AND ") } else { String::new() };
            connection.execute_batch(&format!(
                /* sqlite-large-value: allow-for-now lv-ha2046fcea9c3 owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "CREATE TRIGGER IF NOT EXISTS assistant_grant_proof_{stem}_{event}
                 AFTER {event} ON {table} WHEN {changed}EXISTS({dependent})
                 BEGIN
                   INSERT INTO assistant_grant_proof_invalidations(writer,sequence,envelope_hash,member_key)
                   VALUES(OLD.writer,OLD.sequence,OLD.envelope_hash,{key});
                 END;"
            ))?;
        }
    }
    let dependent = "SELECT claim_id FROM assistant_grant_pending
        WHERE claim_id=OLD.claim_id AND origin=OLD.writer
          AND auth_sequence=OLD.sequence AND auth_envelope_hash=OLD.envelope_hash
          AND auth_record_position=OLD.position";
    for event in ["DELETE","UPDATE"] {
        let changed = if event=="UPDATE" {
            "(OLD.writer IS NOT NEW.writer OR OLD.sequence IS NOT NEW.sequence
              OR OLD.envelope_hash IS NOT NEW.envelope_hash OR OLD.position IS NOT NEW.position
              OR OLD.claim_id IS NOT NEW.claim_id
              OR (OLD.state='valid' AND NEW.state<>'valid')) AND "
        } else { "" };
        connection.execute_batch(&format!(
            /* sqlite-large-value: allow-for-now lv-h0d8e722e48bb owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "DROP TRIGGER IF EXISTS assistant_grant_proof_record_{event};
             CREATE TRIGGER assistant_grant_proof_record_{event}
             AFTER {event} ON replica_records WHEN {changed}EXISTS({dependent})
             BEGIN
               INSERT OR IGNORE INTO assistant_grant_proof_rechecks {dependent};
               DELETE FROM assistant_grant_auth_cursor WHERE claim_id IN ({dependent});
             END;"
        ))?;
    }
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
            OR EXISTS(SELECT 1 FROM assistant_grant_retractions)
            OR EXISTS(SELECT 1 FROM assistant_grant_additions)
            OR EXISTS(SELECT 1 FROM assistant_grant_recompute)
            OR EXISTS(SELECT 1 FROM assistant_grant_attachments)
            OR EXISTS(SELECT 1 FROM assistant_grant_attachment_witnesses)
            OR EXISTS(SELECT 1 FROM assistant_grant_proof_invalidations)
            OR EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks)",
        [VERSION], |row| row.get(0),
    ).map_err(internal)?;
    Ok(pending)
}
// Keep the subject lookup outermost even when the global work queues are large.
// Cost still grows with this subject's claims and matching proof obligations.
pub const SUBJECT_PENDING_QUERY: &str =
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_attachments WHERE grant_subject=?1)
            OR EXISTS(SELECT 1 FROM assistant_grant_attachment_witnesses WHERE grant_subject=?1)
            OR EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_index
                CROSS JOIN assistant_grant_dirty AS dirty
                WHERE claims.subject=?1 AND dirty.claim_id=claims.id)
            OR EXISTS(SELECT 1 FROM assistant_grant_pending AS pending INDEXED BY assistant_grant_pending_subject_index
                CROSS JOIN assistant_grant_dirty AS dirty
                WHERE pending.grant_subject=?1 AND dirty.claim_id=pending.claim_id)
            OR EXISTS(SELECT 1 FROM claims INDEXED BY claims_subject_index
                CROSS JOIN assistant_grant_auth_cursor AS cursor
                WHERE claims.subject=?1 AND cursor.claim_id=claims.id)
            OR EXISTS(SELECT 1 FROM assistant_grant_pending
                WHERE grant_subject=?1 AND authenticated=0)
            OR EXISTS(SELECT 1 FROM assistant_grant_pending AS pending INDEXED BY assistant_grant_pending_subject_index
                CROSS JOIN assistant_grant_recompute AS work
                WHERE pending.grant_subject=?1 AND work.claim_id=pending.claim_id)
            OR EXISTS(SELECT 1 FROM assistant_grant_pending AS pending INDEXED BY assistant_grant_pending_subject_index
                CROSS JOIN assistant_grant_proof_rechecks AS work
                WHERE pending.grant_subject=?1 AND work.claim_id=pending.claim_id)
            OR EXISTS(SELECT 1 FROM assistant_grant_pending AS pending INDEXED BY assistant_grant_pending_subject_index
                CROSS JOIN assistant_grant_proof_invalidations AS work INDEXED BY assistant_grant_proof_invalidations_identity_index
                WHERE work.writer=pending.origin AND work.sequence=pending.auth_sequence
                    AND work.envelope_hash=pending.auth_envelope_hash
                  AND pending.grant_subject=?1 AND pending.claim_id>work.claim_id
                  AND (work.member_key='' OR work.member_key=pending.auth_member_key))
            OR EXISTS(SELECT 1 FROM assistant_grant_pending AS pending INDEXED BY assistant_grant_pending_subject_index
                CROSS JOIN assistant_grant_retractions AS dirty INDEXED BY assistant_grant_retractions_writer_key_index
                ON dirty.writer=pending.origin AND dirty.member_key=pending.auth_member_key
                WHERE pending.grant_subject=?1
                  AND pending.auth_sequence BETWEEN dirty.lower_sequence AND dirty.upper_sequence
                  AND (pending.auth_sequence,pending.claim_id)>(dirty.sequence,dirty.claim_id)
                  AND NOT EXISTS(SELECT 1 FROM assistant_grant_membership_windows AS windows
                    WHERE windows.writer=pending.origin AND windows.member_key=pending.auth_member_key
                      AND pending.auth_sequence>=windows.start
                      AND (windows.end IS NULL OR pending.auth_sequence<=windows.end)))";
fn subject_pending(connection: &Connection, subject: &str) -> Result<bool, FractalError> {
    connection.query_row(/* sqlite-large-value: allow-for-now lv-h648b9952e737 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ SUBJECT_PENDING_QUERY, [subject], |row| row.get(0)).map_err(internal)
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

/// Management evidence only. Callers must authenticate the authoritative owner;
/// this must never be used to authorize a bearer while its subject is fenced.
pub fn management_latest(connection: &Connection, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
    let record: Option<String> = connection.query_row(
        "SELECT record FROM assistant_grant_authority WHERE grant_subject=?1 AND kind=?2 ORDER BY store_index DESC LIMIT 1",
        params![subject,kind], |row| row.get(0)).optional().map_err(internal)?;
    record.map(decode).transpose()
}

pub const MANAGEMENT_REVOKE_QUERY: &str = "SELECT record FROM assistant_grant_authority
    INDEXED BY assistant_grant_authority_management_revoke_index
    WHERE grant_subject=?1 AND kind=?2 AND origin=?3 AND actor=?4 AND principal=?5 AND profile=?6
      AND json_extract(record,'$.body.fields.grant_id') IS ?7
      AND json_extract(record,'$.body.fields.issuer') IS ?8
    ORDER BY store_index DESC LIMIT 1";
pub const MANAGEMENT_POLICY_QUERY: &str = "SELECT record FROM assistant_grant_authority
    INDEXED BY assistant_grant_authority_management_policy_index
    WHERE grant_subject=?1 AND kind=?2 AND origin=?3 AND actor=?4 AND principal=?5 AND profile=?6
      AND json_extract(record,'$.body.fields.grant_id') IS ?7
      AND json_extract(record,'$.body.fields.issuer') IS ?8
      AND json_extract(record,'$.body.fields.profile') IS ?9
      AND json_extract(record,'$.body.fields.session_actor') IS ?10
    ORDER BY store_index DESC LIMIT 1";
/// Owner management selects one applicable authenticated transition independently
/// of reverse attachment. Every immutable identity component precedes ordering
/// in the index, so foreign or unrelated history never becomes a scanned prefix.
pub fn management_transition(connection: &Connection, granted: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
    let fields = &granted.body["fields"];
    let record: Option<String> = if kind==REVOKED {
        connection.query_row(/* sqlite-large-value: allow-for-now lv-h0acc69f486f8 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ MANAGEMENT_REVOKE_QUERY,
            params![granted.subject,kind,granted.origin,granted.actor,principal(fields),profile(fields),
                fields["grant_id"].as_str(),fields["issuer"].as_str()],
            |row| row.get(0)).optional()
    } else if kind==POLICY {
        connection.query_row(/* sqlite-large-value: allow-for-now lv-ha81fbb31a83b owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ MANAGEMENT_POLICY_QUERY,
            params![granted.subject,kind,granted.origin,granted.actor,principal(fields),profile(fields),
                fields["grant_id"].as_str(),fields["issuer"].as_str(),
                fields["profile"].as_str(),fields["session_actor"].as_str()],
            |row| row.get(0)).optional()
    } else {
        return Err(FractalError::new("invalid-input","unsupported owner management transition kind"));
    }.map_err(internal)?;
    record.map(decode).transpose()
}
pub const CANDIDATES_QUERY: &str = "SELECT grant_record FROM assistant_grant_effective
    WHERE ?3 IS NOT NULL AND principal=?1 AND owner=?2 AND revoked=0 AND expires_at>?3
      AND (?4 IS NULL OR claim_id<>?4) AND (?5 IS NULL OR profile<>?5)
    UNION ALL
    SELECT grant_record FROM assistant_grant_effective
    WHERE ?3 IS NULL AND principal=?1 AND owner=?2
      AND (?4 IS NULL OR claim_id<>?4) AND (?5 IS NULL OR profile<>?5)
    UNION
    SELECT witness.grant_record FROM assistant_grant_attachment_witnesses AS witness
    JOIN assistant_grant_pending AS pending ON pending.claim_id=witness.base_claim_id
    WHERE witness.principal=?1 AND witness.owner=?2
      AND (?3 IS NULL OR (witness.revoked=0 AND witness.expires_at>?3))
      AND (?4 IS NULL OR witness.base_claim_id<>?4) AND (?5 IS NULL OR witness.profile<>?5)
      AND pending.authenticated=1
      AND EXISTS(SELECT 1 FROM assistant_grant_attachments WHERE grant_subject=witness.grant_subject)";
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
// Full reconstruction is a test oracle operation, never maintenance work.
#[cfg(test)]
pub fn rebuild_subject(connection: &Connection, subject: &str) -> Result<()> {
    connection.execute("DELETE FROM assistant_grant_effective WHERE grant_subject=?1", [subject])?;
    let mut statement = connection.prepare_cached(
        "SELECT claim_id FROM assistant_grant_authority WHERE grant_subject=?1 AND kind=?2")?;
    let bases = statement.query_map(params![subject,GRANTED], |row| row.get::<_,String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for base in bases { recompute_base(connection,&base)?; }
    Ok(())
}

// One base PK lookup and at most two indexed latest-transition seeks. Neither
// another base nor an earlier policy/revoke is replayed during recomputation.
fn recompute_base(connection: &Connection, base_id: &str) -> Result<()> {
    connection.execute(/* sqlite-large-value: allow-for-now lv-h67e21448c6af owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_effective WHERE claim_id=?1", [base_id])?;
    let record: Option<String> = connection.query_row(
        "SELECT authority.record FROM assistant_grant_authority AS authority
         JOIN assistant_grant_pending AS pending ON pending.claim_id=authority.claim_id
         WHERE authority.claim_id=?1 AND authority.base_claim_id=?1 AND pending.authenticated=1",
        [base_id], |row| row.get(0)).optional()?;
    let Some(record) = record else { return Ok(()); };
    let grant: ClaimRecord = serde_json::from_str(&record)?;
    let policy_claim = transition_unchecked(connection,&grant,POLICY)?;
    let policy = policy_claim.as_ref().unwrap_or(&grant);
    if !structurally_valid(&grant,policy) {
        // The final repairing policy disappeared. The raw base remains pending,
        // but must no longer mask a valid older credential in authority lookup.
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hd5498a2a9ed1 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_authority WHERE claim_id=?1 AND base_claim_id=?1",
            [base_id])?;
        return Ok(());
    }
    let fields = &policy.body["fields"];
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h3f4f98505546 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_effective(claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![grant.id,grant.subject,principal(fields),grant.actor,profile(fields),
            expiry_key(fields["expires_at_unix_ms"].as_u64().unwrap_or(0)),
            transition_unchecked(connection,&grant,REVOKED)?.is_some(),record])?;
    Ok(())
}
// Authentication creates constant-sized state; opposite-side history is queued.
fn attach_claim(connection: &Connection, claim: &ClaimRecord) -> Result<()> {
    if claim.kind==GRANTED && structurally_valid(claim,claim)
        && insert_authority(connection,claim,claim)? {
        recompute_base(connection,&claim.id)?;
    }
    let kinds: &[&str] = if claim.kind==GRANTED { &[POLICY,REVOKED] } else { &[GRANTED] };
    for kind in kinds {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h8f01e3faea8f owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_attachments
             (claim_id,candidate_kind,grant_subject,origin,actor,after_claim_id,upper_claim_id)
             SELECT ?1,?2,?3,?4,?5,'',claim_id FROM assistant_grant_pending
             INDEXED BY assistant_grant_pending_attachment_index
             WHERE grant_subject=?3 AND authenticated=1 AND origin=?4 AND actor=?5 AND kind=?2
             ORDER BY claim_id DESC LIMIT 1",
            params![claim.id,kind,claim.subject,claim.origin,claim.actor],
        )?;
    }
    Ok(())
}

fn attachment_one(connection: &Connection) -> Result<bool> {
    let Some((owner,partition)) = queue_owner(connection,"attachment")? else { return Ok(false); };
    attachment_task_one(connection, &owner, &partition)
}

fn attachment_task_one(connection: &Connection, owner: &str, partition: &str) -> Result<bool> {
    let task: Option<(String,String,String,String,String,String,String)> = connection.query_row(
        "SELECT claim_id,candidate_kind,grant_subject,origin,actor,after_claim_id,upper_claim_id
         FROM assistant_grant_attachments WHERE claim_id=?1 AND candidate_kind=?2", params![owner,partition],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).optional()?;
    let Some((id,kind,subject,origin,actor,after,upper)) = task else { return Ok(false); };
    let record: Option<String> = connection.query_row(
        "SELECT record FROM assistant_grant_pending WHERE claim_id=?1 AND authenticated=1",
        [&id], |row| row.get(0),
    ).optional()?;
    let candidate: Option<(String,String)> = if record.is_some() {
        connection.query_row(
            "SELECT claim_id,record FROM assistant_grant_pending
             INDEXED BY assistant_grant_pending_attachment_index
             WHERE grant_subject=?1 AND authenticated=1 AND origin=?2 AND actor=?3 AND kind=?4
               AND claim_id>?5 AND claim_id<=?6 ORDER BY claim_id LIMIT 1",
            params![subject,origin,actor,kind,after,upper],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?
    } else { None };
    let Some((candidate_id,candidate)) = candidate else {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h0545b8ffe4b6 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_attachments WHERE claim_id=?1 AND candidate_kind=?2", params![id,kind],
        )?;
        return Ok(true);
    };
    let claim: ClaimRecord = serde_json::from_str(&record.expect("authenticated task owner"))?;
    let candidate: ClaimRecord = serde_json::from_str(&candidate)?;
    let (base,transition) = if claim.kind==GRANTED { (&claim,&candidate) } else { (&candidate,&claim) };
    // Both endpoint rows were re-read as authenticated in this transaction.
    // Demotion precedes attachment and cancels the owner's tasks.
    if transition_valid(base,transition) {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h7e9db0ae826a owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_attachment_witnesses
             SELECT claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record
             FROM assistant_grant_effective WHERE claim_id=?1", [&base.id],
        )?;
        let mut changed = false;
        if transition.kind==POLICY { changed |= insert_authority(connection,base,base)?; }
        changed |= insert_authority(connection,transition,base)?;
        if changed { recompute_base(connection,&base.id)?; }
    }
    // Invalid candidates advance and consume exactly the same unit.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h8fb933e31076 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_attachments SET after_claim_id=?3 WHERE claim_id=?1 AND candidate_kind=?2",
        params![id,kind,candidate_id],
    )?;
    Ok(true)
}

// A local transition has only the GRANTED partition. Visit one candidate, then
// probe only for completion: never consume a second candidate in this request.
// The earlier newest-base fast path plus this unit insert at most two pairs.
fn finish_local_transition(connection: &Connection, claim: &ClaimRecord) -> Result<()> {
    attachment_task_one(connection, &claim.id, GRANTED)?;
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h45c096d84c6c owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_attachments
         WHERE claim_id=?1 AND candidate_kind=?2
           AND NOT EXISTS(SELECT 1 FROM assistant_grant_pending AS candidate
               INDEXED BY assistant_grant_pending_attachment_index
               WHERE candidate.grant_subject=assistant_grant_attachments.grant_subject
                 AND candidate.authenticated=1 AND candidate.origin=assistant_grant_attachments.origin
                 AND candidate.actor=assistant_grant_attachments.actor AND candidate.kind=?2
                 AND candidate.claim_id>assistant_grant_attachments.after_claim_id
                 AND candidate.claim_id<=assistant_grant_attachments.upper_claim_id)",
        params![claim.id, GRANTED],
    )?;
    // At most two base witnesses were touched above. Only finished subjects
    // permit cleanup; unrelated tasks and proof/dirty fences are never removed.
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h133b802d1e9c owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_attachment_witnesses WHERE base_claim_id IN (
             SELECT base_claim_id FROM assistant_grant_attachment_witnesses
             WHERE grant_subject=?1
               AND NOT EXISTS(SELECT 1 FROM assistant_grant_attachments WHERE grant_subject=?1)
             LIMIT 2)",
        [&claim.subject],
    )?;
    Ok(())
}

fn attachment_witness_one(connection: &Connection) -> Result<bool> {
    let Some((base,_)) = queue_owner(connection,"witness")? else { return Ok(false); };
    let subject: String = connection.query_row(
        "SELECT grant_subject FROM assistant_grant_attachment_witnesses WHERE base_claim_id=?1",
        [&base], |row| row.get(0),
    )?;
    let pending: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_attachments WHERE grant_subject=?1)",
        [&subject], |row| row.get(0),
    )?;
    if !pending {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h65210e524ade owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_attachment_witnesses WHERE base_claim_id=?1", [&base])?;
    }
    Ok(true)
}
fn local_origin(connection: &Connection) -> Result<String> {
    Ok(connection.query_row("SELECT value FROM meta WHERE key='assistant_grant_authority_local_origin'", [], |row| row.get(0))?)
}
/// Admission pays structural/authentication cost once. Invalid rows never enter
/// the authority or the pending-authentication writer/window index.
pub fn refresh_claim(connection: &Connection, id: &str) -> Result<()> {
    // Receipt boundary: remote first-authentication runs in maintenance after
    // generation validation; this path has no local request work allowance.
    refresh_claim_inner(connection, id, false)
}

/// Only the actual append transaction may spend the separate bounded request
/// allowance; maintenance refreshes retain their one-unit slice accounting.
pub fn refresh_local_append(connection: &Connection, id: &str) -> Result<()> {
    refresh_claim_with_authentication(connection,id,false,None,true)
}

fn refresh_claim_inner(connection: &Connection, id: &str, membership_current: bool) -> Result<()> {
    refresh_claim_with_authentication(connection,id,membership_current,None,false)
}

#[derive(Default)]
struct PreviousAuthentication {
    admitted: bool,
    key: Option<String>,
    sequence: Option<u64>,
    hash: Option<String>,
    position: Option<i64>,
}

fn refresh_claim_with_authentication(connection: &Connection, id: &str,
    membership_current: bool, verified: Option<Authentication>, local_append_transaction: bool) -> Result<()> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(||
        format!("SELECT {CLAIM_COLUMNS} FROM claims WHERE id=?1"));
    let claim = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h1665a4a8cfdf owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ &QUERY)?.query_row(/* sqlite-large-value: allow-for-now lv-h5520ddeb5d20 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ [id], claim_from_row).optional()?;
    let Some(claim) = claim else {
        // Trim can remove a receipt and its raw claim in one transaction.
        // Retire cached state through bounded demotion, not a stuck dirty row.
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h28df63bc590a owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "UPDATE assistant_grant_pending SET authenticated=0,auth_member_key=NULL,
             auth_sequence=NULL,auth_envelope_hash=NULL,auth_record_position=NULL WHERE claim_id=?1", [id],
        )?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h018f62ad3faf owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_attachments WHERE claim_id=?1", [id])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h7ab907abb853 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_recompute
             SELECT claim_id FROM assistant_grant_pending WHERE claim_id=?1", [id],
        )?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h1a4864d9f6c8 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h3221720b83b7 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1", [id])?;
        return Ok(());
    };
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
        let previous: Option<PreviousAuthentication> = connection.query_row(
            "SELECT authenticated,auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position
             FROM assistant_grant_pending WHERE claim_id=?1",
            [&claim.id], |row| Ok(PreviousAuthentication {
                admitted: row.get(0)?, key: row.get(1)?, sequence: row.get(2)?,
                hash: row.get(3)?, position: row.get(4)?,
            }),
        ).optional()?;
        let PreviousAuthentication { admitted, key: previous_key, sequence: previous_sequence,
            hash: previous_hash, position: previous_position } = previous.unwrap_or_default();
        // Trust depends on precise evidence, never monotonic admission. Retraction
        // tasks target that dependency; local append is a separate durable fact.
        let authentication = if let Some(verified) = verified {
            verified
        } else if local_append && claim.origin == local_origin(connection)? {
            Authentication::Authenticated(None)
        } else if membership_current {
            authenticated_claim(connection,&claim.id)?
        } else if admitted {
            Authentication::Authenticated(previous_key.clone().zip(previous_sequence)
                .zip(previous_hash.clone()).zip(previous_position)
                .map(|(((key,sequence),hash),position)| (key,sequence,hash,position)))
        } else {
            Authentication::Pending
        };
        let (authenticated, proof_key, proof_sequence, proof_hash, proof_position) = match &authentication {
            Authentication::Authenticated(proof) => (true,proof.as_ref().map(|p|p.0.clone()),
                proof.as_ref().map(|p|p.1),proof.as_ref().map(|p|p.2.clone()),proof.as_ref().map(|p|p.3)),
            Authentication::Unauthenticated => (false,None,None,None,None),
            Authentication::Pending => (admitted,previous_key,previous_sequence,previous_hash,previous_position),
        };
        let changed = connection.execute(/* sqlite-large-value: allow-for-now lv-h5e92793f2a03 owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_pending(claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record,auth_member_key,auth_sequence,auth_envelope_hash,auth_record_position)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(claim_id) DO UPDATE SET
              authenticated=excluded.authenticated,auth_member_key=excluded.auth_member_key,auth_sequence=excluded.auth_sequence,
              auth_envelope_hash=excluded.auth_envelope_hash,auth_record_position=excluded.auth_record_position
            WHERE assistant_grant_pending.authenticated<>excluded.authenticated
               OR assistant_grant_pending.auth_member_key IS NOT excluded.auth_member_key
               OR assistant_grant_pending.auth_sequence IS NOT excluded.auth_sequence
               OR assistant_grant_pending.auth_envelope_hash IS NOT excluded.auth_envelope_hash
               OR assistant_grant_pending.auth_record_position IS NOT excluded.auth_record_position",
            params![claim.id,claim.subject,claim.kind,claim.origin,claim.actor,sequence,authenticated,serde_json::to_string(&claim)?,proof_key,proof_sequence,proof_hash,proof_position])? != 0;
        // One local transition materializes its newest base pair for management.
        // Its own bounded attachment unit below also completes the usual
        // single-base case without imposing a needless bearer fence.
        if changed && !admitted && authenticated && local_append_transaction && local_append && claim.kind!=GRANTED
            && let Some(base) = management_latest(connection,&claim.subject,GRANTED)?
            && transition_valid(&base,&claim) {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h41fad2bc67c1 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_attachment_witnesses
                 SELECT claim_id,grant_subject,principal,owner,profile,expires_at,revoked,grant_record
                 FROM assistant_grant_effective WHERE claim_id=?1", [&base.id],
            )?;
            if insert_authority(connection,&claim,&base)? {
                recompute_base(connection,&base.id)?;
            }
        }
        if changed && !admitted && authenticated {
            attach_claim(connection,&claim)?;
            if local_append_transaction && local_append && claim.kind!=GRANTED {
                finish_local_transition(connection,&claim)?;
            }
        }
        if admitted && !authenticated { rebuild_authenticated_subject(connection,&claim)?; }
        if matches!(authentication, Authentication::Pending) {
            connection.execute(/* sqlite-large-value: allow-for-now lv-he3fa452add20 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_dirty VALUES(?1)", [id])?;
            return Ok(());
        }
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-hd91341d01a91 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1", [id])?;
    Ok(())
}

// Rebuild both bases and transitions: losing a policy may invalidate a repaired
// malformed base, and losing a revoke must produce exactly fresh-backfill state.
// A still-pending unauthenticated transition continues to fence bearer use.
fn rebuild_authenticated_subject(connection: &Connection, claim: &ClaimRecord) -> Result<()> {
    // Fanout is durable work, not a subject-history replay in this claim's unit.
    // At most two kind partitions belong to an owner; cancellation is constant.
    connection.execute(/* sqlite-large-value: allow-for-now lv-h5ff6179b5135 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_attachments WHERE claim_id=?1", [&claim.id])?;
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h53b6a1e0835c owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_recompute(claim_id) VALUES(?1)",
        [&claim.id])?;
    Ok(())
}

// Consume exactly one authority pair (or one completed task). The queue must
// drain before authenticating another claim, so a later readmission cannot race
// deletion of the previous admission's dependent rows.
fn recompute_one(connection: &Connection) -> Result<bool> {
    let claim_id: Option<String> = connection.query_row(
        "SELECT claim_id FROM assistant_grant_recompute ORDER BY claim_id LIMIT 1",
        [], |row| row.get(0)).optional()?;
    let Some(claim_id) = claim_id else { return Ok(false); };
    let pair: Option<(String,String)> = connection.query_row(
        "SELECT claim_id,base_claim_id FROM
            (SELECT claim_id,base_claim_id FROM assistant_grant_authority WHERE claim_id=?1 LIMIT 1)
         UNION ALL
         SELECT claim_id,base_claim_id FROM
            (SELECT claim_id,base_claim_id FROM assistant_grant_authority
             INDEXED BY assistant_grant_authority_base_index WHERE base_claim_id=?1 LIMIT 1)
         LIMIT 1",
        [&claim_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((dependent,base)) = pair {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h67b7b4921e3a owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_authority WHERE claim_id=?1 AND base_claim_id=?2",
            params![dependent,base])?;
        recompute_base(connection,&base)?;
    } else {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h981cb88354da owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_recompute WHERE claim_id=?1", [&claim_id])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h9ab29a0fbe52 owner=smallclaims::store::assistant_grant_authority TODO=prepare-body-and-predecessor-values */ "DELETE FROM assistant_grant_pending WHERE claim_id=?1
             AND NOT EXISTS(SELECT 1 FROM claims WHERE id=?1)", [&claim_id],
        )?;
    }
    Ok(true)
}

enum Authentication {
    Pending,
    Unauthenticated,
    // None denotes durable local append. Writer is the claim's validated origin.
    Authenticated(Option<(String,u64,String,i64)>),
}

#[cfg(test)]
thread_local! { static AUTHENTICATION_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
pub fn take_authentication_attempts() -> usize {
    AUTHENTICATION_ATTEMPTS.with(|count| count.replace(0))
}
#[cfg(test)]
thread_local! { static AUTHENTICATION_CLAIMS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) }; }
#[cfg(test)]
pub fn take_authentication_claims() -> Vec<String> {
    AUTHENTICATION_CLAIMS.with(|claims| std::mem::take(&mut *claims.borrow_mut()))
}
/// One record and at most sixteen membership keys per refresh. Nonmember
/// signatures are never enumerated: every signature lookup supplies its full PK.
/// Persist progress even through missing/invalid evidence, so a later valid key
/// or record cannot be hidden behind an arbitrary number of earlier candidates.
fn authenticated_claim(connection: &Connection, id: &str) -> Result<Authentication> {
    #[cfg(test)]
    AUTHENTICATION_ATTEMPTS.with(|count| count.set(count.get()+1));
    #[cfg(test)]
    AUTHENTICATION_CLAIMS.with(|claims| claims.borrow_mut().push(id.to_owned()));
    let Some(fleet_id) = fleet_meta(connection,"assistant_grant_authority_fleet_id")? else {
        return Ok(Authentication::Unauthenticated);
    };
    let shard = fleet_meta(connection,"shard_id")?;
    let cursor: Option<(i64,i64,String,i64)> = connection.query_row(
        "SELECT position,record_rowid,member_key,membership_start FROM assistant_grant_auth_cursor WHERE claim_id=?1",
        [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    let initial = cursor.is_none();
    let (position_cursor,rowid_cursor,key_cursor,start_cursor) =
        cursor.unwrap_or((i64::MIN,i64::MIN,String::new(),-1));
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
        connection.execute(/* sqlite-large-value: allow-for-now lv-hdb10afe3ddca owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
        return Ok(Authentication::Unauthenticated);
    };
    // A nonempty key resumes this record; an empty key advances past it.
    // The index seek skips at most the one completed record at the cursor.
    let mut next_key = String::new();
    let mut next_start = -1;
    if eligible {
        let mut statement = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-hf53f8e0e2462 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ AUTHENTICATION_KEYS_QUERY)?;
        let keys = statement.query_map(
            params![writer,sequence,hash,
                if (position,record_rowid)==(position_cursor,rowid_cursor) { start_cursor } else { -1 },
                if (position,record_rowid)==(position_cursor,rowid_cursor) { key_cursor.as_str() } else { "" },
                MEMBERSHIP_KEYS_PER_REFRESH],
            |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?,
                row.get::<_,i64>(2)?,row.get::<_,Option<u64>>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key,signature,_,end) in &keys {
            // Expired windows consume a candidate slot too: filtering them in
            // SQL would permit unbounded scans before reaching the page limit.
            if end.is_none_or(|end| sequence<=end) && signature.as_deref().is_some_and(|signature|
                VerifiedEnvelopeSignature::verify(shard.as_deref(),&fleet_id,&writer,sequence,&hash,key,signature).is_some()) {
                connection.execute(/* sqlite-large-value: allow-for-now lv-h773f1bab1a89 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
                return Ok(Authentication::Authenticated(Some((key.clone(),sequence,hash,position))));
            }
        }
        if keys.len()==MEMBERSHIP_KEYS_PER_REFRESH {
            let (key,_,start,_) = keys.into_iter().last().expect("full key page");
            next_key = key;
            next_start = start;
        }
    }
    if next_key.is_empty() {
        let more: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM replica_records INDEXED BY replica_records_claim
             WHERE claim_id=?1 AND (position,rowid)>(?2,?3))",
            params![id,position,record_rowid], |row| row.get(0))?;
        if !more {
            connection.execute(/* sqlite-large-value: allow-for-now lv-h0a908802271b owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [id])?;
            return Ok(Authentication::Unauthenticated);
        }
    }
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h87d2d3e94117 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_auth_cursor(claim_id,position,record_rowid,member_key,membership_start)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(claim_id) DO UPDATE SET position=excluded.position,
            record_rowid=excluded.record_rowid,member_key=excluded.member_key,
            membership_start=excluded.membership_start",
        params![id,position,record_rowid,next_key,next_start])?;
    Ok(Authentication::Pending)
}
pub const AUTHENTICATION_KEYS_QUERY: &str =
    "SELECT windows.member_key,signatures.signature,windows.start,windows.end
     FROM assistant_grant_membership_windows AS windows INDEXED BY assistant_grant_membership_windows_seek
     LEFT JOIN replica_envelope_signatures AS signatures
       ON signatures.writer=windows.writer AND signatures.sequence=?2
      AND signatures.envelope_hash=?3 AND signatures.member_key=windows.member_key
     WHERE windows.writer=?1 AND windows.start<=?2
       AND (windows.start,windows.member_key)>(?4,?5)
     ORDER BY windows.start,windows.member_key LIMIT ?6";
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
        connection.execute(/* sqlite-large-value: allow-for-now lv-h188a54cc0c91 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&id])?;
        refresh_claim(connection,&id)?;
    }
    Ok(())
}

// Inclusive membership bounds; an absent end denotes a current incarnation.
type MembershipWindow = (u64, Option<u64>);

struct MembershipUpdate {
    generation: i64,
    fleet_id: Option<String>,
    // Only changed/deleted keys, never a replacement of the complete snapshot.
    windows: Vec<(String, String, Option<MembershipWindow>)>,
    complete: bool,
}

impl MembershipUpdate {
    fn prepare(connection: &Connection, generation: i64, fleet_id: Option<String>,
        membership: &crate::fleet::Membership, limit: usize) -> Result<Self> {
        let desired: BTreeMap<_,_> = membership.incarnations().map(|incarnation| (
            (incarnation.name.clone(),incarnation.member_key.clone()),
            (incarnation.start,incarnation.end),
        )).collect();
        let current: BTreeMap<_,_> = connection.prepare(
            "SELECT writer,member_key,start,end FROM assistant_grant_membership_windows",
        )?.query_map([], |row| Ok(((row.get::<_,String>(0)?,row.get::<_,String>(1)?),
                                  (row.get::<_,u64>(2)?,row.get::<_,Option<u64>>(3)?))))?
            .collect::<rusqlite::Result<_>>()?;
        let mut windows = Vec::with_capacity(limit);
        let mut complete = true;
        for (identity,window) in &desired {
            if current.get(identity)==Some(window) { continue; }
            if windows.len()==limit { complete=false; break; }
            windows.push((identity.0.clone(),identity.1.clone(),Some(*window)));
        }
        if complete {
            for identity in current.keys() {
                if desired.contains_key(identity) { continue; }
                if windows.len()==limit { complete=false; break; }
                windows.push((identity.0.clone(),identity.1.clone(),None));
            }
        }
        Ok(Self { generation, fleet_id, windows, complete })
    }

    fn apply(&self, connection: &Connection) -> Result<usize> {
        for (writer,key,window) in &self.windows {
            let old: Option<MembershipWindow> = connection.query_row(
                "SELECT start,end FROM assistant_grant_membership_windows WHERE writer=?1 AND member_key=?2",
                params![writer,key], |row| Ok((row.get(0)?,row.get(1)?)),
            ).optional()?;
            // Additions cannot invalidate a proof. Contractions queue only the
            // recorded key's lost ranges; actual verification can use other keys.
            // Legitimate removal costs O(rows authenticated by that key), with
            // bounded transactions and those subjects fenced until rechecked.
            // Only an authenticated fleet member authorized to remove members
            // can trigger this membership cost; nonmember relay noise cannot.
            if let Some((old_start,old_end)) = old {
                let enqueue = |lower: u64, upper: u64| -> Result<()> {
                    if lower<=upper {
                        connection.execute(
                            /* sqlite-large-value: allow-for-now lv-h63ca01e24d8b owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions(writer,member_key,lower_sequence,upper_sequence)
                             SELECT ?1,?2,?3,?4 WHERE EXISTS(
                               SELECT 1 FROM assistant_grant_pending INDEXED BY assistant_grant_pending_evidence_index
                               WHERE origin=?1 AND auth_member_key=?2 AND auth_sequence BETWEEN ?3 AND ?4)",
                            params![writer,key,lower,upper],
                        )?;
                    }
                    Ok(())
                };
                let old_end = old_end.unwrap_or(i64::MAX as u64);
                match window {
                    None => enqueue(old_start,old_end)?,
                    Some((start,end)) => {
                        if *start>old_start { enqueue(old_start,old_end.min(start-1))?; }
                        if let Some(end) = end && *end<old_end {
                            enqueue(old_start.max(end+1),old_end)?;
                        }
                    }
                }
            }
            if let Some((start,end)) = window {
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-h1941e256906a owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows VALUES(?1,?2,?3,?4)
                     ON CONFLICT(writer,member_key) DO UPDATE SET start=excluded.start,end=excluded.end",
                    params![writer,key,start,end],
                )?;
            } else {
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-h177538cdd4af owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_membership_windows WHERE writer=?1 AND member_key=?2",
                    params![writer,key],
                )?;
            }
            if let Some((start,end)) = window {
                let enqueue = |lower: u64, upper: u64| -> Result<()> {
                    // Snapshot the current receipt frontier. Future-only windows
                    // cannot help existing rows and must not create retry work.
                    connection.execute(
                        /* sqlite-large-value: allow-for-now lv-h2fc5feb42670 owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO assistant_grant_additions(writer,member_key,lower_sequence,upper_sequence)
                         SELECT ?1,?2,?3,MIN(?4,sequence) FROM replica_records
                         WHERE writer=?1 AND sequence>=?3 ORDER BY sequence DESC LIMIT 1",
                        params![writer,key,lower,upper],
                    )?;
                    Ok(())
                };
                let end = end.unwrap_or(i64::MAX as u64);
                if let Some((old_start,old_end)) = old {
                    if *start<old_start { enqueue(*start,end.min(old_start-1))?; }
                    let old_end = old_end.unwrap_or(i64::MAX as u64);
                    if end>old_end { enqueue((*start).max(old_end+1),end)?; }
                } else {
                    enqueue(*start,end)?;
                }
            }
        }
        if self.complete {
            if let Some(fleet_id) = &self.fleet_id {
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-h0ce05c43b4a2 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_fleet_id',?1)",
                    [fleet_id],
                )?;
            } else {
                connection.execute(/* sqlite-large-value: allow-for-now lv-ha3bce0201694 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='assistant_grant_authority_fleet_id'", [])?;
            }
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h86c509920fc9 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_membership_state SET applied_generation=?1 WHERE id=1",
                [self.generation],
            )?;
        }
        Ok(self.windows.len())
    }
}

/// Prepare a real default-store sealed snapshot without opening or upgrading a
/// Store. Raw-connection fixtures use the same preparation/application as
/// production maintenance; this never fabricates membership or local signers.
#[cfg(any(test, feature = "test-support"))]
pub fn prepare_membership_for_test(connection: &Connection) -> Result<bool> {
    let transaction = connection.unchecked_transaction()?;
    let generation = fleet_generation(&transaction)?;
    let membership = fleet_membership_tx(&transaction)?;
    let fleet_id = fleet_meta(&transaction,"fleet_id")?;
    let update = MembershipUpdate::prepare(&transaction,generation,fleet_id,&membership,CLAIMS_PER_SLICE)?;
    update.apply(&transaction)?;
    transaction.commit()?;
    Ok(update.complete)
}
/// Resumable, idempotent versioned backfill. Each page uses claims_kind_index;
/// unrelated claims are never visited. Cursor and projection changes commit
/// together in the caller's transaction. Old-build appends and signature-only
/// arrivals are captured by persistent dirty triggers even after completion.
/// This raw driver cannot prepare membership: membership-dependent queues wait
/// for a durable verified snapshot matching `fleet_generation`. Startup calls it once; use
/// `Store::advance_assistant_grant_authority_backfill` for repeated maintenance,
/// which prepares the real sealed membership view before advancing these queues.
pub fn backfill_slice(connection: &Connection, budget: Duration) -> Result<bool> {
    let applied: i64 = connection.query_row(
        "SELECT applied_generation FROM assistant_grant_membership_state WHERE id=1", [], |row| row.get(0),
    )?;
    backfill_slice_with_limit(connection, budget, CLAIMS_PER_SLICE, fleet_generation(connection)?==applied)
}

// A destructive signature/envelope write queues one proof identity, not its
// history-sized dependent set. Indexed dispatch and real authentication consume
// separate shared budget units. Only the verifier may discharge the latter.
fn proof_invalidation_one(connection: &Connection) -> Result<bool> {
    let Some((owner,_)) = queue_owner(connection,"invalidation")? else { return Ok(false); };
    let task: Option<(i64,String,u64,String,String,String)> = connection.query_row(
        "SELECT id,writer,sequence,envelope_hash,member_key,claim_id
         FROM assistant_grant_proof_invalidations WHERE id=?1", [&owner],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
    ).optional()?;
    let Some((task,writer,sequence,hash,key,cursor)) = task else { return Ok(false); };
    let query = if key.is_empty() {
        "SELECT claim_id FROM assistant_grant_pending INDEXED BY assistant_grant_pending_envelope_index
         WHERE origin=?1 AND auth_sequence=?2 AND auth_envelope_hash=?3 AND claim_id>?5
         ORDER BY claim_id LIMIT 1"
    } else {
        "SELECT claim_id FROM assistant_grant_pending INDEXED BY assistant_grant_pending_signature_index
         WHERE origin=?1 AND auth_sequence=?2 AND auth_envelope_hash=?3
           AND auth_member_key=?4 AND claim_id>?5 ORDER BY claim_id LIMIT 1"
    };
    let claim: Option<String> = connection.query_row(/* sqlite-large-value: allow-for-now lv-h8247508e87c0 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ query,params![writer,sequence,hash,key,cursor],
        |row| row.get(0)).optional()?;
    if let Some(claim) = claim {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h921f15324dcd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&claim])?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-hfc49ec7f7720 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks VALUES(?1)", [&claim])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hcd8ada722bbd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_proof_invalidations SET claim_id=?2 WHERE id=?1",
            params![task,claim],
        )?;
    } else {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h83f6fdabc1cb owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_invalidations WHERE id=?1", [task])?;
    }
    Ok(true)
}

struct RetractionRange<'a> {
    task: i64,
    writer: &'a str,
    key: &'a str,
    lower: u64,
    upper: u64,
    sequence: i64,
    claim: &'a str,
}

// Coverage restoration immediately exempts covered dependencies from fencing.
// Lazily subtract its intersection from one queued range per budget unit, so
// cancellation itself never expands into an unbounded writer transaction.
fn trim_restored_retraction(connection: &Connection, range: RetractionRange<'_>) -> Result<bool> {
    let RetractionRange { task, writer, key, lower, upper, sequence, claim } = range;
    let window: Option<MembershipWindow> = connection.query_row(
        "SELECT start,end FROM assistant_grant_membership_windows WHERE writer=?1 AND member_key=?2",
        params![writer,key], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let Some((start,end)) = window else { return Ok(false); };
    let covered_lower = lower.max(start);
    let covered_upper = upper.min(end.unwrap_or(i64::MAX as u64));
    if covered_lower>covered_upper { return Ok(false); }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h4e21f68dabd5 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_retractions WHERE id=?1", [task])?;
    for remainder in [
        (lower<covered_lower).then(|| (lower,covered_lower-1)),
        (covered_upper<upper).then(|| (covered_upper+1,upper)),
    ].into_iter().flatten() {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hd746bcc76346 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_retractions
             (writer,member_key,lower_sequence,upper_sequence,sequence,claim_id)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![writer,key,remainder.0,remainder.1,sequence,claim],
        )?;
    }
    Ok(true)
}

struct AdditionTask {
    task: i64,
    writer: String,
    key: String,
    lower: u64,
    upper: u64,
    sequence: i64,
    hash: String,
    position: i64,
    after: String,
    claim: String,
}

// Each addition seeks only indexed grant work, then that grant's own receipts.
// A claim selection, receipt verification or completed claim costs one unit.
fn addition_one(connection: &Connection) -> Result<bool> {
    let Some((owner,_)) = queue_owner(connection,"addition")? else { return Ok(false); };
    let task: Option<AdditionTask> = connection.query_row(
        "SELECT id,writer,member_key,lower_sequence,upper_sequence,sequence,envelope_hash,position,claim_id,active_claim
         FROM assistant_grant_additions WHERE id=?1", [&owner],
        |row| Ok(AdditionTask {
            task: row.get(0)?, writer: row.get(1)?, key: row.get(2)?,
            lower: row.get(3)?, upper: row.get(4)?, sequence: row.get(5)?,
            hash: row.get(6)?, position: row.get(7)?, after: row.get(8)?, claim: row.get(9)?,
        }),
    ).optional()?;
    let Some(AdditionTask { task, writer, key, lower, upper, sequence, hash, position,
        after, claim }) = task else { return Ok(false); };
    if claim.is_empty() {
        let next: Option<String> = connection.query_row(
            "SELECT claim_id FROM assistant_grant_auth_work WHERE writer=?1 AND claim_id>?2 ORDER BY claim_id LIMIT 1",
            params![writer,after], |row| row.get(0),
        ).optional()?;
        if let Some(next) = next {
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-h73bc9ba2f84a owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "UPDATE assistant_grant_additions SET active_claim=?2,sequence=-1,envelope_hash='',position=-1 WHERE id=?1",
                params![task,next],
            )?;
        } else {
            connection.execute(/* sqlite-large-value: allow-for-now lv-h4bcc00c86e41 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_additions WHERE id=?1", [task])?;
        }
        return Ok(true);
    }
    let record: Option<(u64,String,i64,bool)> = connection.query_row(
        "SELECT sequence,envelope_hash,position,state='valid' FROM replica_records INDEXED BY replica_records_writer_grant_proof_index
         WHERE claim_id=?1 AND writer=?2 AND sequence BETWEEN ?3 AND ?4
           AND (sequence,envelope_hash,position)>(?5,?6,?7)
           AND EXISTS(SELECT 1 FROM assistant_grant_auth_work WHERE writer=?2 AND claim_id=?1)
         ORDER BY sequence,envelope_hash,position LIMIT 1",
        params![claim,writer,lower,upper,sequence,hash,position],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    let Some((sequence,hash,position,valid)) = record else {
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-hb39485ba125b owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_additions SET claim_id=active_claim,active_claim='' WHERE id=?1", [task],
        )?;
        return Ok(true);
    };
    if valid {
        let signature: Option<String> = connection.query_row(
            "SELECT signatures.signature FROM replica_envelope_signatures AS signatures
             WHERE signatures.writer=?1 AND signatures.sequence=?2
               AND signatures.envelope_hash=?3 AND signatures.member_key=?4
               AND EXISTS(SELECT 1 FROM claims WHERE id=?5 AND origin=?1)
               AND EXISTS(SELECT 1 FROM assistant_grant_pending AS pending
                 WHERE pending.claim_id=?5 AND pending.origin=?1
                   AND (pending.authenticated=0
                     OR EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?5)
                     OR EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?5)))
               AND EXISTS(SELECT 1 FROM assistant_grant_membership_windows
                 WHERE writer=?1 AND member_key=?4 AND ?2>=start AND (end IS NULL OR ?2<=end))
               AND EXISTS(SELECT 1 FROM replica_envelopes WHERE writer=?1 AND sequence=?2
                 AND envelope_hash=?3 AND receipt_state IN ('validated','degraded'))",
            params![writer,sequence,hash,key,claim], |row| row.get(0),
        ).optional()?;
        if let Some(signature) = signature {
            let fleet_id = fleet_meta(connection,"assistant_grant_authority_fleet_id")?;
            let shard = fleet_meta(connection,"shard_id")?;
            if fleet_id.as_deref().is_some_and(|fleet_id|
                VerifiedEnvelopeSignature::verify(shard.as_deref(),fleet_id,&writer,sequence,&hash,&key,&signature).is_some()) {
                refresh_claim_with_authentication(connection,&claim,true,
                    Some(Authentication::Authenticated(Some((key,sequence,hash.clone(),position)))),false)?;
                connection.execute(/* sqlite-large-value: allow-for-now lv-h4e1ea146a20c owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&claim])?;
                connection.execute(/* sqlite-large-value: allow-for-now lv-h1e384e782172 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_rechecks WHERE claim_id=?1", [&claim])?;
            }
        }
    }
    connection.execute(
        /* sqlite-large-value: allow-for-now lv-h9bbb6621cf95 owner=smallclaims::store::assistant_grant_authority TODO=chunk-or-reference-exact-wire-values */ "UPDATE assistant_grant_additions SET sequence=?2,envelope_hash=?3,position=?4 WHERE id=?1",
        params![task,sequence,hash,position],
    )?;
    Ok(true)
}

// Both dispatch queues invoke the same owning verifier, including when a
// one-unit slice encounters a dirty claim before reaching the recheck queue.
fn refresh_recheck(connection: &Connection, claim: &str) -> Result<bool> {
    refresh_claim_inner(connection,claim,true)?;
    let unfinished: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_auth_cursor WHERE claim_id=?1)
             OR EXISTS(SELECT 1 FROM assistant_grant_dirty WHERE claim_id=?1)",
        [claim], |row| row.get(0),
    )?;
    if !unfinished {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h5a430ec6f457 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_proof_rechecks WHERE claim_id=?1", [claim])?;
    }
    Ok(!unfinished)
}

fn dirty_one(connection: &Connection, membership_current: bool) -> Result<bool> {
    let Some((claim,_)) = queue_owner(connection,"dirty")? else { return Ok(false); };
    let recheck_owned: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks WHERE claim_id=?1)",
        [&claim], |row| row.get(0),
    )?;
    if recheck_owned {
        if membership_current { refresh_recheck(connection,&claim)?; }
    } else {
        refresh_claim_inner(connection,&claim,membership_current)?;
    }
    Ok(true)
}

fn recheck_one(connection: &Connection) -> Result<bool> {
    let Some((claim,_)) = queue_owner(connection,"recheck")? else { return Ok(false); };
    refresh_recheck(connection,&claim)?;
    Ok(true)
}

fn retraction_one(connection: &Connection) -> Result<bool> {
    let Some((owner,_)) = queue_owner(connection,"retraction")? else { return Ok(false); };
    let task: Option<(i64,String,String,u64,u64,i64,String)> = connection.query_row(
        "SELECT id,writer,member_key,lower_sequence,upper_sequence,sequence,claim_id
         FROM assistant_grant_retractions WHERE id=?1", [&owner],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
    ).optional()?;
    let Some((task,writer,key,lower,upper,sequence,id)) = task else { return Ok(false); };
    if trim_restored_retraction(connection, RetractionRange {
        task, writer: &writer, key: &key, lower, upper, sequence, claim: &id,
    })? {
        return Ok(true);
    }
    let pending: Option<(u64,String)> = connection.query_row(
        "SELECT auth_sequence,claim_id FROM assistant_grant_pending INDEXED BY assistant_grant_pending_evidence_index
         WHERE origin=?1 AND auth_member_key=?2 AND auth_sequence BETWEEN ?3 AND ?4
           AND (auth_sequence,claim_id)>(?5,?6)
         ORDER BY auth_sequence,claim_id LIMIT 1",
        params![writer,key,lower,upper,sequence,id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    if let Some((sequence,id)) = pending {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h2daf7a592c21 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_auth_cursor WHERE claim_id=?1", [&id])?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-hbc290dd6ae0d owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO assistant_grant_proof_rechecks VALUES(?1)", [&id])?;
        connection.execute(
            /* sqlite-large-value: allow-for-now lv-h7be65771e125 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "UPDATE assistant_grant_retractions SET sequence=?2,claim_id=?3 WHERE id=?1",
            params![task,sequence,id],
        )?;
    } else {
        connection.execute(/* sqlite-large-value: allow-for-now lv-hc1e17d6d240e owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_retractions WHERE id=?1", [task])?;
    }
    Ok(true)
}

fn history_one(connection: &Connection, membership_current: bool) -> Result<bool> {
    // A completed version never reopens historical scanning.
    if fleet_meta(connection,"assistant_grant_authority_version")?.as_deref()==Some(VERSION) {
        return Ok(false);
    }
    for kind in KINDS {
        let key = format!("assistant_grant_authority_cursor/{kind}");
        let cursor: u64 = connection.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",
            [&key], |row| row.get(0),
        ).optional()?.unwrap_or(0);
        let claim: Option<(String,u64)> = connection.query_row(
            "SELECT id,store_index FROM claims INDEXED BY claims_kind_index
             WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT 1",
            params![kind,cursor], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        if let Some((id,index)) = claim {
            refresh_claim_inner(connection,&id,membership_current)?;
            connection.execute(
                /* sqlite-large-value: allow-for-now lv-hba2ee169759a owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */
                "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
                params![key,index.to_string()],
            )?;
            return Ok(true);
        }
    }
    Ok(false)
}

const MAINTENANCE_QUEUES: usize = 8;

fn maintenance_one(connection: &Connection, turn: usize, membership_current: bool) -> Result<bool> {
    match turn {
        0 => dirty_one(connection,membership_current),
        1 => if membership_current { recheck_one(connection) } else { Ok(false) },
        2 => proof_invalidation_one(connection),
        3 => retraction_one(connection),
        4 => if membership_current { addition_one(connection) } else { Ok(false) },
        5 => if membership_current { attachment_one(connection) } else { Ok(false) },
        6 => if membership_current { attachment_witness_one(connection) } else { Ok(false) },
        7 => history_one(connection,membership_current),
        _ => unreachable!("maintenance turn is reduced modulo queue count"),
    }
}

fn backfill_slice_with_limit(connection: &Connection, budget: Duration, mut remaining: usize, membership_current: bool) -> Result<bool> {
    let started = Instant::now();
    if remaining==0 { return Ok(false); }
    let mut turn = fleet_meta(connection,"assistant_grant_maintenance_turn")?
        .and_then(|value| value.parse::<usize>().ok()).unwrap_or(0)%MAINTENANCE_QUEUES;
    while remaining>0 {
        // Demotion only shrinks authority. Finish its bounded units before any
        // positive work; all other queues share one persistent round-robin turn.
        let mut visited = recompute_one(connection)?;
        if !visited {
            for _ in 0..MAINTENANCE_QUEUES {
                visited = maintenance_one(connection,turn,membership_current)?;
                turn = (turn+1)%MAINTENANCE_QUEUES;
                if visited { break; }
            }
            if visited {
                connection.execute(
                    /* sqlite-large-value: allow-for-now lv-hedcbf9131f97 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_maintenance_turn',?1)",
                    [turn as i64],
                )?;
            }
        }
        if !visited { break; }
        remaining -= 1;
        // Persisted turn already points past this unit, even for ZERO slices.
        if remaining==0 || started.elapsed()>=budget { return Ok(false); }
    }
    let dirty: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_grant_dirty)
             OR EXISTS(SELECT 1 FROM assistant_grant_retractions)
             OR EXISTS(SELECT 1 FROM assistant_grant_additions)
             OR EXISTS(SELECT 1 FROM assistant_grant_recompute)
             OR EXISTS(SELECT 1 FROM assistant_grant_attachments)
             OR EXISTS(SELECT 1 FROM assistant_grant_attachment_witnesses)
             OR EXISTS(SELECT 1 FROM assistant_grant_proof_invalidations)
             OR EXISTS(SELECT 1 FROM assistant_grant_proof_rechecks)", [], |row| row.get(0))?;
    if dirty { return Ok(false); }
    if fleet_meta(connection, "assistant_grant_authority_version")?.as_deref() != Some(VERSION) {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h9b70e51e754e owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_version',?1)",[VERSION])?;
    }
    Ok(true)
}

impl Store {
    fn assistant_grant_membership_generation(&self) -> Result<i64> {
        if let Some(authority) = self.fleet_authority.get() {
            return authority.fleet_generation();
        }
        fleet_generation(&self.readers.get())
    }

    pub fn assistant_grant_authority_maintenance_pending(&self) -> Result<bool> {
        if maintenance_pending(&self.readers.get()).map_err(anyhow::Error::new)? {
            return Ok(true);
        }
        Ok(self.assistant_grant_membership_generation()? !=
            self.assistant_grant_membership_observed_generation.load(Ordering::Acquire))
    }

    // Reuse the inherited sealed-view invalidation contract for changes to the
    // projected writer/key/start/end windows, including delayed bootstrap proofs.
    // Shard bounds do not affect those tuples (their generation triggers cover
    // INSERT/DELETE, not UPDATE). Reader-fold amplification remains #6124; do
    // not mirror membership invalidation in another trigger/revision.
    // Only actual changed windows acquire the writer; equal snapshots update an
    // in-memory observation, without SQL writes or authentication cursor resets.
    fn prepare_assistant_grant_membership(&self, force: bool, limit: usize) -> Result<Option<MembershipUpdate>> {
        let generation = self.assistant_grant_membership_generation()?;
        if !force && generation==self.assistant_grant_membership_observed_generation.load(Ordering::Acquire) {
            return Ok(None);
        }
        let connection = self.readers.get();
        let membership = self.fleet_membership_sealed()?;
        let fleet_id = if let Some(authority) = self.fleet_authority.get() {
            authority.fleet_id()?
        } else {
            fleet_meta(&connection, "fleet_id")?
        };
        Ok(Some(MembershipUpdate::prepare(&connection,generation,fleet_id,&membership,limit)?))
    }

    /// Background work only; receipt transactions never fold fleet history.
    pub fn advance_assistant_grant_authority_backfill(&self) -> Result<bool> {
        self.advance_assistant_grant_authority_with_budget(Duration::from_millis(100),CLAIMS_PER_SLICE)
    }

    // The raw-connection driver cannot see Store's deliberately write-free
    // in-memory membership observation. Persist a real sealed snapshot before
    // fixtures hand control to that driver; never synthesize authentication.
    #[cfg(test)]
    pub(super) fn persist_assistant_grant_membership_for_test(&self) -> Result<()> {
        loop {
            let update = self.prepare_assistant_grant_membership(true,CLAIMS_PER_SLICE)?
                .expect("forced membership preparation always returns its snapshot");
            let mut connection = self.connection.write_background();
            let transaction = connection.transaction()?;
            let generation = if let Some(authority) = self.fleet_authority.get() {
                authority.fleet_generation()?
            } else {
                fleet_generation(&transaction)?
            };
            if generation!=update.generation { continue; }
            update.apply(&transaction)?;
            transaction.commit()?;
            if update.complete {
                self.assistant_grant_membership_observed_generation.store(generation,Ordering::Release);
                return Ok(());
            }
        }
    }

    #[cfg(test)]
    pub(super) fn advance_assistant_grant_authority_backfill_for_test(&self, budget: Duration, units: usize) -> Result<bool> {
        assert!(units>0 && units<=CLAIMS_PER_SLICE);
        self.advance_assistant_grant_authority_with_budget(budget,units)
    }

    fn advance_assistant_grant_authority_with_budget(&self, budget: Duration, units: usize) -> Result<bool> {
        let prepared = self.prepare_assistant_grant_membership(false,units)?;
        let generation = prepared.as_ref().map_or_else(
            || self.assistant_grant_membership_observed_generation.load(Ordering::Acquire),
            |update| update.generation,
        );
        let claim_work = maintenance_pending(&self.readers.get()).map_err(anyhow::Error::new)?;
        if !claim_work && prepared.as_ref().is_none_or(|update| update.windows.is_empty()) {
            if self.assistant_grant_membership_generation()? != generation { return Ok(false); }
            self.assistant_grant_membership_observed_generation.store(generation,Ordering::Release);
            return Ok(true);
        }
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        let current_generation = if let Some(authority) = self.fleet_authority.get() {
            authority.fleet_generation()?
        } else {
            fleet_generation(&transaction)?
        };
        if current_generation != generation { return Ok(false); }
        let used = if let Some(update) = &prepared {
            update.apply(&transaction)?
        } else { 0 };
        let membership_current = prepared.as_ref().is_none_or(|update| update.complete);
        let done = backfill_slice_with_limit(
            &transaction, budget, units-used, membership_current,
        )?;
        transaction.commit()?;
        if membership_current {
            self.assistant_grant_membership_observed_generation.store(generation,Ordering::Release);
        }
        Ok(done && membership_current)
    }
}
