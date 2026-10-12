//! The protected review command accepts one immutable daemon claim ID, no authored facts.
use super::{ci_transport::Github, observers::ObserverSigner, review_observer::*, *};
use base64::Engine as _;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    repository: String,
    scratch_repo: PathBuf,
    daemon_socket: PathBuf,
    trusted_signers: PathBuf,
    resource_prefixes: BTreeSet<String>,
    review_origins: BTreeSet<String>,
    snapshot_origins: BTreeSet<String>,
    reviewers: BTreeSet<String>,
    test_signers: BTreeSet<String>,
    required_checks: BTreeSet<String>,
    tier: TrustTier,
    signer: Option<String>,
    ssh_key: Option<PathBuf>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// GETs only on a fixed operator-owned Unix socket. No caller URL, endpoint, headers,
/// redirects, curl configuration or user proxy. The installed curl comes from profile PATH.
fn daemon_json(socket: &Path, path: &str) -> Result<Value> {
    let output = Command::new("curl")
        .args([
            "-q",
            "--silent",
            "--fail",
            "--max-time",
            "30",
            "--noproxy",
            "*",
            "--unix-socket",
        ])
        .arg(socket)
        .arg(format!("http://localhost{path}"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .context("read observer daemon")?;
    ensure!(output.status.success(), "daemon observation unavailable");
    ensure!(
        output.stdout.len() <= 4 * 1024 * 1024,
        "daemon observation size bound"
    );
    let envelope: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(
        matches!(
            envelope["api_version"].as_str(),
            Some("st3.v1" | "st3.client.v0")
        ),
        "unsupported daemon observation envelope"
    );
    envelope
        .get("value")
        .cloned()
        .context("daemon response value")
}

/// Fetch only operator-bound data into a private bare repository. This never checks
/// out the reviewed source or executes its hooks, configuration, binaries or scripts.
fn fetch(repo: &Path, repository: &str, revisions: &[String]) -> Result<()> {
    let status = Command::new("timeout")
        .args(["--kill-after=2s", "30s", "git", "-C"])
        .arg(repo)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "transfer.fsckObjects=true",
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--",
        ])
        .arg(format!("https://github.com/{repository}.git"))
        .args(revisions)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    ensure!(status.success(), "review source/citations unavailable");
    Ok(())
}

