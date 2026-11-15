//! Operator reads and explicit human resolution share the daemon's effect contract.
use super::*;
use fractal_daemon::cli_println as println;

#[derive(Subcommand)]
pub(super) enum EffectsCommand {
    /// Read the durable effect using its full native-effect idempotency key.
    Status {
        #[arg(long)]
        key: String,
    },
    /// Confirm or abandon an unresolved effect with a person's authority.
    Resolve(ResolveArgs),
}

#[derive(Args)]
pub(super) struct ResolveArgs {
    #[arg(long)]
    subject: String,
    #[arg(long = "as", value_parser = parse_actor_subject)]
    pub(super) actor: String,
    /// Record that the action happened; requires --provider-ref.
    #[arg(long, conflicts_with = "abandoned", required_unless_present = "abandoned", requires = "provider_ref")]
    confirmed: bool,
    /// Explicitly accept that the action will not be completed under this key.
    #[arg(long, conflicts_with_all = ["confirmed", "provider_ref"])]
    abandoned: bool,
    #[arg(long)]
    provider_ref: Option<String>,
    /// Durable evidence supporting the person's decision (repeatable).
    #[arg(long, required = true)]
    evidence: Vec<String>,
    #[arg(long)]
    idempotency_key: String,
}

pub(super) async fn run(endpoint: &Endpoint, command: EffectsCommand, json_output: bool) -> Result<()> {
    use fractal_daemon::api::effects::{EffectResolutionOutcome, EffectResponse, ResolveEffectRequest};
    let client = cli_client(endpoint);
    let result: EffectResponse = match command {
        EffectsCommand::Status { key } => client.get(&format!(
            "/v1/effects/by-key?key={}", urlencoding::encode(&key),
        )).await?,
        EffectsCommand::Resolve(args) => {
            reject_foreign_agent_actor(&args.actor)?;
            let actor = parse_person_subject(&args.actor).map_err(anyhow::Error::msg)?;
            let client = execution_client(endpoint, actor.clone())?;
            let body = ResolveEffectRequest {
                actor,
                outcome: match (args.confirmed, args.abandoned) {
                    (true, false) => EffectResolutionOutcome::Confirmed,
                    (false, true) => EffectResolutionOutcome::Abandoned,
                    _ => anyhow::bail!("choose exactly one of --confirmed or --abandoned"),
                },
                provider_ref: args.provider_ref,
                evidence: args.evidence,
                idempotency_key: args.idempotency_key,
                delegation: None,
            };
            client.post(&format!("/v1/effects/{}/resolve", urlencoding::encode(&args.subject)), &body).await?
        }
    };
    if json_output {
        return print_value(&result, true);
    }
    println!("{}  {}", result.effect_id, result.state);
    if let Some(reference) = &result.record.provider_ref {
        println!("  provider: {reference}");
    }
    if result.state == "unresolved" {
        println!("  resolve: fractal effects resolve --subject {} --as person/<id> --confirmed --provider-ref <reference> --evidence <claim> --idempotency-key <key>",
            result.effect_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_requires_explicit_outcome_and_confirmation_evidence() {
        let base = ["fractal", "effects", "resolve", "--subject", "effect/example",
            "--as", "person/example", "--evidence", "message/example", "--idempotency-key", "example"];
        assert!(Cli::try_parse_from(base).is_err());
        assert!(Cli::try_parse_from(base.into_iter().chain(["--confirmed"])).is_err());
        assert!(Cli::try_parse_from(base.into_iter().chain(["--abandoned"])).is_ok());
        assert!(Cli::try_parse_from(base.into_iter().chain(["--confirmed", "--provider-ref", "message/example"])).is_ok());
        assert!(Cli::try_parse_from(base.into_iter().chain(["--confirmed", "--abandoned"])).is_err());
    }
}
