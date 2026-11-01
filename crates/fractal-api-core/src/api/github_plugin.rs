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
    /// This read for a poller that takes typed reads (capability `github-plugin` version 2): it
    /// builds the path itself.
    fn typed(&self, owner: &str, repository: &str) -> crate::peer::ClientReadOperation {
        let (read, number, state, limit) = match self {
            Self::Pulls { state, limit } => ("pulls", None, Some(state.clone().unwrap_or_else(|| "all".into())), Some(*limit)),
            Self::Pull(number) => ("pull", Some(*number), None, None),
            Self::Checks(number) => ("checks", Some(*number), None, None),
            Self::Events { number, limit } => ("events", *number, None, Some(*limit)),
        };
        crate::peer::ClientReadOperation::GithubPluginRead {
            owner: owner.to_owned(),
            repository: repository.to_owned(),
            read: read.to_owned(),
            number,
            state,
            limit,
        }
    }

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
                // Until it is filled from GitHub, and again once it falls behind, a mirror may be
                // missing or stale facts; it answers nothing.
                Some(row) if mirror.filled(&resource).map_err(internal_error)? => {
                    Ok(Some((row, from_mirror(&mirror, &resource, &read_mirror).map_err(internal_error)?)))
                }
                Some(_) => Ok(Some((Value::Null, Value::Null))),
                None => Ok(None),
            }
        })
        .await?;
        match mirrored {
            Some((row, body)) if !row.is_null() => {
                // The mirror holds the open items it was filled with and every item a look
                // recorded since; an older closed item it never saw is the graph's to answer.
                let unseen = matches!(read, Read::Pull(_) | Read::Checks(_))
                    && body.as_object().and_then(|body| body.values().next()).is_some_and(Value::is_null);
                if unseen {
                    return fallback(
                        "the poller's mirror holds the open items it was filled with and the items changed since; this one is not among them".into(),
                        None,
                    )
                    .await;
                }
                envelope["source"] = json!({
                    "kind": "mirror",
                    "host": client_host_id(&here),
                    "mirrored_at_unix_ms": row["mirrored_at_unix_ms"],
                    "filled_at_unix_ms": row["filled_at_unix_ms"],
                });
                merge(&mut envelope, body);
                return Ok(Json(envelope));
            }
            Some(_) => {
                return fallback("the poller is refilling its mirror from GitHub".into(), None).await;
            }
            None => {}
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
    let advertised = advertised.as_ref().and_then(Value::as_i64);
    if advertised.is_none_or(|version| version < 1) {
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
        // A poller of plugin version 2 builds the path from the read's parts; version 1 takes it.
        request: if advertised.is_some_and(|version| version >= 2) {
            read.typed(&owner, &repository)
        } else {
            crate::peer::ClientReadOperation::GithubPlugin { path: read.path(&owner, &repository) }
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_watch::ThreadRef;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;

    async fn get(state: &AppState, path: &str) -> (StatusCode, Value) {
        let response = super::super::router(state.clone())
            .oneshot(Request::get(path).header(client_v0::LOCAL_PERSON_HEADER, "person/ada").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let mut body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body.get_mut("value").map(Value::take).unwrap_or(body))
    }

    fn observe(store: &Store, number: u64, facts: Value) -> String {
        store.append_claim(&crate::model::ClaimInput {
            subject: format!("resource/github/acme/garden/pull-request/{number}"),
            kind: "resource.observed".into(),
            actor: None,
            fields: BTreeMap::from([("kind".into(), json!("vcs.pull-request")), ("facts".into(), facts)]),
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: None,
        }).unwrap().id
    }

    /// The member that polls a repository answers from its mirror once the mirror has taken a
    /// look, and from its graph, saying so, before then.
    #[tokio::test]
    async fn the_poller_answers_from_its_mirror_and_labels_a_graph_answer() {
        let root = tempfile::tempdir().unwrap();
        let state = super::super::tests::state(root.path());
        let store = state.store.clone();
        let intent = crate::graph::parse_internal_intent(
            "version 2\nagent \"example.planner\" { workspace \"/tmp\"; command \"true\" }\n",
            "node",
        ).unwrap();
        store.apply_internal(&intent, "watch-seat").unwrap();
        store.declare_watch(&ThreadRef::parse("acme/garden#7").unwrap(), "agent/example.planner", None).unwrap();
        let facts = json!({"number": 7, "title": "Garden paths", "state": "open", "head_sha": "a",
            "required_checks": {"state": "pass"}, "checks": [{"name": "build", "conclusion": "success"}]});
        let claim = observe(&store, 7, facts.clone());
        observe(&store, 8, json!({"number": 8, "title": "Old path", "state": "closed", "merged": true}));
        let mirror = Arc::new(GithubPluginStore::memory());
        GithubPluginStore::share(root.path(), mirror.clone());

        let (status, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls").await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["source"]["kind"], "graph", "{answer}");
        assert!(answer["source"]["reason"].as_str().unwrap().contains("has not taken a look"));
        assert_eq!(answer["poller"], "host/node");
        assert_eq!(answer["pulls"].as_array().unwrap().len(), 1, "open by default");
        assert_eq!(answer["pulls"][0]["number"], 7);

        let item = |number: u64, facts: Value, claim: &str| github_plugin_store::MirrorItem {
            subject: format!("resource/github/acme/garden/pull-request/{number}"),
            item_kind: "vcs.pull-request".into(),
            facts: facts.as_object().unwrap().clone(),
            claim: Some(claim.into()),
        };
        let resource = "resource/github/acme/garden";
        let observer = "observer/github/acme/garden";
        mirror.mirror(resource, observer, "acme/garden", None, Some("c1"), &[item(7, facts.clone(), &claim)]).unwrap();
        let (_, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls").await;
        assert_eq!(answer["source"]["kind"], "graph", "an unfilled mirror answers nothing: {answer}");
        assert_eq!(answer["source"]["reason"], "the poller is refilling its mirror from GitHub");
        assert!(mirror.fill(resource, observer, "acme/garden", Some("c1"), &[item(7, facts.clone(), &claim)]).unwrap());
        mirror.mirror(resource, observer, "acme/garden", Some("c1"), Some("c2"), &[item(7, json!({
            "number": 7, "title": "Garden paths", "state": "open", "head_sha": "b", "required_checks": {"state": "fail", "failed": ["build"]}}), "c2")]).unwrap();
        let (_, answer) = get(&state, "/v1/plugins/github/repos/ACME/garden/pulls?state=all").await;
        assert_eq!(answer["source"]["kind"], "mirror", "{answer}");
        assert_eq!(answer["source"]["host"], "host/node");
        assert_eq!(answer["pulls"][0]["required_checks_state"], "fail");
        let (_, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls/7").await;
        assert_eq!(answer["pull"]["facts"]["head_sha"], "b");
        let (_, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls/7/checks").await;
        assert_eq!(answer["checks"]["heads"].as_array().unwrap().len(), 2);
        let (_, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls/9").await;
        assert!(answer["pull"].is_null());
        // A closed item the mirror never saw is the graph's to answer.
        let (_, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls/8").await;
        assert_eq!(answer["source"]["kind"], "graph", "{answer}");
        assert_eq!(answer["pull"]["facts"]["title"], "Old path");
        let (_, answer) = get(&state, "/v1/plugins/github/repos/acme/garden/events?number=7").await;
        let events: Vec<_> = answer["events"].as_array().unwrap().iter().map(|event| event["event"].as_str().unwrap()).collect();
        assert_eq!(events, ["checks-failed", "head-changed"]);

        let (status, _) = get(&state, "/v1/plugins/github/repos/acme/orchard/pulls").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "no member polls it");
        let (status, _) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls?state=merged").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = get(&state, "/v1/plugins/github/repos/acme/garden/pulls?limit=0").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