pub fn observe_review(claim_id: &str) -> Result<Value> {
    sha256(claim_id)?; // A selector cannot inject a URL, revision or path.
    let home = std::env::var_os("HOME").context("protected profile HOME")?;
    let configuration: Configuration = serde_json::from_slice(&std::fs::read(
        PathBuf::from(home).join(".config/st/evidence-review.json"),
    )?)
    .context("private review observer configuration")?;
    for path in [
        &configuration.scratch_repo,
        &configuration.daemon_socket,
        &configuration.trusted_signers,
    ] {
        ensure!(path.is_absolute(), "operator paths must be absolute");
    }
    if let Some(key) = &configuration.ssh_key {
        ensure!(key.is_absolute(), "private key path must be absolute");
    }
    let signer = match configuration.tier {
        TrustTier::Full => {
            ensure!(
                configuration.expires_at.is_none(),
                "production signer cannot be a test service"
            );
            ObserverSigner::Full {
                principal: configuration
                    .signer
                    .as_deref()
                    .context("configured review principal")?,
                key: configuration
                    .ssh_key
                    .as_deref()
                    .context("configured review key")?,
            }
        }
        TrustTier::Test => ObserverSigner::Test {
            principal: configuration
                .signer
                .as_deref()
                .context("configured throwaway principal")?,
            key: configuration
                .ssh_key
                .as_deref()
                .context("configured throwaway key")?,
            expires_at: configuration
                .expires_at
                .context("throwaway service expiration")?,
        },
        TrustTier::Degraded => {
            ensure!(
                configuration.signer.is_none()
                    && configuration.ssh_key.is_none()
                    && configuration.expires_at.is_none(),
                "degraded profile has no signer or key"
            );
            ObserverSigner::Unavailable
        }
    };
    let binding = ReviewBinding {
        repository: configuration.repository,
        resource_prefixes: configuration.resource_prefixes,
        review_origins: configuration.review_origins,
        snapshot_origins: configuration.snapshot_origins,
        reviewers: configuration.reviewers,
        test_signers: configuration.test_signers,
        required_checks: configuration.required_checks,
    };
    binding.validate()?;
    let repo = &configuration.scratch_repo;
    ensure!(
        git_text(repo, &["rev-parse", "--is-bare-repository"], None)? == "true",
        "observer requires its private bare repository"
    );
    let claim = daemon_json(
        &configuration.daemon_socket,
        &format!("/v1/claims/by-id/{claim_id}"),
    )?;
    let facts = binding.review_fields(claim_id, &claim)?;
    let snapshot_id = facts["target"]
        .as_str()
        .unwrap()
        .rsplit_once('@')
        .unwrap()
        .1;
    let snapshot = daemon_json(
        &configuration.daemon_socket,
        &format!("/v1/claims/by-id/{snapshot_id}"),
    )?;
    let reference = facts["document"].as_str().unwrap();
    let document = daemon_json(
        &configuration.daemon_socket,
        &format!("/v1/documents/content?reference={reference}"),
    )?;
    ensure!(
        document["reference"].as_str() == Some(reference),
        "document response identity mismatch"
    );
    let bytes: Vec<u8> =
        serde_json::from_value(document["bytes"].clone()).context("immutable document bytes")?;
    let number = binding
        .resource_prefixes
        .iter()
        .find_map(|prefix| {
            facts["target"]
                .as_str()
                .unwrap()
                .split('@')
                .next()
                .unwrap()
                .strip_prefix(prefix)
        })
        .unwrap();
    let github = Github {
        repository: binding.repository.clone(),
    };
    // One authenticated PR read supplies both head and base; base-only movement never
    // creates a general snapshot claim. review_statement checks the node identity too.
    let pull = github.json(&format!("pulls/{number}"))?;
    let head = pull["head"]["sha"].as_str().context("PR head")?;
    let base = pull["base"]["sha"].as_str().context("PR base")?;
    oid(head)?;
    oid(base)?;
    ensure!(
        snapshot["body"]["fields"]["facts"]["head_sha"].as_str() == Some(head),
        "PR moved after review; record a new review observation"
    );
    let mut revisions = vec![base.to_owned(), head.to_owned()];
    if facts["decision"] == "approve" {
        revisions.extend(
            facts
                .get("evidence_tests")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|id| format!("{NOTES_PREFIX}{}", id.as_str().unwrap())),
        );
    }
    fetch(repo, &binding.repository, &revisions)?;
    let inputs = ReviewInputs {
        claim_id,
        claim: &claim,
        snapshot: &snapshot,
        document: &bytes,
        pull_request: &pull,
    };
    let result = produce_review(
        repo,
        &binding,
        &inputs,
        &configuration.trusted_signers,
        &signer,
    )?;
    let source = head.to_owned();
    // A public export contains only explicitly enumerated source/review objects.
    // Citations are immutable IDs; their existing public refs transport their own objects.
    let statement = show(repo, &result.record)?;
    let Fact::Review { base_commit, .. } = &statement.predicate.fact else {
        unreachable!()
    };
    let mut objects = format!("{source}\n{}\n{base_commit}\n", result.record).into_bytes();
    for treeish in [
        tree(repo, &source)?,
        tree(repo, &result.record)?,
        tree(repo, base_commit)?,
    ] {
        objects.extend(git(
            repo,
            &["rev-list", "--objects", "--no-object-names", &treeish],
            None,
        )?);
    }
    let pack = git(repo, &["pack-objects", "--stdout"], Some(&objects))?;
    let full = usize::from(result.newly_recorded && result.tier == TrustTier::Full);
    Ok(
        serde_json::json!({"repository":binding.repository, "review_claim":claim_id,
        "target":facts["target"], "document":reference, "source":source,
        "record":result, "object_pack_base64":base64::engine::general_purpose::STANDARD.encode(pack),
        "metrics":{"fresh_records":usize::from(result.newly_recorded), "full_records":full,
            "full_trust_share":result.newly_recorded.then_some(full as f64),
            "signing_latency_ms":result.signing_latency_ms}}),
    )
}
