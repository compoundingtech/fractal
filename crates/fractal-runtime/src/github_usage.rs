//! Agent gh-compatible accounting for native GitHub HTTP calls.
use std::path::Path;
use std::sync::LazyLock;

/// Best-effort logging must never turn a successful mutation into a retry.
/// `file` is an explicit destination for isolated local-server tests.
pub fn log_github_call(
    method: &reqwest::Method,
    path: &str,
    status: Option<reqwest::StatusCode>,
    headers: &reqwest::header::HeaderMap,
    observer: Option<&str>,
    file: Option<&Path>,
) {
    let configured = std::env::var_os("AGENT_POLICY_STATE_DIR").map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state/agent-policy")))
        .map(|directory| directory.join("gh-usage.jsonl"));
    let Some(file) = file.or(configured.as_deref()) else { return };
    if let Err(error) = append(file, method, path, status, headers, observer) {
        tracing::warn!(%error, "could not append GitHub usage");
    }
}

fn append(
    file: &Path,
    method: &reqwest::Method,
    path: &str,
    status: Option<reqwest::StatusCode>,
    headers: &reqwest::header::HeaderMap,
    observer: Option<&str>,
) -> anyhow::Result<()> {
    use std::io::Write as _;
    static HOST: LazyLock<String> = LazyLock::new(|| std::env::var("HOSTNAME").ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_default().trim().split('.').next().unwrap_or_default().to_owned());
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let mut line = serde_json::to_vec(&serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "host": HOST.as_str(),
        "invocation": std::env::var("AGENT_INVOCATION_ID").unwrap_or_default(),
        "agent": std::env::var("ST_AGENT").unwrap_or_default(),
        "account": std::env::var("AI_ACCOUNT").unwrap_or_default(),
        "category": "fractal-http",
        "argv": ["api", "--method", method.as_str(), path],
        "method": method.as_str(), "path": path,
        "status": status.map(|status| status.as_u16()),
        "remaining": header("x-ratelimit-remaining").and_then(|value| value.parse::<u64>().ok()),
        "resource": header("x-ratelimit-resource"),
        "retry_after": header("retry-after"),
        "not_modified": status == Some(reqwest::StatusCode::NOT_MODIFIED),
        "observer": observer,
    }))?;
    line.push(b'\n');
    if let Some(parent) = file.parent() { std::fs::create_dir_all(parent)?; }
    std::fs::OpenOptions::new().create(true).append(true).open(file)?.write_all(&line)?;
    Ok(())
}
