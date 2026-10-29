//! Current collection cursors. The source adapter owns complete keyed dependency coverage,
//! authorized bounded ranking and row reads; this consumer owns delivery acknowledgment.
//! Registration alone does not select a production adapter.
use crate::EvidenceStore;
use super::*;
use crate::graph_watch_ivm::{IvmViewBridge, ViewPage};
use smallclaims::ivm::{Readiness, events};
use std::cell::RefCell;

pub type Window = (ClientSnapshot, Vec<Value>, bool);
pub type RowReader = dyn Fn(
        &AppState,
        &ClientSession,
        &CollectionSubscribe,
        &rusqlite::Connection,
        &events::Boundary,
        &[events::ViewChanged],
        &BTreeMap<String, Value>,
    ) -> Result<(Vec<Value>, bool), ApiError>
    + Send
    + Sync;

pub struct Adapter {
    pub view: &'static str,
    // Keys are hints, never proof that other retained rows are current. The adapter
    // selects bounded authorized IDs first and verifies every reused row's generation
    // at this boundary, then fetches only changed/new rows. Full window folding is forbidden.
    pub rows: Arc<RowReader>,
}

pub struct Sources {
    store: Arc<Store>,
    bridge: Arc<IvmViewBridge<Store>>,
    adapters: BTreeMap<String, Arc<Adapter>>,
}

