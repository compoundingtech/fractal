use super::*;

const NOW: u64 = 1_799_452_800_000;
const ACTOR: &str = "agent/sample/usage-producer";
const PLAN: &str = "anthropic/pro";

fn time(at: u64) -> String {
    chrono::DateTime::from_timestamp_millis(at as i64).unwrap().to_rfc3339()
}
fn credit(id: &str, state: &str, expiry: u64) -> Value {
    json!({"provider":"anthropic","account":"digest","account_ref":"account/one","provider_plan":PLAN,
        "credit_id":id,"state":state,"expires_at":time(expiry),"measured_at":time(NOW),
        "coverage":"reported","source":"omp-usage","host":"node",
        "disposition":{"kind":"unknown","at":null,"evidence_ref":null}})
}
fn quota(measured: u64, percent: f64) -> Value {
    json!({"provider":"anthropic","account":"digest","account_ref":"account/one","provider_plan":PLAN,
        "window":"weekly","used_percent":percent,"resets_at":null,"measured_at":time(measured),
        "coverage":"reported","source":"omp-usage","host":"node"})
}
fn period(id: &str, unused: f64) -> Value {
    json!({"provider":"anthropic","account":"digest","provider_plan":PLAN,"period_id":id,"window":"weekly",
        "closed_at":time(NOW-2*DAY),"measured_at":time(NOW),"unused_fraction":unused,
        "close_kind":"natural_rollover","coverage":"reported","source":"omp-usage","host":"node",
        "last_sample_at":time(NOW-2*DAY-1),"max_gap_seconds":1.0})
}
fn delta(id: &str, sequence: u64, previous: Option<&str>, through: u64, credits: Vec<Value>) -> Delta {
    let frontier = json!({"producer_incarnation":"incarnation","sequence":sequence,"previous_claim":previous,
        "host":"node","accounts":[{"provider":"anthropic","account":"digest","provider_plan":PLAN}],
        "observed_at":time(through),"covered_from":time(NOW-60*DAY),"covered_through":time(through),"complete":true});
    let fields = json!({"readings":[],"credit_readings":credits,"period_readings":[],"source_frontier":frontier});
    let claim = serde_json::from_value(json!({"id":id,"store_index":sequence+1,"batch_id":"batch",
        "subject":ACTOR,"kind":"usage.limits.observed","origin":"node","actor":ACTOR,
        "body":{"fields":fields},"predecessors":[],"accepted_at_unix_ms":through})).unwrap();
    Delta { claim, frontier: Frontier::deserialize(frontier).unwrap() }
}
fn writer(credits: Vec<Value>) -> Writer {
    let mut writer = Writer::default();
    writer.ingest(delta("enrollment",0,None,NOW-60*DAY,vec![]));
    writer.ingest(delta("first",1,Some("enrollment"),NOW,credits));
    writer
}
fn rendered(writer: &Writer) -> Vec<Value> { render(&[(ACTOR,"incarnation",writer)],NOW,DAY,true) }
fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value { rows.iter().find(|row|row["id"]==id).unwrap() }
fn cam<'a>(rows: &'a [Value], suffix: &str) -> &'a Value { row(rows,&format!("spend/{suffix}/anthropic")) }

#[test]
fn r01_uses_thirty_days_and_confirmed_forfeiture_not_sample_time() {
    let source=writer(vec![credit("old-seven-day-regression","forfeited",NOW-14*DAY),
        credit("inclusive-start","forfeited",NOW-30*DAY),credit("outside","forfeited",NOW-30*DAY-1),
        credit("future","forfeited",NOW+1)]);
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r01")["evidence"]["current_loss_lower"],2.0);
    assert_eq!(cam(&rows,"cam-r01")["window"],"30d");
    assert_eq!(cam(&rows,"cam-r01")["status"],"met");
}

#[test]
fn generous_credit_budget_exhausts_only_after_five_confirmed_losses() {
    let source=writer((0..6).map(|id|credit(&format!("credit-{id}"),"forfeited",NOW-2*DAY)).collect());
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r01")["status"],"exhausted");
    assert_eq!(row(&rows,"spend/forfeited/anthropic/pro")["status"],"exhausted");
}

