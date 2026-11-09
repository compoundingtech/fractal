//! Incremental, authenticated external-grant authority. Raw claims remain evidence only.
use super::*;
use std::time::{Duration, Instant};

pub const GRANTED: &str = "custom.client.assistant.v1.granted";
pub const POLICY: &str = "custom.client.assistant.v1.policy";
pub const REVOKED: &str = "custom.client.assistant.v1.revoked";
const VERSION: &str = "1";
const KINDS: [&str; 3] = [GRANTED, POLICY, REVOKED];
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
         CREATE TRIGGER IF NOT EXISTS assistant_grant_dirty_insert AFTER INSERT ON claims
         WHEN NEW.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked')
         BEGIN INSERT OR IGNORE INTO assistant_grant_dirty VALUES(NEW.id); END;
         CREATE TRIGGER IF NOT EXISTS assistant_grant_dirty_signature_insert
         AFTER INSERT ON replica_envelope_signatures
         BEGIN
            INSERT OR IGNORE INTO assistant_grant_dirty(claim_id)
            SELECT records.claim_id FROM replica_records AS records
            JOIN claims ON claims.id=records.claim_id
            WHERE records.writer=NEW.writer AND records.sequence=NEW.sequence
              AND records.envelope_hash=NEW.envelope_hash
              AND claims.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked');
         END;
         CREATE TABLE IF NOT EXISTS assistant_grant_membership_windows (
            writer TEXT NOT NULL, member_key TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER,
            PRIMARY KEY(writer,member_key));"
    )?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h3f117dab1bdd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_local_origin',?1)", [local_origin])?;
    Ok(())
}

