//! The part of `store::attention_list` that moved up from fractal-seats.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::PublishedViewsAttentionStore;
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
pub use fractal_seats::store::attention_list::*;
pub use fractal_seats::store::attention_list::{AttentionList, AttentionPublication, PUBLISHED_VIEWS, ViewHealth};
#[cfg(test)]
#[path = "attention_list/tests.rs"]
mod tests;

/// A view whose refresh has failed this long is withdrawn: its windows read for themselves, as
/// before, until it publishes again. Once published, a window rereads only on publication, so
/// a view that stopped publishing must not stay served.
pub const WITHDRAW_AFTER_MS: u64 = 30_000;

/// More claims than this between two publications fold again rather than read them all.
const DELTA_LIMIT: usize = 10_000;

/// Claim kinds that no attention source reads, whatever their subject: harness timelines and
/// usage, messages (attention leaves messages to conversations), transport and workspace
/// observations, daemon diagnostics, and a person's glasses and arrangements. Lease renewals
/// move no step's status, run or ask. These are most of a busy host's claims.
const UNREAD_KINDS: &[&str] = &[
    "harness.timeline",
    "harness.usage",
    "message.sent",
    "message.staged",
    "message.delivered",
    "message.read",
    "message.closed",
    "transport.observed",
    "workspace.observed",
    "daemon.diagnostic",
    "glass.upserted",
    "glass.deleted",
    "arrangement.edited",
    "work.renewed",
];

/// Runtime observations a seat writes over and over that attention reads only as the actual
/// state of a retiring requester (`rollouts::retiring_ask_live`) or as a seat's runtime epoch
/// for its login item. Every other kind on a seat (its declaration, its queue) folds.
const SEAT_RUNTIME_KINDS: &[&str] = &[
    "runtime.observed",
    "runtime.reconcile-decision",
    "runtime.restart-window-reset",
    "transport.future-observed",
];

/// Why the newest publication can no longer stand for the rows at a newer cut.
#[derive(Debug, PartialEq, Eq)]
pub enum AttentionDelta {
    /// Nothing attention reads changed: the same rows hold at the newer cut, until the
    /// earliest future acceptance time among the claims read, if any.
    Unchanged(Option<u128>),
    /// Fold again, and why.
    Refold(String),
}

/// What one delta asks about seats, each answered once: whether the previous rows hold a login
/// or prompt item for it, whether it needs a login now, and whether it is retiring.
pub struct SeatChecks<'a> {
    store: &'a Store,
    previous: &'a AttentionPublication,
    seat_items: Option<BTreeSet<String>>,
    needs_login: HashMap<String, bool>,
    retiring: HashMap<String, bool>,
}

impl<'a> SeatChecks<'a> {
    fn new(store: &'a Store, previous: &'a AttentionPublication) -> Self {
        Self {
            store,
            previous,
            seat_items: None,
            needs_login: HashMap::new(),
            retiring: HashMap::new(),
        }
    }

    /// Whether the previous rows hold a login or prompt item for `seat`: any claim about it can
    /// change that item, its episode or its removal. Such an item names its seat as its source
    /// and lists every seat it covers in its targets, so a seat sharing another's login counts.
    fn has_seat_item(&mut self, seat: &str) -> bool {
        let previous = self.previous;
        self.seat_items
            .get_or_insert_with(|| item_seats(&previous.rows))
            .contains(seat)
    }

    /// Whether `seat`'s harness needs a login at this snapshot, by the login fold attention
    /// itself uses: it answers at once for a seat with no login evidence in its runtime epoch.
    fn needs_login(&mut self, seat: &str) -> Result<bool> {
        if !seat.starts_with("agent/") {
            return Ok(false);
        }
        if let Some(needs) = self.needs_login.get(seat) {
            return Ok(*needs);
        }
        let needs = self
            .store
            .current_harness_for_login(seat)?
            .is_some_and(|harness| harness.state == "needs-login");
        self.needs_login.insert(seat.to_owned(), needs);
        Ok(needs)
    }

