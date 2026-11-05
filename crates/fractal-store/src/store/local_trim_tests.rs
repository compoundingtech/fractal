//! Local trim through Fractal's store: a member with `[trim] local` trims its own due cut with
//! the audited retention rules, and every reader answers the same from the full log and the
//! trimmed one.
use super::*;
use crate::store::checkpoint_agreement_tests_shared_attention_tests::{context, kinds, step};
use crate::store::tests_shared_seats::{TEST_FLEET, graph_digest_of};
use proptest::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

const SEATS: &[&str] = &["agent/node.amber", "agent/node.birch"];
const STATES: &[&str] = &["starting", "ready", "idle", "working", "blocked", "ended"];

/// A member with both seats declared, so the seat readers have something to answer.
fn member(trims_locally: bool) -> Store {
    declared(Store::open_memory("node").unwrap(), trims_locally)
}

/// `store` with both seats declared.
fn declared(store: Store, trims_locally: bool) -> Store {
    store.bind_fleet(TEST_FLEET).unwrap();
    let source = "version 2\nagent \"amber\" { command \"true\" }\nagent \"birch\" { command \"true\" }\n";
    let intent = crate::graph::parse_test_intent(source, "node").unwrap();
    let plan = store
        .mission(&intent, crate::model::IntentInput { kdl: source.into(), source_name: None })
        .unwrap();
    store.apply(&intent, &plan.subject_tokens, "local-trim").unwrap();
    store.import_retired_identities_paced(|_| {}).unwrap();
    store.trim_locally(trims_locally).unwrap();
    store
}

/// What `[trim] notify` sets at startup.
fn set_notify(store: &Store, agent: &str) {
    *store.extension::<crate::store::answers_check::TrimSettings>().notify.lock().unwrap() = Some(agent.into());
}

fn local_member(_name: &str) -> Store {
    member(true)
}

/// One write of a random history.
#[derive(Clone, Debug)]
enum Write {
    Observer { observer: u8, status: u8, cursor: Option<u8> },
    /// A pull request's observation, in one of the shapes of `resource_fields`.
    Resource { resource: u8, shape: u8 },
    /// One poll, as an observer seals it: its observation and a resource it saw, in one batch.
    Poll { observer: u8, status: u8, resource: u8, shape: u8 },
    Harness { seat: u8, incarnation: u8, state: u8, restamp: bool, provider_auth: Option<bool> },
}

fn write() -> impl Strategy<Value = Write> {
    prop_oneof![
        (0..3u8, 0..3u8, proptest::option::of(0..4u8))
            .prop_map(|(observer, status, cursor)| Write::Observer { observer, status, cursor }),
        (0..3u8, shape()).prop_map(|(resource, shape)| Write::Resource { resource, shape }),
        (0..3u8, 0..3u8, 0..3u8, shape())
            .prop_map(|(observer, status, resource, shape)| Write::Poll { observer, status, resource, shape }),
        (0..2u8, 0..2u8, 0..6u8, any::<bool>(), proptest::option::of(any::<bool>())).prop_map(
            |(seat, incarnation, state, restamp, provider_auth)| Write::Harness {
                seat,
                incarnation,
                state,
                restamp,
                provider_auth,
            }
        ),
    ]
}

/// A shape of `resource_fields`, rarely the flat one, which keeps its resource's whole history.
fn shape() -> impl Strategy<Value = u8> {
    prop_oneof![9 => 0..6u8, 1 => Just(6u8)]
}

/// The pull request `resource` an observer watches.
fn pull_request(resource: u8) -> String {
    format!("resource/github/acme/garden/pull-request/{resource}")
}

/// An observation of a pull request as `observer` writes it, by shape: open outside the merge
/// queue, queued, merged, closed, a receipt, open with its opener named, or in flat facts.
fn resource_fields(shape: u8, observer: &str) -> serde_json::Map<String, Value> {
    let open = json!({"state": "open", "merge_queue": null});
    let mut fields = json!({"kind": "vcs.pull-request", "observer": observer});
    let more = match shape {
        0 => json!({"facts": open}),
        1 => json!({"facts": {"state": "open", "merge_queue": {"state": "QUEUED", "position": 1}}}),
        2 => json!({"facts": {"state": "closed", "merged": true, "merge_queue": null}}),
        3 => json!({"facts": {"state": "closed", "merged": false, "merge_queue": null}}),
        4 => json!({"facts": open, "attribution_only": true}),
        5 => json!({"facts": {"state": "open", "merge_queue": null, "opened_by": "agent/node.amber"}}),
        _ => json!({"state": "open"}),
    };
    fields.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    fields.as_object().unwrap().clone()
}

fn apply(store: &Store, write: &Write, n: usize) {
    if let Write::Poll { observer, status, resource, shape } = write {
        let observer = format!("observer/watch-{observer}");
        let status = ["ok", "changed", "failed"][usize::from(*status)];
        let mut connection = store.connection.write();
        let transaction = connection.transaction().unwrap();
        let observed = append_claim_tx(
            &transaction,
            &store.origin,
            &observer,
            "observer.observed",
            None,
            &json!({"fields": {"status": status}}),
            &[],
            None,
        )
        .unwrap();
        append_claim_tx(
            &transaction,
            &store.origin,
            &pull_request(*resource),
            "resource.observed",
            None,
            &json!({"fields": resource_fields(*shape, &observer)}),
            &[],
            Some(&observed.batch_id),
        )
        .unwrap();
        transaction.commit().unwrap();
        return;
    }
    let (subject, kind, fields) = match write {
        Write::Observer { observer, status, cursor } => {
            let mut fields = BTreeMap::from([(
                "status".into(),
                json!(["ok", "changed", "failed"][usize::from(*status)]),
            )]);
            if let Some(cursor) = cursor {
                fields.insert("cursor".into(), json!(format!("cursor-{cursor}")));
            }
            (format!("observer/watch-{observer}"), "observer.observed", fields)
        }
        // As an observer writes it: no actor, and its observer named.
        Write::Resource { resource, shape } => {
            let fields = resource_fields(*shape, "observer/watch-0").into_iter().collect();
            (pull_request(*resource), "resource.observed", fields)
        }
        Write::Harness { seat, incarnation, state, restamp, provider_auth } => {
            let seat = SEATS[usize::from(*seat)];
            // The incarnation runs, so the seat readers fold its observations. The key makes
            // this one claim per incarnation.
            store
                .append_claim(&ClaimInput {
                    subject: seat.into(),
                    kind: "runtime.observed".into(),
                    actor: Some(seat.into()),
                    fields: BTreeMap::from([
                        ("status".into(), json!("running")),
                        ("runtime_id".into(), json!(seat.trim_start_matches("agent/"))),
                        ("incarnation_id".into(), json!(format!("{seat}-{incarnation}"))),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("runtime:{seat}-{incarnation}")),
                })
                .unwrap();
            let mut fields = BTreeMap::from([
                ("state".into(), json!(STATES[usize::from(*state)])),
                ("driver".into(), json!("codex")),
                ("incarnation_id".into(), json!(format!("{seat}-{incarnation}"))),
                ("observed_at_ms".into(), json!(n as u64 + 1)),
                ("status_transition".into(), json!(!restamp)),
            ]);
            if let Some(provider_auth) = provider_auth {
                fields.insert("provider_auth".into(), json!(provider_auth));
            }
            (seat.to_owned(), "harness.observed", fields)
        }
        Write::Poll { .. } => unreachable!("written above"),
    };
    store
        .append_claim(&ClaimInput {
            subject: subject.clone(),
            kind: kind.into(),
            actor: (kind == "harness.observed").then(|| subject.clone()),
            fields,
            evidence: Vec::new(),
            expected_subject: None,
            idempotency_key: Some(format!("history-{n}")),
        })
        .unwrap();
}

/// Every answer the readers of these kinds give: the whole graph (with the resource
/// projection), each observer's and resource's newest observation (what resources and reconcile
/// read), each pull request's folded state (`latest_actual`, as JSON), when it left the merge queue as the
/// idle nudge's check walks its observations back to find out, and each seat's current harness,
/// status history, newest turn or end, and newest `working` (when its last turn began, and whether
/// it worked after a given time). The check's answer from the start of time decides its answer
/// from any later time: the exit, if it is later.
fn answers(store: &Store, now: u128) -> (String, Vec<String>) {
    let connection = store.readers.get();
    let mut seats = Vec::new();
    for observer in 0..3 {
        let newest = store.latest_claim(&format!("observer/watch-{observer}"), Some("observer.observed")).unwrap();
        seats.push(format!("observer/watch-{observer}: {:?}", newest.map(|claim| claim.body)));
        let subject = pull_request(observer);
        let newest = store.latest_claim(&subject, Some("resource.observed")).unwrap();
        seats.push(format!("{subject}: {:?}", newest.map(|claim| claim.body)));
        // The fold inserts keys in the order it first meets them, which the trimmed log can change.
        let folded = latest_actual(&connection, &subject).unwrap().map(|value| smallclaims::hash::canonical_json_value(&value));
        seats.push(format!("{subject} folds to {folded:?}"));
        // As `idle_holds` reads a thread's observations.
        let rows = connection
            .prepare(
                "SELECT json_extract(body, '$.fields.facts'), accepted_at_unix_ms,
                 CASE WHEN json_extract(body, '$.fields.attribution_only') IN (1,'true') THEN 1 ELSE 0 END
                 FROM claims WHERE subject=?1 AND kind='resource.observed'
                 ORDER BY length(accepted_at_unix_ms) DESC, accepted_at_unix_ms DESC, store_index DESC LIMIT ?2",
            )
            .unwrap()
            .query_map(rusqlite::params![subject, fractal_github_base::github_watch::THREAD_SCAN], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let exit = fractal_github_base::github_watch::merge_queue_exit(
            rows.into_iter().map(|(facts, at, receipt)| -> serde_json::Result<_> {
                let facts = facts.map(|facts| serde_json::from_str::<Value>(&facts)).transpose()?.unwrap_or(Value::Null);
                Ok((facts, at.parse::<u128>().unwrap_or_default(), receipt))
            }),
            0,
        )
        .unwrap();
        seats.push(format!("{subject} left the merge queue: {exit:?}"));
    }
    for seat in SEATS {
        let harness = store.current_harness(seat).unwrap();
        let history = seat_status::history_at(&connection, seat, now, i64::MAX as u64).unwrap();
        let turn = super::attention_snapshot::last_turn_or_end(&connection, seat).unwrap();
        let worked: Option<(String, Option<i64>)> = connection
            .query_row(
                "SELECT accepted_at_unix_ms, json_extract(body, '$.fields.observed_since_ms') FROM claims
                 WHERE subject=?1 AND kind='harness.observed' AND json_extract(body, '$.fields.state')='working'
                 ORDER BY store_index DESC LIMIT 1",
                [seat],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .unwrap();
        seats.push(format!("{seat}: {harness:?} {history} {turn:?} {worked:?}"));
    }
    (graph_digest_of(store), seats)
}

fn claims_of(store: &Store, kind: &str) -> i64 {
    store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM claims WHERE kind=?1", [kind], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_member_that_trims_locally_trims_its_own_due_cut_once() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    for n in 0..40 {
        apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: Some(1) }, n);
    }
    // Queued, merged and open outside the queue in turn, and queued last.
    for n in 40..50 {
        apply(&store, &Write::Resource { resource: 0, shape: (n % 3) as u8 }, n);
    }
    let before = answers(&store, context.now_unix_ms);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert_eq!(answers(&store, context.now_unix_ms), before, "the readers answer the same");
    assert_eq!(claims_of(&store, "observer.observed"), 2, "the newest of each observer stays");
    assert_eq!(claims_of(&store, "resource.observed"), 1, "a pull request seen queued last walks back no further");
    let retired: i64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM retired_claim_ids", [], |row| row.get(0))
        .unwrap();
    assert!(retired > 0, "what it deleted is retired, so it travels and is never re-admitted");
    // The same day again: nothing more to do, and no staging left behind.
    assert!(step(&store, &context).is_empty());
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().filter(|entry| {
        entry.as_ref().unwrap().file_name().to_string_lossy().starts_with("capture-")
    }).count(), 0);
}

