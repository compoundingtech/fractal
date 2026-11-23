//! One explicit page of locally replicated declarations, never a live host census.
use super::*;
use fractal_daemon::cli_println as println;
use std::fmt::Write as _;
use futures_util::StreamExt as _;

#[derive(Subcommand)]
pub(super) enum SchedulesCommand {
    /// List declarations, recent occurrence outcomes and nominal next ticks, with local knowledge age.
    Ls(SchedulesListArgs),
}
#[derive(Args)]
pub(super) struct SchedulesListArgs {
    /// Existing declaration-read authority: this agent, or the configured local person.
    #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)]
    actor: Option<String>,
    /// One page, 1 through 200 rows. Follow the printed cursor explicitly.
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    cursor: Option<String>,
    /// Display threshold only; never host health. First-page default: 5m.
    /// Maximum: 9007199254740991ms (the JSON exact integer ceiling).
    #[arg(long)]
    unheard_after: Option<String>,
}
fn threshold(value: &str) -> Result<u64> {
    let duration=fractal_daemon::graph::parse_duration(value,true)?;
    anyhow::ensure!(duration>0 && duration<=9_007_199_254_740_991,"unheard-after must be positive and finite");
    Ok(duration)
}
pub(super) async fn run(endpoint: &Endpoint, configured_person: Option<&str>, command: SchedulesCommand, json_output: bool) -> Result<()> {
    let SchedulesCommand::Ls(args)=command;
    if let Some(limit)=args.limit { anyhow::ensure!((1..=200).contains(&limit),"the schedule limit must be 1 through 200"); }
    let unheard=args.unheard_after.as_deref().map(threshold).transpose()?;
    let actor=args.actor.as_deref().or(configured_person).context("fractal schedules ls needs --as person/NAME or agent/NAME, ST_AGENT, or a configured person for the existing read.declarations scope")?;
    let response=generated_client(endpoint,Some(actor))?.schedules_list(args.cursor.as_deref(),args.limit,unheard).await
        .map_err(describe_error)?;
    if json_output { return print_value(&response,true); }
    // Keep the client-v0 schedule envelope (including --json) unchanged. Recent outcomes
    // are CLI-only reads through the existing exact-subject claims route.
    let page = serde_json::to_value(&response.value)?;
    let client = cli_client(endpoint);
    let items = page["items"].as_array().context("schedule page items missing")?;
    let subjects = items.iter().filter_map(|item| item["id"].as_str());
    let outcomes = load_outcomes(subjects, |subject| {
        let client = &client;
        async move { client.get::<ClaimsPage>(&recent_claims_path(subject)).await }
    }).await;
    println!("{}",render(&page,actor,&outcomes)?);
    Ok(())
}
fn describe_error(error: fractal_client::ClientError) -> anyhow::Error {
    match error {
        fractal_client::ClientError::Api(fractal_client::ErrorCode::Forbidden,message,_) if message.contains("read.declarations") =>
            anyhow::anyhow!("fractal schedules ls requires the read.declarations scope; the authenticated session does not have it"),
        fractal_client::ClientError::Api(fractal_client::ErrorCode::NotFound | fractal_client::ErrorCode::UnsupportedCapability,_,_) =>
            anyhow::anyhow!("this server does not support schedules.list; upgrade the server to list replicated schedule declarations"),
        error=>anyhow::Error::new(error),
    }
}
fn cell(value: &Value) -> String {
    value.as_str().map(|text|text.chars().map(|c|if c.is_control() {' '} else {c}).collect())
        .unwrap_or_else(||"unknown".into())
}
fn age(knowledge: &Value) -> String {
    match knowledge["age_ms"].as_u64() {
        Some(age) if knowledge["basis"]=="local-read"=>format!("{age}ms (local read)"),
        Some(age)=>format!("{age}ms (direct exchange)"),
        None=>"unknown".into(),
    }
}
// Both the generic claims reader and store reader look ahead one row for pagination:
// requesting six returns at most six claims from one indexed SQL read with LIMIT 8.
const RECENT_CLAIMS: usize = 6;
const OUTCOME_SCHEDULE_LIMIT: usize = 50;
const OUTCOME_CONCURRENCY: usize = 4;
const OUTCOME_TIMEOUT: Duration = Duration::from_secs(2);

