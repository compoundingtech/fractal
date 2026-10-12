//! Reviews come from daemon claim metadata and an immutable snapshot, never caller JSON.
//! This API is for the installed observer; its transport supplies independently fetched bytes.
use super::{
    observers::{ObserverSigner, Produced},
    *,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Operator-owned trust and repository identity, outside the reviewed checkout.
pub struct ReviewBinding {
    pub repository: String,
    /// Stable resource names can predate a GitHub repository rename.
    pub resource_prefixes: BTreeSet<String>,
    pub review_origins: BTreeSet<String>,
    pub snapshot_origins: BTreeSet<String>,
    pub reviewers: BTreeSet<String>,
    pub test_signers: BTreeSet<String>,
    pub required_checks: BTreeSet<String>,
}

/// All fields are fetched by the protected transport, not accepted as CLI arguments.
pub struct ReviewInputs<'a> {
    pub claim_id: &'a str,
    pub claim: &'a Value,
    pub snapshot: &'a Value,
    pub document: &'a [u8],
    pub pull_request: &'a Value,
}

impl ReviewBinding {
    pub fn validate(&self) -> Result<()> {
        let (owner, name) = self
            .repository
            .split_once('/')
            .context("repository identity")?;
        let valid = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        };
        ensure!(valid(owner) && valid(name), "invalid repository identity");
        ensure!(
            !self.resource_prefixes.is_empty()
                && self
                    .resource_prefixes
                    .iter()
                    .all(|prefix| prefix.starts_with("resource/github/")
                        && prefix.ends_with("/pull-request/")
                        && !prefix.chars().any(char::is_whitespace)),
            "invalid PR resource prefixes"
        );
        for (values, label) in [
            (&self.review_origins, "review origins"),
            (&self.snapshot_origins, "snapshot origins"),
            (&self.reviewers, "reviewers"),
            (&self.test_signers, "test signers"),
            (&self.required_checks, "required checks"),
        ] {
            ensure!(
                !values.is_empty() && values.iter().all(|value| !value.trim().is_empty()),
                "operator must configure {label}"
            );
        }
        ensure!(
            self.reviewers
                .iter()
                .all(|reviewer| reviewer.starts_with("agent/")),
            "reviewer must be a seat subject"
        );
        Ok(())
    }

    /// Decode only the immutable claim identity and its typed decision/citations.
    /// The transport uses this before fetching the snapshot, document and Git objects.
    pub fn review_fields<'a>(&self, claim_id: &str, claim: &'a Value) -> Result<&'a Value> {
        self.validate()?;
        sha256(claim_id)?;
        let actor = claim["actor"].as_str().context("review claim actor")?;
        ensure!(
            claim["id"].as_str() == Some(claim_id)
                && claim["kind"] == "resource.observed"
                && claim["origin"]
                    .as_str()
                    .is_some_and(|origin| self.review_origins.contains(origin))
                && self.reviewers.contains(actor)
                && claim["subject"]
                    .as_str()
                    .is_some_and(|subject| subject.starts_with("resource/")),
            "review claim identity, origin or actor is not trusted"
        );
        let fields = &claim["body"]["fields"];
        ensure!(
            fields["kind"] == "human.review",
            "claim is not a human review"
        );
        let facts = fields.get("facts").unwrap_or(fields);
        ensure!(
            facts["reviewer"].as_str() == Some(actor),
            "reviewer differs from daemon claim actor"
        );
        ensure!(
            matches!(
                facts["decision"].as_str(),
                Some("approve" | "request-changes")
            ),
            "informational or conditional review is not a verdict"
        );
        chrono::DateTime::parse_from_rfc3339(
            facts["submitted_at"]
                .as_str()
                .context("review submission time")?,
        )?;
        let target = facts["target"]
            .as_str()
            .context("immutable review target")?;
        let (subject, snapshot) = target.rsplit_once('@').context("immutable review target")?;
        sha256(snapshot)?;
        let number = self
            .resource_prefixes
            .iter()
            .find_map(|prefix| subject.strip_prefix(prefix))
            .context("review target repository is not configured")?;
        let parsed = number.parse::<u64>().context("PR number")?;
        ensure!(
            parsed > 0 && parsed.to_string() == number,
            "invalid PR number"
        );
        let document = facts["document"]
            .as_str()
            .context("immutable review document")?;
        let (name, hash) = document
            .rsplit_once('@')
            .context("immutable review document")?;
        sha256(hash)?;
        ensure!(
            name.starts_with("doc/")
                && name.len() > 4
                && name.bytes().all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'/' | b'_' | b'-' | b'.')),
            "invalid document name"
        );
        if let Some(tests) = facts.get("evidence_tests") {
            let tests = tests
                .as_array()
                .context("test citations must be an array")?;
            let mut seen = BTreeSet::new();
            for test in tests {
                let id = test
                    .as_str()
                    .context("test citation must be an object ID")?;
                oid(id)?;
                ensure!(seen.insert(id), "duplicate test citation");
            }
        }
        Ok(facts)
    }
}

