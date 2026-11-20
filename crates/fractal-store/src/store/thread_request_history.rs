//! Read-only related requests. An arrow explains history, never a dependency or an answer.
//!
//! Runtime person asks have retained payload readers. Authored epochs without a durable
//! opening recipient are not authorized, even when the current assignment matches.
//! Other qualified source kinds are explicitly marked `unsupported-source`: discussion
//! authority does not imply a history payload reader. Unsupported retained families keep
//! their existing live-only policy; thread membership alone never grants source access.
use super::*;
use serde::{Deserialize, Serialize};

const PAGE_LIMIT: usize = 50;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceEpisode {
    pub source_id: String,
    pub episode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "kebab-case")]
pub enum HistoricalPayload {
    Available {
        title: Option<String>,
        text: String,
        request: Option<Value>,
        references: Vec<HistoricalReference>,
        provenance: RequestProvenance,
    },
    Unavailable { reason: PayloadUnavailable },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PayloadUnavailable {
    RetainedDefinitionUnavailable,
    RetainedStepUnavailable,
    OpeningUnavailable,
    HistoricalInterpolationUnavailable,
    BoundaryUnavailable,
    PayloadUnavailable,
    ReadBoundExceeded,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RequestProvenance {
    RuntimeAsk { asking_claim_id: String },
    AuthoredEpisode { mission_revision: String, opening_claim_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReference {
    pub target_id: String,
    pub revision: Option<String>,
    pub semantics: ReferenceSemantics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReferenceSemantics { RetainedReference, LiveReference }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "kebab-case")]
pub enum EpisodeState {
    Available { status: String, answers: Vec<Value> },
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryItem {
    pub key: SourceEpisode,
    pub payload: HistoricalPayload,
    pub state: EpisodeState,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryRelation {
    pub predecessor: SourceEpisode,
    pub request: SourceEpisode,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceEpisodeGroup {
    pub source_id: String,
    pub episodes: Vec<SourceEpisode>,
}

/// Source support is disclosed only after current exact-source authorization. Unsupported
/// origins do not produce invented person-step rows; clients may render “no request history
/// for this source”. This marker grants no read or mutation authority for that source.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistorySourceSupport { PersonStep, UnsupportedSource }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHistoryPage {
    pub items: Vec<RequestHistoryItem>,
    pub relations: Vec<RequestHistoryRelation>,
    pub source_episodes: Vec<SourceEpisodeGroup>,
    pub boundary_items: Vec<RequestHistoryItem>,
    pub source_support: Option<HistorySourceSupport>,
    /// Scoped keyset over exact visible source/episode keys, not a store-index snapshot.
    /// Every page reauthorizes its anchor and rows and reads status fresh. Items introduced
    /// before the anchor after page one may be missed until refresh, as with `list_context`.
    pub next_cursor: Option<String>,
    /// Completeness of the authorized selection and its retained evidence. Hidden endpoints
    /// never contribute a count, cursor anchor, or an incompleteness indication.
    pub complete: bool,
}


/// Compose one authorized bounded source reader with one bounded same-source episode reader.
/// The missions crate supplies authored episode reconstruction without reversing crate edges.
/// Callers authenticate/revalidate their current grant/session before this store projection.
/// `read_source` must return only the requested exact key and current person's permitted row.
/// `episode_keys` must authorize its seed before enumeration and bound retained lifecycle work.
#[allow(clippy::too_many_arguments)]
pub fn read_with(
    store: &Store,
    person: &str,
    thread: &str,
    cursor: Option<&str>,
    limit: usize,
    mut read_source: impl FnMut(&Connection, &str, &SourceEpisode, u64)
        -> Result<Option<RequestHistoryItem>, FractalError>,
    mut episode_keys: impl FnMut(&Connection, &str, &SourceEpisode, u64)
        -> Result<(Vec<SourceEpisode>, bool), FractalError>,
) -> Result<RequestHistoryPage, FractalError> {
    store.graph.read_snapshot(|cut| {
        let connection = store.readers.get();
        Ok(read_conn(store, &connection, person, thread, cursor, limit, cut,
            &mut read_source, &mut episode_keys))
    }).map_err(internal)?
}

#[allow(clippy::too_many_arguments)]
fn read_conn(
    store: &Store,
    conn: &Connection,
    person: &str,
    thread: &str,
    cursor: Option<&str>,
    limit: usize,
    cut: u64,
    read_source: &mut impl FnMut(&Connection, &str, &SourceEpisode, u64)
        -> Result<Option<RequestHistoryItem>, FractalError>,
    episode_keys: &mut impl FnMut(&Connection, &str, &SourceEpisode, u64)
        -> Result<(Vec<SourceEpisode>, bool), FractalError>,
) -> Result<RequestHistoryPage, FractalError> {
    let created = owned_creation(conn, thread, person, false)?.ok_or_else(unavailable)?;
    let person = identities::resolve_reference_conn(conn, person)?;
    let mut page = RequestHistoryPage {
        items: Vec::new(), relations: Vec::new(), source_episodes: Vec::new(),
        boundary_items: Vec::new(), source_support: None, next_cursor: None, complete: true,
    };
    let seed = origin_pair(&created).map(|(source_id, episode)| SourceEpisode {
        source_id: source_id.into(), episode: episode.into(),
    });
    if let Some(seed) = &seed {
        let runtime: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE subject=?1 AND kind='work.person-asked')",
            [&seed.source_id], |row| row.get(0),
        ).map_err(internal)?;
        if seed.source_id.starts_with("step-run/")
            && (runtime || seed.episode.starts_with("run-generation/"))
        {
            if !discussion_source::qualify_history(conn, &person, seed, now_ms())?.is_authorized() {
                return Err(unavailable());
            }
            page.source_support = Some(HistorySourceSupport::PersonStep);
        } else {
            // Retained discussion uses the same exact-opening authority as the writer.
            // Families with no retained proof retain their existing live-only admission.
            if !discussion_source::qualify_history(conn, &person, seed, now_ms())?.is_authorized() {
                inbox_source::resolve(store, conn, &person, &InboxOrigin {
                    source_id: seed.source_id.clone(), episode: seed.episode.clone(),
                }, now_ms())?;
            }
            page.source_support = Some(HistorySourceSupport::UnsupportedSource);
        }
    }
    let limit = limit.clamp(1, PAGE_LIMIT);
    let scope = canonical_hash(&("thread.request-history.v1", &person, thread)).map_err(internal)?;
    let after = scoped_cursor(cursor, &scope)?.map(|marker| {
        if marker.len() > 2048 { return Err(invalid_context_cursor()); }
        let key: SourceEpisode = serde_json::from_str(marker)
            .map_err(|_| invalid_context_cursor())?;
        if !discussion_source::qualify_history(conn, &person, &key, now_ms())?.is_authorized() {
            return Err(invalid_context_cursor());
        }
        Ok(key)
    }).transpose()?;

    let after = after.as_ref();
    let mut anchor_is_member = match after {
        Some(key) => is_member(conn, &created, thread, key, cut)?,
        None => true,
    };
    // The single origin is merged outside SQL: a UNION would materialize all links before
    // applying the keyset bound. Authored episode enumeration below is separately bounded.
    let mut members = BTreeSet::new();
    if let Some(seed) = &seed
        && matches!(page.source_support, Some(HistorySourceSupport::PersonStep))
        && after.is_none_or(|after| seed > after)
    {
        members.insert(seed.clone());
    }

    // Only a permitted seed can disclose the same source's other exact episodes. The source
    // reader returns a bounded window, not a second event store or cross-thread traversal.
    let mut groups_complete = true;
    if let Some(seed) = &seed
        && page.source_support.as_ref().is_some_and(|kind| matches!(kind, HistorySourceSupport::PersonStep))
    {
        let (keys, complete) = episode_keys(conn, &person, seed, cut)?;
        groups_complete = complete;
        for key in keys {
            if after == Some(&key) && key.source_id == seed.source_id {
                anchor_is_member = true;
            }
            if key.source_id == seed.source_id
                && after.is_none_or(|after| &key > after)
                && discussion_source::qualify_history(conn, &person, &key, now_ms())?.is_authorized()
            {
                members.insert(key);
            }
        }
    }
    if !anchor_is_member { return Err(invalid_context_cursor()); }
    while members.len() > limit + 1 { members.pop_last(); }
    // Seek directly after the visible anchor. Canonical link membership and exact-source
    // authorization are checked BEFORE consuming visible lookahead, using the same
    // is_member/latest_applied_claim selection as cursor and boundary validation. Replicated
    // losing links and denied sources cannot change pagination metadata.
    //
    // Work is proportional to index rows between page anchors (including denied/losing
    // rows), not to all earlier thread links. No UNION, DISTINCT, or sort materializes the
    // entire thread. The ordered iterator stops once limit+1 visible keys are established.
    let mut statement = conn.prepare_cached(
        "SELECT json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode')
         FROM claims INDEXED BY claims_thread_history_link_index
         WHERE kind='thread.ask-linked' AND json_extract(body, '$.fields.thread_id')=?1
           AND (json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode'))>(?2,?3)
         ORDER BY json_extract(body, '$.fields.ask'), json_extract(body, '$.fields.ask_episode')"
    ).map_err(internal)?;
    let mut rows = statement.query(params![thread,
        after.map_or("", |key| key.source_id.as_str()),
        after.map_or("", |key| key.episode.as_str())]).map_err(internal)?;
    while let Some(row) = rows.next().map_err(internal)? {
        let key = SourceEpisode { source_id: row.get(0).map_err(internal)?,
            episode: row.get(1).map_err(internal)? };
        if members.len() > limit && members.last().is_some_and(|last| &key >= last) {
            break;
        }
        if members.contains(&key)
            || !discussion_source::qualify_history(conn, &person, &key, now_ms())?.is_authorized()
            || !is_member(conn, &created, thread, &key, cut)?
        {
            continue;
        }
        members.insert(key);
        if members.len() > limit + 1 { members.pop_last(); }
    }
    // Revalidate each selected exact key before any content, including lookahead. The reader
    // must not return a different episode as a substitute.
    for key in members {
        if !discussion_source::qualify_history(conn, &person, &key, now_ms())?.is_authorized() { continue; }
        if let Some(item) = read_source(conn, &person, &key, cut)?
            && item.key == key
        {
            page.items.push(item);
            if page.items.len() > limit { break; }
        }
    }
    let has_more = page.items.len() > limit;
    page.items.truncate(limit);
    if has_more && let Some(last) = page.items.last() {
        page.next_cursor = Some(format!("{scope}:{}",
            serde_json::to_string(&last.key).map_err(internal)?));
    }

    let primary = page.items.iter().map(|item| item.key.clone()).collect::<BTreeSet<_>>();
    for child in &primary {
        let subject = thread_links::ask_link_subject(&child.source_id, &child.episode)?;
        let selected = match latest_applied_claim(conn, &subject, "thread.ask-linked") {
            Ok(selected) => selected,
            Err(error) if error.code == "idempotency-conflict" => continue,
            Err(error) => return Err(error),
        };
        let Some(link) = selected.filter(|link| link.store_index <= cut) else { continue; };
        let fields = &link.body["fields"];
        if fields["thread_id"] != thread || fields["ask"] != child.source_id
            || fields["ask_episode"] != child.episode { continue; }
        let (Some(source), Some(episode), Some(reason)) = (
            fields["decision_predecessor"].as_str(), fields["decision_predecessor_episode"].as_str(),
            fields["decision_reason"].as_str().filter(|reason| !reason.is_empty() && reason.len() <= 1024),
        ) else { continue; };
        let parent = SourceEpisode { source_id: source.into(), episode: episode.into() };
        if !is_member(conn, &created, thread, &parent, cut)?
            || !discussion_source::qualify_history(conn, &person, &parent, now_ms())?.is_authorized() { continue; }
        if !primary.contains(&parent) && !page.boundary_items.iter().any(|item| item.key == parent) {
            let Some(row) = read_source(conn, &person, &parent, cut)?.filter(|row| row.key == parent) else { continue; };
            page.boundary_items.push(row);
        }
        page.relations.push(RequestHistoryRelation {
            predecessor: parent, request: child.clone(), reason: reason.into(),
        });
    }
    let mut groups = BTreeMap::<String, BTreeSet<SourceEpisode>>::new();
    for item in page.items.iter().chain(&page.boundary_items) {
        groups.entry(item.key.source_id.clone()).or_default().insert(item.key.clone());
    }
    page.source_episodes = groups.into_iter().filter(|(_, episodes)| episodes.len() > 1)
        .map(|(source_id, episodes)| SourceEpisodeGroup { source_id, episodes: episodes.into_iter().collect() })
        .collect();
    page.complete = groups_complete && page.items.iter().chain(&page.boundary_items).all(|item| item.complete);
    if serde_json::to_vec(&page).map_err(internal)?.len() > MAX_RESPONSE_BYTES {
        return Err(FractalError::new("thread-history-budget-exceeded", "request history exceeds its response bound"));
    }
    Ok(page)
}

fn is_member(conn: &Connection, created: &ClaimRecord, thread: &str, key: &SourceEpisode, cut: u64)
    -> Result<bool, FractalError>
{
    if origin_pair(created) == Some((key.source_id.as_str(), key.episode.as_str())) { return Ok(true); }
    let subject = thread_links::ask_link_subject(&key.source_id, &key.episode)?;
    match latest_applied_claim(conn, &subject, "thread.ask-linked") {
        Ok(link) => Ok(link.is_some_and(|link| link.store_index <= cut
            && link.body["fields"]["thread_id"] == thread
            && link.body["fields"]["ask"] == key.source_id
            && link.body["fields"]["ask_episode"] == key.episode)),
        Err(error) if error.code == "idempotency-conflict" => Ok(false),
        Err(error) => Err(error),
    }
}
