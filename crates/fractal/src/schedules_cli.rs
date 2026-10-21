//! One explicit page of locally replicated declarations, never a live host census.
use super::*;
use fractal_daemon::cli_println as println;
use std::fmt::Write as _;

#[derive(Subcommand)]
pub(super) enum SchedulesCommand {
    /// List declaration owners, hosts and nominal next ticks, with local knowledge age.
    Ls(SchedulesListArgs),
}
#[derive(Args)]
pub(super) struct SchedulesListArgs {
    /// Existing declaration-read authority: this seat, or the configured local person.
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
    let actor=args.actor.as_deref().or(configured_person).context("st schedules ls needs --as person/NAME or agent/NAME, ST_AGENT, or a configured person for the existing read.declarations scope")?;
    let response=generated_client(endpoint,Some(actor))?.schedules_list(args.cursor.as_deref(),args.limit,unheard).await
        .map_err(describe_error)?;
    if json_output { return print_value(&response,true); }
    println!("{}",render(&serde_json::to_value(&response.value)?,actor)?);
    Ok(())
}
fn describe_error(error: fractal_client::ClientError) -> anyhow::Error {
    match error {
        fractal_client::ClientError::Api(fractal_client::ErrorCode::Forbidden,message,_) if message.contains("read.declarations") =>
            anyhow::anyhow!("st schedules ls requires the read.declarations scope; the authenticated session does not have it"),
        fractal_client::ClientError::Api(fractal_client::ErrorCode::NotFound | fractal_client::ErrorCode::Unsupported | fractal_client::ErrorCode::UnsupportedCapability,_,_) =>
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
fn render(page: &Value, actor: &str) -> Result<String> {
    let mut out=String::from("SCHEDULE\tOWNER RUN / GENERATION / STEP\tPRIMARY\tSTANDBY NODE\tSTATE\tNEXT DECLARED\tKNOWLEDGE AGE\n");
    for item in page["items"].as_array().context("schedule page items missing")? {
        let owner=&item["owner"];
        let owner=if owner["state"]=="recorded" { format!("{} / {} / {}",cell(&owner["run_id"]),cell(&owner["generation_id"]),cell(&owner["step_id"])) } else { cell(&owner["state"]) };
        writeln!(out,"{}\t{}\t{}\t{}\t{}\t{}\t{}",cell(&item["id"]),owner,cell(&item["primary_host_id"]),cell(&item["standby_state"]),cell(&item["state"]),
            item["next_declared_fire_at"].as_str().map(str::to_owned).unwrap_or_else(||cell(&item["next_fire_state"])),age(&item["knowledge"]))?;
        if let Some(problem)=item["read_problem"].as_str() { writeln!(out,"  read limit: {}",cell(&Value::String(problem.into())))?; }
    }
    writeln!(out,"Next declared is a nominal tick from the declaration only; it does not predict admission, catch-up or completion.")?;
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
        writeln!(out,"More declarations: st schedules ls --cursor {} --as {}",presentation::shell_argument(cursor),presentation::shell_argument(actor))?;
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_page_keeps_unheard_footer_and_never_says_the_fleet_is_complete() {
        let rendered=render(&json!({"items":[],"page":{"has_more":false},"coverage":{"unheard_after_ms":300000,"unheard_hosts":[{"host_id":"host/laptop","reason":"never-recorded","age_ms":null}],"covered_host_count":2,"omitted_configured_peer_count":0,"omitted_covered_host_count":0}}),"person/ada").unwrap();
        assert!(rendered.contains("host/laptop\tnever-recorded\tage unknown"));
        assert!(rendered.contains("does not say they are down or that the list is complete"));
        assert!(rendered.contains("Next declared is a nominal tick"));
    }
    #[test]
    fn forbidden_listing_explains_the_missing_existing_declaration_scope() {
        let envelope:fractal_client::ErrorEnvelope=serde_json::from_value(json!({
            "api_version":"st3.client.v0","error_version":"st3.client.error.v0","request_id":"request/denied",
            "code":"forbidden","message":"the authenticated client session is not granted `read.declarations`",
            "retryable":false,"details":{}
        })).unwrap();
        let message=describe_error(fractal_client::ClientError::Api(envelope.code.clone(),envelope.message.clone(),Box::new(envelope))).to_string();
        assert_eq!(message,"st schedules ls requires the read.declarations scope; the authenticated session does not have it");
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
}
