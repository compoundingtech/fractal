use anyhow::Result;
use clap::{Args, Subcommand};
use fractal_daemon::client::{Client, Endpoint};
use fractal_daemon::cli_println as println;
use fractal_schema::deploy::{PackageIdentity, VerifyGoRequest};

#[derive(Args)]
pub struct DeployArgs {
    #[command(subcommand)]
    command: DeployCommand,
}

#[derive(Subcommand)]
enum DeployCommand {
    /// Verify an existing GO using only the local replica and packaged policy.
    VerifyGo(VerifyGoArgs),
}

#[derive(Args)]
struct VerifyGoArgs {
    go_id: String,
    #[arg(long)]
    host: String,
    /// Exact executor bound by the GO; this does not confer signer authority.
    #[arg(long)]
    actor: String,
    #[arg(long, value_parser = ["install", "rollback"])]
    operation: String,
    #[arg(long)]
    train_id: String,
    #[arg(long)]
    old_package_path: String,
    #[arg(long)]
    old_nar_hash: String,
    #[arg(long)]
    old_commit: String,
    #[arg(long)]
    new_package_path: String,
    #[arg(long)]
    new_nar_hash: String,
    #[arg(long)]
    new_commit: String,
    #[arg(long)]
    preview_digest: String,
    #[arg(long, action = clap::ArgAction::Set, default_value_t = false)]
    failure_restore: bool,
}

impl From<VerifyGoArgs> for VerifyGoRequest {
    fn from(args: VerifyGoArgs) -> Self {
        Self {
            go_id: args.go_id,
            host: args.host,
            actor: args.actor,
            operation: args.operation,
            train_id: args.train_id,
            old: PackageIdentity {
                package_path: args.old_package_path,
                nar_hash: args.old_nar_hash,
                commit: args.old_commit,
            },
            new: PackageIdentity {
                package_path: args.new_package_path,
                nar_hash: args.new_nar_hash,
                commit: args.new_commit,
            },
            preview_digest: args.preview_digest,
            failure_restore: args.failure_restore,
        }
    }
}

pub async fn run(endpoint: &Endpoint, args: DeployArgs, json: bool) -> Result<()> {
    let DeployCommand::VerifyGo(args) = args.command;
    let receipt = Client::new(endpoint.clone()).verify_deploy_go(&args.into()).await?;
    // Both formats are complete machine-readable receipts, not an authorization hint.
    if json {
        println!("{}", serde_json::to_string(&receipt)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&receipt)?);
    }
    Ok(())
}