    /// Whether `seat`'s current declaration is a stop: only then can its asks read its runtime.
    fn retiring(&mut self, seat: &str) -> Result<bool> {
        if let Some(retiring) = self.retiring.get(seat) {
            return Ok(*retiring);
        }
        let retiring = declared_stop(&self.store.readers.get(), seat)?;
        self.retiring.insert(seat.to_owned(), retiring);
        Ok(retiring)
    }
}

/// Attention kinds made of a seat's harness and runtime claims.
const SEAT_ITEM_KINDS: &[&str] = &["harness-login", "harness-prompt"];

/// Every seat a login or prompt item in `rows` covers: its source and its targets.
fn item_seats(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| row["attention_kind"].as_str().is_some_and(|kind| SEAT_ITEM_KINDS.contains(&kind)))
        .flat_map(|row| {
            row["source_id"]
                .as_str()
                .into_iter()
                .chain(row["targets"].as_array().into_iter().flatten().filter_map(Value::as_str))
                .map(str::to_owned)
        })
        .collect()
}

/// Whether the harness observation `id` says its seat is blocked on a person.
fn blocked_on_human(connection: &Connection, id: &str) -> Result<bool> {
    Ok(connection
        .prepare_cached(
            "SELECT COALESCE(json_extract(body, '$.fields.blocked_on'), json_extract(body, '$.blocked_on'))='human'
             FROM claims WHERE id=?1",
        )?
        .query_row(/* sqlite-large-value: allow-for-now lv-h0a54149352cd owner=fractal_attention::store::attention_list TODO=trace-statement-and-value-representation */ [id], |row| row.get::<_, Option<bool>>(0))
        .optional()?
        .flatten()
        .unwrap_or(false))
}

/// Whether `subject`'s current declaration is a stop: only then can a retiring seat's ask read
/// its runtime.
fn declared_stop(connection: &Connection, subject: &str) -> Result<bool> {
    Ok(connection
        .prepare_cached("SELECT kind='stop' FROM desired WHERE subject=?1")?
        .query_row(/* sqlite-large-value: allow-for-now lv-hb49b53d4bf64 owner=fractal_attention::store::attention_list TODO=trace-statement-and-value-representation */ [subject], |row| row.get(0))
        .optional()?
        .unwrap_or(false))
}