/// A local cut that deletes in many chunks leaves the roster's generation alone until its run
/// stops, so a fold under way publishes. Forgetting the views after every chunk restarted the
/// roster's fold before it could publish, all through a run.
#[test]
fn a_local_cut_forgets_the_views_once_however_many_chunks_it_deletes() {
    let scratch = tempfile::tempdir().unwrap();
    let store = local_member("alder");
    store.set_trim_pause(std::time::Duration::ZERO);
    store.set_trim_chunk_envelopes(1);
    for n in 0..30 {
        apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: None }, n);
    }
    let all = observer_ids(&store);
    // A capped day stops at TrimDeferred, and the next day's run reaches the final mark.
    store.trim_locally_at_most(15);
    for (day, outcome) in [(0, "deferred"), (1, "trimmed")] {
        let context = context(scratch.path(), day);
        let before = answers(&store, context.now_unix_ms);
        let deleted = all.difference(&observer_ids(&store)).count();
        let generation = store.fractal.presented_agents.generation();
        let sequence = claims_sequence(&store);
        let actions = kinds(&step(&store, &context));
        assert_eq!(actions.first(), Some(&outcome), "day {day}: {actions:?}");
        let chunks = all.difference(&observer_ids(&store)).count() - deleted;
        let forgets = store.fractal.presented_agents.generation() - generation;
        assert!(chunks > 4, "day {day}: the run deletes in many chunks ({chunks})");
        // Once when a cut's tombstones are recorded and once when its run stops: the second
        // day finishes the first cut, then records the next one and stops at the cap.
        assert!(forgets <= 3, "day {day}: {forgets} forgets for {chunks} chunks");
        // A stopped run still forgets what the views hold of the claims it deleted, and the
        // final mark still expires every cursor taken before it.
        assert!(forgets >= 1, "day {day}: the run's stop forgets the views");
        assert_eq!(claims_sequence(&store) > sequence, outcome == "trimmed", "day {day}: cursors expire at the final mark");
        assert_eq!(answers(&store, context.now_unix_ms), before, "day {day}: the readers answer the same");
    }
}

/// The agents roster publishes while a local cut deletes in many chunks: a fold begun after
/// one chunk publishes after the next. Forgetting the views after every chunk failed it with
/// "the agents source reset while materializing its publication", all through the run.
#[test]
fn the_roster_publishes_between_the_chunks_of_a_local_cut() {
    use super::agents_publication::{AgentsStatusWatermark, PresentedAgents};
    use super::published_list::Publication;
    let scratch = tempfile::tempdir().unwrap();
    let store = local_member("alder");
    store.set_trim_pause(std::time::Duration::ZERO);
    store.set_trim_chunk_envelopes(1);
    for n in 0..30 {
        apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: None }, n);
    }
    let fractal = Arc::clone(&store.fractal);
    fractal.presented_agents.start();
    let published = Arc::new(Mutex::new(Vec::new()));
    let hook_published = Arc::clone(&published);
    let mut building = None;
    *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |chunk| {
        // A fold begun after one chunk ends after the next, as a fold of seconds does.
        if let Some(build) = building.take() {
            let row = serde_json::json!({ "id": format!("agent/example-{chunk}") });
            let done = fractal.presented_agents.publish_identified(build, |identity| Publication {
                cut: 1,
                published_at_unix_ms: 1,
                valid_until_unix_ms: None,
                rows: PresentedAgents::new(
                    uuid::Uuid::nil(), identity, AgentsStatusWatermark { store_index: 1, local_frontier: 1 }, 1,
                    vec![row], vec![format!("agent/example-{chunk}")],
                ),
            });
            hook_published.lock().unwrap().push(done.is_some());
        }
        building = fractal.presented_agents.begin_materialization();
    }));
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))).first(), Some(&"trimmed"));
    store.trim_chunk_hook.lock().unwrap().take();
    let published = published.lock().unwrap();
    assert!(published.len() > 4, "the cut deletes in many chunks ({})", published.len());
    assert!(published.iter().all(|done| *done), "the agents source reset while materializing its publication: {published:?}");
}

/// A local cut's run lets the views refold once the interval is due, keeping the roster's
/// generation; within the interval it leaves them serving.
#[test]
fn a_local_cut_refolds_the_views_within_its_run_once_the_interval_is_due() {
    for (interval, refolds) in [(std::time::Duration::ZERO, true), (smallclaims::store::checkpoint_trim::TRIM_VIEW_REFOLD_INTERVAL, false)] {
        let scratch = tempfile::tempdir().unwrap();
        let store = local_member("alder");
        store.set_trim_pause(std::time::Duration::ZERO);
        store.set_trim_chunk_envelopes(1);
        store.set_trim_view_refold_interval(interval);
        for n in 0..30 {
            apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: None }, n);
        }
        let fractal = Arc::clone(&store.fractal);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hook_seen = Arc::clone(&seen);
        let mut generation = None;
        *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |_| {
            let now = fractal.presented_agents.generation();
            let refolded = fractal.glass_list.refold.swap(false, std::sync::atomic::Ordering::AcqRel);
            hook_seen.lock().unwrap().push((refolded, *generation.get_or_insert(now) == now));
        }));
        assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))).first(), Some(&"trimmed"));
        store.trim_chunk_hook.lock().unwrap().take();
        let seen = seen.lock().unwrap();
        assert!(seen.len() > 4, "{interval:?}: the cut deletes in many chunks ({})", seen.len());
        assert_eq!(seen.iter().any(|(refolded, _)| *refolded), refolds, "{interval:?}: {seen:?}");
        assert!(seen.iter().all(|(_, kept)| *kept), "{interval:?}: a refold keeps the roster's generation: {seen:?}");
    }
}

/// The frames in a store's WAL, which the next write appends after until a restart starts it
/// over: `mxFrame` of the WAL-index header, read from the shared-memory file so that looking
/// checkpoints nothing.
fn wal_frames(store: &Store) -> i32 {
    let mut shm = store.path.clone().into_os_string();
    shm.push("-shm");
    let header = std::fs::read(shm).unwrap_or_default();
    header.get(16..20).map_or(0, |frames| i32::from_ne_bytes(frames.try_into().unwrap()))
}

/// A file-backed member trimming locally, with its trim unpaced and one envelope a chunk, and a
/// cut's worth of observations to drop.
fn file_member(scratch: &std::path::Path) -> Arc<Store> {
    let store = declared(Store::open(&scratch.join("claims.sqlite3"), "node").unwrap(), true);
    store.set_trim_pause(std::time::Duration::ZERO);
    store.set_trim_chunk_envelopes(1);
    for n in 0..300 {
        apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: None }, n);
    }
    Arc::new(store)
}

/// Read transactions of 60 ms on two readers half a read apart, so one is always open and was
/// begun before the trim's latest commit, as a busy daemon's are; and the daemon's idle recycle
/// every 20 ms. Stop with `stop`.
fn busy_daemon(store: &Arc<Store>, stop: &Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
    let mut threads = Vec::new();
    for offset in [0, 30] {
        let (store, stop) = (Arc::clone(store), Arc::clone(stop));
        threads.push(std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(offset));
            let reader = store.readers.get();
            while !stop.load(Ordering::Acquire) {
                let read = reader.unchecked_transaction().unwrap();
                read.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get::<_, i64>(0)).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(60));
                read.commit().unwrap();
            }
        }));
    }
    let (store, stop) = (Arc::clone(store), Arc::clone(stop));
    threads.push(std::thread::spawn(move || {
        let checkpoint = rusqlite::Connection::open(&store.path).unwrap();
        while !stop.load(Ordering::Acquire) {
            store.checkpoint_idle_wal_report(&checkpoint).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }));
    threads
}

/// A trim writes nearly all the time and a busy daemon always has a reader on an older snapshot,
/// so the idle recycle never finds the WAL copied: on a large member the WAL grew 650 MB a minute
/// until the trim was stopped. Past its bound the trim stops deleting and restarts the WAL, so the
/// WAL stays near the bound for the whole cut however much the cut deletes.
#[test]
fn a_paced_local_trim_keeps_the_wal_under_its_bound_with_readers_always_open() {
    let scratch = tempfile::tempdir().unwrap();
    let store = file_member(scratch.path());
    let bound = 1_000;
    store.set_trim_wal(smallclaims::store::checkpoint_trim::TrimWal {
        frames: bound,
        check: std::time::Duration::ZERO,
        ..smallclaims::store::checkpoint_trim::TRIM_WAL
    });
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&logged);
    *store.trim_log_hook.lock().unwrap() = Some(Box::new(move |line: &str| seen.lock().unwrap().push(line.to_owned())));
    let stop = Arc::new(AtomicBool::new(false));
    let daemon = busy_daemon(&store, &stop);
    let sampled = Arc::new(Mutex::new((0, 0, 0)));
    let samples = Arc::clone(&sampled);
    let hook_store = Arc::clone(&store);
    let mut last = wal_frames(&store);
    *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |_| {
        let frames = wal_frames(&hook_store);
        let mut samples = samples.lock().unwrap();
        // The largest WAL, the most one chunk added, and every frame the trim wrote.
        samples.0 = samples.0.max(frames);
        samples.1 = samples.1.max(frames - last);
        samples.2 += (frames - last).max(0);
        last = frames;
    }));
    let actions = kinds(&step(&store, &context(scratch.path(), 0)));
    store.trim_chunk_hook.lock().unwrap().take();
    stop.store(true, Ordering::Release);
    daemon.into_iter().for_each(|thread| thread.join().unwrap());
    assert_eq!(actions.first(), Some(&"trimmed"), "{actions:?}");
    assert!(observer_ids(&store).len() < 10, "the cut dropped the observations");
    let (largest, chunk, written) = *sampled.lock().unwrap();
    assert!(written > 10 * bound, "the trim writes far past the bound ({written} frames)");
    // Past the bound by what one chunk and the run between two looks write, never by the cut.
    assert!(largest < bound + bound / 4, "the WAL reached {largest} frames, bound {bound}, chunk {chunk}");
    // Each restart says so on the daemon's stderr.
    let restarts = logged.lock().unwrap().iter().filter(|line| line.starts_with("st3: a trim restarted the WAL {")).count();
    assert!(restarts * bound as usize >= written as usize / 2, "{restarts} restart lines for {written} frames");
}

