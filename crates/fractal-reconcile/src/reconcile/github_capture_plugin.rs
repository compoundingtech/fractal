//! Canonical PR images for the existing plugin wire API; operational details stay in its mirror.
use super::*;

pub struct RepositoryPullRequests {
    /// False means replay or authoritative acquisition coverage is still pending, not an empty repo.
    pub complete: bool,
    pub pulls: Vec<PullRequestFact>,
}

/// `None` means this repository has no enabled canonical source. Filters precede the row bound.
pub fn repository_pull_requests(
    store: &Store,
    state_dir: &Path,
    locator: &str,
    state: Option<&str>,
    number: Option<u64>,
    limit: usize,
) -> Result<Option<RepositoryPullRequests>> {
    let mut definitions = store.github_source_definitions()?.into_iter().filter(|definition| {
        definition.enabled && definition.repositories.iter().any(|value| value.eq_ignore_ascii_case(locator))
    });
    let Some(definition) = definitions.next() else { return Ok(None) };
    ensure!(definitions.next().is_none(), "repository belongs to two enabled acquisition sources");
    // The legacy plugin route has no host selector and represents github.com only.
    ensure!(definition.source.host == "github.com", "GitHub plugin route cannot select an enterprise source host");
    let pending = || Some(RepositoryPullRequests { complete: false, pulls: Vec::new() });
    let plugin = GithubPluginStore::shared(state_dir)?;
    let mut connection = plugin.connection.lock().unwrap_or_else(PoisonError::into_inner);
    if !ready(store, &mut connection, &definition)? { return Ok(pending()); }
    let source = projection_key(&definition)?;
    let repositories = {
        let mut statement = connection.prepare_cached("SELECT record FROM capture_v1_entities WHERE source=?1 AND family='repository' AND lower(json_extract(record,'$.image.locator'))=lower(?2) LIMIT 2")?;
        let rows = statement.query_map(params![source,locator], |row| row.get::<_,String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    ensure!(repositories.len() <= 1, "repository locator has ambiguous canonical identities");
    let Some(record) = repositories.first() else { return Ok(pending()) };
    let FactImage::Repository(repository) = serde_json::from_str(record)? else { return Ok(pending()) };
    let coverage: Option<String> = connection.query_row(
        "SELECT record FROM capture_v1_coverage WHERE source=?1 AND repository_id=?2 AND family='pull_request'",
        params![source,repository.id], |row| row.get(0),
    ).optional()?;
    let Some(coverage) = coverage else { return Ok(pending()) };
    let coverage: Coverage = serde_json::from_str(&coverage)?;
    if !coverage.complete || !coverage.authoritative || coverage.error.is_some() { return Ok(pending()); }
    let mut statement = connection.prepare_cached(
        "SELECT record FROM capture_v1_entities WHERE source=?1 AND family='pull_request' AND repository_id=?2
         AND json_extract(record,'$.family')='pull_request'
         AND (?3 IS NULL OR (?3='open' AND json_extract(record,'$.image.state')='open') OR (?3='closed' AND json_extract(record,'$.image.state') IN ('closed','merged')))
         AND (?4 IS NULL OR json_extract(record,'$.image.number')=?4)
         ORDER BY json_extract(record,'$.image.number') DESC LIMIT ?5",
    )?;
    let rows = statement.query_map(params![source,repository.id,state,number,limit.min(super::super::github_plugin_store::MAX_READ_ROWS)], |row| row.get::<_,String>(0))?;
    let mut pulls = Vec::new();
    for row in rows {
        if let FactImage::PullRequest(pull) = serde_json::from_str(&row?)? { pulls.push(pull); }
    }
    Ok(Some(RepositoryPullRequests { complete: true, pulls }))
}
