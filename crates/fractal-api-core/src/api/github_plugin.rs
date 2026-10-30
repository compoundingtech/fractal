//! `/v1/plugins/github/*`: the GitHub plugin's read API (docs/st3/plugins.md). The member that
//! polls a repository answers from its plugin mirror. Any other member relays the read to it
//! through the client read relay, once that member advertises the plugin. When the poller runs
//! an older build, cannot be reached, or has no mirror of the repository yet, the member answers
//! from its own graph instead and says so in `source`, so an answer is never silently stale.

use super::*;
use axum::http::HeaderMap;
use crate::WatchesStore;
use crate::reconcile::github_plugin_store::{self, GithubPluginStore};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PluginQuery {
    /// `open`, `closed` or `all`; open by default.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// For events, one item's events only.
    #[serde(default)]
    pub number: Option<u64>,
    /// Answer from this member's mirror alone: set by a relayed read on the poller.
    #[serde(default)]
    pub local: Option<String>,
}

#[derive(Clone, Debug)]
enum Read {
    Pulls { state: Option<String>, limit: usize },
    Pull(u64),
    Checks(u64),
    Events { number: Option<u64>, limit: usize },
}

const DEFAULT_LIMIT: usize = 50;

impl Read {
    /// The path and query the poller answers this read at.
    fn path(&self, owner: &str, repository: &str) -> String {
        let base = format!("/v1/plugins/github/repos/{owner}/{repository}");
        match self {
            Self::Pulls { state, limit } => format!(
                "{base}/pulls?limit={limit}&state={}",
                state.as_deref().unwrap_or("all")
            ),
            Self::Pull(number) => format!("{base}/pulls/{number}"),
            Self::Checks(number) => format!("{base}/pulls/{number}/checks"),
            Self::Events { number, limit } => match number {
                Some(number) => format!("{base}/events?limit={limit}&number={number}"),
                None => format!("{base}/events?limit={limit}"),
            },
        }
    }
}

pub async fn pulls(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((owner, repository)): AxumPath<(String, String)>,
    Query(query): Query<PluginQuery>,
) -> Result<Json<Value>, ApiError> {
    let wanted = match query.state.as_deref() {
        None | Some("open") => Some("open".to_owned()),
        Some("closed") => Some("closed".to_owned()),
        Some("all") => None,
        Some(other) => {
            return Err(ApiError::bad(St3Error::new(
                "validation-failed",
                format!("state is open, closed or all, not {other}"),
            )));
        }
    };
    let limit = limit(&query)?;
    answer(state, &headers, owner, repository, Read::Pulls { state: wanted, limit }, &query).await
}

pub async fn pull(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((owner, repository, number)): AxumPath<(String, String, u64)>,
    Query(query): Query<PluginQuery>,
) -> Result<Json<Value>, ApiError> {
    answer(state, &headers, owner, repository, Read::Pull(number), &query).await
}

pub async fn checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((owner, repository, number)): AxumPath<(String, String, u64)>,
    Query(query): Query<PluginQuery>,
) -> Result<Json<Value>, ApiError> {
    answer(state, &headers, owner, repository, Read::Checks(number), &query).await
}

pub async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((owner, repository)): AxumPath<(String, String)>,
    Query(query): Query<PluginQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = limit(&query)?;
    answer(state, &headers, owner, repository, Read::Events { number: query.number, limit }, &query).await
}

fn limit(query: &PluginQuery) -> Result<usize, ApiError> {
    match query.limit {
        None => Ok(DEFAULT_LIMIT),
        Some(limit) if (1..=github_plugin_store::MAX_READ_ROWS).contains(&limit) => Ok(limit),
        Some(_) => Err(ApiError::bad(St3Error::new(
            "validation-failed",
            format!("limit is 1 to {}", github_plugin_store::MAX_READ_ROWS),
        ))),
    }
}