/// A trim whose WAL will not restart, because a reader holds a snapshot past the trim's wait,
/// stops where it is, deletes nothing more, and goes on at the next step once the reader is gone.
#[test]
fn a_trim_that_cannot_restart_the_wal_stops_until_the_next_step_and_resumes() {
    let scratch = tempfile::tempdir().unwrap();
    let store = file_member(scratch.path());
    let bound = 200;
    store.set_trim_wal(smallclaims::store::checkpoint_trim::TrimWal {
        frames: bound,
        busy: std::time::Duration::from_millis(20),
        wait: std::time::Duration::from_millis(300),
        check: std::time::Duration::ZERO,
    });
    let all = observer_ids(&store);
    // From the third chunk, one reader holds its snapshot for longer than the trim waits.
    let (begun, release) = (Arc::new(std::sync::Barrier::new(2)), Arc::new(AtomicBool::new(false)));
    let holder = Arc::new(Mutex::new(None));
    let (hook_store, hook_begun, hook_release, hook_holder) =
        (Arc::clone(&store), Arc::clone(&begun), Arc::clone(&release), Arc::clone(&holder));
    *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |chunk| {
        if chunk == 3 {
            let (store, begun, release) = (Arc::clone(&hook_store), Arc::clone(&hook_begun), Arc::clone(&hook_release));
            *hook_holder.lock().unwrap() = Some(std::thread::spawn(move || {
                let reader = store.readers.get();
                let read = reader.unchecked_transaction().unwrap();
                read.query_row("SELECT COUNT(*) FROM claims", [], |row| row.get::<_, i64>(0)).unwrap();
                begun.wait();
                while !release.load(Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                read.commit().unwrap();
            }));
            hook_begun.wait();
        }
    }));
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&logged);
    *store.trim_log_hook.lock().unwrap() = Some(Box::new(move |line: &str| seen.lock().unwrap().push(line.to_owned())));
    let actions = kinds(&step(&store, &context(scratch.path(), 0)));
    store.trim_chunk_hook.lock().unwrap().take();
    let stopped_at = wal_frames(&store);
    assert!(
        logged.lock().unwrap().iter().any(|line| line.starts_with("st3: a trim could not restart the WAL and stops until the next step {")),
        "the stop is logged: {:?}", logged.lock().unwrap()
    );
    let left = observer_ids(&store);
    release.store(true, Ordering::Release);
    holder.lock().unwrap().take().unwrap().join().unwrap();
    assert_eq!(actions.first(), Some(&"deferred"), "{actions:?}");
    assert!(cut_states(&store).iter().any(|state| state == "trimming"), "{:?}", cut_states(&store));
    assert!(left.len() < all.len() && left.len() > 10, "the run stopped part way ({} of {} left)", left.len(), all.len());
    assert!(stopped_at < 2 * bound, "the trim deleted past the bound ({stopped_at} frames)");
    // The reader gone, the next step restarts the WAL and finishes the cut.
    store.set_trim_wal(smallclaims::store::checkpoint_trim::TRIM_WAL);
    let actions = kinds(&step(&store, &context(scratch.path(), 0)));
    assert_eq!(actions.first(), Some(&"trimmed"), "{actions:?}");
    assert!(observer_ids(&store).len() < 10, "the cut dropped the observations");
}

/// Send `to` what `from` holds and `to` lacks, `to` describing itself with range digests.
fn sync_into(from: &Store, to: &Store) -> usize {
    let remote = to.export_replication_summary(TEST_FLEET).unwrap().inventory;
    let exchange = from.export_replication_exchange(TEST_FLEET, &remote).unwrap();
    let received = to.receive_replication_exchange(&from.origin, TEST_FLEET, &exchange).unwrap().received;
    to.validate_replication_backlog().unwrap();
    to.project_replication_backlog().unwrap();
    received
}

fn inventory_generation(store: &Store) -> i64 {
    store
        .readers
        .get()
        .query_row("SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key='replication_inventory_generation'),0)", [], |row| row.get(0))
        .unwrap()
}

/// A local cut deletes payloads of envelopes it still lists, so the inventory's identities and
/// digests do not change while it runs: the replication snapshot is not rebuilt chunk after
/// chunk, which made every exchange wait behind an inventory build the next chunk made stale.
/// A peer that syncs in the middle of the run and after it ends with the same inventory, and
/// the snapshot built on through the run matches one built from nothing.
#[test]
fn a_local_cut_keeps_the_replication_snapshot_and_a_peer_syncing_through_it_agrees() {
    let scratch = tempfile::tempdir().unwrap();
    let store = Arc::new(local_member("alder"));
    store.set_trim_pause(std::time::Duration::ZERO);
    store.set_trim_chunk_envelopes(1);
    // No catch-up within the run, so any rebuild the run causes shows.
    store.set_trim_view_refold_interval(std::time::Duration::from_secs(3_600));
    for n in 0..30 {
        apply(&store, &Write::Observer { observer: (n % 2) as u8, status: (n % 3) as u8, cursor: None }, n);
    }
    let peer = Arc::new(Store::open_memory("birch").unwrap());
    peer.bind_fleet(TEST_FLEET).unwrap();
    while sync_into(&store, &peer) != 0 {}
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (hook_store, hook_peer, hook_seen) = (Arc::clone(&store), Arc::clone(&peer), Arc::clone(&seen));
    let mut first = None;
    *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |chunk| {
        let snapshot = hook_store.replication_snapshot().unwrap();
        let first = first.get_or_insert_with(|| (Arc::clone(&snapshot), inventory_generation(&hook_store)));
        hook_seen.lock().unwrap().push((chunk, Arc::ptr_eq(&first.0, &snapshot), inventory_generation(&hook_store) == first.1));
        if chunk == 3 {
            sync_into(&hook_store, &hook_peer);
            sync_into(&hook_peer, &hook_store);
        }
    }));
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))).first(), Some(&"trimmed"));
    store.trim_chunk_hook.lock().unwrap().take();
    // The first exchange after the stop builds on from the patched snapshot.
    smallclaims::store::FULL_INVENTORY_LOADS.with(|loads| loads.set(0));
    store.replication_snapshot().unwrap();
    assert_eq!(smallclaims::store::FULL_INVENTORY_LOADS.with(std::cell::Cell::get), 0, "the run's stop loaded the inventory from nothing");
    let seen = seen.lock().unwrap();
    assert!(seen.len() > 4, "the cut deletes in many chunks ({})", seen.len());
    assert!(seen.iter().all(|(_, same, _)| *same), "a chunk rebuilt the replication snapshot: {seen:?}");
    assert!(seen.iter().all(|(_, _, same)| *same), "a chunk moved the inventory generation: {seen:?}");
    while sync_into(&store, &peer) != 0 {}
    while sync_into(&peer, &store) != 0 {}
    let built_on = store.replication_snapshot().unwrap();
    store.forget_replication_snapshot();
    let from_nothing = store.replication_snapshot().unwrap();
    assert!(!Arc::ptr_eq(&built_on, &from_nothing));
    assert_eq!(built_on.inventory.digest, from_nothing.inventory.digest, "the snapshot built on through the run matches one built from nothing");
    assert_eq!(built_on.inventory.payloadless, from_nothing.inventory.payloadless, "the run's stop patched which payloads it holds");
    assert_eq!(built_on.envelope_rows, from_nothing.envelope_rows, "the run's stop patched how many envelope rows it holds");
    assert_eq!(peer.replication_snapshot().unwrap().inventory.digest, from_nothing.inventory.digest, "the peer lists the same identities");
}

fn max_envelope_rowid(store: &Store) -> i64 {
    store.readers.get().query_row("SELECT COALESCE(MAX(rowid), 0) FROM replica_envelopes", [], |row| row.get(0)).unwrap()
}