pub fn ready(connection: &Connection) -> Result<(), FractalError> {
    let complete: bool = connection.query_row(
        "SELECT COALESCE((SELECT value FROM meta WHERE key='assistant_grant_authority_version'),'')=?1
            AND NOT EXISTS(SELECT 1 FROM assistant_grant_dirty)",
        [VERSION], |row| row.get(0),
    ).map_err(internal)?;
    if !complete {
        return Err(FractalError::new("assistant-grant-authority-not-ready", "external-grant authority backfill is in progress; retry after it completes"));
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
    ready(connection)?;
    transition_unchecked(connection, granted, kind).map_err(internal)
}
fn transition_unchecked(connection: &Connection, granted: &ClaimRecord, kind: &str) -> Result<Option<ClaimRecord>> {
    let record: Option<String> = connection.query_row(/* sqlite-large-value: allow-for-now lv-h80fb9d4725e0 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ TRANSITION_QUERY,
        params![granted.subject, kind, granted.origin, granted.actor, granted.id], |row| row.get(0)).optional()?;
    record.map(|record| serde_json::from_str(&record).map_err(Into::into)).transpose()
}
pub fn latest(connection: &Connection, subject: &str, kind: &str) -> Result<Option<ClaimRecord>, FractalError> {
    ready(connection)?;
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
    record.map(decode).transpose()
}
pub fn contains(connection: &Connection, claim_id: &str) -> Result<bool, FractalError> {
    ready(connection)?;
    connection.query_row("SELECT EXISTS(SELECT 1 FROM assistant_grant_authority WHERE claim_id=?1)", [claim_id], |row| row.get(0)).map_err(internal)
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
pub fn refresh_claim(connection: &Connection, id: &str, trusted_local: bool) -> Result<()> {
    static QUERY: std::sync::LazyLock<String> = std::sync::LazyLock::new(||
        format!("SELECT {CLAIM_COLUMNS} FROM claims WHERE id=?1"));
    let claim = connection.prepare_cached(/* sqlite-large-value: allow-for-now lv-h1665a4a8cfdf owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ &QUERY)?.query_row(/* sqlite-large-value: allow-for-now lv-h5520ddeb5d20 owner=smallclaims::store::assistant_grant_authority TODO=trace-statement-and-value-representation */ [id], claim_from_row).optional()?;
    let Some(claim) = claim else { return Ok(()); };
    if !KINDS.contains(&claim.kind.as_str()) { return Ok(()); }
    let valid = if claim.kind==GRANTED { base_valid(&claim) }
        else if claim.kind==POLICY { policy_fields(&claim) } else { identity(&claim) };
    if valid {
        let sequence: u64 = connection.query_row("SELECT replica_sequence FROM batches WHERE id=?1", [&claim.batch_id], |row| row.get(0))?;
        let authenticated = (trusted_local && claim.origin == local_origin(connection)?)
            || authenticated_claim(connection,&claim.id)?;
        let changed = connection.execute(/* sqlite-large-value: allow-for-now lv-h9cf86ad98d1e owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_pending(claim_id,grant_subject,kind,origin,actor,sequence,authenticated,record)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(claim_id) DO UPDATE SET authenticated=excluded.authenticated
            WHERE assistant_grant_pending.authenticated<excluded.authenticated",
            params![claim.id,claim.subject,claim.kind,claim.origin,claim.actor,sequence,authenticated,serde_json::to_string(&claim)?])? != 0;
        if changed && authenticated { attach_claim(connection,&claim)?; }
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-h3221720b83b7 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1", [id])?;
    Ok(())
}
fn authenticated_claim(connection: &Connection, id: &str) -> Result<bool> {
    let fleet_id = fleet_meta(connection,"assistant_grant_authority_fleet_id")?;
    let Some(fleet_id) = fleet_id else { return Ok(false); };
    let shard = fleet_meta(connection,"shard_id")?;
    let mut statement = connection.prepare_cached(
        "SELECT records.writer,records.sequence,records.envelope_hash,signatures.member_key,signatures.signature
         FROM replica_records AS records JOIN claims ON claims.id=records.claim_id AND claims.origin=records.writer
         JOIN replica_envelopes AS envelopes USING(writer,sequence,envelope_hash)
         JOIN replica_envelope_signatures AS signatures USING(writer,sequence,envelope_hash)
         JOIN assistant_grant_membership_windows AS windows
           ON windows.writer=records.writer AND windows.member_key=signatures.member_key
          AND records.sequence>=windows.start AND (windows.end IS NULL OR records.sequence<=windows.end)
         WHERE records.claim_id=?1 AND records.state='valid' AND envelopes.receipt_state IN ('validated','degraded')")?;
    let rows = statement.query_map([id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?)))?;
    for row in rows {
        let (writer,sequence,hash,key,signature) = row?;
        if VerifiedEnvelopeSignature::verify(shard.as_deref(),&fleet_id,&writer,sequence,&hash,&key,&signature).is_some() { return Ok(true); }
    }
    Ok(false)
}
/// Refresh only this envelope's grant records, after receipt/signature state has
/// been written, and before its transaction is committed.
pub fn refresh_envelope(connection: &Connection, writer: &str, sequence: u64, hash: &str) -> Result<()> {
    refresh_membership(connection)?;
    let ids = {
        let mut statement = connection.prepare_cached("SELECT records.claim_id FROM replica_records AS records JOIN claims ON claims.id=records.claim_id
            WHERE records.writer=?1 AND records.sequence=?2 AND records.envelope_hash=?3
              AND claims.kind IN ('custom.client.assistant.v1.granted','custom.client.assistant.v1.policy','custom.client.assistant.v1.revoked')")?;
        statement.query_map(params![writer,sequence,hash], |row| row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    // A replicated same-origin claim does NOT acquire the local-write exception.
    for id in ids { refresh_claim(connection,&id,false)?; }
    Ok(())
}
/// Membership changes promote pending rows only in newly changed keyed windows.
/// Authentication is an admission fact, not a live membership predicate: a later
/// removal/fence cannot retract authentication already established for a past
/// sequence. It can only prevent a pending row from first acquiring it.
pub fn refresh_membership(connection: &Connection) -> Result<()> {
    if fleet_meta(connection,"assistant_grant_authority_shared_fleet")?.is_some() {
        return Ok(());
    }
    let generation = fleet_generation(connection)?;
    let previous: Option<i64> = connection.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key='assistant_grant_authority_membership_generation'",[],|row| row.get(0)).optional()?;
    if previous==Some(generation) { return Ok(()); }
    refresh_membership_from(connection, &fleet_membership_tx(connection)?, generation, fleet_meta(connection,"fleet_id")?.as_deref())?;
    Ok(())
}

fn refresh_membership_from(connection: &Connection, membership: &crate::fleet::Membership, generation: i64, fleet_id: Option<&str>) -> Result<()> {
    let mut changed = Vec::new();
    for incarnation in membership.incarnations() {
        let prior: Option<(u64,Option<u64>)> = connection.query_row(
            "SELECT start,end FROM assistant_grant_membership_windows WHERE writer=?1 AND member_key=?2",
            params![incarnation.name,incarnation.member_key], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if prior!=Some((incarnation.start,incarnation.end)) {
            changed.push((incarnation.name.clone(),incarnation.start,incarnation.end));
        }
    }
    // Replace the window snapshot before promotions, including removal of keys
    // no longer present in a conflicted membership fold.
    connection.execute(/* sqlite-large-value: allow-for-now lv-hd11f3b7cf006 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_membership_windows", [])?;
    for incarnation in membership.incarnations() {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h679dd6776599 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO assistant_grant_membership_windows(writer,member_key,start,end) VALUES(?1,?2,?3,?4)",
            params![incarnation.name,incarnation.member_key,incarnation.start,incarnation.end])?;
    }
    if let Some(fleet_id) = fleet_id {
        connection.execute(/* sqlite-large-value: allow-for-now lv-h0ce05c43b4a2 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_fleet_id',?1)",[fleet_id])?;
    } else {
        connection.execute(/* sqlite-large-value: allow-for-now lv-ha3bce0201694 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM meta WHERE key='assistant_grant_authority_fleet_id'", [])?;
    }
    connection.execute(/* sqlite-large-value: allow-for-now lv-hbcecffaa18c4 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_membership_generation',?1)",[generation.to_string()])?;
    for (writer,start,end) in changed {
        let mut cursor = (0_u64, String::new());
        loop {
            let ids = {
                let mut statement = connection.prepare_cached(
                    "SELECT sequence,claim_id FROM assistant_grant_pending INDEXED BY assistant_grant_pending_writer_index
                     WHERE origin=?1 AND authenticated=0 AND sequence>=?2
                       AND (?3 IS NULL OR sequence<=?3) AND (sequence,claim_id)>(?4,?5)
                     ORDER BY sequence,claim_id LIMIT 64")?;
                statement.query_map(params![writer,start,end,cursor.0,cursor.1],
                    |row| Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            if ids.is_empty() { break; }
            for (sequence,id) in ids {
                refresh_claim(connection,&id,false)?;
                cursor = (sequence,id);
            }
        }
    }
    Ok(())
}
/// Resumable, idempotent versioned backfill. Each page uses claims_kind_index;
/// unrelated claims are never visited. Cursor and projection changes commit
/// together in the caller's transaction. Old-build appends and signature-only
/// arrivals are captured by persistent dirty triggers even after completion.
pub fn backfill_slice(connection: &Connection, budget: Duration) -> Result<bool> {
    let started = Instant::now();
    let origin = local_origin(connection)?;
    refresh_membership(connection)?;
    // A completed version only drains the old-build dirty queue. Readiness also
    // includes that queue, but must never cause a completed history rescan.
    if fleet_meta(connection,"assistant_grant_authority_version")?.as_deref() != Some(VERSION) {
        for kind in KINDS {
            let key = format!("assistant_grant_authority_cursor/{kind}");
            loop {
                let cursor: u64 = connection.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key=?1",[&key],|row|row.get(0)).optional()?.unwrap_or(0);
                let rows = {
                    let mut statement = connection.prepare_cached("SELECT id,store_index,origin FROM claims INDEXED BY claims_kind_index WHERE kind=?1 AND store_index>?2 ORDER BY store_index LIMIT 64")?;
                    statement.query_map(params![kind,cursor],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u64>(1)?,row.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
                };
                if rows.is_empty() { break; }
                for (id,index,writer) in rows {
                    refresh_claim(connection,&id,writer==origin)?;
                    connection.execute(/* sqlite-large-value: allow-for-now lv-h3f49c0e579cd owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",params![key,index.to_string()])?;
                    if started.elapsed()>=budget { return Ok(false); }
                }
            }
        }
    }
    loop {
        let rows = {
            let mut statement = connection.prepare_cached("SELECT claim_id FROM assistant_grant_dirty ORDER BY claim_id LIMIT 64")?;
            statement.query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() { break; }
        for id in rows {
            let writer: Option<String> = connection.query_row("SELECT origin FROM claims WHERE id=?1",[&id],|row|row.get(0)).optional()?;
            refresh_claim(connection,&id,writer.as_deref()==Some(origin.as_str()))?;
            connection.execute(/* sqlite-large-value: allow-for-now lv-hd91341d01a91 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "DELETE FROM assistant_grant_dirty WHERE claim_id=?1",[&id])?;
            if started.elapsed()>=budget { return Ok(false); }
        }
    }
    refresh_membership(connection)?;
    connection.execute(/* sqlite-large-value: allow-for-now lv-h9b70e51e754e owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_version',?1)",[VERSION])?;
    Ok(true)
}

impl Store {
    /// Background work only; request handlers never call this method.
    pub fn advance_assistant_grant_authority_backfill(&self) -> Result<bool> {
        let mut connection = self.connection.write_background();
        let transaction = connection.transaction()?;
        self.sync_assistant_grant_membership(&transaction)?;
        let done = backfill_slice(&transaction,Duration::from_millis(100))?;
        transaction.commit()?;
        Ok(done)
    }

    pub(super) fn sync_assistant_grant_membership(&self, connection: &Connection) -> Result<()> {
        let Some(authority) = self.fleet_authority.get() else {
            return refresh_membership(connection);
        };
        let generation = authority.fleet_generation()?;
        let previous: Option<i64> = connection.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='assistant_grant_authority_membership_generation'", [], |row| row.get(0),
        ).optional()?;
        if previous==Some(generation) && fleet_meta(connection,"assistant_grant_authority_shared_fleet")?.is_some() {
            return Ok(());
        }
        let membership = authority.fleet_membership_sealed()?;
        connection.execute(/* sqlite-large-value: allow-for-now lv-h437f9d145151 owner=smallclaims::store::assistant_grant_authority TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO meta(key,value) VALUES('assistant_grant_authority_shared_fleet','1')", [])?;
        refresh_membership_from(connection,&membership,generation,authority.fleet_id()?.as_deref())
    }
}