async fn load_outcomes<'a, F, Fut>(
    subjects: impl Iterator<Item = &'a str>,
    read: F,
) -> BTreeMap<String, String>
where
    F: Fn(&'a str) -> Fut,
    Fut: Future<Output = Result<ClaimsPage>>,
{
    futures_util::stream::iter(subjects.take(OUTCOME_SCHEDULE_LIMIT))
        .map(|subject| {
            let request = read(subject);
            async move {
                let outcome = match tokio::time::timeout(OUTCOME_TIMEOUT, request).await {
                    Ok(Ok(page)) => latest_outcome(&page.claims),
                    Ok(Err(_)) | Err(_) => "outcome: unavailable".into(),
                };
                (subject.to_owned(), outcome)
            }
        })
        .buffer_unordered(OUTCOME_CONCURRENCY)
        .collect()
        .await
}

fn recent_claims_path(subject: &str) -> String {
    format!("/v1/claims?subject={}&order=desc&limit={RECENT_CLAIMS}", urlencoding::encode(subject))
}

fn short_reason(reason: &str) -> String {
    let mut text: String = reason.chars().take(121)
        .map(|c| if c.is_control() { ' ' } else { c }).collect();
    if let Some((end, _)) = text.char_indices().nth(120) {
        let cut = text[..end].char_indices().nth(117).map_or(end, |(cut, _)| cut);
        text.truncate(cut);
        text.push_str("...");
    }
    text
}

fn outcome_occurrence(claim: &ClaimRecord, claims: &[ClaimRecord]) -> Option<u64> {
    let fields = &claim.body["fields"];
    match claim.kind.as_str() {
        "schedule.occurrence-reached" | "schedule.work-requested" |
        "schedule.occurrence-cancelled" => fields["occurrence"].as_u64(),
        "schedule.work-started" | "schedule.work-failed" => {
            let request = fields["request"].as_str()?;
            claims.iter().find(|claim| claim.id == request && claim.kind == "schedule.work-requested")
                .and_then(|claim| claim.body["fields"]["occurrence"].as_u64())
        }
        _ => None,
    }
}

fn latest_outcome(claims: &[ClaimRecord]) -> String {
    // Input is newest first, but late standby refusals can concern older occurrences.
    // Compare occurrence numbers within this bounded tail, excluding future arming.
    let is_outcome = |claim: &&ClaimRecord| matches!(claim.kind.as_str(),
        "schedule.occurrence-reached" | "schedule.work-requested" |
        "schedule.work-started" | "schedule.work-failed" | "schedule.occurrence-cancelled");
    if !claims.iter().any(|claim| is_outcome(&claim)) { return "none".into(); }
    let Some(occurrence) = claims.iter().filter(is_outcome)
        .filter_map(|claim| outcome_occurrence(claim, claims)).max()
    else { return "occurrence unknown: outcome unknown".into(); };
    // An unattributed observation newer than the selected occurrence's outcome might
    // belong to it. Do not report an older state as current when that link is missing.
    let Some(claim) = claims.iter().filter(is_outcome).find(|claim|
        outcome_occurrence(claim, claims).is_none_or(|known| known == occurrence))
    else { return format!("occurrence {occurrence}: outcome unknown"); };
    if outcome_occurrence(claim, claims).is_none() {
        return format!("occurrence {occurrence}: outcome unknown");
    }
    let fields = &claim.body["fields"];
    let outcome = match claim.kind.as_str() {
        "schedule.occurrence-reached" => "reached".into(),
        "schedule.work-requested" => "work requested".into(),
        "schedule.work-started" => format!("started {}", cell(&fields["mission_run"])),
        "schedule.work-failed" => format!("failed {}: {}", cell(&fields["code"]),
            short_reason(fields["reason"].as_str().unwrap_or("unknown"))),
        "schedule.occurrence-cancelled" => format!("cancelled: {}",
            short_reason(fields["reason"].as_str().unwrap_or("unknown"))),
        _ => unreachable!(),
    };
    format!("{occurrence}: {outcome}")
}

fn render(page: &Value, actor: &str, outcomes: &BTreeMap<String, String>) -> Result<String> {
    let mut out=String::from("SCHEDULE\tOWNER RUN / GENERATION / STEP\tPRIMARY\tSTANDBY NODE\tSTATE\tNEXT (DECLARED)\tLATEST OUTCOME\tKNOWLEDGE AGE\n");
    for item in page["items"].as_array().context("schedule page items missing")? {
        let owner=&item["owner"];
        let owner=if owner["state"]=="recorded" { format!("{} / {} / {}",cell(&owner["run_id"]),cell(&owner["generation_id"]),cell(&owner["step_id"])) } else { cell(&owner["state"]) };
        let outcome=outcomes.get(item["id"].as_str().unwrap_or("")).map(String::as_str).unwrap_or("outcome: not loaded");
        writeln!(out,"{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",cell(&item["id"]),owner,cell(&item["primary_host_id"]),cell(&item["standby_state"]),cell(&item["state"]),
            item["next_declared_fire_at"].as_str().map(str::to_owned).unwrap_or_else(||cell(&item["next_fire_state"])),outcome,age(&item["knowledge"]))?;
        if let Some(problem)=item["read_problem"].as_str() { writeln!(out,"  read limit: {}",cell(&Value::String(problem.into())))?; }
    }
    writeln!(out,"Next declared is a nominal tick from the declaration only; it does not predict admission, catch-up or completion.")?;
    writeln!(out,"Latest outcome uses only the newest {RECENT_CLAIMS} local claims, separately from declarations (first {OUTCOME_SCHEDULE_LIMIT} schedules, {OUTCOME_CONCURRENCY} concurrent reads, {}s per read); none means no outcome in that tail. Older history: fractal claims ls --subject SCHEDULE --order desc --json.",OUTCOME_TIMEOUT.as_secs())?;
    let coverage=&page["coverage"];
    writeln!(out,"No successful direct exchange recorded in {}ms:",coverage["unheard_after_ms"].as_u64().context("unheard threshold missing")?)?;
    let unheard=coverage["unheard_hosts"].as_array().context("unheard hosts missing")?;
    if unheard.is_empty() { writeln!(out,"  none among the covered hosts")?; }
    for host in unheard {
        writeln!(out,"  {}\t{}\t{}",cell(&host["host_id"]),cell(&host["reason"]),host["age_ms"].as_u64().map(|age|format!("{age}ms")).unwrap_or_else(||"age unknown".into()))?;
    }
    writeln!(out,"Coverage: configured peers and this page's declaration-origin/primary hosts only ({} hosts; {} configured and {} covered hosts omitted).",coverage["covered_host_count"],coverage["omitted_configured_peer_count"],coverage["omitted_covered_host_count"])?;
    writeln!(out,"This is local knowledge; it does not say they are down or that the list is complete. Standby is a raw declared node, without takeover interpretation.")?;
    if page["page"]["has_more"]==true {
        let cursor=page["page"]["next_cursor"].as_str().context("continuation cursor missing")?;
        writeln!(out,"More declarations: fractal schedules ls --cursor {} --as {}",presentation::shell_argument(cursor),presentation::shell_argument(actor))?;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_page_keeps_unheard_footer_and_never_says_the_fleet_is_complete() {
        let rendered=render(&json!({"items":[],"page":{"has_more":false},"coverage":{"unheard_after_ms":300000,"unheard_hosts":[{"host_id":"host/laptop","reason":"never-recorded","age_ms":null}],"covered_host_count":2,"omitted_configured_peer_count":0,"omitted_covered_host_count":0}}),"person/ada",&BTreeMap::new()).unwrap();
        assert!(rendered.contains("host/laptop\tnever-recorded\tage unknown"));
        assert!(rendered.contains("does not say they are down or that the list is complete"));
        assert!(rendered.contains("Next declared is a nominal tick"));
        assert!(rendered.contains("NEXT (DECLARED)"));
    }
    #[test]
    fn forbidden_listing_explains_the_missing_existing_declaration_scope() {
        let envelope:fractal_client::ErrorEnvelope=serde_json::from_value(json!({
            "api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"request/denied",
            "code":"forbidden","message":"the authenticated client session is not granted `read.declarations`",
            "retryable":false,"details":{}
        })).unwrap();
        let message=describe_error(fractal_client::ClientError::Api(envelope.code.clone(),envelope.message.clone(),Box::new(envelope))).to_string();
        assert_eq!(message,"fractal schedules ls requires the read.declarations scope; the authenticated session does not have it");
    }
    #[test]
    fn positive_display_threshold_and_explicit_paging_are_cli_contracts() {
        assert_eq!(threshold("5m").unwrap(),300000);
        assert!(threshold("0s").is_err());
        assert!(threshold("forever").is_err());
        assert!(threshold("9007199254740992ms").is_err());
        let help=Cli::command().find_subcommand("schedules").unwrap().find_subcommand("ls").unwrap().clone().render_long_help().to_string();
        assert!(help.contains("9007199254740991ms"));
        assert!(Cli::try_parse_from(["st","schedules","ls","--limit","1","--json"]).is_ok());
        assert!(Cli::try_parse_from(["st","schedules","ls","--all"]).is_err());
    }

    fn claim(id: &str, kind: &str, fields: Value) -> ClaimRecord {
        serde_json::from_value(json!({
            "id":id,"store_index":1,"batch_id":"batch/test",
            "subject":"schedule/example/daily","kind":kind,"origin":"alder","actor":null,
            "body":{"fields":fields},"predecessors":[],"accepted_at_unix_ms":0
        })).unwrap()
    }

    #[test]
    fn reached_then_failed_shows_failure_and_reason_despite_future_arming() {
        let claims = [
            claim("armed", "schedule.occurrence-scheduled", json!({"occurrence":35})),
            claim("failed", "schedule.work-failed", json!({"request":"requested","code":"unsafe-workspace","reason":"scratch is inside a checkout"})),
            claim("requested", "schedule.work-requested", json!({"occurrence":34})),
            claim("reached", "schedule.occurrence-reached", json!({"occurrence":34})),
        ];
        assert_eq!(latest_outcome(&claims),
            "occurrence 34: failed unsafe-workspace: scratch is inside a checkout");
    }

    #[test]
    fn started_shows_the_run_and_empty_tail_shows_none() {
        let claims = [
            claim("started", "schedule.work-started", json!({"request":"requested","mission_run":"mission-run/example/one"})),
            claim("requested", "schedule.work-requested", json!({"occurrence":34})),
        ];
        assert_eq!(latest_outcome(&claims), "occurrence 34: started mission-run/example/one");
        assert_eq!(latest_outcome(&[]), "none");
        assert_eq!(latest_outcome(&claims[..1]), "occurrence unknown: outcome unknown");
    }

    #[test]
    fn recent_states_are_selected_without_folding_older_occurrences() {
        for (kind, expected) in [
            ("schedule.occurrence-reached", "reached"),
            ("schedule.work-requested", "work requested"),
            ("schedule.occurrence-cancelled", "cancelled: revised"),
        ] {
            let claims = [
                claim("current", kind, json!({"occurrence":35,"reason":"revised"})),
                claim("old", "schedule.work-failed", json!({"code":"old","reason":"old"})),
            ];
            assert_eq!(latest_outcome(&claims), format!("occurrence 35: {expected}"));
        }
        assert_eq!(recent_claims_path("schedule/example/daily"),
            "/v1/claims?subject=schedule%2Fexample%2Fdaily&order=desc&limit=6");
    }

    #[test]
    fn late_refusal_for_an_older_request_does_not_hide_a_newer_occurrence() {
        let mut claims = [
            claim("late-failure", "schedule.work-failed", json!({"request":"old-request","code":"standby-occurrence-unavailable","reason":"already reached"})),
            claim("new-request", "schedule.work-requested", json!({"occurrence":35})),
            claim("new-reached", "schedule.occurrence-reached", json!({"occurrence":35})),
            claim("old-request", "schedule.work-requested", json!({"occurrence":34})),
            claim("old-reached", "schedule.occurrence-reached", json!({"occurrence":34})),
        ];
        for (index, claim) in claims.iter_mut().enumerate() {
            claim.store_index = 104 - index as u64;
        }
        assert_eq!(latest_outcome(&claims), "occurrence 35: work requested");
        claims[0].kind = "schedule.work-started".into();
        claims[0].body["fields"]["mission_run"] = json!("mission-run/example/old");
        assert_eq!(latest_outcome(&claims), "occurrence 35: work requested");
    }

    #[test]
    fn unresolved_newer_request_link_leaves_the_latest_occurrence_outcome_unknown() {
        let claims = [
            claim("failed", "schedule.work-failed", json!({"request":"outside-tail","code":"unsafe-workspace","reason":"not attributable"})),
            claim("requested", "schedule.work-requested", json!({"occurrence":35})),
            claim("reached", "schedule.occurrence-reached", json!({"occurrence":35})),
        ];
        assert_eq!(latest_outcome(&claims), "occurrence 35: outcome unknown");
    }

    #[test]
    fn failure_reason_is_terminal_safe_and_capped_at_120_characters() {
        assert_eq!(short_reason("line\nnext\tcell"), "line next cell");
        assert_eq!(short_reason(&"é".repeat(120)), "é".repeat(120));
        let reason = short_reason(&"é".repeat(121));
        assert_eq!(reason.chars().count(), 120);
        assert!(reason.ends_with("..."));
    }

    #[tokio::test(start_paused = true)]
    async fn outcome_reads_are_capped_concurrent_and_fail_open() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let subjects: Vec<_> = (0..60).map(|n| format!("schedule/example/{n}")).collect();
        let outcomes = load_outcomes(subjects.iter().map(String::as_str), |subject| {
            calls.fetch_add(1, Ordering::SeqCst);
            let active = &active;
            let peak = &peak;
            async move {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                tokio::task::yield_now().await;
                active.fetch_sub(1, Ordering::SeqCst);
                if subject.ends_with("/0") {
                    anyhow::bail!("claims read failed");
                }
                if subject.ends_with("/1") {
                    return std::future::pending().await;
                }
                Ok(ClaimsPage { claims: vec![], next_cursor: None })
            }
        }).await;
        assert_eq!(calls.load(Ordering::SeqCst), OUTCOME_SCHEDULE_LIMIT);
        assert_eq!(peak.load(Ordering::SeqCst), OUTCOME_CONCURRENCY);
        assert_eq!(outcomes["schedule/example/0"], "outcome: unavailable");
        assert_eq!(outcomes["schedule/example/1"], "outcome: unavailable");
        assert_eq!(outcomes["schedule/example/2"], "none");
        assert!(!outcomes.contains_key("schedule/example/50"));

        let page = json!({
            "items":[{"id":"schedule/example/50","owner":{"state":"not-recorded"}}],
            "page":{"has_more":false},
            "coverage":{"unheard_after_ms":300000,"unheard_hosts":[],"covered_host_count":1,
                "omitted_configured_peer_count":0,"omitted_covered_host_count":0}
        });
        assert!(render(&page, "person/ada", &outcomes).unwrap().contains("outcome: not loaded"));
    }

    #[test]
    fn outcome_column_renders_failure_run_and_none() {
        let page = json!({
            "items":[{"id":"schedule/example/daily","owner":{"state":"not-recorded"}}],
            "page":{"has_more":false},
            "coverage":{"unheard_after_ms":300000,"unheard_hosts":[],"covered_host_count":1,
                "omitted_configured_peer_count":0,"omitted_covered_host_count":0}
        });
        for claims in [
            vec![
                claim("failed", "schedule.work-failed", json!({"request":"requested","code":"unsafe-workspace","reason":"scratch is inside a checkout"})),
                claim("requested", "schedule.work-requested", json!({"occurrence":34})),
                claim("reached", "schedule.occurrence-reached", json!({"occurrence":34})),
            ],
            vec![
                claim("started", "schedule.work-started", json!({"request":"requested","mission_run":"mission-run/example/one"})),
                claim("requested", "schedule.work-requested", json!({"occurrence":34})),
            ],
            vec![],
        ] {
            let expected = latest_outcome(&claims);
            let outcomes = BTreeMap::from([("schedule/example/daily".into(), expected.clone())]);
            let output = render(&page, "person/ada", &outcomes).unwrap();
            assert!(output.contains(&format!("\t{expected}\t")));
        }
    }
}