/// Make the envelope of the newest observer claim the trim drops the newest envelope row, where an
/// old envelope a peer sent late sits, and return its rowid. The trim deletes it after the other
/// dropped ones, and envelope rows have no AUTOINCREMENT, so the next envelope this member admits
/// reuses that rowid.
fn last_dropped_observer_envelope_to_the_tail(store: &Store) -> i64 {
    store.seal_local_batches().unwrap();
    store
        .connection
        .write()
        .execute(
            /* sqlite-large-value: allow-for-now lv-he5f64674d59f owner=fractal_store::store::local_trim_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET rowid=(SELECT MAX(rowid)+1 FROM replica_envelopes)
             WHERE batch_id=(SELECT batch_id FROM claims WHERE kind='observer.observed' ORDER BY store_index DESC LIMIT 1 OFFSET 1)",
            [],
        )
        .unwrap();
    max_envelope_rowid(store)
}

/// Build the replication snapshot once, after the run's first chunk: a run starts by recording
/// its cut, which moves the inventory generation, so only a snapshot built within the run is one
/// the run's catch-ups and its stop build on. Return the inventory generation it was built at,
/// and whether `tail` was still held then.
fn snapshot_after_the_first_chunk(store: &Arc<Store>, tail: i64) -> Arc<Mutex<Option<(i64, bool)>>> {
    let built = Arc::new(Mutex::new(None));
    let (hook_store, hook_built) = (Arc::clone(store), Arc::clone(&built));
    *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |chunk| {
        if chunk == 1 {
            hook_store.replication_snapshot().unwrap();
            *hook_built.lock().unwrap() = Some((inventory_generation(&hook_store), max_envelope_rowid(&hook_store) == tail));
        }
    }));
    built
}

/// `peer` writes one claim and `store` admits its envelope; return the rowid it took.
fn admit_a_peer_envelope(store: &Store, peer: &Store) -> i64 {
    apply(peer, &Write::Observer { observer: 7, status: 0, cursor: None }, 1_000);
    peer.seal_local_batches().unwrap();
    assert_eq!(sync_into(peer, store), 1);
    store
        .readers
        .get()
        .query_row("SELECT MAX(rowid) FROM replica_envelopes WHERE writer=?1", [&peer.origin], |row| row.get(0))
        .unwrap()
}

/// The replication snapshot lists what a load from nothing lists, without forgetting it.
fn snapshot_lists_the_store(store: &Store) -> bool {
    let (full, _) = smallclaims::store::full_compact_replication_inventory(&store.readers.get()).unwrap();
    store.replication_snapshot().unwrap().inventory.digest == full.digest
}

/// A member with one observer's history whose last dropped envelope sits at the tail, and a peer
/// that holds all of it.
fn tail_member() -> (Arc<Store>, Arc<Store>, i64) {
    let store = Arc::new(local_member("alder"));
    store.set_trim_pause(std::time::Duration::ZERO);
    store.set_trim_chunk_envelopes(1);
    for n in 0..12 {
        apply(&store, &Write::Observer { observer: 0, status: (n % 3) as u8, cursor: None }, n);
    }
    let tail = last_dropped_observer_envelope_to_the_tail(&store);
    let peer = Arc::new(Store::open_memory("birch").unwrap());
    peer.bind_fleet(TEST_FLEET).unwrap();
    while sync_into(&store, &peer) != 0 {}
    (store, peer, tail)
}

/// A trim that deletes the newest envelope row moves the inventory generation, so the envelope
/// admitted next, which reuses its rowid, is listed: a snapshot built on from one that ended at
/// that rowid looks only past it. After the run's stop, which patches the snapshot instead of
/// forgetting it.
#[test]
fn after_a_trim_deletes_the_newest_envelope_row_the_envelope_that_reuses_it_is_listed() {
    let scratch = tempfile::tempdir().unwrap();
    let (store, peer, tail) = tail_member();
    store.set_trim_view_refold_interval(std::time::Duration::from_secs(3_600));
    let built = snapshot_after_the_first_chunk(&store, tail);
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))).first(), Some(&"trimmed"));
    store.trim_chunk_hook.lock().unwrap().take();
    let (generation, held) = built.lock().unwrap().expect("the run deleted in chunks");
    assert!(held, "the snapshot built within the run ends at the tail");
    assert!(max_envelope_rowid(&store) < tail, "the trim deleted the newest envelope row");
    let moved = inventory_generation(&store) != generation;
    assert_eq!(admit_a_peer_envelope(&store, &peer), tail, "the envelope admitted next reuses the rowid");
    assert!(snapshot_lists_the_store(&store), "the snapshot lists the envelope that reused the rowid");
    assert!(moved, "deleting the newest envelope row moved the inventory generation");
}

/// The same within a run: the snapshot is patched and built on at each catch-up, and an envelope
/// admitted between two chunks reuses the rowid of the newest row a chunk deleted.
#[test]
fn within_a_trim_s_run_the_envelope_that_reuses_a_deleted_newest_rowid_is_listed() {
    let scratch = tempfile::tempdir().unwrap();
    let (store, peer, tail) = tail_member();
    store.set_trim_view_refold_interval(std::time::Duration::ZERO);
    let seen = Arc::new(Mutex::new((false, None)));
    let (hook_store, hook_peer, hook_seen) = (Arc::clone(&store), Arc::clone(&peer), Arc::clone(&seen));
    *store.trim_chunk_hook.lock().unwrap() = Some(Box::new(move |chunk| {
        let mut seen = hook_seen.lock().unwrap();
        if chunk == 1 {
            hook_store.replication_snapshot().unwrap();
            seen.0 = max_envelope_rowid(&hook_store) == tail;
        } else if seen.1.is_none() && max_envelope_rowid(&hook_store) < tail {
            let reused = admit_a_peer_envelope(&hook_store, &hook_peer);
            seen.1 = Some((reused, snapshot_lists_the_store(&hook_store)));
        }
    }));
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))).first(), Some(&"trimmed"));
    store.trim_chunk_hook.lock().unwrap().take();
    let seen = seen.lock().unwrap().clone();
    assert!(seen.0, "the snapshot built within the run ends at the tail");
    assert_eq!(seen.1, Some((tail, true)), "admitted between chunks, reusing the rowid, and listed");
    assert!(snapshot_lists_the_store(&store));
}

/// After a run's stop, a lagging peer that holds nothing in a range of trimmed envelopes is sent
/// the envelope it lacks: the patched snapshot knows their payloads are gone, so the page it lists
/// is not filled with identities export then skips, exchange after exchange.
#[test]
fn after_a_trim_s_stop_a_lagging_peer_is_not_listed_the_same_empty_page_twice() {
    let scratch = tempfile::tempdir().unwrap();
    let store = Arc::new(local_member("alder"));
    store.set_trim_pause(std::time::Duration::ZERO);
    store.set_trim_chunk_envelopes(1);
    store.set_trim_view_refold_interval(std::time::Duration::from_secs(3_600));
    let peer = Store::open_memory("birch").unwrap();
    peer.bind_fleet(TEST_FLEET).unwrap();
    while sync_into(&store, &peer) != 0 {}
    // Enough that the newest fill a sequence range the peer holds nothing in.
    for n in 0..smallclaims::store::REPLICATION_BUCKET_WIDTH as usize + 16 {
        apply(&store, &Write::Observer { observer: 0, status: (n % 3) as u8, cursor: None }, n);
    }
    store.seal_local_batches().unwrap();
    let built = snapshot_after_the_first_chunk(&store, 0);
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))).first(), Some(&"trimmed"));
    store.trim_chunk_hook.lock().unwrap().take();
    assert!(built.lock().unwrap().is_some(), "the run deleted in chunks");
    let mut remote = peer.export_replication_summary(TEST_FLEET).unwrap().inventory;
    remote.accepts = Some(3);
    let sent: Vec<_> = (0..2).map(|_| store.export_replication_exchange(TEST_FLEET, &remote).unwrap().envelopes.len()).collect();
    assert!(sent.iter().all(|sent| *sent > 0), "a lagging peer was listed an empty page: {sent:?}");
}

fn triggers(store: &Store) -> BTreeSet<String> {
    store
        .readers
        .get()
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE '%_inventory_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Opening a store replaces the inventory triggers that counted every trimmed payload with ones
/// that count only identities the inventory stops or starts listing. An older build that opens
/// it recreates its own trigger beside the new one and counts every row again, which is what it
/// always did; the next open of this build drops it again.
#[test]
fn opening_a_store_replaces_the_inventory_triggers_and_an_older_build_s_come_back_beside_them() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("claims.sqlite3");
    let store = Store::open(&path, "node").unwrap();
    let names = triggers(&store);
    assert!(names.contains("replica_envelopes_inventory_DELETE_v2") && names.contains("retired_envelopes_inventory_INSERT_v2"), "{names:?}");
    assert!(!names.contains("replica_envelopes_inventory_DELETE") && !names.contains("retired_envelopes_inventory_INSERT"), "{names:?}");
    // As an older build's open creates it.
    store
        .connection
        .write()
        .execute_batch(
            /* sqlite-large-value: allow-for-now lv-h5f1625904b9b owner=fractal_store::store::local_trim_tests TODO=chunk-or-reference-exact-wire-values */ "CREATE TRIGGER IF NOT EXISTS replica_envelopes_inventory_DELETE AFTER DELETE ON replica_envelopes
             BEGIN
                 INSERT INTO meta(key,value) VALUES('replication_inventory_generation','1')
                 ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1;
             END;",
        )
        .unwrap();
    assert!(triggers(&store).contains("replica_envelopes_inventory_DELETE"));
    drop(store);
    let store = Store::open(&path, "node").unwrap();
    let names = triggers(&store);
    assert!(!names.contains("replica_envelopes_inventory_DELETE"), "this build's open drops it again: {names:?}");
    assert!(names.contains("replica_envelopes_inventory_DELETE_v2"), "{names:?}");
    // The trigger that counts deleting the newest envelope row is created once, whatever opens
    // the store: an older build's open drops only the two triggers named above, and it already
    // compares the inventory generation this one moves, so it builds its next snapshot from
    // nothing.
    assert!(names.contains("replica_envelopes_inventory_DELETE_TAIL"), "{names:?}");
    let tail_triggers: i64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name='replica_envelopes_inventory_DELETE_TAIL'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tail_triggers, 1, "a second open does not add another");
}

/// The claims sequence a page cursor carries, which the final mark of a trim moves on.
fn claims_sequence(store: &Store) -> i64 {
    store
        .readers
        .get()
        .query_row("SELECT seq FROM sqlite_sequence WHERE name='claims'", [], |row| row.get(0))
        .unwrap()
}

