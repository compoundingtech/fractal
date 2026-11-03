//! Private workspaces are derived directly from durable claims. No cache is authority.
use super::*;
use fractal_schema::glasses::MAX_GLASSES;

pub fn glass_claims(connection: &Connection, person: &str, through: u64) -> Result<Vec<ClaimRecord>> {
    let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-h75174e01c7b9 owner=fractal_store::store::glasses TODO=trace-statement-and-value-representation */ &canonical_sql(
        "WITH ranked AS (
            SELECT claims.id, ROW_NUMBER() OVER (PARTITION BY claims.subject, claims.kind ORDER BY CANONICAL_ASC(claims)) AS first,
                ROW_NUMBER() OVER (PARTITION BY claims.subject, claims.kind ORDER BY CANONICAL_DESC(claims)) AS last
            FROM claims WHERE claims.kind IN ('glass.upserted','glass.deleted') AND claims.store_index<=?2
                AND substr(claims.subject,1,length(?1))=?1
        )
        SELECT claims.id, claims.store_index, claims.batch_id, claims.subject, claims.kind, claims.origin, claims.actor, claims.body, claims.predecessors, claims.accepted_at_unix_ms
        FROM claims JOIN ranked ON ranked.id=claims.id WHERE ranked.first=1 OR ranked.last=1
        ORDER BY CANONICAL_ASC(claims)"))?;
    Ok(statement
        .query_map(
            params![format!("glass/{person}/"), through.min(i64::MAX as u64)],
            claim_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Deleted IDs stay retired even when an offline writer sends a later edit. For quota,
/// earliest-created live IDs win the available slots. Overflow is retained, and becomes
/// visible when a slot opens. Updating a glass cannot reorder its creation priority.
pub fn live_claims(claims: Vec<ClaimRecord>) -> Vec<ClaimRecord> {
    let mut heads = BTreeMap::new();
    let mut created = Vec::new();
    let mut deleted = BTreeSet::new();
    for claim in claims {
        if claim.kind == "glass.deleted" {
            deleted.insert(claim.subject.clone());
        } else {
            if !heads.contains_key(&claim.subject) {
                created.push(claim.subject.clone());
            }
            heads.insert(claim.subject.clone(), claim);
        }
    }
    created
        .into_iter()
        .filter(|id| !deleted.contains(id))
        .take(MAX_GLASSES)
        .filter_map(|id| heads.remove(&id))
        .collect()
}

pub fn resource(claim: &ClaimRecord) -> Result<Value> {
    let body = fractal_schema::glasses::body_for_read(&claim.body["fields"]["body"])?;
    Ok(
        json!({"id":claim.subject, "kind":"glass", "revision":claim.id,
        "body":body, "deleted":false,
        "base_revision":claim.body["fields"]["base_revision"],
        "replaced_revision":claim.body["fields"]["replaced_revision"],
        "updated_at":chrono::DateTime::from_timestamp_millis(i64::try_from(claim.accepted_at_unix_ms).unwrap_or(i64::MAX)).unwrap_or(chrono::DateTime::UNIX_EPOCH).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)}),
    )
}

pub fn glasses_at(
    connection: &Connection,
    person: &str,
    through: u64,
) -> Result<Vec<Value>> {
    let claims = match glass_heads::current_claims(connection, person, through)? {
        Some(claims) => claims,
        None => live_claims(glass_claims(connection, person, through)?),
    };
    let mut result: Vec<_> = claims
        .iter()
        .map(resource)
        .collect::<Result<_>>()?;
    result.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    Ok(result)
}
/// Called under the claim writer's transaction, after idempotency has been resolved.
/// Never validate quotas at replication admission: that would depend on arrival order.
pub fn prepare(
    transaction: &Transaction<'_>,
    input: &ClaimInput,
) -> Result<Option<BTreeMap<String, Value>>, St3Error> {
    if !input.subject.starts_with("glass/") {
        return Ok(None);
    }
    let person =
        fractal_schema::glasses::owner(&input.subject).map_err(|e| St3Error::new(e.code, e.message))?;
    glass_heads::flush(transaction).map_err(internal)?;
    let head = glass_heads::head(transaction, &input.subject).map_err(internal)?;
    if head.as_ref().is_some_and(|head| head.deleted) {
        return Err(St3Error::new(
            "glass-deleted",
            "a deleted glass ID cannot be reused",
        ));
    }
    let head = head.as_ref().and_then(|head| head.claim_id.as_deref());
    if input.kind == "glass.deleted" && head.is_none() {
        return Err(St3Error::new("not-found", "the glass does not exist"));
    }
    if input.kind == "glass.upserted" && head.is_none() {
        if !input.fields.get("base_revision").is_none_or(Value::is_null) {
            return Err(St3Error::new(
                "invalid-glass-base",
                "creation requires a null base_revision",
            ));
        }
        if glass_heads::live_count(transaction, person).map_err(internal)? >= MAX_GLASSES {
            return Err(St3Error::new(
                "glass-limit",
                "at most 100 glasses may be live",
            ));
        }
    }
    let mut fields = input.fields.clone();
    fields.entry("base_revision".into()).or_insert(Value::Null);
    fields.insert(
        "replaced_revision".into(),
        head.map_or(Value::Null, |id| json!(id)),
    );
    Ok(Some(fields))
}
