//! A set publication's source map: one `source.mapped` claim per member revision, written once
//! at that revision's first mapped publication, with file texts in `blobs` by sha256.
use super::*;
use fractal_schema::source_map::{self, ClaimContext, KIND, SourceMap, SourceMapCounts};

fn invalid(message: impl Into<String>) -> FractalError {
    FractalError::new("invalid-source-map", message)
}

/// Shape checks plus membership: every mapped subject must be a non-retiring member of this
/// publication. Runs before the noop return so an unknown subject is refused either way.
pub fn validate_tx(map: &SourceMap, plan: &owned_sets::Plan) -> Result<(), FractalError> {
    let encoded = serde_json::to_vec(map).map_err(internal)?.len();
    map.validate(encoded).map_err(|e| FractalError::new(e.code, e.message))?;
    for subject in map.subjects.keys() {
        if plan.preview.changes.get(subject).is_none_or(|change| change == "retiring") {
            return Err(invalid(format!(
                "source map subject `{subject}` is not a member of {}",
                plan.preview.set
            )));
        }
    }
    Ok(())
}

/// The recorded claim fields for `subject@revision`, if any.
pub fn read(connection: &Connection, subject: &str, revision: &str) -> Result<Option<Value>, FractalError> {
    let body: Option<String> = connection
        .query_row(
            /* sqlite-large-value: allow-for-now lv-hcd0064dd968e owner=fractal_store::store::source_maps TODO=trace-statement-and-value-representation */ &smallclaims::store::canonical_sql("SELECT body FROM claims WHERE subject=?1 AND kind=?2 ORDER BY CANONICAL_ASC(claims) LIMIT 1"),
            params![source_map::claim_subject(subject, revision), KIND],
            |row| row.get(0),
        )
        .optional()
        .map_err(internal)?;
    body.map(|body| serde_json::from_str::<Value>(&body).map(|mut body| body["fields"].take()))
        .transpose()
        .map_err(internal)
}

/// Record after the set receipt is appended, in the same transaction. First write wins per
/// subject revision: an already mapped revision is kept, not refused.
pub fn record_tx(
    transaction: &Transaction<'_>,
    origin: &str,
    map: &SourceMap,
    receipt: &ClaimRecord,
    actor: Option<&str>,
    batch: &str,
) -> Result<(SourceMapCounts, Vec<String>), FractalError> {
    let fields = &receipt.body["fields"];
    let hash = fields["revision"].as_str().ok_or_else(|| internal("set receipt without revision"))?;
    let revision: owned_sets::Revision = serde_json::from_value(fields["body"].clone()).map_err(internal)?;
    let set_revision = format!("{}@{hash}", receipt.subject);
    let mut counts = SourceMapCounts::default();
    let mut pending = Vec::new();
    for (subject, entry) in &map.subjects {
        let member = revision
            .members
            .get(subject)
            .ok_or_else(|| invalid(format!("source map subject `{subject}` is not a member of {set_revision}")))?;
        if read(transaction, subject, &member.revision)?.is_some() {
            counts.kept += 1;
        } else {
            pending.push((subject, entry, member.revision.as_str()));
        }
    }
    // Only files a newly recorded map names are hashed and stored. Texts are staged in
    // `local_blobs`; appending the claim that names them promotes them into the shared `blobs`
    // in this transaction, as for every other claim-referenced blob.
    let needed: BTreeSet<usize> = pending.iter().flat_map(|(_, entry, _)| entry.referenced()).collect();
    let mut hashes = vec![String::new(); map.files.len()];
    for &file in &needed {
        let input = &map.files[file];
        hashes[file] = match (&input.text, &input.sha256) {
            (Some(text), _) => source_map::sha256_hex(text),
            (None, Some(sha256)) => sha256.clone(),
            (None, None) => return Err(invalid(format!("source file `{}` needs text or sha256", input.path))),
        };
    }
    let texts: BTreeMap<&str, &str> = needed
        .iter()
        .filter_map(|&file| map.files[file].text.as_deref().map(|text| (hashes[file].as_str(), text)))
        .collect();
    let distinct: BTreeSet<&str> = needed.iter().map(|&file| hashes[file].as_str()).collect();
    for hash in distinct {
        let shared = transaction
            .query_row("SELECT 1 FROM blobs WHERE hash=?1", [hash], |_| Ok(()))
            .optional()
            .map_err(internal)?
            .is_some();
        if shared {
            counts.blobs_deduped += 1;
            continue;
        }
        let Some(text) = texts.get(hash) else {
            return Err(FractalError::new(
                "source-blob-missing",
                format!("a source file omitted its text, but sha256 {hash} is not stored"),
            ));
        };
        transaction
            .execute(
                /* sqlite-large-value: allow-for-now lv-hefc9d72adcbd owner=fractal_store::store::source_maps TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_blobs(hash, bytes, size) VALUES (?1, ?2, ?3)",
                params![hash, text.as_bytes(), text.len() as u64],
            )
            .map_err(internal)?;
        counts.blobs_new += 1;
    }
    let mut claim_ids = Vec::new();
    for (subject, entry, member_revision) in pending {
        let claim = map.claim(
            subject,
            entry,
            &hashes,
            ClaimContext {
                revision: member_revision,
                set_revision: &set_revision,
                repository: &revision.source.repository,
                r#ref: &revision.source.r#ref,
                sha: &revision.source.sha,
                sequence: revision.source.sequence,
            },
        );
        let record = append_claim_tx(
            transaction,
            origin,
            &source_map::claim_subject(subject, member_revision),
            KIND,
            actor,
            &json!({ "fields": claim }),
            &[],
            Some(batch),
        )
        .map_err(internal)?;
        claim_ids.push(record.id);
        counts.recorded += 1;
    }
    Ok((counts, claim_ids))
}

/// The revision a member resolves to now: its desired row, or its mission definition.
pub fn current_revision(connection: &Connection, subject: &str) -> Result<Option<String>, FractalError> {
    if let Some(id) = subject.strip_prefix("mission/") {
        connection
            .query_row("SELECT revision FROM mission_definitions WHERE mission_id=?1", [id], |row| row.get(0))
            .optional()
            .map_err(internal)
    } else {
        Ok(current_desired_row_tx(connection, subject).map_err(internal)?.map(|row| row.revision))
    }
}

impl Store {
    /// A subject's recorded source map at `revision`, or at its current revision.
    pub fn source_map(&self, subject: &str, revision: Option<&str>) -> Result<(String, Option<Value>), FractalError> {
        let connection = self.readers.get();
        let revision = match revision {
            Some(revision) => revision.to_owned(),
            None => current_revision(&connection, subject)?.ok_or_else(|| {
                FractalError::new("unknown-subject", format!("{subject} has no current agent or mission revision"))
            })?,
        };
        let fields = read(&connection, subject, &revision)?;
        Ok((revision, fields))
    }

    pub fn source_blob(&self, hash: &str) -> Result<Option<Vec<u8>>, FractalError> {
        self.readers
            .get()
            .query_row("SELECT bytes FROM blobs WHERE hash=?1", [hash], |row| row.get(0))
            .optional()
            .map_err(internal)
    }
}