/// How many claims this member retired, and whether every one of `deleted` is among them.
fn retired_exactly(store: &Store, deleted: &BTreeSet<String>) -> bool {
    let connection = store.readers.get();
    let count: usize = connection.query_row("SELECT COUNT(*) FROM retired_claim_ids", [], |row| row.get(0)).unwrap();
    let is_retired = format!(
        "SELECT EXISTS(SELECT 1 FROM retired_claim_ids WHERE id={})",
        smallclaims::store::retired::identity_sql("?1")
    );
    let mut retired = connection.prepare(/* sqlite-large-value: allow-for-now lv-h15c5aca2ae00 owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ &is_retired).unwrap();
    count == deleted.len() && deleted.iter().all(|id| retired.query_row(/* sqlite-large-value: allow-for-now lv-h79e3a98c6d5e owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ [id], |row| row.get::<_, bool>(0)).unwrap())
}

fn observer_ids(store: &Store) -> BTreeSet<String> {
    store
        .readers
        .get()
        .prepare("SELECT id FROM claims WHERE kind='observer.observed'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn cut_states(store: &Store) -> Vec<String> {
    store
        .readers
        .get()
        .prepare("SELECT state FROM checkpoints ORDER BY cut_unix_ms")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A cut that drops more than the day's claims stops at the cap, stays `trimming`, and goes on
/// the next day where it stopped; only what it deleted is retired, and the readers answer the
/// same each day.
#[test]
fn a_capped_local_cut_stops_at_the_day_s_claims_and_goes_on_the_next_day() {
    let scratch = tempfile::tempdir().unwrap();
    let store = local_member("alder");
    store.trim_locally_at_most(10);
    for n in 0..30 {
        apply(&store, &Write::Observer { observer: 0, status: (n % 3) as u8, cursor: None }, n);
    }
    let all = observer_ids(&store);
    // Each day's clock, and what the readers answer at it from the whole log.
    let contexts: Vec<_> = (0..3).map(|day| context(scratch.path(), day)).collect();
    let before: Vec<_> = contexts.iter().map(|context| answers(&store, context.now_unix_ms)).collect();
    for (day, deleted) in [(0_u128, 10), (1, 20), (2, 29)] {
        let context = &contexts[day as usize];
        let actions = kinds(&step(&store, context));
        if day < 2 {
            assert_eq!(actions, ["deferred"], "day {day}");
            assert_eq!(cut_states(&store), ["trimming"], "day {day}");
        } else {
            assert_eq!(actions.first(), Some(&"trimmed"), "day {day}: {actions:?}");
        }
        let gone: BTreeSet<_> = all.difference(&observer_ids(&store)).cloned().collect();
        assert_eq!(gone.len(), deleted, "day {day}: ten a day, then the rest; the newest stays");
        assert!(retired_exactly(&store, &gone), "day {day}: exactly what it deleted is retired");
        assert_eq!(answers(&store, context.now_unix_ms), before[day as usize], "day {day}");
        step(&store, context);
        assert_eq!(all.difference(&observer_ids(&store)).count(), deleted, "day {day}: no more the same day");
    }
    assert!(orphans(&store).iter().all(|(_, count)| *count == 0), "{:?}", orphans(&store));
}

/// The next day's cut waits for the leftover of a capped one, then trims its own new claims
/// with what the day has left: both in order, with the retired ledger exact.
#[test]
fn the_next_day_s_cut_follows_the_leftover_of_a_capped_one() {
    let scratch = tempfile::tempdir().unwrap();
    let store = local_member("alder");
    store.trim_locally_at_most(10);
    for n in 0..16 {
        apply(&store, &Write::Observer { observer: 0, status: (n % 3) as u8, cursor: None }, n);
    }
    let all_first = observer_ids(&store);
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))), ["deferred"]);
    // Observations of another observer after the first cut was recorded: the next cut's.
    for n in 16..24 {
        apply(&store, &Write::Observer { observer: 1, status: (n % 3) as u8, cursor: None }, n);
    }
    let all: BTreeSet<_> = all_first.union(&observer_ids(&store)).cloned().collect();
    let next_day = context(scratch.path(), 1);
    let before = answers(&store, next_day.now_unix_ms);
    // The leftover's five go first; then the new cut takes the five the day has left.
    assert_eq!(kinds(&step(&store, &next_day)), ["trimmed", "deferred"]);
    assert_eq!(cut_states(&store), ["trimmed", "trimming"]);
    assert_eq!(claims_of(&store, "observer.observed"), 1 + 8 - 5);
    assert_eq!(answers(&store, next_day.now_unix_ms), before);
    let gone: BTreeSet<_> = all.difference(&observer_ids(&store)).cloned().collect();
    assert!(retired_exactly(&store, &gone));
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 2))).first(), Some(&"trimmed"));
    assert!(cut_states(&store).iter().all(|state| state == "trimmed"), "{:?}", cut_states(&store));
    assert_eq!(claims_of(&store, "observer.observed"), 2, "the newest of each observer stays");
    let gone: BTreeSet<_> = all.difference(&observer_ids(&store)).cloned().collect();
    assert_eq!(gone.len(), 22);
    assert!(retired_exactly(&store, &gone), "every deleted claim is retired, and only those");
    assert!(orphans(&store).iter().all(|(_, count)| *count == 0), "{:?}", orphans(&store));
}

/// A member restarted while a capped cut is `trimming` goes on from where it stopped: the walk
/// and the day's count are durable, so the same day deletes no more, and the next days the rest.
#[test]
fn a_restart_during_a_capped_cut_goes_on_where_it_stopped() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("claims.sqlite3");
    let open = |first: bool| {
        let store = Store::open(&path, "node").unwrap();
        store.bind_fleet(TEST_FLEET).unwrap();
        if first {
            store.import_retired_identities_paced(|_| {}).unwrap();
        }
        store.trim_locally(true).unwrap();
        store.trim_locally_at_most(10);
        store
    };
    let all = {
        let store = open(true);
        for n in 0..25 {
            apply(&store, &Write::Observer { observer: 0, status: (n % 3) as u8, cursor: None }, n);
        }
        let all = observer_ids(&store);
        assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))), ["deferred"]);
        all
    };
    let store = open(false);
    assert_eq!(cut_states(&store), ["trimming"]);
    assert_eq!(claims_of(&store, "observer.observed"), 15);
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 0))), ["deferred"]);
    assert_eq!(claims_of(&store, "observer.observed"), 15, "the same day deletes no more");
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 1))), ["deferred"]);
    assert_eq!(claims_of(&store, "observer.observed"), 5);
    assert_eq!(kinds(&step(&store, &context(scratch.path(), 2))).first(), Some(&"trimmed"));
    let left = observer_ids(&store);
    assert_eq!(left.len(), 1, "the newest stays");
    assert!(retired_exactly(&store, &all.difference(&left).cloned().collect()));
}

/// Delete the middle of three observations of `resource` as a trim of that kind would: recorded
/// as a tombstone first, so its preparation rebuild is deferred.
fn defer_a_resource_delete(store: &Store, resource: u8) {
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    for phase in ["first", "middle", "newest"] {
        let body = json!({"fields": {"kind": "vcs.pull-request", "state": "open", "observer": "observer/watch-0", "facts": {"phase": phase}}});
        append_claim_tx(&transaction, &store.origin, &format!("resource/pr/{resource}"), "resource.observed", None, &body, &[], None).unwrap();
    }
    let middle: String = transaction
        .query_row(
            "SELECT id FROM claims WHERE subject=?1 AND kind='resource.observed' ORDER BY store_index DESC LIMIT 1 OFFSET 1",
            [format!("resource/pr/{resource}")],
            |row| row.get(0),
        )
        .unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h4eb31bda17fb owner=fractal_store::store::local_trim_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoints(id, cut_unix_ms, state, updated_at_unix_ms)
             VALUES ('checkpoint/example-resource', 0, 'trimmed', 0) ON CONFLICT(id) DO NOTHING",
            [],
        )
        .unwrap();
    transaction
        .execute(
            /* sqlite-large-value: allow-for-now lv-h2d276ff3d187 owner=fractal_store::store::local_trim_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_claims(id, writer, sequence, envelope_hash, subject, kind, actor,
                 predecessors, operation_id, request_digest, accepted_at_unix_ms, checkpoint)
             SELECT id, 'example-writer', 1, 'example-hash', subject, kind, actor, '[]', NULL, NULL, 0,
                 'checkpoint/example-resource' FROM claims WHERE id=?1",
            [&middle],
        )
        .unwrap();
    transaction.execute(/* sqlite-large-value: allow-for-now lv-hf46030d1c556 owner=fractal_store::store::local_trim_tests TODO=prepare-body-and-predecessor-values */ "DELETE FROM claims WHERE id=?1", [&middle]).unwrap();
    transaction.commit().unwrap();
}

/// What preparation answers for `subject`, and what a rebuild from every claim present answers.
fn prepared_and_rebuilt(store: &Store, subject: &str) -> (String, String) {
    let mut writer = store.connection.write();
    let transaction = writer.transaction().unwrap();
    let prepared = format!("{:?}", super::resources::preparation_observation(&transaction, subject).unwrap());
    super::resources::rebuild_preparation(&transaction).unwrap();
    let rebuilt = format!("{:?}", super::resources::preparation_observation(&transaction, subject).unwrap());
    transaction.rollback().unwrap();
    (prepared, rebuilt)
}

/// Over a capped cut's days, what a trim's deletes deferred is rebuilt by the end of each day's
/// run: nothing stays deferred past it, and preparation answers what a rebuild answers.
#[test]
fn a_capped_cut_s_days_each_rebuild_what_their_deletes_deferred() {
    let scratch = tempfile::tempdir().unwrap();
    let store = local_member("alder");
    store.trim_locally_at_most(10);
    // Superseded observer observations and each day's superseded resource observation: three
    // days of at most ten.
    for n in 0..27 {
        apply(&store, &Write::Observer { observer: 0, status: (n % 3) as u8, cursor: None }, n);
    }
    let contexts: Vec<_> = (0..3).map(|day| context(scratch.path(), day)).collect();
    for (day, context) in contexts.iter().enumerate() {
        defer_a_resource_delete(&store, day as u8);
        let deferred = |store: &Store| -> i64 {
            store.readers.get().query_row("SELECT COUNT(*) FROM local_resource_preparation_deferred", [], |row| row.get(0)).unwrap()
        };
        assert_eq!(deferred(&store), 1, "day {day}: one subject waits");
        let before = answers(&store, context.now_unix_ms);
        step(&store, context);
        assert_eq!(deferred(&store), 0, "day {day}: rebuilt by the end of the day's run");
        let (prepared, rebuilt) = prepared_and_rebuilt(&store, &format!("resource/pr/{day}"));
        assert_eq!(prepared, rebuilt, "day {day}");
        assert_eq!(answers(&store, context.now_unix_ms), before, "day {day}");
        let state = if day < 2 { "trimming" } else { "trimmed" };
        assert_eq!(cut_states(&store).last().map(String::as_str), Some(state), "day {day}");
    }
}