impl Sources {
    /// Called on a blocking worker after the explicit Store installer. Receipt waits and
    /// sockets obtain the same runtime-held Arc; this path never attaches a publisher.
    pub fn from_store(
        store: Arc<Store>,
        adapters: BTreeMap<String, Arc<Adapter>>,
    ) -> anyhow::Result<Option<Arc<Self>>> {
        if adapters.is_empty() {
            return Ok(None);
        }
        let Some(views) = store.ivm_views() else {
            return Ok(None);
        };
        let publisher = store
            .ivm_publisher()?
            .ok_or_else(|| anyhow::anyhow!("registered collection source has no publisher"))?;
        Ok(Some(Arc::new(Self {
            store: store.clone(),
            bridge: Arc::new(IvmViewBridge::from_shared(store, views, publisher)),
            adapters,
        })))
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<events::Notice> {
        self.bridge.subscribe()
    }

    pub fn adapter(&self, collection: &str) -> Option<Arc<Adapter>> {
        self.adapters.get(collection).cloned()
    }
}

#[derive(Clone)]
pub struct Delivered {
    boundary: events::Boundary,
    authority: String,
    continuation: Option<events::SnapshotVersion>,
}

#[derive(Default)]
pub struct Held {
    pub cursor: Option<Delivered>,
    pub rows: Arc<BTreeMap<String, Value>>,
}

pub enum Output {
    Window(Window),
    /// Keep the last delivered rows stale until Ready or a client resubscription.
    Unavailable,
    /// A delivered resync must be followed by a new authoritative snapshot.
    Resync,
    /// No visible change, or a continuation whose page version changed.
    Silent,
}

pub struct Candidate {
    pub output: Output,
    pub delivered: Option<Delivered>,
    pub again: bool,
    pub replace: bool,
}

fn authority(session: &ClientSession) -> String {
    json!({"actor":session.actor,"authority":session.authority_actor,
        "grant":session.pairing_grant,"scopes":session.scopes,
        "custom_forms":session.custom_forms,"conversation_blocks":session.conversation_blocks,"transport":session.transport})
    .to_string()
}

fn authorize(
    state: &AppState,
    session: &ClientSession,
    request: &CollectionSubscribe,
) -> Result<ClientSession, ApiError> {
    let current = revalidate_session(state, session)?;
    require_scope(&current, "read.projections")?;
    let limit = request.limit.unwrap_or(CLIENT_DEFAULT_PAGE_ITEMS);
    if !(1..=CLIENT_MAX_PAGE_ITEMS).contains(&limit) {
        return Err(validation("collection limit must be 1 through 200"));
    }
    if request.collection == "attention" {
        person_filter(&current, request.person.as_deref())?;
    }
    if request.status.is_some() && request.collection != "agents" {
        return Err(validation("status filters are supported for agents only"));
    }
    validate_agent_selector(&request.collection, request.agent.as_deref())?;
    if request.subject.is_some() {
        return Err(validation(
            "subject filters are supported for arrangements only",
        ));
    }
    Ok(current)
}

/// Read a candidate without changing any delivered cursor. The socket applies Candidate
/// only after sending its snapshot/changes/resync successfully (or proving silence).
pub async fn read(
    state: AppState,
    session: ClientSession,
    request: CollectionSubscribe,
    permit: tokio::sync::OwnedSemaphorePermit,
    sources: Arc<Sources>,
    adapter: Arc<Adapter>,
    held: Held,
) -> Result<Candidate, ApiError> {
    if !Arc::ptr_eq(&state.store, &sources.store) {
        return Err(ApiError::internal(anyhow::anyhow!(
            "collection source belongs to another Store"
        )));
    }
    blocking_store(move || {
        let Held { cursor: previous, rows: previous_rows } = held;
        let _permit = permit;
        let current = RefCell::new(None);
        let refusal = RefCell::new(None);
        let check = |connection: &rusqlite::Connection| match authorize(&state, &session, &request) {
            Ok(session) => {
                *current.borrow_mut() = Some(session);
                // Check even silent key pages; a missed durable source write cannot be
                // acknowledged merely because the registry still reports Ready.
                let index = smallclaims::store::current_index(connection)?;
                let cut = smallclaims::ivm::source_cut(connection)?;
                anyhow::ensure!(cut.is_some_and(|cut|cut.admitted == index),
                    "collection source admission coverage is pending");
                Ok(())
            }
            Err(error) => {
                *refusal.borrow_mut() = Some(error);
                anyhow::bail!("collection authorization refused")
            }
        };
        let rows = |connection: &rusqlite::Connection,
                    boundary: &events::Boundary,
                    keys: &[events::ViewChanged]| {
            let held = current.borrow();
            let session = held.as_ref().expect("authorization precedes row reads");
            // Registration cannot certify writes omitted from the source dispatch.
            let index = smallclaims::store::current_index(connection)?;
            anyhow::ensure!(boundary.source_cut.projected == index
                && boundary.source_cut.admitted == index, "collection source prefix is pending");
            let empty = BTreeMap::new();
            let reusable = previous.as_ref().is_some_and(|previous|
                previous.authority == authority(session)) && keys.len() < 256;
            let retained = if reusable { previous_rows.as_ref() } else { &empty };
            match (adapter.rows)(&state, session, &request, connection, boundary, keys, retained) {
                Ok((items, has_more)) => Ok((
                    client_snapshot_at(&state, boundary.source_cut.projected),
                    items,
                    has_more,
                )),
                Err(error) => {
                    *refusal.borrow_mut() = Some(error);
                    anyhow::bail!("collection row read refused")
                }
            }
        };
        let result = if let Some(previous) = &previous {
            sources
                .bridge
                .page_with_rows(
                    &previous.boundary.keys,
                    &previous.boundary.status,
                    256,
                    previous.continuation.as_ref(),
                    check,
                    |connection, boundary, keys, _| {
                        let auth =
                            authority(current.borrow().as_ref().expect("authorized session"));
                        // Shared source-cut notices on unrelated writes advance cursors without
                        // reading rows. A Ready transition or authority change still refreshes.
                        let changed = !keys.is_empty()
                            || !matches!(
                                previous.boundary.availability.readiness,
                                Readiness::Ready(_)
                            )
                            || previous.authority != auth;
                        changed
                            .then(|| rows(connection, boundary, keys))
                            .transpose()
                    },
                )
                .map(|page| match page {
                    ViewPage::Changes {
                        output,
                        next_keys,
                        next_status,
                        more,
                        mut boundary,
                        has_availability_change,
                    } => {
                        let ready = matches!(boundary.availability.readiness, Readiness::Ready(_));
                        let continuation = more.then(|| boundary.snapshot.clone());
                        boundary.keys = next_keys;
                        boundary.status = next_status;
                        Candidate {
                            output: if !ready && (has_availability_change
                                || matches!(previous.boundary.availability.readiness, Readiness::Ready(_))) {
                                Output::Unavailable
                            } else if !ready {
                                Output::Silent
                            } else if let Some(window) = output.flatten() {
                                Output::Window(window)
                            } else {
                                Output::Silent
                            },
                            delivered: Some(Delivered {
                                boundary,
                                authority: authority(
                                    current.borrow().as_ref().expect("authorized session"),
                                ),
                                continuation,
                            }),
                            again: more,
                            replace: false,
                        }
                    }
                    ViewPage::Resync { reason, boundary } => {
                        tracing::debug!(?reason, identity=?boundary.identity, "collection journal gap");
                        Candidate {
                        output: Output::Resync,
                        delivered: None,
                        again: true,
                        replace: true,
                    }},
                    ViewPage::SnapshotChanged { boundary } => {
                        tracing::debug!(snapshot=?boundary.snapshot, "collection continuation changed");
                        let mut delivered = previous.clone();
                        delivered.continuation = None;
                        Candidate {
                            output: Output::Silent,
                            delivered: Some(delivered),
                            again: true,
                            replace: false,
                        }
                    }
                })
        } else {
            sources
                .bridge
                .subscribe_snapshot(adapter.view, check, |connection, boundary| {
                    rows(connection, boundary, &[])
                })
                .map(|open| {
                    // The socket owns an earlier receiver; this per-capture subscription is
                    // redundant for it but preserves the standalone bridge ordering contract.
                    drop(open.notices);
                    Candidate {
                    output: open.snapshot.map_or(Output::Unavailable, Output::Window),
                    delivered: Some(Delivered {
                        boundary: open.boundary,
                        authority: authority(
                            current.borrow().as_ref().expect("authorized session"),
                        ),
                        continuation: None,
                    }),
                    again: false,
                    replace: true,
                }})
        };
        Ok(result.map_err(|error| {
            refusal
                .into_inner()
                .unwrap_or_else(|| ApiError::internal(error))
        }))
    })
    .await?
}