pub trait AttentionListStore {
    /// Register the one task that keeps the attention list published, and return its wake.
    /// `None` when one is already registered.
    fn start_attention_list_refresher(&self) -> Option<Arc<tokio::sync::Notify>>;
    /// Whether a refresher was started for the attention list and its sibling views.
    fn attention_list_refresher_running(&self) -> bool;
    /// Whether windows of `view` serve its publication rather than reading for themselves: its
    /// refresher runs and has not withdrawn it.
    fn published_view_serving(&self, view: &str) -> bool;
    /// Record one refresh of `view`. A view failing for [`WITHDRAW_AFTER_MS`] is withdrawn, so
    /// its windows follow commits again; its next good refresh serves it again.
    fn note_view_refreshed(&self, view: &str, succeeded: bool);
    /// A paired client's grant completed or was revoked since the last check: every window of a
    /// served view rereads, rechecking its session, as a commit of such a claim made it before
    /// views were published.
    fn recheck_pairings(&self) -> Result<()>;
    /// The refresher task ended: windows of every view read for themselves from now on.
    fn stop_attention_list_refresher(&self);
    /// Ask the refresher, if one runs, to publish the list at the newest cut. Requests made
    /// while it folds coalesce into one more refresh.
    fn request_attention_list_refresh(&self);
    /// The newest publication, and note that a window read it. A window serves these rows
    /// under the publication's own cut, never relabelled with its own, even when the window's
    /// snapshot began before that cut was published.
    fn published_attention_list(&self) -> Option<Arc<AttentionPublication>>;
    /// The newest publication, whatever its cut, for the refresher to start from, and the
    /// generation to publish under: read it before the snapshot that folds.
    fn newest_attention_list(&self) -> (u64, Option<Arc<AttentionPublication>>);
    /// Whether nothing was ever published or forgotten: the refresher's first fold is due.
    fn attention_list_starting(&self) -> bool;
    /// Note that a window read one of the refresher's views: it follows commits only while
    /// windows read them.
    fn note_published_view_read(&self);
    /// Whether a window read one of the refresher's views within `within_ms`.
    fn attention_list_read_within(&self, within_ms: u64) -> bool;
    /// Swap in a newer publication. A publication never goes back to an older cut. Only
    /// `changed` rows raise the revision: the same rows at a newer cut give streams nothing to
    /// reread, and a window that reads later serves them under the newer cut.
    /// A refresh that began before views were forgotten (`generation` is stale) publishes
    /// nothing: what it read may no longer hold.
    fn publish_attention_list(
            &self,
            generation: u64,
            publication: AttentionPublication,
            changed: bool,
        );
    /// Count one fold of the whole list, by why.
    fn note_attention_list_fold(&self, cause: &str);
    /// How many times the attention list was folded, by why.
    fn attention_list_folds(&self) -> BTreeMap<String, u64>;
    /// When the newest list must be evaluated again with no claim: its clock period after it
    /// was evaluated, or sooner when a claim it read becomes eligible. `None` before the first.
    fn attention_list_due_at(&self, period_ms: u128) -> Option<u128>;
    /// The earliest acceptance time after `now` among the claims in `(after, through]`: a
    /// fold at `now` left them out as not yet eligible. More claims than a delta reads are
    /// left to the clock period.
    fn attention_future_since(&self, after: u64, through: u64, now: u128) -> Result<Option<u128>>;
    /// The graph projection's health row, read in the snapshot that folds or compares.
    fn attention_projection_frontier(&self) -> Result<Option<(String, u64)>>;
    /// Whether the rows of `previous` still hold at `index`, evaluated at `now`, read in the
    /// snapshot at `index`. Only the subjects, kinds, actors and acceptance times of the claims
    /// admitted (or projected) since are read, and for a seat's runtime claims whether the seat
    /// has or now needs a login item, or is retiring.
    fn attention_list_delta(
            &self,
            previous: &AttentionPublication,
            index: u64,
            now: u128,
        ) -> Result<AttentionDelta>;
    /// The first reason a claim in `(after, through]` can change the attention list, if any,
    /// noting in `due` the earliest acceptance time after `now` among them.
    fn attention_claims_matter(
            &self,
            seats: &mut SeatChecks<'_>,
            after: u64,
            through: u64,
            now: u128,
            due: &mut Option<u128>,
        ) -> Result<Option<String>>;
}

impl AttentionListStore for Store {

    fn start_attention_list_refresher(&self) -> Option<Arc<tokio::sync::Notify>> {
        let wake = Arc::new(tokio::sync::Notify::new());
        self.fractal
            .attention_list
            .refresher
            .set(Arc::clone(&wake))
            .ok()?;
        Some(wake)
    }

    fn attention_list_refresher_running(&self) -> bool {
        let list = &self.fractal.attention_list;
        list.refresher.get().is_some() && !list.stopped.load(AtomicOrdering::Acquire)
    }

    fn published_view_serving(&self, view: &str) -> bool {
        let Some(position) = PUBLISHED_VIEWS.iter().position(|name| *name == view) else {
            return false;
        };
        self.attention_list_refresher_running()
            && !self.fractal.attention_list.health[position]
                .withdrawn
                .load(AtomicOrdering::Acquire)
    }

    fn note_view_refreshed(&self, view: &str, succeeded: bool) {
        let Some(position) = PUBLISHED_VIEWS.iter().position(|name| *name == view) else {
            return;
        };
        let health = &self.fractal.attention_list.health[position];
        let now = now_ms() as u64;
        if succeeded {
            health.failing_since.store(0, AtomicOrdering::Release);
            if health.withdrawn.swap(false, AtomicOrdering::AcqRel) {
                eprintln!("st3: the {view} view publishes again; its windows serve it");
                self.publish_collection_view(view);
            }
            return;
        }
        let since = match health.failing_since.compare_exchange(
            0, now, AtomicOrdering::AcqRel, AtomicOrdering::Acquire,
        ) {
            Ok(_) => now,
            Err(since) => since,
        };
        if now.saturating_sub(since) >= WITHDRAW_AFTER_MS
            && !health.withdrawn.swap(true, AtomicOrdering::AcqRel)
        {
            eprintln!("st3: WARN the {view} view has failed to refresh for {} s; its windows read for themselves until it publishes again", now.saturating_sub(since) / 1000);
            self.withdraw_collection_view(view);
        }
    }