/// Rows this member holds for a claim it no longer has. A `claim_positions` row outlives its
/// claim by design (see schema.md), so it is not one.
fn orphans(store: &Store) -> Vec<(&'static str, i64)> {
    ["replica_records", "claim_signatures", "claim_verdicts"]
        .into_iter()
        .map(|table| {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE claim_id NOT IN (SELECT id FROM claims)");
            (table, store.readers.get().query_row(/* sqlite-large-value: allow-for-now lv-he41d7d662db4 owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ &sql, [], |row| row.get(0)).unwrap())
        })
        .collect()
}

#[test]
fn a_member_that_trims_locally_drops_superseded_claims_from_envelopes_it_keeps() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    for n in 0..20 {
        apply(&store, &Write::Poll { observer: 0, status: (n % 3) as u8, resource: 0, shape: 0 }, n);
    }
    store.seal_local_batches().unwrap();
    let envelopes = || -> i64 {
        store.readers.get().query_row("SELECT COUNT(*) FROM replica_envelopes", [], |row| row.get(0)).unwrap()
    };
    let sealed = envelopes();
    let before = answers(&store, context.now_unix_ms);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert_eq!(answers(&store, context.now_unix_ms), before, "the readers answer the same");
    assert_eq!(claims_of(&store, "observer.observed"), 1, "the newest observation stays");
    assert_eq!(claims_of(&store, "resource.observed"), 20, "a pull request open outside the queue keeps what a check reads");
    // Each poll's envelope stays whole, for its resource observation and for peers; only this
    // member's own copies of the superseded observations go, and each goes into the ledger.
    assert_eq!(envelopes(), sealed);
    let orphaned = orphans(&store);
    assert!(orphaned.iter().all(|(_, count)| *count == 0), "{orphaned:?}");
    let retired: i64 = store
        .readers
        .get()
        .query_row("SELECT COUNT(*) FROM retired_claim_ids", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retired, 19);
    // Admitting the kept envelopes again brings back neither a claim nor its record.
    store.connection.write().execute(/* sqlite-large-value: allow-for-now lv-h2bc6841e508b owner=fractal_store::store::local_trim_tests TODO=chunk-or-reference-exact-wire-values */ "UPDATE replica_envelopes SET receipt_state='pending'", []).unwrap();
    store.validate_replication_backlog().unwrap();
    assert_eq!(claims_of(&store, "observer.observed"), 1);
    let orphaned = orphans(&store);
    assert!(orphaned.iter().all(|(_, count)| *count == 0), "{orphaned:?}");
    assert_eq!(answers(&store, context.now_unix_ms), before);
}

/// A fresh member that joins from one that dropped claims from envelopes it kept: what it
/// joins from serves each kept envelope whole, its payload and signature as the writer sealed
/// them, so the joiner admits every envelope with no fault, hold or degraded receipt, and its
/// readers and projection digests agree. An ordinary joiner keeps the full history, as from any
/// peer; one that trims locally drops the same superseded claims at its own cut. Sending back
/// brings nothing the first member retired back to it.
#[test]
fn a_fresh_member_joining_from_one_that_split_envelopes_admits_them_whole_and_agrees() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let alder = local_member("alder");
    for n in 0..20 {
        apply(&alder, &Write::Poll { observer: 0, status: (n % 3) as u8, resource: 0, shape: 0 }, n);
    }
    alder.seal_local_batches().unwrap();
    assert_eq!(kinds(&step(&alder, &context)), ["trimmed"]);
    assert_eq!(claims_of(&alder, "observer.observed"), 1, "alder split its envelopes");
    let answers_of_alder = answers(&alder, context.now_unix_ms);
    let retired = |store: &Store| -> Vec<String> {
        store
            .readers
            .get()
            .prepare("SELECT hex(id) FROM retired_claim_ids ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let retired_by_alder = retired(&alder);
    assert_eq!(retired_by_alder.len(), 19);
    let receipts = |store: &Store| -> Vec<(String, i64)> {
        store
            .readers
            .get()
            .prepare("SELECT receipt_state, COUNT(*) FROM replica_envelopes GROUP BY receipt_state ORDER BY receipt_state")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let unsettled_records = |store: &Store| -> i64 {
        store
            .readers
            .get()
            .query_row("SELECT COUNT(*) FROM replica_records WHERE state NOT IN ('valid', 'repaired')", [], |row| row.get(0))
            .unwrap()
    };
    for trims_locally in [false, true] {
        let cedar = Store::open_memory("cedar").unwrap();
        cedar.bind_fleet(TEST_FLEET).unwrap();
        cedar.import_retired_identities_paced(|_| {}).unwrap();
        cedar.trim_locally(trims_locally).unwrap();
        send(&alder, &cedar);
        let rest = crate::store::tests_shared::exchange_from(&alder, &cedar.replication_inventory().unwrap());
        assert!(rest.envelopes.is_empty(), "cedar holds every envelope alder serves");
        // Every envelope arrived whole: its hash and signature checked, every record admitted.
        assert_eq!(receipts(&cedar), receipts(&alder), "trims locally: {trims_locally}");
        assert!(receipts(&cedar).iter().all(|(state, _)| state == "validated"), "{:?}", receipts(&cedar));
        assert_eq!(unsettled_records(&cedar), 0);
        assert_eq!(answers(&cedar, context.now_unix_ms), answers_of_alder, "trims locally: {trims_locally}");
        // The projections agree: a claim alder dropped is its tombstone there and a claim here.
        assert_eq!(
            projection_digest::tables(&cedar.readers.get()).unwrap(),
            projection_digest::tables(&alder.readers.get()).unwrap(),
            "trims locally: {trims_locally}"
        );
        assert_eq!(graph_digest_of(&cedar), graph_digest_of(&alder));
        // Every observation the payloads hold, as from any peer that kept them.
        assert_eq!(claims_of(&cedar, "observer.observed"), 20);
        if trims_locally {
            // Its own cut drops the same superseded claims, and retires the same IDs.
            assert_eq!(kinds(&step(&cedar, &context)), ["trimmed"]);
            assert_eq!(claims_of(&cedar, "observer.observed"), 1);
            assert_eq!(claims_of(&cedar, "resource.observed"), 20);
            assert_eq!(retired(&cedar), retired_by_alder);
            let orphaned = orphans(&cedar);
            assert!(orphaned.iter().all(|(_, count)| *count == 0), "{orphaned:?}");
            assert_eq!(answers(&cedar, context.now_unix_ms), answers_of_alder);
        }
        // What cedar sends back brings nothing alder retired back to alder.
        send(&cedar, &alder);
        assert_eq!(claims_of(&alder, "observer.observed"), 1);
        let orphaned = orphans(&alder);
        assert!(orphaned.iter().all(|(_, count)| *count == 0), "{orphaned:?}");
        assert_eq!(answers(&alder, context.now_unix_ms), answers_of_alder);
        assert_eq!(unsettled_records(&alder), 0);
    }
}

#[test]
fn an_ordinary_member_does_not_trim_locally() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = member(false);
    for n in 0..10 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    assert!(!kinds(&step(&store, &context)).contains(&"trimmed"));
    assert_eq!(claims_of(&store, "observer.observed"), 10);
}

#[test]
fn a_plan_that_would_drop_the_newest_claim_of_a_key_stops_before_deleting_anything() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    for n in 0..6 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    let claims = claims_of(&store, "observer.observed");
    set_notify(&store, "agent/example/operations");
    store.set_trim_fault(Some(smallclaims::store::TrimFault::DropNewest));
    let actions = step(&store, &context);
    assert_eq!(kinds(&actions), ["graph-changed"]);
    assert_eq!(claims_of(&store, "observer.observed"), claims, "nothing is deleted");
    // The stop alerts the `[trim] notify` agent, once however often it is recorded.
    store.record_local_trim_stops(&actions).unwrap();
    store.record_local_trim_stops(&actions).unwrap();
    let faults = || -> Vec<(String, String)> {
        store
            .readers
            .get()
            .prepare(
                "SELECT kind, json_extract(body,'$.fields.reviewer') FROM claims
                 WHERE subject='daemon/node' AND kind IN ('operational.failure','operational.recovered')
                   AND json_extract(body,'$.fields.episode') LIKE 'local-trim-stopped/%'
                 ORDER BY store_index",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default())))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(faults(), [("operational.failure".to_string(), "agent/example/operations".to_string())]);
    // Halted until resumed. Only the `[trim] notify` agent or a person may resume, and the
    // resume ends the fault.
    assert!(step(&store, &context).is_empty());
    assert!(store.resume_stopped_trims("agent/example/other", "looked at the plan").is_err());
    assert!(store.resume_stopped_trims("agent/example/operations", " ").is_err());
    assert_eq!(faults().len(), 1, "a refused resume leaves the fault open");
    assert!(step(&store, &context).is_empty(), "a refused resume leaves the member halted");
    store.resume_stopped_trims("agent/example/operations", "looked at the plan").unwrap();
    assert_eq!(faults().last().unwrap().0, "operational.recovered");
    assert!(step(&store, &context).is_empty(), "the stopped cut is set aside, not retried");
}

#[test]
fn only_a_person_or_the_notify_agent_resumes_a_member_that_trims_locally() {
    let store = local_member("alder");
    // With no `[trim] notify` agent, only a person may.
    assert!(store.resume_stopped_trims("agent/example/operations", "looked").is_err());
    store.resume_stopped_trims("person/operator", "looked").unwrap();
    set_notify(&store, "agent/example/operations");
    assert!(store.resume_stopped_trims("agent/example/other", "looked").is_err());
    store.resume_stopped_trims("agent/example/operations", "looked").unwrap();
    store.resume_stopped_trims("person/operator", "looked").unwrap();
    // An ordinary member: a person only, whatever `[trim] notify` names.
    let ordinary = member(false);
    set_notify(&ordinary, "agent/example/operations");
    assert!(ordinary.resume_stopped_trims("agent/example/operations", "looked").is_err());
    ordinary.resume_stopped_trims("person/operator", "looked").unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// For random histories of observer, resource and harness observations, some sealed together
    /// as one poll, every reader answers the same
    /// before and after the local trim, and the newest claim of each subject and kind stays.
    #[test]
    fn readers_answer_the_same_from_the_full_and_the_trimmed_log(
        writes in proptest::collection::vec(write(), 1..60)
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let context = context(scratch.path(), 0);
        let store = local_member("alder");
        for (n, write) in writes.iter().enumerate() {
            apply(&store, write, n);
        }
        let newest: BTreeSet<String> = store
            .readers
            .get()
            .prepare(
                "SELECT id FROM claims AS claim WHERE kind IN ('observer.observed', 'resource.observed', 'harness.observed')
                 AND store_index=(SELECT MAX(store_index) FROM claims AS newer
                                  WHERE newer.subject=claim.subject AND newer.kind=claim.kind)",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let before = answers(&store, context.now_unix_ms);
        let actions = kinds(&step(&store, &context));
        prop_assert!(!actions.contains(&"graph-changed"), "{actions:?}");
        prop_assert_eq!(answers(&store, context.now_unix_ms), before);
        for id in &newest {
            prop_assert!(store.claim_by_id(id).unwrap().is_some(), "the newest claim {} stays", id);
        }
        let orphaned = orphans(&store);
        prop_assert!(orphaned.iter().all(|(_, count)| *count == 0), "{:?}", orphaned);
    }
}

/// A needs-login seat keeps its `since` through local trim: the rule keeps the credential report
/// that ended the run before the current one (see #4237), while the transitions it supersedes go.
#[test]
fn local_trim_keeps_where_a_needs_login_run_began() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    // Ready without provider auth, then starting, working and idle with it, then working, and
    // ready and idle without it again: the starting, working and idle in between go.
    let history = [(0, None), (1, Some(false)), (0, Some(true)), (3, Some(true)), (2, Some(true)), (3, Some(true)), (1, Some(false)), (2, Some(false))];
    for (n, (state, provider_auth)) in history.into_iter().enumerate() {
        let write = Write::Harness { seat: 0, incarnation: 0, state, restamp: false, provider_auth };
        apply(&store, &write, n);
    }
    for n in history.len()..20 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    let harness = claims_of(&store, "harness.observed");
    let before = answers(&store, context.now_unix_ms);
    assert!(before.1.iter().any(|answer| answer.contains("needs-login") || answer.contains("NeedsLogin")), "{:?}", before.1);
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    assert_eq!(answers(&store, context.now_unix_ms), before);
    assert_eq!(claims_of(&store, "harness.observed"), harness - 3, "the superseded transitions go");
    assert_eq!(claims_of(&store, "observer.observed"), 1);
}

fn harness_write() -> impl Strategy<Value = Write> {
    (0..2u8, 0..6u8, any::<bool>(), proptest::option::of(any::<bool>())).prop_map(|(incarnation, state, restamp, provider_auth)| {
        Write::Harness { seat: 0, incarnation, state, restamp, provider_auth }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// For random histories of one seat's harness observations, over two incarnations, the
    /// seat's current harness and status history answer the same before and after the local
    /// trim, and the newest observation stays.
    #[test]
    fn a_seat_answers_the_same_from_its_full_and_trimmed_harness_observations(
        writes in proptest::collection::vec(harness_write(), 1..40)
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let context = context(scratch.path(), 0);
        let store = local_member("alder");
        for (n, write) in writes.iter().enumerate() {
            apply(&store, write, n);
        }
        let newest = store.latest_claim(SEATS[0], Some("harness.observed")).unwrap().map(|claim| claim.id);
        let before = answers(&store, context.now_unix_ms);
        let actions = kinds(&step(&store, &context));
        prop_assert!(!actions.contains(&"graph-changed"), "{actions:?}");
        prop_assert_eq!(answers(&store, context.now_unix_ms), before);
        if let Some(id) = newest {
            prop_assert!(store.claim_by_id(&id).unwrap().is_some(), "the newest observation {} stays", id);
        }
        let orphaned = orphans(&store);
        prop_assert!(orphaned.iter().all(|(_, count)| *count == 0), "{:?}", orphaned);
    }
}

/// The idle nudge finds when a pull request left the merge queue by walking its observations
/// back to the one that saw it queued, which can be before the cut. Local trim keeps those and
/// drops the open observations before them; a resource in flat facts keeps every observation.
#[test]
fn local_trim_keeps_a_pull_requests_observations_back_to_the_merge_queue() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    let writes = [(0, 0), (0, 0), (0, 1), (0, 0), (0, 0), (1, 6), (1, 0), (1, 0)];
    for (n, (resource, shape)) in writes.into_iter().enumerate() {
        apply(&store, &Write::Resource { resource, shape }, n);
    }
    for n in writes.len()..20 {
        apply(&store, &Write::Observer { observer: 0, status: 0, cursor: None }, n);
    }
    let before = answers(&store, context.now_unix_ms);
    assert!(before.1.iter().any(|answer| answer.contains("pull-request/0 left the merge queue: Some(Some(")), "{before:?}");
    assert_eq!(kinds(&step(&store, &context)), ["trimmed"]);
    let count = |resource: u8| -> i64 {
        store.readers.get().query_row("SELECT COUNT(*) FROM claims WHERE subject=?1", [pull_request(resource)], |row| row.get(0)).unwrap()
    };
    assert_eq!(count(0), 3, "the queued observation and those after it stay");
    assert_eq!(count(1), 3, "a resource in flat facts keeps every observation");
    assert_eq!(claims_of(&store, "observer.observed"), 1);
    assert_eq!(answers(&store, context.now_unix_ms), before);
}

/// One step of a history for the capture equivalence test: a write, or something that makes a
/// guard of the local cut answer for an observer claim.
#[derive(Clone, Debug)]
enum Step {
    Write(Write),
    /// A claim of another kind whose evidence cites the `nth` observer claim so far.
    Cite(u8),
    /// A mission definition naming the `nth` observer claim, which protects it.
    Protect(u8),
    /// A repair naming the `nth` observer claim its replacement, by claim or by record.
    Repair(u8, bool),
    /// An observer claim and a claim of another kind in one operation, in two envelopes.
    Shared(u8),
    /// An earlier checkpoint's tombstone of one of this writer's envelopes, newer than any it
    /// holds (`newer`) or older: the writer's-newest guard reads tombstones too.
    Tombstone(bool),
    /// A peer's claim citing the `nth` observer claim, written now and received only at the end
    /// of the history: it arrives after later claims, with an older accepted time.
    PeerCite(u8),
    /// The citation cache caught up now, all the way or (`partial`) stopped after one page.
    CatchUp(bool),
    /// The `nth` claim retired as a local trim deletes it, an odd `nth` from the claims that
    /// cite one, so a claim another still cites loses one of its citers; with `stale`, its
    /// citation rows are put back afterwards, as a binary that kept no cache would leave them.
    Retire(u8, bool),
    /// Two observations of one observer in one batch: the older is not the newest of its slot
    /// yet sits in the writer's newest observer envelope, so the writer's-newest guard decides it.
    Twice(u8),
    /// One source of a seat's status history: a harness transition (`kind` 0), observed inside
    /// the seven-day history window or (`old`) nine days back, outside it; a runtime reset to an
    /// incarnation (1); or a restored provider prompt (2), both dated by acceptance, inside it.
    /// The status-history rule reads all three to judge a harness observation.
    Seat { kind: u8, seat: u8, incarnation: u8, state: u8, old: bool },
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => write().prop_map(Step::Write),
        1 => any::<u8>().prop_map(Step::Cite),
        1 => any::<u8>().prop_map(Step::Protect),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(nth, by_claim)| Step::Repair(nth, by_claim)),
        1 => (0..3u8).prop_map(Step::Shared),
        1 => (0..3u8).prop_map(Step::Twice),
        6 => (0..3u8, 0..2u8, 0..3u8, 0..6u8, any::<bool>())
            .prop_map(|(kind, seat, incarnation, state, old)| Step::Seat { kind, seat, incarnation, state, old }),
        1 => any::<bool>().prop_map(Step::Tombstone),
        1 => any::<u8>().prop_map(Step::PeerCite),
        1 => any::<bool>().prop_map(Step::CatchUp),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(nth, stale)| Step::Retire(nth, stale)),
    ]
}

/// Everything `from` holds that `to` lacks, received and admitted as replication would.
fn send(from: &Store, to: &Store) {
    for _ in 0..64 {
        let exchange = crate::store::tests_shared::exchange_from(from, &to.replication_inventory().unwrap());
        if exchange.envelopes.is_empty() {
            return;
        }
        crate::store::tests_shared::receive_and_project(to, from.origin(), &exchange);
    }
}

fn take_step(store: &Store, peer: &Store, step: &Step, n: usize) {
    let observed = || -> Vec<String> {
        store
            .readers
            .get()
            .prepare("SELECT id FROM claims WHERE kind='observer.observed' ORDER BY store_index")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let nth = |nth: u8| {
        let ids = observed();
        (!ids.is_empty()).then(|| ids[usize::from(nth) % ids.len()].clone())
    };
    match step {
        Step::Write(write) => apply(store, write, n),
        Step::Cite(k) => {
            let Some(id) = nth(*k) else { return };
            store
                .append_claim(&ClaimInput {
                    subject: "resource/pr/9".into(),
                    kind: "resource.observed".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("kind".into(), json!("vcs.pull-request")),
                        ("state".into(), json!("open")),
                        ("observer".into(), json!("observer/watch-0")),
                    ]),
                    evidence: vec![id],
                    expected_subject: None,
                    idempotency_key: Some(format!("history-{n}")),
                })
                .unwrap();
        }
        Step::Protect(k) => {
            let Some(id) = nth(*k) else { return };
            store
                .connection
                .write()
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h044f482464e7 owner=fractal_store::store::local_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR REPLACE INTO mission_definitions(mission_id, revision, state, claim_id)
                     VALUES (?1, 'r', 'active', ?2)",
                    params![format!("mission/protects-{n}"), id],
                )
                .unwrap();
        }
        Step::Repair(k, true) => {
            let Some(id) = nth(*k) else { return };
            store
                .append_claim(&ClaimInput {
                    subject: format!("repair/test-{n}"),
                    kind: "record.repaired".into(),
                    actor: None,
                    fields: BTreeMap::from([
                        ("record".into(), json!(format!("record/original-{n}"))),
                        ("replacement".into(), json!(id)),
                        ("reason".into(), json!("a test repair")),
                    ]),
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("history-{n}")),
                })
                .unwrap();
        }
        Step::Repair(k, false) => {
            let Some(id) = nth(*k) else { return };
            store
                .connection
                .write()
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h0ad6d7859fc0 owner=fractal_store::store::local_trim_tests TODO=chunk-or-reference-exact-wire-values */ "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash, position, raw,
                         state, claim_id, replacement_claim_id, updated_at_unix_ms)
                     VALUES (?1, 'node.elsewhere', ?2, 'repaired', 0, X'', 'repaired', ?3, ?4, '1')",
                    params![format!("record/repaired-{n}"), n as i64, format!("claim/original-{n}"), id],
                )
                .unwrap();
        }
        Step::PeerCite(k) => {
            let Some(id) = nth(*k) else { return };
            // A writer cites what it holds.
            store.seal_local_batches().unwrap();
            send(store, peer);
            peer.append_claim(&ClaimInput {
                subject: "resource/pr/8".into(),
                kind: "resource.observed".into(),
                actor: None,
                fields: BTreeMap::from([
                    ("kind".into(), json!("vcs.pull-request")),
                    ("state".into(), json!("open")),
                    ("observer".into(), json!("observer/watch-1")),
                ]),
                evidence: vec![id],
                expected_subject: None,
                idempotency_key: Some(format!("peer-{n}")),
            })
            .unwrap();
        }
        Step::CatchUp(partial) => {
            if *partial {
                store.set_trim_fault(Some(smallclaims::store::TrimFault::AfterCitationPages(1)));
            }
            let _ = store.catch_up_local_trim_citations();
            store.set_trim_fault(None);
        }
        Step::Retire(k, stale) => {
            let claims: Vec<ClaimRecord> = {
                let connection = store.readers.get();
                let which = if k % 2 == 1 {
                    "json_array_length(body, '$.evidence') > 0"
                } else {
                    "kind IN ('observer.observed', 'resource.observed')"
                };
                let sql = format!("SELECT {} FROM claims WHERE {which} ORDER BY store_index", smallclaims::store::CLAIM_COLUMNS);
                let mut statement = connection.prepare(/* sqlite-large-value: allow-for-now lv-hcc86b5e9b027 owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ &sql).unwrap();
                statement.query_map([], smallclaims::store::claim_from_row).unwrap().collect::<rusqlite::Result<_>>().unwrap()
            };
            if claims.is_empty() {
                return;
            }
            let claim = &claims[usize::from(*k / 2) % claims.len()];
            let rows: Vec<(String, String)> = store
                .readers
                .get()
                .prepare("SELECT cited, citer FROM local_trim_citations WHERE cited=?1 OR citer=?1")
                .unwrap()
                .query_map([&claim.id], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            // A claim a projection row still names is one a trim keeps: retire only what may go.
            transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h7ce7384320fa owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ "SAVEPOINT retire").unwrap();
            let retired = smallclaims::store::checkpoint::delete_dropped_rows_tx(
                &transaction,
                &[],
                &[smallclaims::store::checkpoint::ClaimTombstone {
                    id: claim.id.clone(),
                    operation_id: smallclaims::store::operation_parts(&claim.body).map(|(id, _)| id.to_owned()),
                    writer: String::new(),
                    sequence: 0,
                    envelope_hash: String::new(),
                    subject: claim.subject.clone(),
                    kind: claim.kind.clone(),
                    actor: claim.actor.clone(),
                    predecessors: claim.predecessors.clone(),
                    request_digest: smallclaims::store::operation_parts(&claim.body).map(|(_, digest)| digest.to_owned()),
                    accepted_at_unix_ms: claim.accepted_at_unix_ms,
                }],
            );
            if retired.is_err() {
                transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h147460d38c0c owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ "ROLLBACK TO retire; RELEASE retire").unwrap();
                return;
            }
            transaction.execute_batch(/* sqlite-large-value: allow-for-now lv-h3fec8e4cbeb7 owner=fractal_store::store::local_trim_tests TODO=trace-statement-and-value-representation */ "RELEASE retire").unwrap();
            if *stale {
                for (cited, citer) in rows {
                    transaction
                        .execute(/* sqlite-large-value: allow-for-now lv-h3458778f2d90 owner=fractal_store::store::local_trim_tests TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_trim_citations(cited, citer) VALUES (?1, ?2)", params![cited, citer])
                        .unwrap();
                }
            }
            transaction.commit().unwrap();
        }
        Step::Tombstone(newer) => {
            let sequence = if *newer { 1_000_000 + n as i64 } else { 0 };
            store
                .connection
                .write()
                .execute(
                    /* sqlite-large-value: allow-for-now lv-h23de9485734a owner=fractal_store::store::local_trim_tests TODO=stage-generation-and-keyed-reclamation */ "INSERT INTO checkpoint_envelopes(writer, sequence, envelope_hash, accepted_at_unix_ms, checkpoint)
                     VALUES (?1, ?2, ?3, 1, 'checkpoint/1970-01-02')",
                    params![store.origin, sequence, format!("tombstone-{n}")],
                )
                .unwrap();
        }
        Step::Seat { kind, seat, incarnation, state, old } => {
            let seat = SEATS[usize::from(*seat)];
            let incarnation = format!("{seat}-{incarnation}");
            let observed_at = now_ms() - if *old { 9 * smallclaims::store::checkpoint::DAY_MS } else { 0 };
            let (kind, fields) = match kind {
                0 => ("harness.observed", BTreeMap::from([
                    ("state".into(), json!(STATES[usize::from(*state)])),
                    ("driver".into(), json!("codex")),
                    ("incarnation_id".into(), json!(incarnation)),
                    ("status_transition".into(), json!(true)),
                ])),
                1 => ("runtime.observed", BTreeMap::from([
                    ("status".into(), json!("running")),
                    ("runtime_id".into(), json!(seat.trim_start_matches("agent/"))),
                    ("incarnation_id".into(), json!(incarnation)),
                ])),
                _ => ("harness.diagnostic", BTreeMap::from([
                    ("code".into(), json!("provider-auth-restored")),
                    ("incarnation_id".into(), json!(incarnation)),
                    ("driver".into(), json!("codex")),
                    ("reason".into(), json!("fixture")),
                    ("severity".into(), json!("warning")),
                ])),
            };
            let mut fields = fields;
            // Only a harness observation carries its own observation time; the others are dated
            // by acceptance, inside the window.
            if kind == "harness.observed" {
                fields.insert("observed_at_ms".into(), json!(observed_at as u64));
            }
            store
                .append_claim(&ClaimInput {
                    subject: seat.into(),
                    kind: kind.into(),
                    actor: Some(seat.into()),
                    fields,
                    evidence: Vec::new(),
                    expected_subject: None,
                    idempotency_key: Some(format!("seat-{n}")),
                })
                .unwrap();
        }
        Step::Twice(observer) => {
            let mut connection = store.connection.write();
            let transaction = connection.transaction().unwrap();
            let mut batch = None;
            for status in ["ok", "changed"] {
                let claim = append_claim_tx(
                    &transaction,
                    &store.origin,
                    &format!("observer/watch-{observer}"),
                    "observer.observed",
                    None,
                    &json!({"fields": {"status": status}}),
                    &[],
                    batch.as_deref(),
                )
                .unwrap();
                batch = Some(claim.batch_id);
            }
            transaction.commit().unwrap();
        }
        Step::Shared(observer) => {
            let operation = json!({"id": format!("operation/shared-{n}"), "request_digest": format!("digest-{n}")});
            for (subject, kind, fields) in [
                (format!("observer/watch-{observer}"), "observer.observed", json!({"status": "ok"})),
                (
                    format!("resource/pr/{observer}"),
                    "resource.observed",
                    json!({"kind": "vcs.pull-request", "state": "open", "observer": format!("observer/watch-{observer}")}),
                ),
            ] {
                // One transaction each: two batches, so two envelopes.
                let mut connection = store.connection.write();
                let transaction = connection.transaction().unwrap();
                append_claim_tx(
                    &transaction,
                    &store.origin,
                    &subject,
                    kind,
                    None,
                    &json!({"fields": fields, "_operation": operation}),
                    &[],
                    None,
                )
                .unwrap();
                transaction.commit().unwrap();
            }
        }
    }
}

/// Stage `store` at `cut` as a local cut does, fully or scoped, and plan it.
fn local_plan(store: &Store, cut: u128, dir: &std::path::Path, scoped: bool) -> (Vec<String>, Vec<String>, Vec<String>) {
    let kinds = smallclaims::store::LOCAL_TRIM_KINDS;
    let capture = smallclaims::store::local_trim_capture_kinds();
    let staged = store.checkpoint_staged_set_scoped(cut, None, dir, scoped.then_some(capture.as_slice())).unwrap();
    staged.protect_all_but(kinds).unwrap();
    staged.split_envelopes().unwrap();
    smallclaims::store::checkpoint_staging::without_reference_plan(|| store.plan_staged_checkpoint(&staged)).unwrap();
    let claims = staged.dropped_claims(0, usize::MAX >> 1).unwrap().into_iter().map(|claim| claim.id).collect();
    let envelopes = staged
        .dropped_envelopes(0, usize::MAX >> 1)
        .unwrap()
        .into_iter()
        .map(|envelope| format!("{}/{}/{}", envelope.writer, envelope.sequence, envelope.envelope_hash))
        .collect();
    (claims, envelopes, staged.guard_answers(kinds).unwrap())
}

#[test]
fn a_local_cut_s_scoped_capture_stages_every_kind_its_rules_read() {
    let capture = smallclaims::store::local_trim_capture_kinds();
    for kind in smallclaims::store::LOCAL_TRIM_KINDS {
        let missing = checkpoint_rules::kinds_read_for(kind)
            .into_iter()
            .filter(|read| !capture.contains(read))
            .collect::<Vec<_>>();
        assert!(missing.is_empty(), "judging {kind} reads {missing:?}, which a local cut does not stage");
    }
    // The status-history rule reads a seat's runtime resets and diagnostics.
    assert!(checkpoint_rules::kinds_read_for("harness.observed").contains("runtime.observed"));
}

/// A seat whose harness transitions are all old, with a provider prompt raised and restored
/// inside the history window between them: the status-history rule keeps the transition before
/// the prompt (its predecessor, and what the restored prompt exposes), which only the
/// diagnostics show and no harness rule keeps. A capture that left them out would drop it, and
/// the seat's status history would change.
#[test]
fn a_scoped_capture_keeps_the_transition_a_seat_s_restored_prompt_depends_on() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    let seat = SEATS[0];
    let old = (now_ms() - 9 * smallclaims::store::checkpoint::DAY_MS) as u64;
    let mut n = 0;
    let mut append = |kind: &str, fields: Value| -> String {
        n += 1;
        store
            .append_claim(&ClaimInput {
                subject: seat.into(),
                kind: kind.into(),
                actor: Some(seat.into()),
                fields: serde_json::from_value(fields).unwrap(),
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: Some(format!("prompt-{n}")),
            })
            .unwrap()
            .id
    };
    let harness = |state: &str, at: u64| json!({"state": state, "driver": "codex", "incarnation_id": "one", "observed_at_ms": at});
    let diagnostic = |code: &str| json!({"incarnation_id": "one", "code": code, "driver": "codex", "reason": "fixture", "severity": "warning"});
    append("harness.observed", harness("ready", old));
    append("harness.observed", harness("working", old + 1));
    let before_prompt = append("harness.observed", harness("idle", old + 2));
    append("harness.diagnostic", diagnostic("provider-auth-expired"));
    append("harness.diagnostic", diagnostic("provider-auth-restored"));
    // Later transitions the harness rule keeps instead: the newest that is not working, and the
    // working run the seat is in now.
    append("harness.observed", harness("ready", old + 3));
    append("harness.observed", harness("working", old + 4));
    store.seal_local_batches().unwrap();
    let cut = smallclaims::store::checkpoint::newest_due_cut(context.now_unix_ms);
    let full = local_plan(&store, cut, &scratch.path().join("full"), false);
    let scoped = local_plan(&store, cut, &scratch.path().join("scoped"), true);
    assert!(!full.0.contains(&before_prompt), "the whole capture keeps it");
    assert_eq!(scoped.0, full.0, "and so does the scoped one");
    assert_eq!(scoped.1, full.1);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// For random histories, a local cut's scoped capture plans the same drop as a full capture
    /// restricted to the kinds it drops, and every guard answers the same for each claim it
    /// could drop: newest of slot, writer's newest envelope, operations, envelopes, protections,
    /// repairs and citations.
    #[test]
    fn a_scoped_capture_plans_what_a_full_one_plans(
        steps in proptest::collection::vec(step_strategy(), 1..60)
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let context = context(scratch.path(), 0);
        let store = local_member("alder");
        let peer = Store::open_memory("birch").unwrap();
        for (n, step) in steps.iter().enumerate() {
            take_step(&store, &peer, step, n);
        }
        store.seal_local_batches().unwrap();
        // The peer's claims arrive last, after claims accepted later than theirs.
        send(&peer, &store);
        let cut = smallclaims::store::checkpoint::newest_due_cut(context.now_unix_ms);
        let full = local_plan(&store, cut, &scratch.path().join("full"), false);
        let scoped = local_plan(&store, cut, &scratch.path().join("scoped"), true);
        prop_assert_eq!(&scoped.0, &full.0, "dropped claims");
        prop_assert_eq!(&scoped.1, &full.1, "dropped envelopes");
        prop_assert_eq!(&scoped.2, &full.2, "guard answers");
        let check = store.verify_local_trim_citations().unwrap();
        prop_assert!(check.missing.is_empty() && check.extra.is_empty(), "{:?}", check);
    }
}