#[test]
fn r02_retains_early_timely_decision_despite_newer_disposition_revision() {
    let expiry=NOW-10*DAY;
    let mut early=credit("one","available",expiry);
    early["measured_at"]=json!(time(expiry-2*DAY));
    early["disposition"]=json!({"kind":"planned","at":time(expiry-2*DAY),"evidence_ref":"document/decision"});
    let source=writer(vec![early,credit("one","redeemed",expiry)]);
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r02")["evidence"]["bad"],0.0);
    assert_eq!(cam(&rows,"cam-r02")["evidence"]["total"],1.0);
    assert_eq!(cam(&rows,"cam-r02")["window"],"30d");
}

#[test]
fn r02_uses_entry_cohort_and_inclusive_expiry_deadline_not_next_day_snapshot() {
    let expiry=NOW-2*DAY;
    let mut timely=credit("timely","redeemed",expiry);
    timely["disposition"]=json!({"kind":"redeemed","at":time(expiry),"evidence_ref":"document/decision"});
    let mut late=credit("late","redeemed",expiry);
    late["disposition"]=json!({"kind":"planned","at":time(expiry+1),"evidence_ref":"document/late"});
    let source=writer(vec![timely,late,credit("outside-cohort","redeemed",NOW-30*DAY)]);
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r02")["evidence"]["total"],2.0);
    assert_eq!(cam(&rows,"cam-r02")["evidence"]["bad"],1.0);
    assert_eq!(cam(&rows,"cam-r02")["status"],"exhausted");
}

#[test]
fn not_yet_due_undocumented_credit_is_pending_not_failure_or_recovery() {
    let source=writer(vec![credit("pending","available",NOW+DAY/2)]);
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r02")["pending"],1);
    assert_eq!(cam(&rows,"cam-r02")["evidence"]["bad"],0.0);
    assert_eq!(cam(&rows,"cam-r02")["status"],"no-data");
    assert!(cam(&rows,"cam-r02")["remaining_percent"].is_null());
}

#[test]
fn expired_available_is_not_confirmed_loss_and_does_not_erase_a_disposition() {
    let mut reading=credit("one","available",NOW-DAY);
    reading["disposition"]=json!({"kind":"declined","at":time(NOW-2*DAY),"evidence_ref":"document/declined"});
    let source=writer(vec![reading]);
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r01")["evidence"]["current_loss_lower"],0.0);
    assert_eq!(cam(&rows,"cam-r01")["status"],"met");
    assert_eq!(cam(&rows,"cam-r02")["evidence"]["bad"],0.0);
}

#[test]
fn all_configured_producers_and_full_cohort_history_are_required() {
    let mut source=writer(vec![credit("one","redeemed",NOW-DAY)]);
    source.from=NOW-30*DAY;
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r01")["status"],"no-data");
    assert_eq!(cam(&rows,"cam-r02")["status"],"no-data");
    for row in render(&[(ACTOR,"incarnation",&source)],NOW,DAY,false) {
        if row["id"].as_str().unwrap().contains("cam-") { assert_eq!(row["status"],"no-data"); }
    }
}

#[test]
fn missing_predecessor_stays_pending_until_actual_claim_closes_chain() {
    let mut source=Writer::default();
    source.ingest(delta("second",1,Some("first"),NOW,vec![credit("one","redeemed",NOW-DAY)]));
    // An unauthenticated pending successor cannot declare its provider population.
    assert_eq!(row(&rendered(&source),"spend/cam-r01/unknown")["status"],"no-data");
    assert_eq!(source.next,0);
    source.ingest(delta("first",0,None,NOW-60*DAY,vec![]));
    assert_eq!(source.next,2);
    assert_eq!(cam(&rendered(&source),"cam-r01")["status"],"met");
}

#[test]
fn forged_host_wrong_predecessor_changed_roster_and_future_acceptance_fail_closed() {
    for case in 0..4 {
        let mut source=writer(vec![]);
        let mut next=delta("second",2,Some("first"),NOW,vec![]);
        match case {
            0=>next.claim.origin="foreign".into(),
            1=>next.frontier.previous_claim=Some("invented".into()),
            2=>next.frontier.accounts[0].account="different".into(),
            _=>next.claim.accepted_at_unix_ms=u128::from(NOW-1),
        }
        source.ingest(next);
        assert!(source.invalid);
        assert_eq!(cam(&rendered(&source),"cam-r01")["status"],"no-data");
    }
}

