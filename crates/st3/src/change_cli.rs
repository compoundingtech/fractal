use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;
use st3::changes::{
    State, WorkBinding,
    mutation::{Operation, Request},
};
use st3::client::Client;

#[derive(Args)]
pub struct ChangesArgs {
    #[command(subcommand)]
    pub command: ChangesCommand,
}

#[derive(Args)]
pub struct StageArgs {
    #[arg(long)]
    repository: String,
    /// The already claimed st work step, owned by this agent on the Git owner node.
    #[arg(long)]
    step: String,
    #[arg(long, env = "ST3_INCARNATION")]
    incarnation: String,
    /// Optional exact durable work claim; otherwise obtain a bounded owner-runtime fence.
    #[arg(long)]
    work_claim: Option<String>,
}

#[derive(Subcommand)]
pub enum ChangesCommand {
    /// Publish an existing source commit with its canonical trailer as a draft change.
    Create {
        #[command(flatten)]
        stage: StageArgs,
        #[arg(long)]
        source: String,
        #[arg(long)]
        base: String,
    },
    /// Publish ready and an optional next lane under the exact current-record fence.
    Ready {
        id: String,
        #[command(flatten)]
        stage: StageArgs,
        #[arg(long)]
        expected_current: String,
        #[arg(long)]
        next: Option<String>,
    },
    /// Return an unclaimed change to draft under the exact current-record fence.
    Draft {
        id: String,
        #[command(flatten)]
        stage: StageArgs,
        #[arg(long)]
        expected_current: String,
    },
    /// Pull ready work in build or review by atomically advancing current and creating its claim ref.
    Claim {
        id: String,
        #[command(flatten)]
        stage: StageArgs,
        #[arg(long)]
        expected_current: String,
        #[arg(long)]
        lane: String,
    },
    /// Voluntarily release this exact worker/work/nonce claim, optionally naming its next lane.
    Release {
        id: String,
        #[command(flatten)]
        stage: StageArgs,
        #[arg(long)]
        expected_current: String,
        #[arg(long)]
        nonce: String,
        #[arg(long)]
        next: Option<String>,
    },
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
        ChangesCommand::Create {
            stage,
            source,
            base,
        } => return publish(client, stage, Operation::Create { source, base }, json).await,
        ChangesCommand::Ready {
            id,
            stage,
            expected_current,
            next,
        } => {
            return publish(
                client,
                stage,
                Operation::State {
                    id,
                    expected_current,
                    state: State::Ready,
                    next,
                },
                json,
            )
            .await;
        }
        ChangesCommand::Draft {
            id,
            stage,
            expected_current,
        } => {
            return publish(
                client,
                stage,
                Operation::State {
                    id,
                    expected_current,
                    state: State::Draft,
                    next: None,
                },
                json,
            )
            .await;
        }
        ChangesCommand::Claim {
            id,
            stage,
            expected_current,
            lane,
        } => {
            return publish(
                client,
                stage,
                Operation::Claim {
                    id,
                    expected_current,
                    lane,
                },
                json,
            )
            .await;
        }
        ChangesCommand::Release {
            id,
            stage,
            expected_current,
            nonce,
            next,
        } => {
            return publish(
                client,
                stage,
                Operation::Release {
                    id,
                    expected_current,
                    nonce,
                    next,
                },
                json,
            )
            .await;
        }
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
            st3::changes::change_id(&id)?;
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

async fn publish(
    client: &Client,
    stage: StageArgs,
    operation: Operation,
    json: bool,
) -> Result<()> {
    operation.validate()?;
    let work = if let Some(claim_id) = stage.work_claim {
        WorkBinding {
            step: stage.step,
            claim_id,
            incarnation: stage.incarnation,
        }
    } else {
        let mut query = reqwest::Url::parse("http://localhost/v1/client/changes/work-binding")?;
        query
            .query_pairs_mut()
            .append_pair("step", &stage.step)
            .append_pair("incarnation", &stage.incarnation);
        let response: Value = client
            .get(&format!(
                "/v1/client/changes/work-binding?{}",
                query.query().unwrap_or_default()
            ))
            .await?;
        serde_json::from_value(response.get("value").unwrap_or(&response).clone())?
    };
    let request = Request {
        repository: stage.repository,
        work,
        operation,
    };
    let response: Value = match client.post("/v1/client/changes/mutate", &request).await {
        Ok(response) => response,
        Err(error) => {
            if let Some((_, _, _, details)) = st3::client::api_error_parts(&error)
                && let Some(outcome) = details.get("outcome")
            {
                let rendered = serde_json::to_string_pretty(outcome)?;
                if json {
                    println!("{rendered}");
                } else {
                    eprintln!("{rendered}");
                }
            }
            return Err(error);
        }
    };
    let value = response.get("value").unwrap_or(&response);
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "{}  current {}  Git state {}",
            text(&value["revision"]["record"]["change_id"]),
            text(&value["revision"]["oid"]),
            text(&value["revision"]["record"]["state"])
        );
        if let Some(nonce) = value["revision"]["record"]["claim"]["nonce"].as_str() {
            println!("Claim nonce: {}", escape(nonce));
        }
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
    #[test]
    fn stage_parser_requires_work_and_current_fences_without_an_actor_selector() {
        let id = format!("I{}", "a".repeat(40));
        let oid = "b".repeat(40);
        let args = [
            "fixture",
            "claim",
            &id,
            "--repository",
            "fixture/garden",
            "--step",
            "step-run/fixture/build",
            "--incarnation",
            "fixture:one",
            "--expected-current",
            &oid,
            "--lane",
            "build",
        ];
        let parsed = TestCli::try_parse_from(args).unwrap();
        assert!(
            matches!(parsed.args.command, ChangesCommand::Claim { id: selected, stage, expected_current, lane } if selected==id && stage.step=="step-run/fixture/build" && stage.incarnation=="fixture:one" && expected_current==oid && lane=="build")
        );
        assert!(
            TestCli::try_parse_from([
                "fixture",
                "claim",
                &id,
                "--repository",
                "fixture/garden",
                "--lane",
                "build"
            ])
            .is_err()
        );
        let mut foreign = args.to_vec();
        foreign.extend(["--actor", "agent/fixture/other"]);
        assert!(TestCli::try_parse_from(foreign).is_err());
    }
}
