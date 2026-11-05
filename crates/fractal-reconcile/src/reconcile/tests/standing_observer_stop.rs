//! A person stops a repository's standing GitHub observer, and a watch on another member then
//! declares it there, which polls on from the observer's cursor (#5182).
use super::*;
// The real parser: this module's tests alias `parse_intent` to one that admits any top-level node.
use serde_json::json;

const SEATS: &str = "version 2\nagent \"example.local\" { workspace \"/tmp\"; command \"true\" }\nagent \"example.away\" { host \"away\"; workspace \"/tmp\"; command \"true\" }\n";
const LOCAL: &str = "agent/example.local";
const AWAY: &str = "agent/example.away";
const STOP: &str = "version 2\nstop \"observer/github/acme/garden\"\n";

/// Publish `source` as `actor`, as `st apply` does.
fn publish(
    store: &Store,
    source: &str,
    key: &str,
    actor: &str,
) -> Result<crate::model::ApplyResponse, crate::model::St3Error> {
    let intent = crate::graph::parse_intent(source, "node")?;
    let preview = store.mission(
        &intent,
        crate::model::IntentInput { kdl: source.into(), source_name: None },
    )?;
    store.apply_as(&intent, &preview.subject_tokens, key, Some(actor))
}

fn thread(number: u64) -> crate::github_watch::ThreadRef {
    crate::github_watch::ThreadRef::parse(&format!("acme/garden#{number}")).unwrap()
}

#[test]
fn only_a_person_stops_a_repositorys_standing_observer() {
    let fixture = WatchFixture::new();
    fixture.watch("acme/garden#12", "agent/example.planner");
    assert!(fixture.running(&fixture.observer));

    let refused = publish(&fixture.store, STOP, "agent-stop", "agent/example.planner").unwrap_err();
    assert_eq!(refused.code, "observer-stop-person-only", "{refused:?}");
    assert!(fixture.running(&fixture.observer), "an agent's stop writes nothing");

    publish(&fixture.store, STOP, "person-stop", "person/operator").unwrap();
    assert!(!fixture.running(&fixture.observer));
    // It declares what a host writes when the observer falls unused, which every build reads.
    let declared = |store: &Store| {
        store.desired_subjects_named(std::slice::from_ref(&fixture.observer)).unwrap()[0].clone()
    };
    let host_stop = crate::graph::parse_internal_intent(
        &crate::github_watch::stop_source(&fixture.observer).unwrap(),
        "node",
    )
    .unwrap()
    .subjects[&fixture.observer]
        .clone();
    let stopped = declared(&fixture.store);
    assert_eq!((stopped.kind.as_str(), &stopped.desired), ("observer", &host_stop.desired));
    assert_eq!(stopped.owner_run, None);

    // Nothing else changed: a stop with a reason, another observer, a top-level observer and an
    // agent, exec or PTY stop keep the rules they had.
    let code = |source: &str| match crate::graph::parse_intent(source, "node") {
        Ok(intent) => panic!("{source:?} parsed: {:?}", intent.subjects),
        Err(error) => error.code,
    };
    assert_eq!(
        code("version 2\nstop \"observer/github/acme/garden\" { reason \"moving it\" }\n"),
        "observer-stop-reason"
    );
    assert_eq!(code("version 2\nstop \"observer/intake\"\n"), "invalid-stop-subject");
    assert_eq!(code("version 2\nstop \"observer/github/acme/garden/more\"\n"), "invalid-stop-subject");
    assert_eq!(
        code("version 2\nobserver \"github/acme/garden\" { stop }\n"),
        "runtime-outside-mission"
    );
    assert!(crate::graph::parse_intent("version 2\nstop \"agent/example.reviewer\"\n", "node").is_ok());
}

/// Answers one poll with `facts` and `cursor`, and keeps the cursor it was asked from.
struct OnePoll {
    asked: Arc<Mutex<Vec<Option<String>>>>,
    facts: Value,
    cursor: String,
}