#[test]
fn incomplete_chunk_never_advances_certified_source_frontier() {
    let mut source=Writer::default();
    let mut first=delta("first",0,None,NOW,vec![]);
    first.frontier.complete=false;
    source.ingest(first);
    assert_eq!(source.through,0);
    assert_eq!(cam(&rendered(&source),"cam-r01")["status"],"no-data");
    source.ingest(delta("second",1,Some("first"),NOW,vec![]));
    assert_eq!(source.through,NOW);
}

#[test]
fn cross_producer_common_cut_excludes_newer_facts_until_every_source_reaches_them() {
    let mut fast=Writer::default();
    fast.ingest(delta("first",0,None,NOW-10,vec![]));
    fast.ingest(delta("second",1,Some("first"),NOW,vec![credit("loss","forfeited",NOW-1)]));
    let mut slow=Writer::default();
    slow.ingest(delta("other",0,None,NOW-10,vec![]));
    let rows=render(&[(ACTOR,"a",&fast),("agent/sample/other","b",&slow)],NOW,DAY,true);
    assert_eq!(cam(&rows,"cam-r01")["evidence"]["current_loss_lower"],0.0);
    assert_eq!(cam(&rows,"cam-r01")["source"]["covered_through_unix_ms"],NOW-10);
}

#[test]
fn terminal_state_precedes_clock_and_accounts_remain_distinct_across_hosts() {
    let mut redeemed=credit("one","redeemed",NOW-DAY);
    redeemed["measured_at"]=json!(time(NOW-1));
    let mut other=credit("one","forfeited",NOW-DAY);
    other["account"]=json!("other-digest");
    let source=writer(vec![redeemed,credit("one","forfeited",NOW-DAY),other]);
    let rows=rendered(&source);
    assert_eq!(cam(&rows,"cam-r01")["evidence"]["current_loss_lower"],1.0);
}

#[test]
fn stale_source_cannot_be_renewed_by_read_clock_and_gap_does_not_recover() {
    let source=writer(vec![]);
    let rows=render(&[(ACTOR,"a",&source)],NOW+DAY+1,DAY,true);
    assert_eq!(cam(&rows,"cam-r01")["status"],"no-data");
    assert_eq!(source.observed,NOW);
}

#[test]
fn natural_period_bounds_never_invent_exact_terminal_loss_or_count_early_resets() {
    let mut source=writer(vec![]);
    source.periods=(0..6).map(|id|Fact{claim_id:format!("period-{id}"),value:period(&format!("period-{id}"),1.0)}).collect();
    let rows=rendered(&source);
    let loss=row(&rows,"spend/forfeited/anthropic/pro");
    assert_eq!(loss["evidence"]["current_loss_lower"],0.0);
    assert_eq!(loss["evidence"]["current_loss_upper"],6.0);
    assert_eq!(loss["status"],"no-data");
    source.periods[0].value["close_kind"]=json!("early_reset");
    let rows=rendered(&source);
    assert_eq!(row(&rows,"spend/forfeited/anthropic/pro")["status"],"met");
}

#[test]
fn missing_plan_cannot_be_silently_replaced_by_provider_aggregate() {
    let mut source=writer(vec![]);
    source.roster[0].provider_plan=None;
    let rows=rendered(&source);
    assert_eq!(row(&rows,"spend/forfeited/anthropic/unknown-plan")["status"],"no-data");
    assert!(!rows.iter().any(|row|row["id"]=="spend/forfeited/anthropic"));
}