/// Derive the complete reviewed path/blob map from Git. The moving base is read only
/// at observation time; it is never added to the general PR snapshot.
pub fn review_statement(
    repo: &Path,
    binding: &ReviewBinding,
    inputs: &ReviewInputs<'_>,
    trusted_signers: &Path,
) -> Result<Statement> {
    let facts = binding.review_fields(inputs.claim_id, inputs.claim)?;
    let (subject, snapshot_id) = facts["target"].as_str().unwrap().rsplit_once('@').unwrap();
    ensure!(
        inputs.snapshot["id"].as_str() == Some(snapshot_id)
            && inputs.snapshot["subject"].as_str() == Some(subject)
            && inputs.snapshot["kind"] == "resource.observed"
            && inputs.snapshot["origin"]
                .as_str()
                .is_some_and(|origin| binding.snapshot_origins.contains(origin))
            && inputs.snapshot["body"]["fields"]["kind"] == "vcs.pull-request",
        "reviewed snapshot identity or origin mismatch"
    );
    let snapshot = &inputs.snapshot["body"]["fields"]["facts"];
    let number = snapshot["number"]
        .as_u64()
        .filter(|number| *number > 0)
        .context("snapshot PR number")?;
    ensure!(
        binding
            .resource_prefixes
            .iter()
            .any(|prefix| subject == format!("{prefix}{number}")),
        "snapshot number differs from resource identity"
    );
    let pull = inputs.pull_request;
    let head = snapshot["head_sha"].as_str().context("reviewed head")?;
    oid(head)?;
    ensure!(
        pull["number"].as_u64() == Some(number)
            && pull["node_id"]
                .as_str()
                .is_some_and(|node| !node.is_empty() && snapshot["node_id"].as_str() == Some(node))
            && pull["base"]["repo"]["full_name"].as_str() == Some(binding.repository.as_str())
            && pull["head"]["sha"].as_str() == Some(head),
        "live PR identity/head differs from the immutable reviewed snapshot"
    );
    let live_base = pull["base"]["sha"]
        .as_str()
        .context("observed base commit")?;
    oid(live_base)?;
    let head = commit(repo, head)?;
    let base = git_text(repo, &["merge-base", "--all", &head, live_base], None)?;
    oid(&base)?; // Criss-cross merges with multiple bases are refused, not guessed.
    let files = changed_files(repo, &base, &head)?;
    let document_hash = facts["document"]
        .as_str()
        .unwrap()
        .rsplit_once('@')
        .unwrap()
        .1;
    ensure!(
        hex::encode(Sha256::digest(inputs.document)) == document_hash,
        "immutable review document digest mismatch"
    );
    let verdict = if facts["decision"] == "approve" {
        Verdict::GO
    } else {
        Verdict::STOP
    };
    let tests = facts
        .get("evidence_tests")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if verdict == Verdict::GO {
        ensure!(
            !tests.is_empty(),
            "GO must explicitly cite considered tests"
        );
        let change = format!("github:{}#{number}", binding.repository);
        let mut passed = BTreeSet::new();
        for id in &tests {
            let test = verify_record(repo, id, trusted_signers)?;
            ensure!(
                test.predicate.tier == TrustTier::Full
                    && test.predicate.change_id == change
                    && test
                        .predicate
                        .signer
                        .as_ref()
                        .is_some_and(|signer| binding.test_signers.contains(signer)),
                "GO citation has the wrong change, trust tier or signer role"
            );
            let source = &test.subject[0].name;
            // The platform tests a synthetic merge, not necessarily the author head.
            git(repo, &["merge-base", "--is-ancestor", &head, source], None)
                .context("cited test source does not contain the reviewed head")?;
            let tested_files = changed_files(repo, &base, source)?;
            ensure!(
                files
                    .iter()
                    .all(|(path, blob)| tested_files.get(path) == Some(blob)),
                "cited test source changes a reviewed file"
            );
            let Fact::Tests { checks, .. } = test.predicate.fact else {
                bail!("review citation is not tests")
            };
            ensure!(
                checks.values().all(|check| check.exit_code == 0),
                "GO cites failed checks"
            );
            passed.extend(checks.into_keys());
        }
        ensure!(
            binding.required_checks.is_subset(&passed),
            "GO citations lack required checks"
        );
    }
    let accepted = inputs.claim["accepted_at_unix_ms"]
        .as_u64()
        .context("daemon claim acceptance time")?;
    let observed_at = chrono::DateTime::from_timestamp_millis(i64::try_from(accepted)?)
        .context("claim time out of range")?;
    Ok(Statement {
        statement_type: STATEMENT_TYPE.into(),
        predicate_type: PREDICATE_TYPE.into(),
        subject: vec![Subject {
            name: head.clone(),
            digest: BTreeMap::from([("gitTree".into(), tree(repo, &head)?)]),
        }],
        predicate: Observation {
            schema_version: 1,
            change_id: format!("github:{}#{number}", binding.repository),
            signer: None,
            tier: TrustTier::Degraded,
            re_attests: None,
            producer_sha256: "0".repeat(64),
            observed_at: observed_at.to_rfc3339(),
            fact: Fact::Review {
                reviewer: inputs.claim["actor"].as_str().unwrap().into(),
                verdict,
                base_commit: base,
                files,
                tests,
            },
        },
    })
}

