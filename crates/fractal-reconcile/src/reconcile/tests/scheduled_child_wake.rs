//! A scheduled occurrence's ready step wakes its assigned seat like any other ready work (#4449).
//! The reconciler runs its own loop here, so a change that never wakes it fails the test instead
//! of a test-driven pass hiding it.
use super::*;

/// Headroom for a loaded build host, well under a full pass, so only the occurrence's own
/// changes and notifications can reach the wake.
const WOKEN_WITHIN: Duration = Duration::from_secs(15);

struct Looping {
    store: Arc<Store>,
    notify: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Looping {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The idle seat, settled by its first passes, with the reconciler then left to its own loop.
fn looping() -> Looping {
    assert!(WOKEN_WITHIN.as_millis() < crate::incremental::FULL_PASS_INTERVAL_MS);
    let store = Arc::new(Store::open_memory("node").unwrap());
    let SeatQueueFixture { store: _, reconciler } = SeatQueueFixture::with_store(store.clone());
    let reconciler = Arc::new(reconciler.skipping_unneeded(true));
    for _ in 0..3 {
        reconciler.reconcile_once().unwrap();
    }
    assert!(woken(&store).unwrap().is_empty());
    reconciler.set_max_passes_per_minute(600).unwrap();
    let notify = reconciler.notify.clone();
    let task = tokio::spawn(reconciler.run());
    Looping { store, notify, task }
}

/// Steps named by open wake messages. Fallible, so a read that meets a pass can be retried.
fn woken(store: &Store) -> anyhow::Result<Vec<String>> {
    Ok(store
        .messages(Some(SEAT), false)?
        .into_iter()
        .filter_map(|message| work_message_target(&message).map(|(step, ..)| step.to_owned()))
        .collect())
}

fn lantern(root: &tempfile::TempDir) -> String {
    let at = (Utc::now() - chrono::Duration::seconds(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    format!(
        r#"schedule "lantern" {{ host "node"
 at {at:?}; work {{ mission "queued"; workspace {:?} }}
}}"#,
        root.path().to_str().unwrap()
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scheduled_occurrence_wakes_its_idle_assignee() {
    let seat = looping();
    let root = tempfile::tempdir().unwrap();
    apply_source_beside_the_loop(
        &seat.store,
        &format!("version 2\n{}", lantern(&root)),
        "scheduled-wake",
    );
    seat.notify.notify_one();
    let run = Store::scheduled_mission_run_subject("schedule/lantern", 0);
    assert_woken(&seat.store, &run).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_owned_schedules_child_occurrence_wakes_its_idle_assignee() {
    let seat = looping();
    let root = tempfile::tempdir().unwrap();
    let source = format!(
        r#"version 2
mission "keeper" state="ready" {{
  goal "Keep the lantern schedule while it starts child runs."
  step "hold" {{ assigned-to "agent/node.helper" }}
  {}
}}"#,
        lantern(&root)
    );
    apply_source_beside_the_loop(&seat.store, &source, "scheduled-child-wake");
    let parent = beside_the_loop(|| {
        seat.store.create_mission_run(&crate::model::MissionRunRequest {
            mission: "keeper".into(),
            revision: None,
            workspace: "/tmp".into(),
            requester: Some("person/requester".into()),
            mode: Some("run".into()),
            inputs: BTreeMap::new(),
            idempotency_key: "scheduled-child-parent".into(),
        })
    });
    seat.notify.notify_one();
    let schedule = tokio::time::timeout(WOKEN_WITHIN, async {
        loop {
            if let Some(schedule) = beside_the_loop(|| {
                seat.store.desired_subjects_for_owner_run(&parent.subject)
            })
            .into_iter()
            .find(|desired| desired.kind == "schedule")
            {
                return schedule.subject;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the parent run never declared its schedule");
    let run = Store::scheduled_mission_run_subject(&schedule, 0);
    assert_woken(&seat.store, &run).await;
}

async fn assert_woken(store: &Store, run: &str) {
    let woken = tokio::time::timeout(WOKEN_WITHIN, async {
        loop {
            let woken = beside_the_loop(|| woken(store));
            if !woken.is_empty() {
                return woken;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let view = beside_the_loop(|| store.mission_run(run));
        panic!(
            "the scheduled step never woke its seat; run {:?}, work {:?}",
            view.map(|view| view
                .steps
                .into_iter()
                .map(|step| (step.subject, step.status))
                .collect::<Vec<_>>()),
            beside_the_loop(|| store.work_for_reconcile(SEAT))
                .into_iter()
                .map(|step| (step.subject, step.status))
                .collect::<Vec<_>>()
        )
    });
    let created = beside_the_loop(|| store.mission_run(run)).expect("the occurrence started its run");
    assert_eq!(woken, [SeatQueueFixture::step(&created, "work")]);
}