fn github_name(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && part != "."
        && part != ".."
        && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// Who polls `locator` and into which resource: the repository's standing observer when it is
/// declared, else the first other running `github.repository` observer of it.
struct Poller {
    observer: String,
    resource: String,
    member: Option<String>,
}

fn poller(store: &Store, locator: &str) -> anyhow::Result<Option<Poller>> {
    let standing = crate::github_watch::standing_observer(locator);
    let mut observers = store
        .repository_observers()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .into_iter()
        .filter(|(_, spec)| {
            spec.provider == "github.repository" && !spec.stopped && spec.locator.eq_ignore_ascii_case(locator)
        })
        .collect::<Vec<_>>();
    observers.sort_by_key(|(subject, _)| (*subject != standing, subject.clone()));
    let Some((observer, spec)) = observers.into_iter().next() else {
        return Ok(None);
    };
    let member = store.selected_desired_origin(&observer)?;
    Ok(Some(Poller { observer, resource: spec.resource, member }))
}

async fn answer(
    state: AppState,
    headers: &HeaderMap,
    owner: String,
    repository: String,
    read: Read,
    query: &PluginQuery,
) -> Result<Json<Value>, ApiError> {
    if !github_name(&owner) || !github_name(&repository) {
        return Err(ApiError::bad(St3Error::new("validation-failed", "a repository is OWNER/REPO")));
    }
    let locator = format!("{owner}/{repository}");
    let local_only = query.local.is_some();
    let store = state.store.clone();
    let state_dir = state.state_dir.clone();
    let here = state.store.origin().to_owned();
    let found = {
        let locator = locator.clone();
        blocking_action(move || poller(&store, &locator).map_err(internal_error)).await?
    };
    let Some(found) = found else {
        return Err(ApiError::not_found(format!(
            "no member of this fleet polls {locator}; `st gh watch {locator}#NUMBER` declares its observer"
        )));
    };
    let poller_host = found.member.as_deref().map(client_host_id);
    let mut envelope = json!({
        "repository": locator,
        "resource": found.resource,
        "observer": found.observer,
        "poller": poller_host,
    });
    let polled_here = found.member.as_deref() == Some(here.as_str());
    let fallback = |reason: String, relay_error: Option<Value>| {
        let store = state.store.clone();
        let resource = found.resource.clone();
        let read = read.clone();
        let here = here.clone();
        let mut envelope = envelope.clone();
        async move {
            let body = blocking_action(move || from_graph(&store, &resource, &read).map_err(internal_error)).await?;
            envelope["source"] = json!({"kind": "graph", "host": client_host_id(&here), "reason": reason});
            if let Some(error) = relay_error {
                envelope["source"]["relay_error"] = error;
            }
            merge(&mut envelope, body);
            Ok::<_, ApiError>(Json(envelope))
        }
    };
    if polled_here || local_only {
        let resource = found.resource.clone();
        let read_mirror = read.clone();
        let locator_for_mirror = locator.clone();
        let mirrored = blocking_action(move || {
            let mirror = GithubPluginStore::shared(&state_dir).map_err(internal_error)?;
            let held = mirror
                .repositories(&locator_for_mirror)
                .map_err(internal_error)?
                .into_iter()
                .find(|row| row["resource"] == resource.as_str());
            match held {
                Some(row) => Ok(Some((row, from_mirror(&mirror, &resource, &read_mirror).map_err(internal_error)?))),
                None => Ok(None),
            }
        })
        .await?;
        if let Some((row, body)) = mirrored {
            envelope["source"] = json!({
                "kind": "mirror",
                "host": client_host_id(&here),
                "mirrored_at_unix_ms": row["mirrored_at_unix_ms"],
                "seeded_at_unix_ms": row["seeded_at_unix_ms"],
            });
            merge(&mut envelope, body);
            return Ok(Json(envelope));
        }
        let reason = if polled_here {
            "this member polls the repository, and its mirror has not taken a look since it started"
        } else {
            "this member has no mirror of the repository"
        };
        return fallback(reason.into(), None).await;
    }
    let Some(member) = found.member.clone() else {
        return fallback("no member's declaration of the repository's observer is selected".into(), None).await;
    };
    let host = client_host_id(&member);
    let advertised = {
        let store = state.store.clone();
        let member = member.clone();
        blocking_action(move || store.advertised_feature(&member, "github_plugin").map_err(internal_error)).await?
    };
    if advertised.as_ref().and_then(Value::as_i64).is_none_or(|version| version < 1) {
        return fallback(format!("{host} polls the repository and runs a build without the GitHub plugin"), None).await;
    }
    let Some(actor) = headers
        .get(client_v0::LOCAL_PERSON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|actor| actor.starts_with("person/") || actor.starts_with("agent/"))
        .map(str::to_owned)
    else {
        return fallback(format!("reading {host}'s mirror needs a person or agent to read as (st --as)"), None).await;
    };
    let Some(relay) = state.client_relay.as_ref() else {
        return fallback(format!("this member has no route to {host}"), None).await;
    };
    let request = crate::peer::ClientReadRequest {
        authority_actor: actor,
        relay: None,
        request: crate::peer::ClientReadOperation::GithubPlugin { path: read.path(&owner, &repository) },
    };
    match relay.read(&host, &request).await {
        Ok(value) => Ok(Json(value)),
        Err(error) => {
            let (code, details) = match error.downcast_ref::<crate::peer::ClientReadRejected>() {
                Some(rejected) => (rejected.code.clone(), Value::Object(rejected.details.clone())),
                None => ("remote-unavailable".to_owned(), Value::Null),
            };
            fallback(
                format!("{host} polls the repository and did not answer: {error:#}"),
                Some(json!({"code": code, "details": details})),
            )
            .await
        }
    }
}

fn internal_error(error: impl std::fmt::Display) -> St3Error {
    St3Error::new("internal", error.to_string())
}

fn merge(envelope: &mut Value, body: Value) {
    if let (Some(envelope), Value::Object(body)) = (envelope.as_object_mut(), body) {
        envelope.extend(body);
    }
}

fn from_mirror(mirror: &GithubPluginStore, resource: &str, read: &Read) -> anyhow::Result<Value> {
    Ok(match read {
        Read::Pulls { state, limit } => json!({"pulls": mirror.pulls(resource, state.as_deref(), *limit)?}),
        Read::Pull(number) => {
            let pull = mirror.item(resource, *number)?.filter(|item| item["kind"] == "vcs.pull-request");
            json!({"pull": pull})
        }
        Read::Checks(number) => json!({"checks": mirror.checks(resource, *number)?}),
        Read::Events { number, limit } => json!({"events": mirror.events(resource, *number, *limit)?}),
    })
}

/// The same read from this member's graph: the item facts it recorded (from step 3b of the plugin
/// store, their compact fields) and the `github.event` claims it holds. Heads it never kept are
/// unknown: `heads` is null.
fn from_graph(store: &Store, resource: &str, read: &Read) -> anyhow::Result<Value> {
    Ok(match read {
        Read::Pulls { state, limit } => {
            let mut items = store
                .repository_items(resource, None)?
                .into_iter()
                .filter(|item| item.kind == "vcs.pull-request")
                .filter(|item| {
                    let open = item.facts.get("state").and_then(Value::as_str) == Some("open");
                    match state.as_deref() {
                        Some("open") => open,
                        Some(_) => !open,
                        None => true,
                    }
                })
                .collect::<Vec<_>>();
            items.reverse();
            items.truncate(*limit);
            let pulls = items
                .into_iter()
                .map(|item| {
                    github_plugin_store::summary(
                        &item.subject,
                        item.facts.as_object().cloned().unwrap_or_default(),
                        Some(item.claim),
                        None,
                    )
                })
                .collect::<Vec<_>>();
            json!({"pulls": pulls})
        }
        Read::Pull(number) => {
            let pull = store
                .repository_items(resource, Some(*number))?
                .into_iter()
                .find(|item| item.kind == "vcs.pull-request")
                .map(|item| json!({
                    "subject": item.subject,
                    "kind": item.kind,
                    "number": number,
                    "facts": item.facts,
                    "claim": item.claim,
                }));
            json!({"pull": pull})
        }
        Read::Checks(number) => {
            let checks = store
                .repository_items(resource, Some(*number))?
                .into_iter()
                .find(|item| item.kind == "vcs.pull-request")
                .map(|item| json!({
                    "subject": item.subject,
                    "number": number,
                    "head_sha": item.facts.get("head_sha"),
                    "checks_state": item.facts.get("checks_state"),
                    "checks": item.facts.get("checks"),
                    "required_checks": item.facts.get("required_checks"),
                    "heads": null,
                }));
            json!({"checks": checks})
        }
        Read::Events { number, limit } => json!({"events": store.github_events(resource, *number, *limit)?}),
    })
}
