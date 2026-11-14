//! Nested under huge_part_rebuild_bench to reuse its private fixture helpers unchanged.
//! Export once, then use offline copies for exact-input base/head digest comparisons.
use super::*;

#[test]
#[ignore = "exports a committed huge fixture to a new /scratch- snapshot file"]
fn export_huge_part_snapshot() {
    let supplied = std::env::var_os("FRACTAL_HUGE_PART_SNAPSHOT")
        .expect("set FRACTAL_HUGE_PART_SNAPSHOT to a new /scratch- database file");
    let path = std::path::PathBuf::from(supplied);
    assert!(path.to_string_lossy().contains("/scratch-"), "refusing non-scratch snapshot path");
    let parent = path.parent().expect("snapshot requires an existing scratch parent directory")
        .canonicalize().expect("snapshot parent must exist");
    let path = parent.join(path.file_name().expect("snapshot must name a file"));
    assert!(path.to_string_lossy().contains("/scratch-"), "refusing non-scratch canonical parent");
    // Reserve exclusively: never overwrite even an empty preexisting file or a symlink.
    let reservation = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .expect("snapshot output must not already exist");
    drop(reservation);

    let children = setting("FRACTAL_HUGE_PART_CHILDREN", 64);
    let steps_per_run = setting("FRACTAL_HUGE_PART_STEPS", 32);
    let extra_generations = setting("FRACTAL_HUGE_PART_GENERATIONS", 1);
    let history = setting("FRACTAL_HUGE_PART_HISTORY", 8);
    assert!(steps_per_run > 0, "FRACTAL_HUGE_PART_STEPS must be positive");
    let controller = Store::open_memory("controller").unwrap();
    let mut revisions = Vec::new();
    for generation in 0..=extra_generations {
        let mut source = format!(
            "version 2\nmission \"huge-part\" state=\"ready\" {{\n goal \"Keep revision {generation} live.\"\n concurrent-runs max=1000000\n"
        );
        for step in 0..steps_per_run {
            source.push_str(&format!(
                " step \"work-{step}\" {{ agentless; goal \"Revision {generation} work.\" }}\n"
            ));
        }
        source.push_str("}\n");
        revisions.push(publish_mission(&controller, &source, &format!("huge-definition-{generation}")));
    }
    let outer = controller.create_mission_run(&request(
        "huge-outer-root".into(), revisions[0].revision.clone(),
    )).unwrap();
    let mut runs: Vec<MissionRunView> = Vec::new();
    let mut subjects = BTreeSet::new();
    for index in 0..=children {
        let parent = if index == 0 { &outer } else { &runs[0] };
        let mut run = controller.create_child_mission_run(
            &request(format!("huge-part-run-{index}"), revisions[0].revision.clone()),
            parent, &parent.steps[index % steps_per_run].subject, None,
        ).unwrap();
        subjects.insert(run.subject.clone());
        for (generation, revision) in revisions.iter().enumerate() {
            if generation != 0 {
                run = controller.adopt_mission_revision(
                    &run.id, revision, "person/test", "Grow the live part's generation history",
                    &format!("huge-revise-{index}-{generation}"),
                ).unwrap();
            }
            subjects.insert(run.generation.clone());
            for step in &run.steps {
                subjects.insert(step.subject.clone());
                write_coalesced_rebuild_backlog(&controller, &step.subject, history);
            }
        }
        runs.push(run);
    }
    let target = &runs[0].steps[0].subject;
    let state = if history % 2 == 1 { "ready" } else { "blocked" };
    assert!(controller.set_step_state(target, state, Some("one out-of-order rebuild")).unwrap());
    let claims: usize = subjects.iter().map(|subject| controller.claims_for(subject, None).unwrap().len()).sum();
    if (children, steps_per_run, extra_generations, history) == (64, 32, 1, 8) {
        assert_eq!(claims, 35_621, "snapshot must match the measured huge fixture");
        assert_eq!(runs.len(), 65);
    }
    // Every fixture write above is committed. VACUUM INTO creates a standalone SQLite
    // image, not a main-file copy missing uncheckpointed WAL. No measured rebuild ran.
    let connection = controller.connection.write();
    assert!(connection.is_autocommit(), "VACUUM INTO must run outside a transaction");
    connection.execute(/* sqlite-large-value: allow-for-now lv-h69c8bdade4b1 owner=fractal_missions::store::tests::huge_part_snapshot TODO=trace-statement-and-value-representation */ "VACUUM INTO ?1", [path.to_str().expect("snapshot path must be UTF-8")]).unwrap();
    println!("huge_part_snapshot path={} subject={} outer_subject={} claims={claims} runs={} children={children} steps={steps_per_run} generations={extra_generations} history={history} backup_restore_writer=absent open_origin=real-part-bench measured_rebuild=false",
        path.display(), runs[0].subject, outer.subject, runs.len());
}