impl ResourceProvider for OnePoll {
    fn observe(
        &self,
        request: ObservationRequest,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<crate::resource::ProviderObservation>> + Send + '_>>
    {
        self.asked.lock().unwrap().push(request.cursor);
        Box::pin(async move {
            Ok(crate::resource::ProviderObservation {
                facts: self.facts.clone(),
                cursor: Some(self.cursor.clone()),
                next_check_unix_ms: now_ms() + 300_000,
            })
        })
    }
}

/// One member's reconciler, as its daemon runs one: it keeps cursors in `plugin` and refills
/// an unfilled mirror.
fn member(
    store: &Arc<Store>,
    host: &str,
    plugin: &Arc<github_plugin_store::GithubPluginStore>,
    asked: &Arc<Mutex<Vec<Option<String>>>>,
    facts: Value,
    cursor: &str,
) -> Reconciler<FakeRuntime> {
    Reconciler::new(store.clone(), Arc::new(FakeRuntime::default()), host.into(), Arc::new(Notify::new()))
        .with_github_plugin(plugin.clone())
        .with_mirror_refill(true)
        .with_resource_provider(Arc::new(OnePoll {
            asked: asked.clone(),
            facts,
            cursor: cursor.into(),
        }))
}

/// Poll the standing observer now, and wait until the poll has kept `cursor`.
async fn poll(reconciler: &Reconciler<FakeRuntime>, plugin: &Arc<github_plugin_store::GithubPluginStore>, cursor: &str) {
    let observer = thread(12).observer();
    let revision = reconciler.store.selected_desired_revision(&observer).unwrap().unwrap();
    // Due now, rather than at the deadline the last poll recorded.
    reconciler.observer_deadlines.lock().unwrap().insert(format!("{observer}:{revision}"), 0);
    reconciler
        .reconcile_resource_observers(&reconciler.store.desired_subjects().unwrap(), &[])
        .unwrap();
    // A bound for a loaded machine, not an expected duration.
    tokio::time::timeout(Duration::from_secs(60), async {
        while plugin.cursor(&observer).unwrap() != Some(Some(cursor.to_owned())) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the poll keeps its cursor in the plugin database");
}

/// birch declares the standing observer and polls it; cedar's seat watches through it. birch
/// stops answering, so a person on cedar stops the observer: cedar's next pass declares it again
/// there, and cedar polls on from the position birch's last claim recorded. Each comment wakes
/// cedar's seat once: the one birch saw, and the one written after, and none of them twice.
#[tokio::test]
async fn a_person_moves_polling_to_the_member_whose_watch_declares_the_observer_again() {
    let birch = Arc::new(Store::open_memory("away").unwrap());
    let cedar = Arc::new(Store::open_memory("node").unwrap());
    let between = |from: &Store, to: &Store| {
        to.import_replication(from.origin(), &from.export_replication(0).unwrap()).unwrap();
    };
    apply_source(&cedar, SEATS, "seats");
    between(&cedar, &birch);
    // birch's seat watches first, so birch declares the observer; cedar's seat watches through it.
    birch.declare_watch(&thread(13), AWAY, None).unwrap();
    between(&birch, &cedar);
    cedar.declare_watch(&thread(12), LOCAL, None).unwrap();
    between(&cedar, &birch);
    let observer = thread(12).observer();
    assert_eq!(cedar.selected_desired_origin(&observer).unwrap().as_deref(), Some("away"));

    let comment = |id: u64, seconds_ago: i64| {
        json!({"kind": "comment", "id": id, "author": "fern", "at": just_now(seconds_ago)})
    };
    let listing = |comments: Vec<Value>| {
        json!({"repository_id": 7, "pull_requests": [
            watched_pull(12, json!({"new": false, "recent_comments": comments})),
            watched_pull(13, json!({"new": false})),
        ]})
    };
    // A repository cursor as the provider writes one, and the read position a claim carries.
    let cursor = |day: u32| {
        json!({"items_since": format!("2026-09-{day}T00:00:00Z"), "seen": {"items": format!("listing-{day}")}})
            .to_string()
    };
    let position = |day: u32| json!({"items_since": format!("2026-09-{day}T00:00:00Z")}).to_string();
    let wakes = |store: &Store| {
        store
            .messages(Some(LOCAL), true)
            .unwrap()
            .into_iter()
            .filter(|message| message.tags.iter().any(|tag| tag == "github-watch"))
            .map(|message| message.title.unwrap_or_default())
            .collect::<Vec<_>>()
    };

    // birch polls: a baseline, then one new comment, which wakes cedar's seat.
    let asked = Arc::new(Mutex::new(Vec::new()));
    let birch_plugin = Arc::new(github_plugin_store::GithubPluginStore::memory());
    let first = listing(vec![comment(90, 3_600)]);
    poll(&member(&birch, "away", &birch_plugin, &asked, first, &cursor(11)), &birch_plugin, &cursor(11)).await;
    let second = listing(vec![comment(91, 60), comment(90, 3_600)]);
    poll(&member(&birch, "away", &birch_plugin, &asked, second, &cursor(12)), &birch_plugin, &cursor(12)).await;
    between(&birch, &cedar);
    assert_eq!(wakes(&cedar), ["@fern commented on acme/garden#12"]);

    // While birch declares it, cedar does not poll the observer.
    let cedar_plugin = Arc::new(github_plugin_store::GithubPluginStore::memory());
    let third = listing(vec![comment(92, 10), comment(91, 60), comment(90, 3_600)]);
    let cedar_member = member(&cedar, "node", &cedar_plugin, &asked, third, &cursor(13));
    let before = asked.lock().unwrap().len();
    cedar_member
        .reconcile_resource_observers(&cedar.desired_subjects().unwrap(), &[])
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(asked.lock().unwrap().len(), before, "only the member that declares it polls");

    // The person's stop; cedar's next pass declares the observer again, on cedar, with the same
    // resource.
    let resource = |store: &Store| {
        crate::graph::observer_spec(&store.desired_subjects_named(std::slice::from_ref(&observer)).unwrap()[0].desired)
            .unwrap()
    };
    let read_into = resource(&cedar).resource;
    publish(&cedar, STOP, "move-polling", "person/operator").unwrap();
    cedar_member.reconcile_github_watches(&cedar.desired_subjects().unwrap()).unwrap();
    let declared = resource(&cedar);
    assert!(!declared.stopped, "a live watch here declares the observer again");
    assert_eq!(declared.resource, read_into);
    assert_eq!(cedar.selected_desired_origin(&observer).unwrap().as_deref(), Some("node"));

    // cedar polls on from the position birch's last claim recorded, and wakes its seat for the
    // comment written since, once.
    poll(&cedar_member, &cedar_plugin, &cursor(13)).await;
    assert_eq!(asked.lock().unwrap()[before], Some(position(12)), "{:?}", asked.lock().unwrap());
    assert_eq!(
        wakes(&cedar),
        ["@fern commented on acme/garden#12", "@fern commented on acme/garden#12"]
    );
    // The same listing again wakes nobody.
    let again = listing(vec![comment(92, 10), comment(91, 60), comment(90, 3_600)]);
    poll(&member(&cedar, "node", &cedar_plugin, &asked, again, &cursor(14)), &cedar_plugin, &cursor(14)).await;
    assert_eq!(wakes(&cedar).len(), 2);
}