    fn recheck_pairings(&self) -> Result<()> {
        let list = &self.fractal.attention_list;
        let checked = list.pairings_checked.load(AtomicOrdering::Acquire);
        let index = self.index()?;
        if checked == 0 || index <= checked {
            list.pairings_checked.store(index.max(checked), AtomicOrdering::Release);
            return Ok(());
        }
        let changed: bool = self.readers.get().prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM claims INDEXED BY claims_kind_index
             WHERE kind IN ('custom.client.pairing-completed','custom.client.pairing-revoked')
               AND store_index>?1 AND store_index<=?2)",
        )?.query_row(/* sqlite-large-value: allow-for-now lv-hbd833478720f owner=fractal_attention::store::attention_list TODO=trace-statement-and-value-representation */ params![checked, index], |row| row.get(0))?;
        list.pairings_checked.store(index, AtomicOrdering::Release);
        if changed {
            for view in PUBLISHED_VIEWS.into_iter().filter(|view| self.published_view_serving(view)) {
                self.publish_collection_view(view);
            }
        }
        Ok(())
    }

    fn stop_attention_list_refresher(&self) {
        let list = &self.fractal.attention_list;
        if list.refresher.get().is_none() || list.stopped.swap(true, AtomicOrdering::AcqRel) {
            return;
        }
        eprintln!("st3: WARN the attention, glasses, arrangements and summary refresher stopped; their windows read for themselves");
        for view in PUBLISHED_VIEWS {
            self.withdraw_collection_view(view);
        }
    }

    fn request_attention_list_refresh(&self) {
        if let Some(wake) = self.fractal.attention_list.refresher.get() {
            wake.notify_one();
        }
    }

    fn published_attention_list(&self) -> Option<Arc<AttentionPublication>> {
        let list = &self.fractal.attention_list;
        list.refresher.get()?;
        self.note_published_view_read();
        list.published
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn newest_attention_list(&self) -> (u64, Option<Arc<AttentionPublication>>) {
        let list = &self.fractal.attention_list;
        let published = list.published.lock().unwrap_or_else(PoisonError::into_inner);
        (list.forgotten.load(AtomicOrdering::Acquire), published.clone())
    }

    fn attention_list_starting(&self) -> bool {
        matches!(self.newest_attention_list(), (0, None))
    }

    fn note_published_view_read(&self) {
        self.fractal
            .attention_list
            .read_at
            .store(now_ms() as u64, AtomicOrdering::Release);
    }

    fn attention_list_read_within(&self, within_ms: u64) -> bool {
        let read_at = self.fractal.attention_list.read_at.load(AtomicOrdering::Acquire);
        read_at != 0 && (now_ms() as u64).saturating_sub(read_at) <= within_ms
    }

    fn publish_attention_list(
            &self,
            generation: u64,
            publication: AttentionPublication,
            changed: bool,
        ) {
        let list = &self.fractal.attention_list;
        {
            let mut published = list.published.lock().unwrap_or_else(PoisonError::into_inner);
            if list.forgotten.load(AtomicOrdering::Acquire) != generation {
                return;
            }
            if published
                .as_ref()
                .is_some_and(|newest| newest.cut > publication.cut)
            {
                return;
            }
            *published = Some(Arc::new(publication));
        }
        if changed {
            list.revision.send_modify(|revision| *revision += 1);
            self.publish_collection_view("attention");
        }
    }

    fn note_attention_list_fold(&self, cause: &str) {
        *self.fractal
            .attention_list
            .folds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(cause.to_owned())
            .or_default() += 1;
    }

    fn attention_list_folds(&self) -> BTreeMap<String, u64> {
        self.fractal
            .attention_list
            .folds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn attention_list_due_at(&self, period_ms: u128) -> Option<u128> {
        let published = self.newest_attention_list().1?;
        let clock = published.evaluated_at_unix_ms.saturating_add(period_ms);
        Some(published.due_at_unix_ms.map_or(clock, |due| due.min(clock)))
    }

    fn attention_future_since(&self, after: u64, through: u64, now: u128) -> Result<Option<u128>> {
        if through.saturating_sub(after) > DELTA_LIMIT as u64 {
            return Ok(None);
        }
        let earliest: Option<i64> = self.readers.get().prepare_cached(
            "SELECT MIN(CAST(accepted_at_unix_ms AS INTEGER)) FROM claims
             WHERE store_index>?1 AND store_index<=?2 AND CAST(accepted_at_unix_ms AS INTEGER)>?3",
        )?.query_row(/* sqlite-large-value: allow-for-now lv-h88ea458f2e12 owner=fractal_attention::store::attention_list TODO=trace-statement-and-value-representation */ params![after, through, i64::try_from(now).unwrap_or(i64::MAX)], |row| row.get(0))?;
        Ok(earliest.and_then(|at| u128::try_from(at).ok()))
    }

    fn attention_projection_frontier(&self) -> Result<Option<(String, u64)>> {
        Ok(self
            .readers
            .get()
            .query_row(
                "SELECT status, last_good_store_index FROM projection_health WHERE aggregate='graph'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<u64>>(1)?.unwrap_or(0))),
            )
            .optional()?)
    }

    fn attention_list_delta(
            &self,
            previous: &AttentionPublication,
            index: u64,
            now: u128,
        ) -> Result<AttentionDelta> {
        if index < previous.cut {
            return Ok(AttentionDelta::Refold("older cut".into()));
        }
        let mut seats = SeatChecks::new(self, previous);
        let projection = self.attention_projection_frontier()?;
        let mut due = None;
        if projection != previous.projection {
            // Deferred replication projection caught up under claims already admitted: the
            // claims it projected decide, as if they had just arrived.
            match (&previous.projection, &projection) {
                (Some((before, after)), Some((status, through)))
                    if before == status && after <= through =>
                {
                    if let Some(cause) = self.attention_claims_matter(&mut seats, *after, *through, now, &mut due)? {
                        return Ok(AttentionDelta::Refold(format!("projection {cause}")));
                    }
                }
                _ => return Ok(AttentionDelta::Refold("projection".into())),
            }
        }
        if index > previous.cut
            && let Some(cause) =
                self.attention_claims_matter(&mut seats, previous.cut, index, now, &mut due)?
        {
            return Ok(AttentionDelta::Refold(cause));
        }
        Ok(AttentionDelta::Unchanged(due))
    }

    fn attention_claims_matter(
            &self,
            seats: &mut SeatChecks<'_>,
            after: u64,
            through: u64,
            now: u128,
            due: &mut Option<u128>,
        ) -> Result<Option<String>> {
        let claims = {
            let connection = self.readers.get();
            let mut statement = connection.prepare_cached(
                "SELECT id, subject, kind, actor, CAST(accepted_at_unix_ms AS TEXT) FROM claims
                 WHERE store_index>?1 AND store_index<=?2 ORDER BY store_index LIMIT ?3",
            )?;
            statement
                .query_map(params![after, through, DELTA_LIMIT as u64 + 1], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if claims.len() > DELTA_LIMIT {
            return Ok(Some("many".into()));
        }
        for (id, subject, kind, actor, accepted) in claims {
            let accepted = accepted.parse::<u128>().unwrap_or(0);
            if accepted > now {
                *due = Some(due.map_or(accepted, |due: u128| due.min(accepted)));
            }
            if UNREAD_KINDS.contains(&kind.as_str()) {
                continue;
            }
            let matters = if kind == "work.progress" {
                // Work by a seat counts as its activity in the login fold: it can only clear a
                // login item the seat already has.
                match actor.as_deref().filter(|actor| actor.starts_with("agent/")) {
                    Some(seat) => seats.has_seat_item(seat),
                    None => false,
                }
            } else if kind.starts_with("harness.") {
                seats.has_seat_item(&subject)
                    || seats.needs_login(&subject)?
                    // An observation blocked on a person is a harness prompt for its seat.
                    || kind == "harness.observed" && blocked_on_human(&self.readers.get(), &id)?
            } else if SEAT_RUNTIME_KINDS.contains(&kind.as_str()) {
                seats.has_seat_item(&subject)
                    || seats.needs_login(&subject)?
                    || seats.retiring(&subject)?
            } else {
                true
            };
            if matters {
                return Ok(Some(kind));
            }
        }
        Ok(None)
    }
}

