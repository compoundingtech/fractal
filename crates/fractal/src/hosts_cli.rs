//! Host holds are durable admission conditions, never executor reservations.
use super::*;
use fractal_daemon::cli_println as println;

#[derive(Subcommand)]
pub(super) enum HostsCommand {
    /// List active holds on the connected execution host.
    Ls,
    /// Close admission for affected operations until the authenticated owner resolves it.
    Hold {
        host: String,
        #[arg(long)]
        reason: String,
        /// Affected native operation; repeat for several, or use * for all operations.
        #[arg(long = "operation", required = true)]
        operations: Vec<String>,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)]
        actor: Option<String>,
        #[arg(long)]
        originating_run: Option<String>,
        #[arg(long)]
        receipt: Option<String>,
    },
    /// Resolve an owned hold at the revision observed by the owner.
    Release {
        host: String,
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        resolution: String,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)]
        actor: Option<String>,
    },
    /// Recover an orphaned hold as the configured host operator; live runs must release their own.
    Recover {
        host: String,
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        reason: String,
        #[arg(long = "as", env = "ST_AGENT", value_parser = parse_publication_actor)]
        actor: Option<String>,
    },
}

pub(super) async fn run(client: &Client, person: Option<&str>, command: HostsCommand, json_output: bool) -> Result<()> {
    let value: Value = match command {
        HostsCommand::Ls => client.get("/v1/hosts/holds").await?,
        HostsCommand::Hold { host, reason, operations, actor, originating_run, receipt } => {
            let actor = actor.as_deref().or(person).context("hosts hold requires --as, ST_AGENT, or a configured person")?;
            client.post("/v1/hosts/holds", &json!({"host":host,"reason":reason,"operations":operations,"actor":actor,"originating_run":originating_run,"receipt":receipt})).await?
        }
        HostsCommand::Release { host, id, expected_revision, resolution, actor } => {
            let actor = actor.as_deref().or(person).context("hosts release requires --as, ST_AGENT, or a configured person")?;
            client.post("/v1/hosts/holds/release", &json!({"host":host,"id":id,"expected_revision":expected_revision,"resolution":resolution,"actor":actor})).await?
        }
        HostsCommand::Recover { host, id, expected_revision, reason, actor } => {
            let actor = actor.as_deref().or(person).context("hosts recover requires the configured host operator")?;
            client.post("/v1/hosts/holds/recover", &json!({"host":host,"id":id,"expected_revision":expected_revision,"reason":reason,"actor":actor})).await?
        }
    };
    if json_output { return print_value(&value, true); }
    if let Some(holds) = value.as_array() {
        for hold in holds { println!("{}  {}  revision={}  owner={}  {}", hold["host"].as_str().unwrap_or_default(), hold["id"].as_str().unwrap_or_default(), hold["revision"], hold["owner"].as_str().unwrap_or_default(), hold["reason"].as_str().unwrap_or_default()); }
        println!("{} active host holds; no lane capacity owned", holds.len());
    } else { print_value(&value, false)?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_hold_commands_preserve_revision_and_identity() {
        let parse = |args: &[&str]| {
            let cli = Cli::try_parse_from(args).unwrap();
            let Command::Hosts { command } = cli.command else { panic!("expected hosts command") };
            command
        };
        assert!(matches!(parse(&["fractal", "hosts", "ls"]), HostsCommand::Ls));
        match parse(&["fractal", "hosts", "hold", "host-a", "--reason", "investigate",
            "--operation", "activation", "--as", "person/operator", "--originating-run", "mission-run/RUN",
            "--receipt", "receipt/failure"]) {
            HostsCommand::Hold { host, actor, reason, operations, originating_run, receipt } => {
                assert_eq!(host, "host-a"); assert_eq!(actor.as_deref(), Some("person/operator"));
                assert_eq!(reason, "investigate"); assert_eq!(operations, ["activation"]);
                assert_eq!(originating_run.as_deref(), Some("mission-run/RUN"));
                assert_eq!(receipt.as_deref(), Some("receipt/failure"));
            }
            _ => panic!("expected hold"),
        }
        match parse(&["fractal", "hosts", "release", "host-a", "host-hold/example",
            "--expected-revision", "7", "--resolution", "repaired", "--as", "person/operator"]) {
            HostsCommand::Release { host, id, actor, expected_revision, resolution } => {
                assert_eq!(host, "host-a"); assert_eq!(id, "host-hold/example");
                assert_eq!(actor.as_deref(), Some("person/operator")); assert_eq!(expected_revision, 7);
                assert_eq!(resolution, "repaired");
            }
            _ => panic!("expected release"),
        }
        match parse(&["fractal", "hosts", "recover", "host-a", "host-hold/example",
            "--expected-revision", "9", "--reason", "owner finished", "--as", "person/operator"]) {
            HostsCommand::Recover { host, id, actor, expected_revision, reason } => {
                assert_eq!(host, "host-a"); assert_eq!(id, "host-hold/example");
                assert_eq!(actor.as_deref(), Some("person/operator")); assert_eq!(expected_revision, 9);
                assert_eq!(reason, "owner finished");
            }
            _ => panic!("expected recovery"),
        }
    }
    #[test]
    fn holds_require_affected_operations_and_release_requires_revision() {
        assert!(Cli::try_parse_from(["fractal", "hosts", "hold", "host-a", "--reason", "investigate"]).is_err());
        assert!(Cli::try_parse_from(["fractal", "hosts", "hold", "host-a", "--reason", "investigate",
            "--operation", "activation", "--operation", "check", "--as", "person/operator"]).is_ok());
        assert!(Cli::try_parse_from(["fractal", "hosts", "release", "host-a", "host-hold/example",
            "--resolution", "repaired", "--as", "person/operator"]).is_err());
        assert!(Cli::try_parse_from(["fractal", "hosts", "release", "host-a", "host-hold/example",
            "--expected-revision", "1", "--resolution", "repaired", "--as", "person/operator"]).is_ok());
    }
    #[test]
    fn recovery_requires_revision_and_reason() {
        assert!(Cli::try_parse_from(["fractal", "hosts", "recover", "host-a", "host-hold/example",
            "--reason", "owner ended", "--as", "person/operator"]).is_err());
        assert!(Cli::try_parse_from(["fractal", "hosts", "recover", "host-a", "host-hold/example",
            "--expected-revision", "1", "--as", "person/operator"]).is_err());
        assert!(Cli::try_parse_from(["fractal", "hosts", "recover", "host-a", "host-hold/example",
            "--expected-revision", "1", "--reason", "owner ended", "--as", "person/operator"]).is_ok());
    }
    #[test]
    fn bound_agents_cannot_borrow_a_hold_owners_identity() {
        let cli = Cli::try_parse_from(["fractal", "hosts", "release", "host-a", "host-hold/example",
            "--expected-revision", "1", "--resolution", "repaired", "--as", "person/operator"]).unwrap();
        assert!(guard_mutating_cli_actor(&cli.command, Some("agent/worker"), None).is_err());
    }
}
