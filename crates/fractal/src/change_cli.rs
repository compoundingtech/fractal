use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;
use fractal_daemon::client::Client;

#[derive(Args)]
pub struct ChangesArgs {
    #[command(subcommand)]
    pub command: ChangesCommand,
}

#[derive(Subcommand)]
pub enum ChangesCommand {
    /// Explicitly fetch and validate retained metadata history within the shared read budget.
    Sync {
        #[arg(long)]
        repository: String,
    },
    /// List a bounded page from shared Git discovery and the local record index.
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        repository: String,
        /// Perform no remote operation; retain the original observation age.
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Show one full Change-Id, its exact indexed source and separate freshness.
    Show {
        id: String,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        offline: bool,
    },
}

pub async fn run(client: &Client, arguments: ChangesArgs, json: bool) -> Result<()> {
    let (path, query) = match arguments.command {
        ChangesCommand::Sync { repository } => {
            let response: Value = client
                .post(
                    "/v1/client/changes/sync",
                    &serde_json::json!({"repository":repository}),
                )
                .await?;
            let value = response.get("value").unwrap_or(&response);
            if json {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!("{}", serde_json::to_string_pretty(value)?);
            }
            return Ok(());
        }
        ChangesCommand::List {
            repository,
            offline,
            limit,
            cursor,
        } => {
            anyhow::ensure!(
                (1..=50).contains(&limit),
                "the changes limit must be 1 through 50"
            );
            let mut query = reqwest::Url::parse("http://localhost/v1/client/changes")?;
            query
                .query_pairs_mut()
                .append_pair("repository", &repository)
                .append_pair("offline", if offline { "true" } else { "false" })
                .append_pair("limit", &limit.to_string());
            if let Some(cursor) = cursor {
                query.query_pairs_mut().append_pair("cursor", &cursor);
            }
            (
                "/v1/client/changes",
                query.query().unwrap_or_default().to_owned(),
            )
        }
        ChangesCommand::Show {
            id,
            repository,
            offline,
        } => {
            fractal_daemon::changes::change_id(&id)?;
            let mut query = reqwest::Url::parse("http://localhost/v1/client/change")?;
            query
                .query_pairs_mut()
                .append_pair("repository", &repository)
                .append_pair("id", &id)
                .append_pair("offline", if offline { "true" } else { "false" });
            (
                "/v1/client/change",
                query.query().unwrap_or_default().to_owned(),
            )
        }
    };
    let response: Value = client.get(&format!("{path}?{query}")).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    let value = response.get("value").unwrap_or(&response);
    let observation = &value["observation"];
    println!(
        "Git refs observed: {}{}",
        text(&observation["refs_observed_at"]),
        if observation["offline"] == true {
            " (offline)"
        } else if observation["reused"] == true {
            " (shared observation)"
        } else {
            ""
        }
    );
    if let Some(error) = observation["error"].as_str() {
        println!(
            "Git observation error: {}. The PR path remains available.",
            escape(error)
        );
    }
    let items = value["items"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![value["item"].clone()]);
    for item in items {
        let current = &item["indexed_current"]["revision"]["record"];
        println!(
            "{}  {}  Git state: {}  tree: {}",
            text(&item["change_id"]),
            text(&item["status"]),
            text(&current["state"]),
            text(&current["source_tree"])
        );
        if item["last_indexed"].is_object() {
            println!(
                "  Previous indexed record: {} (historical)",
                text(&item["last_indexed"]["revision"]["oid"])
            );
        }
        println!(
            "  Evidence freshness: {}",
            text(&item["evidence_indexed_at"])
        );
    }
    if let Some(cursor) = value["next_cursor"].as_str() {
        println!("Next page: --cursor {}", escape(cursor));
    }
    Ok(())
}

fn escape(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(escape)
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: ChangesArgs,
    }

    #[test]
    fn reads_require_repository_and_keep_full_id_and_cursor() {
        assert!(TestCli::try_parse_from(["fixture", "list"]).is_err());
        let list = TestCli::try_parse_from([
            "fixture",
            "ls",
            "--repository",
            "fixture/garden",
            "--offline",
            "--limit",
            "2",
            "--cursor",
            "pinned",
        ])
        .unwrap();
        assert!(matches!(
            list.args.command,
            ChangesCommand::List {
                offline: true,
                limit: 2,
                cursor: Some(_),
                ..
            }
        ));
        let id = format!("I{}", "a".repeat(40));
        let show =
            TestCli::try_parse_from(["fixture", "show", &id, "--repository", "fixture/garden"])
                .unwrap();
        assert!(
            matches!(show.args.command, ChangesCommand::Show { id: actual, .. } if actual == id)
        );
        assert!(
            TestCli::try_parse_from([
                "fixture",
                "show",
                &id,
                "--repository",
                "fixture/garden",
                "--cursor",
                "pinned"
            ])
            .is_err()
        );
    }
}