/// A claim two others cite keeps the citer that is still here after the other is retired: the
/// deleting transaction removes only the retired claim's own citation rows.
#[test]
fn a_retired_citer_leaves_what_the_others_cite() {
    let scratch = tempfile::tempdir().unwrap();
    let context = context(scratch.path(), 0);
    let store = local_member("alder");
    let peer = Store::open_memory("birch").unwrap();
    for n in 0..2 {
        take_step(&store, &peer, &Step::Write(Write::Observer { observer: 0, status: n, cursor: None }), usize::from(n));
    }
    // Both cite the older observation, the one the cut could drop.
    take_step(&store, &peer, &Step::Cite(0), 2);
    take_step(&store, &peer, &Step::Cite(0), 3);
    store.catch_up_local_trim_citations().unwrap();
    let citers: Vec<String> = store
        .readers
        .get()
        .prepare("SELECT id FROM claims WHERE json_array_length(body, '$.evidence') > 0 ORDER BY store_index")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(citers.len(), 2);
    take_step(&store, &peer, &Step::Retire(1, false), 4);
    assert!(store.claim_by_id(&citers[0]).unwrap().is_none(), "the first citer is retired");
    store.seal_local_batches().unwrap();
    let check = store.verify_local_trim_citations().unwrap();
    assert!(check.missing.is_empty() && check.extra.is_empty(), "{check:?}");
    let cut = smallclaims::store::checkpoint::newest_due_cut(context.now_unix_ms);
    let full = local_plan(&store, cut, &scratch.path().join("full"), false);
    let scoped = local_plan(&store, cut, &scratch.path().join("scoped"), true);
    assert_eq!(scoped, full);
    assert!(scoped.2.iter().any(|answer| answer.contains("cited=1")), "{:?}", scoped.2);
}
