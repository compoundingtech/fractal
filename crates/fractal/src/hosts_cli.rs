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
    fn bound_agents_cannot_borrow_a_hold_owners_identity() {
        let cli = Cli::try_parse_from(["fractal", "hosts", "release", "host-a", "host-hold/example",
            "--expected-revision", "1", "--resolution", "repaired", "--as", "person/operator"]).unwrap();
        assert!(guard_mutating_cli_actor(&cli.command, Some("agent/worker"), None).is_err());
    }
}