/// One immutable result per daemon review claim. A retry never silently promotes a
/// degraded result; signing again requires a new independently observed review claim.
pub fn produce_review(
    repo: &Path,
    binding: &ReviewBinding,
    inputs: &ReviewInputs<'_>,
    trusted_signers: &Path,
    signer: &ObserverSigner<'_>,
) -> Result<Produced> {
    let mut statement = review_statement(repo, binding, inputs, trusted_signers)?;
    match signer {
        ObserverSigner::Full { principal, .. } | ObserverSigner::Test { principal, .. } => ensure!(
            !principal.is_empty() && !principal.chars().any(char::is_whitespace),
            "invalid configured signer"
        ),
        ObserverSigner::Unavailable => {}
    }
    if let ObserverSigner::Test { expires_at, .. } = signer {
        let now = chrono::Utc::now();
        ensure!(
            *expires_at > now && *expires_at <= now + chrono::Duration::minutes(15),
            "throwaway signer must have a remaining lifetime of at most 15 minutes"
        );
    }
    let reference = format!("refs/evidence-observed/review/{}", inputs.claim_id);
    let previous = git_text(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            &reference,
        ],
        None,
    )?;
    if !previous.is_empty() {
        let (name, record) = previous.split_once(' ').context("review observation ref")?;
        ensure!(
            name == reference && !record.contains('\n'),
            "ambiguous review observation ref"
        );
        let old = show(repo, record)?;
        ensure!(
            old.subject[0].name == statement.subject[0].name
                && old.predicate.change_id == statement.predicate.change_id
                && serde_json::to_value(&old.predicate.fact)?
                    == serde_json::to_value(&statement.predicate.fact)?,
            "review claim applicability changed after observation"
        );
        ensure!(
            !matches!(signer, ObserverSigner::Test { .. }) || old.predicate.tier != TrustTier::Full,
            "test service must use an isolated observation repository"
        );
        return Ok(Produced {
            record: record.into(),
            tier: old.predicate.tier,
            signing_unavailable: old.predicate.tier == TrustTier::Degraded,
            signing_latency_ms: 0,
            newly_recorded: false,
        });
    }
    let facts = binding.review_fields(inputs.claim_id, inputs.claim)?;
    // Provenance is signed in the commit message without changing the v1 statement schema.
    let provenance = serde_json::json!({"review_claim":inputs.claim_id, "target":facts["target"], "document":facts["document"]});
    let message = format!(
        "st gate observation v1\n\n{}\n",
        serde_json::to_string(&provenance)?
    );
    let started = std::time::Instant::now();
    let signed = match signer {
        ObserverSigner::Unavailable => None,
        ObserverSigner::Full { principal, key } | ObserverSigner::Test { principal, key, .. } => {
            statement.predicate.signer = Some((*principal).into());
            statement.predicate.tier = if matches!(signer, ObserverSigner::Full { .. }) {
                TrustTier::Full
            } else {
                TrustTier::Test
            };
            Some(write_observation_message(
                repo,
                &statement,
                Some(key),
                message.as_bytes(),
            ))
        }
    };
    let record = match signed {
        Some(Ok(record)) => record,
        Some(Err(_)) | None => {
            statement.predicate.signer = None;
            statement.predicate.tier = TrustTier::Degraded;
            write_observation_message(repo, &statement, None, message.as_bytes())?
        }
    };
    let signing_latency_ms = started.elapsed().as_millis();
    let tier = show(repo, &record)?.predicate.tier;
    git(repo, &["update-ref", "--stdin"], Some(format!("start\ncreate {reference} {record}\ncreate {NOTES_PREFIX}{record} {record}\nprepare\ncommit\n").as_bytes()))?;
    Ok(Produced {
        record,
        tier,
        signing_unavailable: tier == TrustTier::Degraded,
        signing_latency_ms,
        newly_recorded: true,
    })
}