#[test]
fn drained_account_is_fresh_coverage_but_quota_alone_never_proves_pool_routing() {
    let mut source=writer(vec![]);
    source.quotas.push(Fact{claim_id:"quota".into(),value:quota(NOW-1,100.0)});
    let rows=rendered(&source);
    let coverage=row(&rows,"spend/coverage/digest");
    assert_eq!(coverage["evidence"]["fast"]["short"]["bad"],299_999.0);
    assert_eq!(row(&rows,"spend/pool-ready/anthropic")["status"],"not-measured");
    assert_eq!(row(&rows,"spend/quota-blocked")["status"],"not-measured");
    assert_eq!(row(&rows,"spend/billed-overage")["status"],"not-measured");
}

#[test]
fn bounded_native_budget_uses_real_paired_windows_not_horizon_fraction() {
    let samples=[(NOW-100,true),(NOW-DAY,false)];
    let row=ratio_row("spend/example","test",NOW,&samples,0.75,true);
    assert_eq!(row["evidence"]["bad"],1.0);
    assert_eq!(row["evidence"]["fast"]["short"]["total"],1.0);
}

#[test]
fn native_store_revocation_restart_and_no_frontier_remain_honest() {
    let directory=tempfile::tempdir().unwrap();
    let path=directory.path().join("spend.db");
    let store=Store::open(&path,"node").unwrap();
    store.set_usage_limits_writers(vec![ACTOR.into()]).unwrap();
    let now=chrono::Utc::now().timestamp_millis() as u64;
    let mut evidence=delta("unused",0,None,NOW,vec![]).claim.body["fields"].clone();
    evidence["source_frontier"]["observed_at"]=json!(time(now));
    evidence["source_frontier"]["covered_through"]=json!(time(now));
    evidence["source_frontier"]["covered_from"]=json!(time(now-31*DAY));
    store.append_claim(&crate::model::ClaimInput{subject:ACTOR.into(),kind:"usage.limits.observed".into(),actor:Some(ACTOR.into()),
        fields:serde_json::from_value(evidence).unwrap(),evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap();
    assert_eq!(cam(&rows(&store,now,DAY).unwrap(),"cam-r01")["status"],"no-data");
    store.set_usage_limits_writers(vec![]).unwrap();
    let scope=smallclaims::sqlite::work::SqliteWorkScope::start();
    let disabled=rows(&store,now,DAY).unwrap();
    assert_eq!(scope.finish().statements,0);
    assert_eq!(row(&disabled,"spend/cam-r01/unknown")["status"],"no-data");
    drop(store);
    let store=Store::open(&path,"node").unwrap();
    store.set_usage_limits_writers(vec![ACTOR.into()]).unwrap();
    assert_eq!(cam(&rows(&store,now,DAY).unwrap(),"cam-r01")["status"],"no-data");
}

#[test]
fn source_enrollment_cannot_be_backdated_by_a_frontier_declaration() {
    let mut source=Writer::default();
    source.ingest(delta("fresh-journal",0,None,NOW,vec![]));
    assert_eq!(source.from,NOW);
    assert_eq!(cam(&rendered(&source),"cam-r01")["status"],"no-data");
    assert_eq!(cam(&rendered(&source),"cam-r02")["status"],"no-data");
}

#[test]
fn a_late_source_fork_is_unknown_even_after_the_original_position_was_consumed() {
    let mut source=writer(vec![]);
    source.ingest(delta("fork",1,Some("enrollment"),NOW,vec![]));
    assert!(source.invalid);
    assert_eq!(cam(&rendered(&source),"cam-r01")["status"],"no-data");
}

#[test]
fn pending_credit_cannot_hide_a_known_overdue_failure_or_its_short_burn() {
    let source=writer(vec![credit("overdue","redeemed",NOW-100),credit("pending","available",NOW+100)]);
    let rows=rendered(&source);
    let row=cam(&rows,"cam-r02");
    assert_eq!(row["pending"],1);
    assert_eq!(row["status"],"exhausted");
    assert_eq!(row["evidence"]["fast"]["short"]["bad"],1.0);
}

#[test]
fn loss_gauge_keeps_bad_time_after_the_rolling_loss_count_recovers() {
    let losses:Vec<_>=(0..6).map(|_|LossEvent{at:NOW-31*DAY,lower:1.0,upper:1.0}).collect();
    let row=loss_gauge("spend/loss","test",NOW,NOW-60*DAY,&losses,true);
    assert_eq!(row["evidence"]["current_loss_lower"],0.0);
    assert_eq!(row["evidence"]["bad"],(29*DAY) as f64);
    assert_eq!(row["status"],"exhausted");
}

#[test]
fn uncertain_loss_intervals_do_not_wash_confirmed_bad_time() {
    let mut losses:Vec<_>=(0..6).map(|_|LossEvent{at:NOW-31*DAY,lower:1.0,upper:1.0}).collect();
    losses.extend((0..6).map(|_|LossEvent{at:NOW-DAY,lower:0.0,upper:1.0}));
    let row=loss_gauge("spend/loss","test",NOW,NOW-60*DAY,&losses,true);
    assert_eq!(row["status"],"exhausted");
    assert_eq!(row["evidence"]["bad"],(29*DAY) as f64);
}

fn append_delta(store:&Store,actor:&str,mut delta:Delta)->ClaimRecord {
    delta.claim.body["fields"]["source_frontier"]["previous_claim"]=json!(delta.frontier.previous_claim);
    store.append_claim(&crate::model::ClaimInput{subject:actor.into(),kind:"usage.limits.observed".into(),
        actor:Some(actor.into()),fields:serde_json::from_value(delta.claim.body["fields"].clone()).unwrap(),
        evidence:vec![],expected_subject:None,idempotency_key:None}).unwrap()
}

#[test]
fn native_accepted_clock_advances_a_real_source_chain_and_quiet_reads_keep_bounded_work() {
    let store=Store::open_memory("node").unwrap();
    store.set_usage_limits_writers(vec![ACTOR.into()]).unwrap();
    store.set_write_clock_at(u128::from(NOW-60*DAY)).unwrap();
    let enrollment=append_delta(&store,ACTOR,delta("unused",0,None,NOW-60*DAY,vec![]));
    assert_eq!(cam(&rows(&store,NOW-60*DAY,DAY).unwrap(),"cam-r01")["status"],"no-data");
    store.set_write_clock_at(u128::from(NOW)).unwrap();
    let credits=(0..6).map(|id|credit(&format!("credit-{id}"),"forfeited",NOW-2*DAY)).collect();
    let successor=append_delta(&store,ACTOR,delta("unused",1,Some(&enrollment.id),NOW,credits));
    let native=rows(&store,NOW,DAY).unwrap();
    assert_eq!(cam(&native,"cam-r01")["status"],"exhausted");
    assert_eq!(cam(&native,"cam-r01")["evidence"]["current_loss_lower"],6.0);
    let scope=smallclaims::sqlite::work::SqliteWorkScope::start();
    assert_eq!(rows(&store,NOW,DAY).unwrap(),native);
    let first=scope.finish();
    let scope=smallclaims::sqlite::work::SqliteWorkScope::start();
    assert_eq!(rows(&store,NOW+1,DAY).unwrap(),native);
    let second=scope.finish();
    assert_eq!(first.statements,second.statements);
    assert!(first.statements<=8,"quiet indexed metadata/cursor work must stay bounded: {first:?}");
    store.connection.write().execute(
        /* sqlite-large-value: allow-for-now lv-hcab44bc5e3c6 owner=fractal_api_core::credit_objectives_tests TODO=prepare-body-and-predecessor-values */
        "UPDATE claims SET body=json_set(body,'$.fields.source_frontier.host','foreign') WHERE id=?1",
        [&successor.id],
    ).unwrap();
    assert_eq!(cam(&rows(&store,NOW,DAY).unwrap(),"cam-r01")["status"],"no-data",
        "rewritten source invalidates a previously rendered good-time/burn cache");
}

#[test]
fn unsupported_account_gap_is_visible_without_fabricated_digest_or_supported_success() {
    let gap=json!({"provider":"unsupported-example","account_ref":"account/example","provider_plan":"unsupported-example/basic",
        "source":"omp-usage","host":"node","reason":"canonical identity unavailable","measured_at":time(NOW)});
    let mut rows=rendered(&writer(vec![]));
    let gaps=BTreeMap::from([((ACTOR.into(),"unsupported-example".into(),"account/example".into()),
        Fact{claim_id:"gap-claim".into(),value:gap})]);
    append_gap_rows(&mut rows,&gaps,true);
    assert_eq!(row(&rows,"spend/coverage/unresolved/unsupported-example/account/example")["status"],"no-data");
    assert_eq!(row(&rows,"spend/forfeited/unsupported-example/basic")["status"],"no-data");
    assert_eq!(cam(&rows,"cam-r01")["status"],"met");
}

#[test]
fn drained_window_does_not_mask_a_stale_unreported_or_missing_second_window() {
    let mut missing=quota(NOW-1,20.0);
    missing["used_percent"]=Value::Null;
    let mut unreported=quota(NOW-1,20.0);
    unreported["coverage"]=json!("unreported");
    for mut second in [quota(NOW-3_600_001,20.0),unreported,missing] {
        second["window"]=json!("five_hour");
        let mut source=writer(vec![]);
        source.quotas=vec![Fact{claim_id:"drained".into(),value:quota(NOW-1,100.0)},
            Fact{claim_id:"second".into(),value:second}];
        let rows=rendered(&source);
        assert_eq!(row(&rows,"spend/coverage/digest")["evidence"]["fast"]["short"]["bad"],300_000.0,
            "a drained but fresh window cannot certify another window's evidence");
    }
}

#[test]
fn retention_uses_the_common_cut_even_for_a_small_fresh_producer_lag() {
    let cut=NOW-60_000;
    let expiry=cut-2*HORIZON+30_000;
    let mut credits:Vec<_>=(0..6).map(|id| {
        let mut reading=credit(&format!("boundary-{id}"),"forfeited",expiry);
        reading["measured_at"]=json!(time(cut));
        reading
    }).collect();
    credits.push(credit("obsolete","forfeited",NOW-90*DAY));
    let mut fast=writer(credits);
    let mut closure=period("boundary-period",1.0);
    closure["closed_at"]=json!(time(expiry));
    closure["last_sample_at"]=json!(time(expiry-1));
    closure["measured_at"]=json!(time(cut));
    fast.periods.push(Fact{claim_id:"period".into(),value:closure});
    let mut slow=writer(vec![]);
    slow.observed=cut;
    slow.through=cut;
    let lagging="agent/sample/lagging";
    let mut mirror=Mirror {
        writers:BTreeSet::from([ACTOR.into(),lagging.into(),"agent/sample/unenrolled".into()]),
        active:BTreeMap::from([(ACTOR.into(),(fast.enrolled_at,"incarnation".into())),
            (lagging.into(),(slow.enrolled_at,"incarnation".into()))]),
        sources:BTreeMap::from([((ACTOR.into(),"incarnation".into()),fast),
            ((lagging.into(),"incarnation".into()),slow)]),
        ..Mirror::default()
    };
    mirror.prune_at_common_cut(NOW);
    assert_eq!(mirror.sources[&(ACTOR.into(),"incarnation".into())].credits.len(),7,
        "an expected writer without a frontier cannot establish any retention floor");
    mirror.writers.remove("agent/sample/unenrolled");
    mirror.prune_at_common_cut(NOW);
    let fast=&mirror.sources[&(ACTOR.into(),"incarnation".into())];
    let slow=&mirror.sources[&(lagging.into(),"incarnation".into())];
    assert_eq!(fast.credits.len(),6);
    assert_eq!(fast.periods.len(),1);
    let rows=render(&[(ACTOR,"incarnation",fast),(lagging,"incarnation",slow)],NOW,DAY,true);
    assert_eq!(cam(&rows,"cam-r01")["evidence"]["bad"],30_000.0,
        "near-boundary losses still contribute historical bad time at the shared cut");
}

#[test]
fn restart_replay_preserves_real_native_binding_children_including_renamed_subtrees() {
    use crate::declared_objectives as bindings;
    use crate::objective_faults::{ObjectiveFaults,episode};
    const LATE:&str="agent/sample/late-usage-producer";
    const CHILDREN:[&str;3]=["spend/coverage/late-digest","example/spend/coverage/late-digest","example/coverage/late-digest"];
    let directory=tempfile::tempdir().unwrap();
    let path=directory.path().join("replay.db");
    let store=Store::open(&path,"node").unwrap();
    store.set_usage_limits_writers(vec![ACTOR.into(),LATE.into()]).unwrap();
    store.set_write_clock_at(u128::from(NOW-60*DAY)).unwrap();
    let mut first=append_delta(&store,ACTOR,delta("unused",0,None,NOW-60*DAY,vec![]));
    for sequence in 1..=PAGE as u64 {
        first=append_delta(&store,ACTOR,delta("unused",sequence,Some(&first.id),NOW-60*DAY,vec![]));
    }
    let late_delta=|sequence,previous,through| {
        let mut delta=delta("unused",sequence,previous,through,vec![]);
        delta.claim.body["fields"]["source_frontier"]["accounts"][0]["account"]=json!("late-digest");
        delta
    };
    let enrollment=append_delta(&store,LATE,late_delta(0,None,NOW-60*DAY));
    store.set_write_clock_at(u128::from(NOW)).unwrap();
    append_delta(&store,ACTOR,delta("unused",PAGE as u64+1,Some(&first.id),NOW,vec![]));
    let mut latest=late_delta(1,Some(&enrollment.id),NOW);
    let mut quota=quota(NOW,100.0);
    quota["account"]=json!("late-digest");
    latest.claim.body["fields"]["readings"]=json!([quota]);
    append_delta(&store,LATE,latest);
    let owner=r#"version 2
agent "node.ops" { command "true"; handles-faults }
objective "spend" {
  about "Unrenamed spend"
  indicator "native" { objective "spend"; }
  on-breach "fault" owner="agent/node.ops" condition="spend-budget-missed"
}
objective "example/spend" {
  about "Renamed spend family"
  indicator "native" { objective "spend"; }
  on-breach "fault" owner="agent/node.ops" condition="spend-budget-missed"
}
objective "example/coverage" {
  about "Renamed spend subtree"
  indicator "native" { objective "spend/coverage"; }
  on-breach "fault" owner="agent/node.ops" condition="spend-budget-missed"
}
"#;
    let intent=crate::graph::parse_intent(owner,"node").unwrap();
    let preview=store.mission(&intent,crate::model::IntentInput{kdl:owner.into(),source_name:None}).unwrap();
    store.apply(&intent,&preview.subject_tokens,"spend-replay-owner").unwrap();
    rows(&store,NOW,DAY).unwrap();
    let native=rows(&store,NOW,DAY).unwrap();
    let closed=bindings::rows(&store,NOW,&native).unwrap();
    for child in CHILDREN { assert_eq!(row(&closed,child)["status"],"exhausted"); }
    // Both account coverage children burn under each of the three real declarations.
    assert_eq!(bindings::check(&store,&closed).unwrap().opened,6);
    drop(store);
    let store=Store::open(&path,"node").unwrap();
    store.set_usage_limits_writers(vec![ACTOR.into(),LATE.into()]).unwrap();
    let native=rows(&store,NOW,DAY).unwrap();
    assert!(native.iter().any(|row|row["id"]=="spend/coverage/digest"));
    assert!(!native.iter().any(|row|row["id"]=="spend/coverage/late-digest"));
    let replaying=bindings::rows(&store,NOW,&native).unwrap();
    for child in CHILDREN {
        assert_eq!(row(&replaying,child)["status"],"no-data");
        assert!(row(&replaying,child)["remaining_percent"].is_null());
    }
    assert_eq!(bindings::check(&store,&replaying).unwrap(),ObjectiveFaults::default());
    assert!(store.claims_for("daemon/node",Some("operational.recovered")).unwrap().is_empty());
    for child in CHILDREN {
        assert_eq!(store.operational_failure(&episode("node",child)).unwrap().unwrap().status,"pending");
    }
    let native=rows(&store,NOW,DAY).unwrap();
    let closed=bindings::rows(&store,NOW,&native).unwrap();
    for child in CHILDREN { assert_eq!(row(&closed,child)["status"],"exhausted"); }
    assert_eq!(bindings::check(&store,&closed).unwrap(),ObjectiveFaults::default(),
        "completed replay must neither recover nor open replacement episodes");
}
