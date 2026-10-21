use crate::MissionsStore;
use crate::WorkListStore;
use crate::RosterAdmissionStore;
use crate::RuntimeStartsStore;
use crate::SeatsStore;
use crate::EvidenceStore;
use crate::PublishedViewsStore;
use super::*;

fn client_launch_approval_resources(
    state: &AppState,
    session: &PlanningSessionView,
) -> anyhow::Result<Vec<Value>> {
    let variants = client_launch_variant_resources(state, session)?;
    let tokens = variants
        .into_iter()
        .filter_map(|variant| {
            Some((
                variant["candidate_revision"].as_u64()?,
                (
                    variant["preview_token"].as_str()?.to_owned(),
                    variant["id"].as_str()?.to_owned(),
                ),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let claims = state.store.claims_for(&session.subject, None)?;
    let mut approvals = claims
        .into_iter()
        .filter(|claim| claim.kind == "planning-session.approved")
        .filter_map(|claim| {
            let revision = claim.body.pointer("/fields/candidate_revision")?.as_u64()?;
            let (token, variant_id) = tokens.get(&revision)?;
            let reviewer = claim
                .body
                .pointer("/fields/requester")
                .and_then(Value::as_str)
                .or(claim.actor.as_deref())?;
            Some(json!({
                "id": format!("launch-approval/{}/{}", session.id, revision),
                "kind": "launch-approval",
                "revision": claim.id,
                "updated_at": client_timestamp(claim.accepted_at_unix_ms),
                "launch_id": format!("launch/{}", session.id),
                "variant_id": variant_id,
                "reviewer": reviewer,
                "state": "approved",
                "preview_token": token,
                "decided_at": client_timestamp(claim.accepted_at_unix_ms),
                "operational": { "layer": "history", "actionable": false, "reasons": ["approved"] }
            }))
        })
        .collect::<Vec<_>>();
    approvals.sort_by(|left, right| {
        left["decided_at"]
            .as_str()
            .cmp(&right["decided_at"].as_str())
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    Ok(approvals)
}

fn client_history_resource(claim: ClaimRecord) -> Value {
    let mut targets = vec![claim.subject.clone()];
    if let Some(actor) = &claim.actor
        && actor.contains('/')
        && !actor.chars().any(char::is_whitespace)
    {
        targets.push(actor.clone());
    }
    json!({
        "id": format!("history/{}", claim.store_index),
        "kind": "history",
        "revision": claim.id,
        "updated_at": client_timestamp(claim.accepted_at_unix_ms),
        "event_type": claim.kind,
        "occurred_at": client_timestamp(claim.accepted_at_unix_ms),
        "store_index": claim.store_index,
        "summary": format!("{} on {}", claim.kind, claim.subject),
        "targets": targets,
        "operational": { "layer": "history", "actionable": false, "reasons": ["audit"] }
    })
}

pub async fn client_work(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    if query.history {
        return client_work_history(&state, snapshot, &query).await;
    }
    // While a refresher governs the work list (from when it holds the list's view, before it
    // starts), pages come from its publications and never fold the list on read. With none,
    // as with ST3_PUBLISHED_LISTS=off, they read directly, as before.
    if !matches!(
        state.store.published_work_list().current(),
        crate::store::published_list::ListRead::NeverStarted
    ) || state.store.collection_view_published("work")
    {
        return client_work_published(state, snapshot, query).await;
    }
    let actor = query.actor.clone();
    client_snapshot_page(&state, snapshot, "work", &query, move |state, snapshot| {
        client_work_resources(
            &state.store,
            actor.as_deref(),
            false,
            client_snapshot_time(snapshot),
            snapshot.store_index,
        )
    })
    .await
}

/// A current work page from the work list's publications. A first page serves the newest
/// current publication, or with `fresh=true` one at least as new as the request; a continuation
/// serves the publication its first page did. Neither folds the list: with nothing current to
/// serve it is not ready, and once the refresher has ended, unavailable.
async fn client_work_published(
    state: AppState,
    snapshot: ClientSnapshot,
    query: ClientListQuery,
) -> Result<ClientPageResponse, ApiError> {
    use crate::store::published_list::ListRead;
    if let Some(encoded) = &query.cursor {
        let cursor = decode_client_cursor(encoded)?;
        // A first page read before the refresher started continues from the page cache.
        if !cursor.items_digest.starts_with(PUBLISHED_WORK_DIGEST) {
            let page = client_page(&state, &snapshot, "work", Vec::new(), &query)?;
            return Ok((Extension(snapshot), Json(page)));
        }
        let page = client_work_published_continuation(&state, &snapshot, &query, cursor)?;
        return Ok((Extension(snapshot), Json(page)));
    }
    let list = state.store.published_work_list();
    let (publication, id) = if query.fresh {
        let waited = Instant::now();
        let fresh = wait_for_work_list(&state.store).await;
        record_work_stage(request_latency::WorkStage::FreshWait, waited.elapsed());
        fresh?
    } else {
        match list.current() {
            ListRead::Served(publication, id) => (publication, id),
            ListRead::NotReady | ListRead::NeverStarted => {
                list.request_refresh();
                return Err(published_lists::work_list_not_ready());
            }
            ListRead::Ended => return Err(published_lists::work_list_ended()),
        }
    };
    let reader = state.clone();
    blocking_store(move || Ok(client_work_published_page(&reader, &publication, id, &query))).await?
}

/// Wait up to [`WORK_LIST_FRESH_WAIT`] for a current publication at or after the store's index
/// when the request arrived. Each way it can end answers differently: such a publication; not
/// fresh, once the wait is over; not ready, if the list is withdrawn or forgotten meanwhile or
/// its change channel closes; unavailable, if the refresher ends. Never stale rows as fresh.
async fn wait_for_work_list(
    store: &Arc<Store>,
) -> Result<
    (
        Arc<crate::store::published_list::Publication<crate::store::work_list::WorkRows>>,
        crate::store::published_list::PublicationId,
    ),
    ApiError,
> {
    use crate::store::published_list::ListRead;
    let list = store.published_work_list();
    let wanted = store.index().map_err(ApiError::internal)?;
    let mut changes = list.subscribe();
    let deadline = tokio::time::Instant::now() + WORK_LIST_FRESH_WAIT;
    let mut asked = false;
    loop {
        drop(changes.borrow_and_update());
        let newest = match list.current() {
            ListRead::Served(publication, id) if publication.cut >= wanted => return Ok((publication, id)),
            // Behind the request: ask once for a fold now, without the rest of the pause.
            ListRead::Served(publication, _) => {
                if !asked {
                    list.request_fresh();
                    asked = true;
                }
                publication.cut
            }
            ListRead::Ended => return Err(published_lists::work_list_ended()),
            ListRead::NotReady | ListRead::NeverStarted => return Err(published_lists::work_list_not_ready()),
        };
        match tokio::time::timeout_at(deadline, changes.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(published_lists::work_list_not_ready()),
            Err(_) => return Err(published_lists::work_list_not_fresh(wanted, Some(newest))),
        }
    }
}

/// The next page of a published work page: the same publication and actor rows, sliced at the
/// cursor. It expires as a cached page does, and also once a forget replaced the projections
/// its rows were folded from. A later publication of the same generation leaves it valid.
fn client_work_published_continuation(
    state: &AppState,
    snapshot: &ClientSnapshot,
    query: &ClientListQuery,
    cursor: ClientPageCursor,
) -> Result<ClientResourcePage, ApiError> {
    if cursor.collection != "work"
        || cursor.history != query.history
        || cursor.person != query.person
        || cursor.actor != query.actor
        || cursor.owner_run != query.owner_run
        || cursor.owner.is_some()
        || cursor.state.is_some()
        || cursor.status != query.status
        || cursor.native_only != query.native_only
        || query.limit.is_some_and(|limit| limit.clamp(1, CLIENT_MAX_PAGE_ITEMS) != cursor.limit)
        || cursor.snapshot.id != snapshot.id
        || cursor.snapshot.store_index != snapshot.store_index
    {
        return Err(client_page_expired(
            "the page cursor does not match this collection, snapshot, or filter",
        ));
    }
    if client_now_ms() > cursor.expires_at_unix_ms {
        return Err(client_page_expired("the page cursor expired"));
    }
    // Served only from the store's own list, in the generation its rows were folded in.
    let list = state.store.published_work_list();
    let (rows, id) = {
        let mut pages = published_work_pages().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        purge_published_work_pages(&mut pages, list);
        // The cursor is the client's to edit: it must repeat the limit, snapshot and filters
        // its first page issued, so it can never ask for more than one bounded page, and two
        // pages with one digest and expiry but different contracts never answer for each other.
        let contract = PublishedWorkContract::of(&cursor);
        if !(1..=CLIENT_MAX_PAGE_ITEMS).contains(&cursor.limit) {
            return Err(client_page_expired("the page cursor does not match the page it continues"));
        }
        let page = pages
            .iter()
            .find(|page| {
                page.digest == cursor.items_digest
                    && page.expires_at_unix_ms == cursor.expires_at_unix_ms
                    && page.contract == contract
            })
            .ok_or_else(|| client_page_expired("the page snapshot is no longer available; restart pagination"))?;
        (Arc::clone(&page.rows), page.id)
    };
    let current = |list: &crate::store::published_list::PublishedList<crate::store::work_list::WorkRows>| {
        list.instance() == id.list && list.generation() == id.generation
    };
    if !current(list) {
        return Err(client_page_expired("the work list was refolded since this page was read; restart pagination"));
    }
    let end = cursor.offset.saturating_add(cursor.limit).min(rows.len());
    let items = rows[cursor.offset.min(end)..end].iter().map(|row| (**row).clone()).collect::<Vec<_>>();
    let has_more = end < rows.len();
    // The final fence: a forget while the page was sliced replaced the rows it holds.
    if !current(list) {
        return Err(client_page_expired("the work list was refolded since this page was read; restart pagination"));
    }
    let next_cursor = if has_more {
        let mut next = cursor.clone();
        next.offset = cursor.offset.saturating_add(items.len());
        Some(encode_client_cursor(&next)?)
    } else {
        None
    };
    Ok(ClientResourcePage {
        kind: "page".into(),
        collection: "work".into(),
        filters: client_page_filters("work", query),
        items,
        page: ClientPageInfo {
            limit: cursor.limit,
            has_more,
            next_cursor,
            cursor_expires_at: has_more.then(|| client_timestamp(cursor.expires_at_unix_ms)),
        },
        sync: client_sync_notice(state),
        replicated: None,
    })
}

/// The work history, read a page at a time in the order it shows, each page inside one SQLite
/// snapshot. Continuation seeks after the last update time and subject, so unrelated writes
/// do not invalidate it or shift its offset.
async fn client_work_history(
    state: &AppState,
    snapshot: ClientSnapshot,
    query: &ClientListQuery,
) -> Result<ClientPageResponse, ApiError> {
    let (offset, limit, expires_at_unix_ms, after_key) = if let Some(encoded) = &query.cursor {
        let cursor = decode_client_cursor(encoded)?;
        if cursor.collection != "work"
            || cursor.snapshot.id != snapshot.id
            || cursor.snapshot.store_index != snapshot.store_index
            || cursor.history != query.history
            || cursor.person != query.person
            || cursor.actor != query.actor
            || cursor.owner_run != query.owner_run
            || cursor.status != query.status
            || cursor.native_only != query.native_only
            || cursor.items_digest != "sql-page"
            || query
                .limit
                .is_some_and(|limit| limit.clamp(1, CLIENT_MAX_PAGE_ITEMS) != cursor.limit)
        {
            return Err(client_page_expired(
                "the page cursor does not match this collection, snapshot, or filter",
            ));
        }
        if client_now_ms() > cursor.expires_at_unix_ms {
            return Err(client_page_expired("the page cursor expired"));
        }
        (
            if cursor.after_key.is_some() {
                0
            } else {
                cursor.offset
            },
            cursor.limit,
            cursor.expires_at_unix_ms,
            cursor.after_key,
        )
    } else {
        (
            0,
            query
                .limit
                .unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS)
                .clamp(1, CLIENT_MAX_PAGE_ITEMS),
            client_now_ms().saturating_add(CLIENT_PAGE_TTL_MS),
            None,
        )
    };
    let reader = state.clone();
    let actor = query.actor.clone();
    let read = blocking_store(move || {
        reader.store.clone().read_snapshot(|index| {
            let snapshot = client_snapshot_at(&reader, index);
            let (items, has_more) = client_work_history_page_after(
                &reader.store,
                actor.as_deref(),
                client_snapshot_time(&snapshot),
                index,
                offset,
                limit,
                after_key.as_ref(),
            )?;
            Ok(Some((snapshot, items, has_more)))
        })
    })
    .await?;
    let Some((snapshot, items, has_more)) = read else {
        return Err(client_page_expired(
            "the snapshot changed; restart pagination from the first page",
        ));
    };
    let next_cursor = has_more
        .then(|| {
            encode_client_cursor(&ClientPageCursor {
                snapshot: snapshot.clone(),
                collection: "work".into(),
                offset: offset.saturating_add(items.len()),
                limit,
                history: query.history,
                person: query.person.clone(),
                actor: query.actor.clone(),
                owner_run: query.owner_run.clone(),
                status: query.status.clone(),
                owner: None,
                state: None,
                native_only: query.native_only,
                items_digest: "sql-page".into(),
                before_index: None,
                after_key: None,
                expires_at_unix_ms,
            })
        })
        .transpose()?;
    let page = ClientResourcePage {
        kind: "page".into(),
        collection: "work".into(),
        filters: client_page_filters("work", query),
        items,
        page: ClientPageInfo {
            limit,
            has_more,
            next_cursor,
            cursor_expires_at: has_more.then(|| client_timestamp(expires_at_unix_ms)),
        },
        sync: client_sync_notice(state),
        replicated: None,
    };
    Ok((Extension(snapshot), Json(page)))
}

pub async fn client_work_detail(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    let actor = query.actor.clone();
    let snapshot_unix_ms = client_snapshot_time(&snapshot);
    let requested = id.clone();
    let item = blocking_store(move || {
        client_work_item(
            &store,
            &requested,
            actor.as_deref(),
            snapshot_unix_ms,
            snapshot.store_index,
        )
    })
    .await?;
    item.map(Json).ok_or_else(|| {
        ApiError::not_found(format!(
            "work `{}` does not exist",
            client_detail_id("work", &id)
        ))
    })
}

pub async fn client_agents(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    if query.min_cut.is_some() && query.cursor.is_some() {
        return Err(client_v0::validation("min_cut is only supported on the first agents page"));
    }
    if let Some(wanted) = query.min_cut {
        let head = state.store.index().map_err(ApiError::internal)?;
        if wanted > head {
            let mut error = agent_roster_cut_not_ready(wanted);
            error.details.insert("head".into(), json!(head));
            return Err(error);
        }
    }
    // A daemon's readers never fold the roster: they answer from its refresher's publications.
    if state.store.agent_roster_refresher_running() {
        return client_agents_published(state, snapshot, query).await;
    }
    // A warm request never queues behind a cold projection. Drop the probe's SQLite
    // snapshot before waiting, then recheck all cache fences in the admitted snapshot.
    // Embedded stores without a refresher build once and reject an unmet cut immediately;
    // only the daemon's publication path has a refresher to wait for.
    let requested = query.min_cut.unwrap_or(snapshot.store_index);
    let fresh = query.fresh || query.min_cut.is_some();
    let min_cut = query.min_cut;
    for admitted in [false, true] {
        let admission = if admitted {
            Some(state.store.admit_agent_resources("http").await)
        } else {
            None
        };
        let reader = state.clone();
        let snapshot = snapshot.clone();
        let query = query.clone();
        let filtered = query.status.is_some();
        let result = blocking_roster(move || {
            let _admission = admission;
            if let Some(admission) = &_admission {
                admission.enter_worker();
            }
            reader.store.clone().read_snapshot(|index| {
                let snapshot = if query.cursor.is_some() {
                    snapshot
                } else {
                    client_snapshot_at(&reader, index)
                };
                Ok(reader.store.with_owned_set_snapshot_reads(|| {
                    client_agents_page_at(&reader, snapshot, &query, admitted)
                }))
            })
        }).await??;
        if let Some(page) = result {
            let page = checked_agent_roster_page(Some(page), min_cut)?
                .expect("an exact roster page is present");
            let path = match (filtered, admitted) {
                (true, _) => "filtered_full",
                (false, true) => "exact_cold",
                (false, false) => "exact_warm",
            };
            crate::otel::record_roster_read(path, requested, Some(page.0.0.store_index), fresh);
            return Ok(page);
        }
    }
    unreachable!("an admitted roster read always builds missing cards")
}

/// An agents page from the refresher's publications. A first page answers at once from the
/// newest one, or says not ready; a legacy fresh read briefly waits for its cut or for proof
/// that a complete publication's cards are unchanged through it, while a named min_cut uses one
/// strict, fixed mutation-cut barrier. A continuation answers from the roster published at
/// exactly its first page's cut, or says the page expired. Neither folds a card.
async fn client_agents_published(
    state: AppState,
    snapshot: ClientSnapshot,
    query: ClientListQuery,
) -> Result<ClientPageResponse, ApiError> {
    state.store.note_agent_roster_read();
    let requested = query.min_cut.unwrap_or(snapshot.store_index);
    if query.cursor.is_some() {
        let reader = state.clone();
        return blocking_roster(move || {
            Ok(client_agents_published_continuation(&reader, snapshot, &query))
        })
        .await?;
    }
    let fresh = query.fresh || query.min_cut.is_some();
    let fresh_wait = if let Some(wanted) = query.min_cut {
        let waited = Instant::now();
        let published = wait_for_agent_roster_cut(&state.store, query.history, wanted).await;
        record_roster_stage(request_latency::RosterStage::FreshWait, waited.elapsed());
        crate::otel::roster_attribute("st.roster.publication_wait_ms", waited.elapsed().as_secs_f64() * 1000.0);
        if !published {
            return Err(agent_roster_cut_not_ready(wanted));
        }
        None
    } else if query.fresh {
        let waited = Instant::now();
        let outcome = wait_for_agent_roster(&state.store, query.history).await;
        record_roster_stage(request_latency::RosterStage::FreshWait, waited.elapsed());
        Some(outcome)
    } else {
        None
    };
    let reader = state.clone();
    let history = query.history;
    let min_cut = query.min_cut;
    let built = Instant::now();
    let page = blocking_roster(move || {
        Ok(client_agents_published_page(&reader, &query, requested, fresh_wait))
    }).await;
    if fresh {
        record_roster_stage(request_latency::RosterStage::FreshPage, built.elapsed());
    }
    match checked_agent_roster_page(page??, min_cut)? {
        Some(page) => Ok(page),
        None => {
            crate::otel::record_roster_read("not_ready", requested, None, fresh);
            if history {
                state.store.request_agent_roster_history();
            } else {
                state.store.request_agent_roster_refresh();
            }
            Err(agent_roster_not_ready())
        }
    }
}

/// A page continuing a first page served from a publication: its cards come from the roster
/// published at exactly that page's cut. Once that roster is gone, pagination restarts.
fn client_agents_published_continuation(
    state: &AppState,
    snapshot: ClientSnapshot,
    query: &ClientListQuery,
) -> Result<ClientPageResponse, ApiError> {
    let store = &state.store;
    let mut page = client_page_read(state, &snapshot, "agents", Vec::new(), query, true)?;
    let published = store.published_agent_roster_at(snapshot.store_index, query.history);
    let path = if query.status.is_some() { "filtered_full" } else { "published_complete" };
    let served = published.as_ref().map(|_| snapshot.store_index);
    crate::otel::record_roster_read(path, snapshot.store_index, served, query.fresh);
    let snapshot = match &published {
        Some((_, published_at)) => roster_snapshot(state, snapshot.store_index, *published_at),
        None => snapshot,
    };
    // A status-filtered first page kept its whole overlaid cards for its continuations.
    if query.status.is_some() {
        return Ok((Extension(snapshot), Json(page)));
    }
    let Some((cards, _)) = published else {
        return Err(client_page_expired(
            "the roster this page was read from is no longer published; restart pagination",
        ));
    };
    page.items = client_agent_cards_from_cached(
        store, page_cards(&cards, &page.items), &page.items, &snapshot.created_at,
    )?;
    Ok((Extension(snapshot), Json(page)))
}

fn client_agents_page_at(
    state: &AppState,
    snapshot: ClientSnapshot,
    query: &ClientListQuery,
    admitted: bool,
) -> Result<Option<ClientPageResponse>, ApiError> {
    let store = &state.store;
    let index = snapshot.store_index;
    let mut items = if query.cursor.is_some() {
        Vec::new()
    } else if query.status.is_some() {
        let mut cards = if admitted {
            client_agent_resources_cached(store, query.history, index).map_err(ApiError::internal)?
        } else {
            let Some(cards) = store.agent_resources_cached_at(index, query.history, None)
                .map_err(ApiError::internal)? else { return Ok(None) };
            (*cards).clone()
        };
        overlay_agent_resources(store, &mut cards, &snapshot.created_at)
            .map_err(ApiError::internal)?;
        cards
    } else if admitted {
        client_agent_page_refs(store, query.history, index).map_err(ApiError::internal)?
    } else {
        let Some(refs) = store.agent_page_refs_cached_at(index, query.history) else {
            return Ok(None);
        };
        (*refs).clone()
    };
    if let Some(status) = query.status.as_deref() {
        items.retain(|item| item["state"].as_str() == Some(status));
    }
    // On a first-page probe, determine coverage before publishing pagination metadata.
    let cached_cards = if !admitted && query.status.is_none() {
        let refs = if query.cursor.is_some() {
            client_page_read(state, &snapshot, "agents", Vec::new(), query, true)?.items
        } else {
            items.iter().take(query.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS)
                .clamp(1, CLIENT_MAX_PAGE_ITEMS)).cloned().collect()
        };
        let selected = refs.iter().filter_map(|r| r["id"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>();
        let Some(cards) = store.agent_resources_cached_at(index, query.history, Some(&selected))
            .map_err(ApiError::internal)? else { return Ok(None) };
        Some(cards.iter().filter(|card| card["id"].as_str()
            .is_some_and(|id| selected.contains(id))).cloned().collect())
    } else {
        None
    };
    let mut page = client_page_read(state, &snapshot, "agents", items, query, true)?;
    if query.status.is_none() {
        page.items = if let Some(cards) = cached_cards {
            client_agent_cards_from_cached(store, cards, &page.items, &snapshot.created_at)?
        } else {
            client_agent_cards_for_page(store, query.history, index, &page.items, &snapshot.created_at)?
        };
    }
    Ok(Some((Extension(snapshot), Json(page))))
}

pub async fn client_agents_detail(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<Value>, ApiError> {
    let store = state.store.clone();
    use crate::store::identities::IdentityKind;
    let requested_id = client_detail_id("agent", &id);
    let resolved = public_identity(&store, IdentityKind::Agent, &requested_id)?;
    let identity = resolved.identity;
    let history = query.history;
    let created_at = snapshot.created_at.clone();
    let snapshot_index = snapshot.store_index;
    let subject = identity.to_owned();
    let items = blocking_store(move || {
        let selected = BTreeSet::from([subject]);
        let mut items = client_agent_resources_selected(
            &store,
            history,
            snapshot_index,
            Some((&selected, &[])),
        )?;
        add_agent_todos(&store, &mut items, snapshot_index)?;
        overlay_agent_resources(&store, &mut items, &created_at)?;
        for item in &mut items {
            if let Some(subject) = item["id"].as_str()
                && let Some(operation) = store.runtime_start_for(subject)?
            {
                let phase = if operation.revoked_by.is_some() { "revoked" }
                    else if operation.result.as_ref().is_some_and(|result| result["phase"] == "unknown") { "unknown" }
                    else if operation.finished { "completed" }
                    else if operation.dispatched { "running" } else { "queued" };
                let (running_jobs, revoked_running_jobs) = store.runtime_start_counts()?;
                let (unresolved_jobs, revoked_unresolved_jobs) = store.unresolved_runtime_start_counts()?;
                item["runtime_start"] = json!({
                    "operation_id": operation.id, "phase": phase,
                    "worker_running": operation.dispatched && !operation.finished,
                    "cleanup_verified": operation.cleanup_verified,
                    "running_jobs": running_jobs,
                    "revoked_running_jobs": revoked_running_jobs,
                    "unresolved_jobs": unresolved_jobs,
                    "revoked_unresolved_jobs": revoked_unresolved_jobs,
                });
            }
        }
        Ok(items)
    })
    .await?;
    client_detail(items, "agent", &identity)
}

pub async fn client_sessions(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    let history = query.history;
    let native_only = query.native_only;
    client_snapshot_page(
        &state,
        snapshot,
        "sessions",
        &query,
        move |state, snapshot| {
            let mut items = client_session_resources(
                &state.store,
                history,
                &snapshot.created_at,
                snapshot.store_index,
                state.native_session_home.as_deref(),
                native_only,
            )?;
            if native_only {
                items.retain(|item| item.get("managed") == Some(&Value::Bool(false)));
            }
            Ok(items)
        },
    )
    .await
}

pub async fn client_sessions_detail(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<client_v0::ClientSession>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<Value>, ApiError> {
    if let Some(id) = id.strip_suffix("/timeline") {
        // An agent's timeline is its current session's: st resolves it, not the client.
        let session_id = crate::relay_trace::result(crate::relay_trace::Phase::Session, || {
            client_v0::conversation_session_id(&state, id)
        })?;
        let id = session_id.as_str();
        let managed = crate::relay_trace::result(crate::relay_trace::Phase::Owner, || {
            managed_session_owner_at(&state.store, snapshot.store_index, &session_id)
        })
        .map_err(ApiError::internal)?;
        if let Some((_, _, origin)) = managed {
            let remote_host = origin
                .as_deref()
                .filter(|origin| *origin != state.store.origin())
                .map(client_host_id);
            if let Some(remote_host) = remote_host {
                let relay = state
                    .client_relay
                    .as_ref()
                    .ok_or_else(|| remote_unavailable_for_owner(&state, &remote_host))?;
                if !client_v0::acting_party(&session) {
                    return Err(ApiError::bad(St3Error::new(
                        "forbidden",
                        "remote session detail requires a concrete person or agent",
                    )));
                }
                let mut value = relay
                    .read(
                        &remote_host,
                        &crate::peer::ClientReadRequest {
                            authority_actor: session.authority_actor.clone(),
                            relay: None,
                            request: crate::peer::ClientReadOperation::Timeline {
                                session_id,
                                limit: query.limit.unwrap_or(50).clamp(1, 200),
                                cursor: query.cursor.clone(),
                            },
                        },
                    )
                    .await
                    .map_err(|error| {
                        client_v0::conversation_blocks::availability(remote_read_error(
                            &remote_host,
                            error,
                        ))
                    })?;
                client_v0::conversation_blocks::legacy(&mut value, &session);
                return Ok(Json(value));
            }
        }
        return client_v0::timeline_value(&state, &snapshot, &session, id, &query);
    }
    // Child transcripts are deliberately absent from the top-level session inventory.
    // Resolve their resource by the same parent/task identity used by timeline reads.
    let session_id = client_detail_id("session", &id);
    if let Some(child) = crate::subagent_sessions::resolve_child(
        state.native_session_home.as_deref(),
        &session_id,
    )
    .map_err(ApiError::internal)?
    {
        return Ok(Json(external_session_resource(child)));
    }
    client_detail(
        client_session_resources(
            &state.store,
            true,
            &snapshot.created_at,
            snapshot.store_index,
            state.native_session_home.as_deref(),
            false,
        )
        .map_err(ApiError::internal)?,
        "session",
        &id,
    )
}

/// A read that is not sent for a stated reason, in the shape of an unreachable owner.
fn remote_unavailable_because(host: &str, reason: &str, message: &str) -> ApiError {
    let mut error = remote_unavailable(host);
    error.details.insert("reason".into(), reason.into());
    error.message = message.into();
    error
}

pub async fn client_attention(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<client_v0::ClientSession>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    let person = client_v0::person_filter(&session, query.person.as_deref())?;
    let mut effective_query = query.clone();
    effective_query.person.clone_from(&person);
    let history = query.history;
    client_snapshot_page(
        &state,
        snapshot,
        "attention",
        &effective_query,
        move |state, _| {
            let mut items = client_attention_resources_with_previews(state, person.as_deref(), history)?;
            client_attention_compatibility(&mut items, session.custom_forms);
            Ok(items)
        },
    )
    .await
}

pub async fn client_attention_detail(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<Value>, ApiError> {
    let person = client_v0::person_filter(&session, query.person.as_deref())?;
    let mut items = client_attention_resources_with_previews(&state, person.as_deref(), query.history)
        .map_err(ApiError::internal)?;
    client_attention_compatibility(&mut items, session.custom_forms);
    client_detail(items, "attention", &id)
}

pub async fn client_messages(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<client_v0::ClientSession>,
    Extension(ClientMessageRequestId(request_id)): Extension<ClientMessageRequestId>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    let person = client_v0::person_filter(&session, query.person.as_deref())?;
    let mut effective_query = query.clone();
    effective_query.person.clone_from(&person);
    let actor = query.actor.clone();
    // An agent another host owns is listed by that host: its messages can reach this node late,
    // and a list that shows only what has arrived reads as empty while the replica lags.
    let owner = match actor.as_deref() {
        Some(actor) => remote_agent_owner(&state, actor).await?,
        None => None,
    };
    let mut replicated = None;
    if let (Some(owner), Some(actor)) = (owner.as_deref(), actor.as_deref()) {
        match relayed_messages_page(&state, &session, owner, actor, &query).await {
            Ok(mut page) => {
                page.replicated = Some(ClientReplicated {
                    owner_host_id: owner.to_owned(),
                    source: "owner".into(),
                    complete: true,
                    state: "current".into(),
                    reason: None,
                });
                if session.messages_paging {
                    client_message_page_size(&snapshot, &page, &request_id)?;
                }
                return Ok((Extension(snapshot), Json(page)));
            }
            Err(error) if error.code == "remote-unavailable" => {
                let lagging = client_sync_notice(&state)
                    .is_some_and(|notice| notice.peers.iter().any(|peer| peer.host_id == owner));
                replicated = Some(ClientReplicated {
                    owner_host_id: owner.to_owned(),
                    source: "replica".into(),
                    complete: false,
                    state: if lagging { "lagging" } else { "unverified" }.into(),
                    reason: error
                        .details
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
            }
            Err(error) => return Err(error),
        }
    }
    if session.messages_paging {
        return client_messages_sql_page_with_envelope(
            &state, snapshot, &effective_query, client_now_ms(), &request_id, replicated,
        ).await;
    }
    let history = query.history;
    let (snapshot, Json(mut page)) = client_snapshot_page(
        &state,
        snapshot,
        "messages",
        &effective_query,
        move |state, _| {
            client_message_resources(&state.store, person.as_deref(), history, actor.as_deref())
        },
    )
    .await
    .map(|(Extension(snapshot), page)| (snapshot, page))?;
    page.replicated = replicated;
    Ok((Extension(snapshot), Json(page)))
}

/// SQL pages retain only issued cut metadata, not a materialized JSON collection.
/// Appends preserve the cut; deletion invalidates it. Delivery age stays at issuance
/// while recipient delivery presence is assessed afresh on each page.
#[cfg(test)]
pub(super) async fn client_messages_sql_page(
    state: &AppState,
    snapshot: ClientSnapshot,
    query: &ClientListQuery,
) -> Result<ClientPageResponse, ApiError> {
    client_messages_sql_page_at(state, snapshot, query, client_now_ms()).await
}

#[cfg(test)]
pub(super) async fn client_messages_sql_page_at(
    state: &AppState,
    snapshot: ClientSnapshot,
    query: &ClientListQuery,
    request_time: u128,
) -> Result<ClientPageResponse, ApiError> {
    client_messages_sql_page_with_envelope(
        state, snapshot, query, request_time, &format!("request/{}", new_request_id()), None,
    ).await
}

pub(super) async fn client_messages_sql_page_with_envelope(
    state: &AppState,
    snapshot: ClientSnapshot,
    query: &ClientListQuery,
    request_time: u128,
    request_id: &str,
    replicated: Option<ClientReplicated>,
) -> Result<ClientPageResponse, ApiError> {
    let cursor = query.cursor.as_deref().map(decode_client_cursor).transpose()?;
    let cut = if let Some(cursor) = &cursor {
        if cursor.collection != "messages"
            || cursor.snapshot != snapshot
            || cursor.history != query.history
            || cursor.person != query.person
            || cursor.actor != query.actor
            || cursor.owner_run != query.owner_run
            || cursor.status != query.status
            || cursor.owner != query.owner
            || cursor.state != query.state
            || cursor.native_only != query.native_only
            || query.limit.is_some_and(|limit| limit.clamp(1, CLIENT_MAX_PAGE_ITEMS) != cursor.limit)
        {
            return Err(client_page_expired("the page cursor does not match this collection, snapshot, or filter"));
        }
        let cuts = CLIENT_MESSAGE_CUTS.lock();
        let store = Arc::downgrade(&state.store);
        let cut = cuts.iter().find(|cut| {
            cut.store.ptr_eq(&store) && cut.cursor == *cursor
        }).cloned().ok_or_else(|| {
            client_page_expired("the message page cut is no longer available; restart pagination")
        })?;
        if request_time >= cut.cursor.expires_at_unix_ms {
            return Err(client_page_expired("the page cursor expired"));
        }
        Some(cut)
    } else {
        None
    };
    let (limit, expires_at, through, after, offset, now, expected_epoch) = if let Some(cut) = &cut {
        (
            cut.cursor.limit, cut.cursor.expires_at_unix_ms, cut.cursor.before_index,
            cut.cursor.after_key.clone(), cut.cursor.offset, cut.request_time, Some(cut.cut_epoch),
        )
    } else {
        (
            query.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS).clamp(1, CLIENT_MAX_PAGE_ITEMS),
            request_time.saturating_add(CLIENT_PAGE_TTL_MS), None, None, 0, request_time, None,
        )
    };
    let reader = state.clone();
    let person = query.person.clone();
    let actor = query.actor.clone();
    let history = query.history;
    let continuing = cut.is_some();
    let (cut_epoch, read) = blocking_store(move || {
        reader.store.clone().read_snapshot(|index| {
            // Check the deletion fence in the same SQLite snapshot as selection/folding.
            let cut_epoch = reader.store.client_messages_cut_epoch()?;
            if expected_epoch.is_some_and(|expected| expected != cut_epoch) {
                return Ok((cut_epoch, None));
            }
            let through = through.unwrap_or(reader.store.client_messages_page_cut(index)?);
            let snapshot = if continuing { snapshot } else { client_snapshot_at(&reader, through) };
            let mut rows = reader.store.client_messages_page(
                person.as_deref(), actor.as_deref(), history, through, after.as_ref(), limit,
            )?;
            let selected_more = rows.len() > limit;
            rows.truncate(limit);
            // Delivery enrichment reads the store too: keep it on the blocking
            // worker and inside the same read snapshot as selector/body folding.
            let resources = rows.into_iter().map(|(message, metadata, current)| {
                let key = (
                    metadata["sent_at"].as_str().unwrap_or("0").parse::<u128>()?,
                    message.subject.clone(),
                );
                let resource = client_message_resource(&reader.store, message, &metadata, current, now)?;
                Ok((key, resource))
            }).collect::<anyhow::Result<Vec<_>>>()?;
            Ok((cut_epoch, Some((snapshot, through, selected_more, resources))))
        })
    }).await?;
    // Retire every affected receipt for this store, not just the cursor that
    // noticed deletion. Keep newer concurrent reads' receipts intact.
    {
        let store = Arc::downgrade(&state.store);
        CLIENT_MESSAGE_CUTS.lock().retain(|cut| {
            !cut.store.ptr_eq(&store) || cut.cut_epoch >= cut_epoch
        });
    }
    let Some((snapshot, through, selected_more, resources)) = read else {
        return Err(client_page_expired("the message page cut was invalidated; restart pagination"));
    };
    let mut keys = Vec::with_capacity(resources.len());
    let mut item_sizes = Vec::with_capacity(resources.len());
    let mut items = Vec::with_capacity(resources.len());
    for (key, resource) in resources {
        keys.push(key);
        item_sizes.push(client_encoded_size(&resource)?);
        items.push(resource);
    }
    let mut items_bytes: usize = item_sizes.iter().sum();
    let mut page = ClientResourcePage {
        kind: "page".into(), collection: "messages".into(),
        filters: client_page_filters("messages", query), items,
        page: ClientPageInfo {
            limit, has_more: selected_more, next_cursor: None, cursor_expires_at: None,
        },
        sync: client_sync_notice(state), replicated,
    };
    // Reuse the receipt identity while finding the largest fitting prefix. Only
    // the last returned key is ever issued; an excluded candidate stays ahead.
    let receipt = format!("message-cut/{}", uuid::Uuid::now_v7());
    loop {
        let has_more = selected_more || page.items.len() < keys.len();
        let cursor = if has_more && !page.items.is_empty() {
            Some(ClientPageCursor {
                snapshot: snapshot.clone(), collection: "messages".into(),
                offset: offset.saturating_add(page.items.len()), limit, history,
                person: query.person.clone(), actor: query.actor.clone(),
                owner_run: query.owner_run.clone(), status: query.status.clone(),
                owner: query.owner.clone(), state: query.state.clone(), native_only: query.native_only,
                items_digest: receipt.clone(), before_index: Some(through),
                after_key: Some(keys[page.items.len() - 1].clone()), expires_at_unix_ms: expires_at,
            })
        } else { None };
        page.page.has_more = has_more;
        page.page.next_cursor = cursor.as_ref().map(encode_client_cursor).transpose()?;
        page.page.cursor_expires_at = has_more.then(|| client_timestamp(expires_at));
        // Count each resource once; only the small envelope/cursor is recounted.
        let items = std::mem::take(&mut page.items);
        let framing_bytes = client_message_envelope_size(&snapshot, &page, request_id)?;
        page.items = items;
        let encoded_bytes = framing_bytes.saturating_add(items_bytes)
            .saturating_add(page.items.len().saturating_sub(1));
        if encoded_bytes <= CLIENT_MAX_RESPONSE_BYTES {
            if has_more && page.items.is_empty() {
                return Err(client_message_page_too_large());
            }
            if let Some(cursor) = cursor {
                let mut cuts = CLIENT_MESSAGE_CUTS.lock();
                let current_time = client_now_ms();
                cuts.retain(|cut| cut.cursor.expires_at_unix_ms > current_time && cut.store.strong_count() != 0);
                while cuts.len() >= CLIENT_MESSAGE_CUT_CAPACITY {
                    cuts.pop_front();
                }
                cuts.push_back(Arc::new(IssuedClientMessageCut {
                    store: Arc::downgrade(&state.store), cursor, cut_epoch, request_time: now,
                }));
            }
            return Ok((Extension(snapshot), Json(page)));
        }
        if page.items.len() <= 1 {
            return Err(client_message_page_too_large());
        }
        page.items.pop();
        items_bytes -= item_sizes[page.items.len()];
    }
}

fn client_message_page_too_large() -> ApiError {
    ApiError::bad(St3Error::new(
        "response-too-large",
        "a message and its page envelope cannot fit the advertised response byte limit; pagination cannot omit it",
    ))
}

fn client_encoded_size(value: &impl Serialize) -> Result<usize, ApiError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(ApiError::internal)?;
    Ok(counter.0)
}

pub(super) fn client_message_envelope_size(
    snapshot: &ClientSnapshot,
    page: &ClientResourcePage,
    request_id: &str,
) -> Result<usize, ApiError> {
    #[derive(Serialize)]
    struct Envelope<'a> {
        api_version: &'a str,
        request_id: &'a str,
        snapshot: &'a ClientSnapshot,
        value: &'a ClientResourcePage,
    }
    client_encoded_size(&Envelope { api_version: CLIENT_API_VERSION, request_id, snapshot, value: page })
}

fn client_message_page_size(
    snapshot: &ClientSnapshot,
    page: &ClientResourcePage,
    request_id: &str,
) -> Result<(), ApiError> {
    if client_message_envelope_size(snapshot, page, request_id)? > CLIENT_MAX_RESPONSE_BYTES {
        return Err(client_message_page_too_large());
    }
    Ok(())
}

/// The host that owns `actor`'s runtime, when that is another host.
async fn remote_agent_owner(state: &AppState, actor: &str) -> Result<Option<String>, ApiError> {
    if !actor.starts_with("agent/") {
        return Ok(None);
    }
    let (store, subject) = (state.store.clone(), actor.to_owned());
    let origin = blocking_store(move || {
        Ok(store
            .status(Some(&subject))?
            .subjects
            .first()
            .and_then(|subject| subject.actual_origin.clone()))
    })
    .await?;
    Ok(origin
        .filter(|origin| origin != state.store.origin())
        .map(|origin| client_host_id(&origin)))
}

/// One page of an agent's messages as its owner lists them.
async fn relayed_messages_page(
    state: &AppState,
    session: &client_v0::ClientSession,
    owner: &str,
    actor: &str,
    query: &ClientListQuery,
) -> Result<ClientResourcePage, ApiError> {
    if !client_v0::acting_party(session) {
        return Err(remote_unavailable_because(
            owner,
            "not-relayed",
            "a remote agent's messages are relayed only for a concrete person or agent",
        ));
    }
    let relay = state
        .client_relay
        .as_ref()
        .filter(|relay| relay.reaches(owner))
        .ok_or_else(|| remote_unavailable_for_owner(state, owner))?;
    let value = relay
        .read(
            owner,
            &crate::peer::ClientReadRequest {
                authority_actor: session.authority_actor.clone(),
                relay: None,
                request: crate::peer::ClientReadOperation::Messages {
                    actor: actor.to_owned(),
                    history: query.history,
                    limit: query.limit.map(|limit| limit.clamp(1, 200)),
                    cursor: query.cursor.clone(),
                    messages_paging: session.messages_paging,
                },
            },
        )
        .await
        .map_err(|error| remote_read_error(owner, error))?;
    serde_json::from_value(value).map_err(ApiError::internal)
}

pub async fn client_messages_detail(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<Value>, ApiError> {
    let person = client_v0::person_filter(&session, query.person.as_deref())?;
    client_detail(
        client_message_resources(&state.store, person.as_deref(), query.history, None)
            .map_err(ApiError::internal)?,
        "message",
        &id,
    )
}

pub async fn client_launches(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<ClientResourcePage>, ApiError> {
    let items = client_launch_resources(&state, query.history).map_err(ApiError::internal)?;
    client_page(&state, &snapshot, "launches", items, &query).map(Json)
}

pub async fn client_launches_detail(
    State(state): State<AppState>,
    Extension(session): Extension<client_v0::ClientSession>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<(Extension<ClientSnapshot>, Json<Value>), ApiError> {
    client_v0::require_scope(&session, "read.projections")?;
    let (snapshot, value) = blocking_store(move || {
        state.store.read_snapshot(|index| {
            let value = client_launch_detail_at(&state, &id, query.history)?.ok_or_else(|| {
                anyhow::anyhow!(St3Error::new(
                    "not-found",
                    format!(
                        "launch `{}` does not exist",
                        client_detail_id("launch", &id)
                    ),
                ))
            })?;
            Ok((client_snapshot_at(&state, index), value))
        })
    })
    .await?;
    Ok((Extension(snapshot), Json(value)))
}

pub async fn client_launch_variants(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<ClientResourcePage>, ApiError> {
    let session = client_launch_session(&state, &id)?;
    let items = client_launch_variant_resources(&state, &session).map_err(ApiError::internal)?;
    client_page(
        &state,
        &snapshot,
        &format!("launches/{id}/variants"),
        items,
        &query,
    )
    .map(Json)
}

pub async fn client_launch_variant_detail(
    State(state): State<AppState>,
    AxumPath((id, variant)): AxumPath<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let session = client_launch_session(&state, &id)?;
    client_detail(
        client_launch_variant_resources(&state, &session).map_err(ApiError::internal)?,
        "launch-variant",
        &format!("{}/{variant}", session.id),
    )
}

pub async fn client_launch_decisions(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<ClientResourcePage>, ApiError> {
    let session = client_launch_session(&state, &id)?;
    let items =
        client_launch_decision_resources(&state.store, &session).map_err(ApiError::internal)?;
    client_page(
        &state,
        &snapshot,
        &format!("launches/{id}/decisions"),
        items,
        &query,
    )
    .map(Json)
}

pub async fn client_launch_decision_detail(
    State(state): State<AppState>,
    AxumPath((id, decision)): AxumPath<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let session = client_launch_session(&state, &id)?;
    client_detail(
        client_launch_decision_resources(&state.store, &session).map_err(ApiError::internal)?,
        "launch-decision",
        &decision,
    )
}

pub async fn client_launch_approvals(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<ClientResourcePage>, ApiError> {
    let session = client_launch_session(&state, &id)?;
    let items = client_launch_approval_resources(&state, &session).map_err(ApiError::internal)?;
    client_page(
        &state,
        &snapshot,
        &format!("launches/{id}/approvals"),
        items,
        &query,
    )
    .map(Json)
}

pub async fn client_launch_approval_detail(
    State(state): State<AppState>,
    AxumPath((id, approval)): AxumPath<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let session = client_launch_session(&state, &id)?;
    client_detail(
        client_launch_approval_resources(&state, &session).map_err(ApiError::internal)?,
        "launch-approval",
        &format!("{}/{approval}", session.id),
    )
}

pub async fn client_history(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<ClientResourcePage>, ApiError> {
    let requested_limit = query
        .limit
        .unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS)
        .clamp(1, CLIENT_MAX_PAGE_ITEMS);
    let (before_index, limit, offset, expires_at_unix_ms) = if let Some(encoded) = &query.cursor {
        let cursor = decode_client_cursor(encoded)?;
        if cursor.collection != "history"
            || cursor.snapshot.id != snapshot.id
            || cursor.snapshot.store_index != snapshot.store_index
            || cursor.history != query.history
            || cursor.person != query.person
            || cursor.actor != query.actor
            || cursor.owner_run != query.owner_run
            || cursor.status != query.status
            || cursor.native_only != query.native_only
            || query
                .limit
                .is_some_and(|limit| limit.clamp(1, CLIENT_MAX_PAGE_ITEMS) != cursor.limit)
        {
            return Err(client_page_expired(
                "the page cursor does not match this collection, snapshot, or filter",
            ));
        }
        if client_now_ms() > cursor.expires_at_unix_ms {
            return Err(client_page_expired("the page cursor expired"));
        }
        let before = cursor
            .before_index
            .ok_or_else(|| client_page_expired("the history cursor is malformed"))?;
        (
            before,
            cursor.limit,
            cursor.offset,
            cursor.expires_at_unix_ms,
        )
    } else {
        (
            snapshot.store_index.saturating_add(1),
            requested_limit,
            0,
            client_now_ms().saturating_add(CLIENT_PAGE_TTL_MS),
        )
    };
    let store = state.store.clone();
    let page =
        blocking_store(move || store.claims_page(None, None, 0, Some(before_index), true, limit))
            .await?;
    let has_more = page.next_cursor.is_some();
    let items = page
        .claims
        .into_iter()
        .map(client_history_resource)
        .collect::<Vec<_>>();
    let next_cursor = page
        .next_cursor
        .map(|next| {
            encode_client_cursor(&ClientPageCursor {
                snapshot: snapshot.clone(),
                collection: "history".into(),
                offset: offset.saturating_add(items.len()),
                limit,
                history: query.history,
                person: query.person.clone(),
                actor: query.actor.clone(),
                owner_run: query.owner_run.clone(),
                status: query.status.clone(),
                owner: None,
                state: None,
                native_only: query.native_only,
                items_digest: String::new(),
                before_index: Some(next),
                after_key: None,
                expires_at_unix_ms,
            })
        })
        .transpose()?;
    let mut filters = BTreeMap::new();
    if query.history {
        filters.insert("history".into(), "all".into());
    }
    for (name, value) in [
        ("person", query.person.as_ref()),
        ("actor", query.actor.as_ref()),
        ("owner_run", query.owner_run.as_ref()),
        ("status", query.status.as_ref()),
    ] {
        if let Some(value) = value {
            filters.insert(name.into(), value.clone());
        }
    }
    if query.native_only {
        filters.insert("native_only".into(), "true".into());
    }
    Ok(Json(ClientResourcePage {
        kind: "page".into(),
        collection: "history".into(),
        filters,
        items,
        page: ClientPageInfo {
            limit,
            has_more,
            next_cursor,
            cursor_expires_at: has_more.then(|| client_timestamp(expires_at_unix_ms)),
        },
        sync: client_sync_notice(&state),
        replicated: None,
    }))
}

pub async fn client_history_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let id = client_detail_id("history", &id);
    let index = id
        .strip_prefix("history/")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|index| *index > 0)
        .ok_or_else(|| ApiError::not_found(format!("history `{id}` does not exist")))?;
    let upper = index
        .checked_add(1)
        .ok_or_else(|| ApiError::not_found(format!("history `{id}` does not exist")))?;
    let store = state.store.clone();
    let page =
        blocking_store(move || store.claims_page(None, None, index - 1, Some(upper), true, 1))
            .await?;
    page.claims
        .into_iter()
        .next()
        .filter(|claim| claim.store_index == index)
        .map(client_history_resource)
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("history `{id}` does not exist")))
}

/// [`blocking_store`] for an HTTP roster read: worker queue wait, then the attributed read.
async fn blocking_roster<T, F>(operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let submitted = crate::otel::export_enabled().then(Instant::now);
    blocking_store(move || roster_worker(true, submitted, None, operation)).await
}
