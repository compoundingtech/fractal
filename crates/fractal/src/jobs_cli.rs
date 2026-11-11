//! Observes daemon-owned jobs; disconnect never cancels or authorizes another execution.
use super::*;
use fractal_daemon::cli_println as println;
use fractal_daemon::client::api_error_code;

#[derive(Subcommand)]
pub(super) enum JobsCommand {
    /// Submit a JSON envelope containing key, execution_host and request (file or -).
    /// Class and priority are declarations validated exclusively by the daemon.
    #[command(after_long_help = HELP)]
    Submit { file: PathBuf },
    /// Observe until terminal or unknown-after-host-loss; never cancel or resubmit.
    #[command(after_long_help = HELP)]
    Wait {
        id: String,
        /// Positive polling interval in milliseconds; required, with no default.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        poll_ms: u64,
    },
    /// Record the authenticated requester's cancellation intent; never signal locally.
    #[command(after_long_help = HELP)]
    Cancel { id: String },
}

pub(super) const HELP: &str = "Always emits JSON with state, reason and daemon response.\n\
Exit codes: 0 accepted/nonterminal or successful completion; 2 input/API/protocol error; \
5 transport uncertainty; 10 unsuccessful command exit/signal; 11 withdrawn/cancelled; \
12 refused; 13 failed; 14 unknown-after-host-loss (execution uncertain, NEVER rerun).\n\
An ambiguous submit is reconciled by replaying the identical envelope once on the same \
authenticated connection target: requester, key, digest and host never change. If the \
reply stays ambiguous, repeat only that same input against that same daemon.\n\
Wait uses only GET; interrupting it does not cancel the job or permit a rerun. \
Cancel confirms intent, not whole-tree termination. Admission remains not wired.";

fn classify(response: &Value) -> Result<(Option<u8>, String)> {
    let state = &response["job"]["state"];
    let name = state["state"].as_str().context("job response missing state")?;
    let (exit, reason) = match name {
        "queued" | "preparing" | "prepared" | "launching" | "running" | "recovering"
        | "cancel-requested" | "recovering-cancel-requested" => (None, name),
        "completed" => {
            let outcome = &state["outcome"];
            match (outcome["kind"].as_str(), outcome["value"].as_i64()) {
                (Some("code"), Some(0)) => (Some(0), "exit-code"),
                (Some("code"), Some(_)) => (Some(10), "exit-code"),
                (Some("signal"), Some(signal)) if signal > 0 => (Some(10), "signal"),
                _ => anyhow::bail!("job response has invalid exit outcome"),
            }
        }
        "withdrawn" | "cancelled" => (Some(11), name),
        "refused" => (Some(12), state["refusal"]["reason"].as_str()
            .context("job refusal missing reason")?),
        "failed" => (Some(13), state["failure"]["reason"].as_str()
            .context("job failure missing reason")?),
        "unknown-after-host-loss" => (Some(14), "unknown-after-host-loss"),
        _ => anyhow::bail!("unknown job state: {name}"),
    };
    Ok((exit, reason.into()))
}

// The closure owns no retry policy: both calls borrow the same frozen client and envelope.
// A lost receipt can be reconciled safely because the daemon owns requester/key/digest CAS.
async fn recover_submit<F, Fut>(mut send: F) -> Result<Value>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    match send().await {
        Err(first) if api_error_code(&first).is_none() => send().await.map_err(|recovery| {
            // A later refusal cannot prove the original append did not happen. Keep its
            // typed error as diagnostic text only, never as this operation's refusal.
            anyhow::anyhow!(
                "submit remains uncertain; original reply: {first:#}; reconciliation: {recovery:#}"
            )
        }),
        result => result,
    }
}

fn emit(value: Value, exit: u8) -> Result<()> {
    println!("{}", serde_json::to_string(&value)?);
    if exit == 0 { Ok(()) } else { Err(CommandExit(exit).into()) }
}

fn failure(error: anyhow::Error, submitting: bool) -> Result<()> {
    let code = api_error_code(&error);
    emit(json!({
        "state": if code.is_some() { "request-refused" } else { "transport-uncertain" },
        "reason": code.unwrap_or(if submitting { "submit-uncertain" } else { "request-uncertain" }),
        "message": error.to_string(),
        "rerun_permitted": false,
    }), if code.is_some() { 2 } else { 5 })
}

fn requester_client(
    endpoint: &Endpoint, explicit: Option<&str>, configured: Option<&str>,
) -> Result<Client> {
    execution_client(endpoint, configured_actor(explicit, configured, "jobs")?)
}

pub(super) async fn run(
    endpoint: &Endpoint, explicit: Option<&str>, configured: Option<&str>, command: JobsCommand,
) -> Result<()> {
    let result = async {
        let client = requester_client(endpoint, explicit, configured)?;
        run_inner(&client, command).await
    }.await;
    match result {
        Err(error) if error.downcast_ref::<CommandExit>().is_none() => emit(json!({
            "state": "request-refused", "reason": "invalid-request-or-response",
            "message": error.to_string(), "rerun_permitted": false,
        }), 2),
        result => result,
    }
}

async fn run_inner(client: &Client, command: JobsCommand) -> Result<()> {
    // Resolve socket discovery exactly once. Recovery must never switch daemon targets.
    let mut client = client.clone();
    if let Some(socket) = client.endpoint.unix_socket() {
        client.endpoint = Endpoint::Unix(socket.into_owned());
    }
    let (id, poll) = match command {
        JobsCommand::Submit { file } => {
            let raw = if file == Path::new("-") {
                let mut raw = String::new();
                std::io::stdin().read_to_string(&mut raw)?;
                raw
            } else {
                fs::read_to_string(file)?
            };
            // Do not reconstruct the request, invent a key, or interpret class/priority.
            let body: Value = serde_json::from_str(&raw)?;
            let result = recover_submit(|| client.post("/v1/client/jobs", &body)).await;
            return finish(result, true);
        }
        JobsCommand::Cancel { id } => {
            let path = format!("/v1/client/jobs/{}/cancel", urlencoding::encode(&id));
            return finish(client.post(&path, &json!({})).await, false);
        }
        JobsCommand::Wait { id, poll_ms } => (id, Duration::from_millis(poll_ms)),
    };
    let path = format!("/v1/client/jobs/{}", urlencoding::encode(&id));
    loop {
        let response: Value = match client.get(&path).await {
            Ok(value) => value,
            Err(error) => return failure(error, false),
        };
        let (exit, _) = classify(&response)?;
        if exit.is_some() { return finish(Ok(response), false); }
        tokio::time::sleep(poll).await;
    }
}

fn finish(result: Result<Value>, submitting: bool) -> Result<()> {
    let response = match result {
        Ok(value) => value,
        Err(error) => return failure(error, submitting),
    };
    let (exit, reason) = classify(&response)?;
    emit(json!({
        "state": response["job"]["state"]["state"],
        "reason": reason,
        "response": response,
        "rerun_permitted": false,
    }), exit.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_have_stable_exit_codes_and_uncertainty_is_not_success() {
        for (state, expected) in [
            (json!({"state":"queued"}), None),
            (json!({"state":"completed","outcome":{"kind":"code","value":0}}), Some(0)),
            (json!({"state":"completed","outcome":{"kind":"code","value":7}}), Some(10)),
            (json!({"state":"completed","outcome":{"kind":"signal","value":9}}), Some(10)),
            (json!({"state":"withdrawn"}), Some(11)),
            (json!({"state":"cancelled"}), Some(11)),
            (json!({"state":"refused","refusal":{"reason":"policy-denied"}}), Some(12)),
            (json!({"state":"failed","failure":{"reason":"max-hold"}}), Some(13)),
            (json!({"state":"unknown-after-host-loss","cancel_requested":false}), Some(14)),
        ] {
            assert_eq!(classify(&json!({"job":{"state":state}})).unwrap().0, expected);
        }
        assert!(classify(&json!({"job":{"state":{"state":"invented"}}})).is_err());
    }

    #[tokio::test]
    async fn ambiguous_reply_reuses_the_exact_submission_identity_and_target() {
        let envelope = json!({"key":"original","execution_host":"original-host",
            "request":{"priority":"normal","requested_class":"heavy"}});
        let mut observed = Vec::new();
        let target = "unix:///original-daemon";
        let requester = "daemon-bound-session";
        let result = recover_submit(|| {
            observed.push((target, requester, serde_json::to_vec(&envelope).unwrap()));
            std::future::ready(if observed.len() == 1 {
                Err(anyhow::anyhow!("reply lost after durable append"))
            } else { Ok(json!({"job":{"id":"same-job"}})) })
        }).await.unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0], observed[1]);
        assert_eq!(result["job"]["id"], "same-job");
    }

    #[tokio::test]
    async fn real_transport_recovers_disconnect_but_never_retries_a_typed_refusal() {
        use fractal_daemon::client::FollowTestReply;
        let path = "/v1/client/jobs";
        let body = json!({"key":"fixed","execution_host":"fixed-host","request":{}});
        let client = Client::scripted_follow_test(vec![
            (path.into(), FollowTestReply::Disconnect),
            (path.into(), FollowTestReply::Value(json!({"job":{"id":"fixed-job"}}))),
        ]);
        let result = recover_submit(|| client.post(path, &body)).await.unwrap();
        assert_eq!(result["job"]["id"], "fixed-job");
        assert_eq!(client.follow_test_result().0.len(), 2);
        let client = Client::scripted_follow_test(vec![
            (path.into(), FollowTestReply::Error(409)),
        ]);
        let error = recover_submit(|| client.post(path, &body)).await.unwrap_err();
        assert_eq!(api_error_code(&error), Some("fixture-refusal"));
        assert_eq!(client.follow_test_result().0.len(), 1);
    }

    #[tokio::test]
    async fn reconciliation_refusal_does_not_erase_an_ambiguous_submission() {
        use fractal_daemon::client::FollowTestReply;
        let path = "/v1/client/jobs";
        let client = Client::scripted_follow_test(vec![
            (path.into(), FollowTestReply::Disconnect),
            (path.into(), FollowTestReply::Error(503)),
        ]);
        let body = json!({"key":"fixed","execution_host":"fixed-host","request":{}});
        let error = recover_submit(|| client.post(path, &body)).await.unwrap_err();
        assert!(error.to_string().contains("fixture-refusal"));
        assert_eq!(api_error_code(&error), None);
        let exit = failure(error, true).unwrap_err();
        assert_eq!(exit.downcast_ref::<CommandExit>().unwrap().0, 5);
        assert_eq!(client.follow_test_result().0.len(), 2);
    }

    #[test]
    fn jobs_resolve_and_carry_the_requester_on_the_trusted_transport() {
        let endpoint = Endpoint::Unix("/unused/jobs.sock".into());
        for actor in ["person/a", "agent/host/worker", "mission-run/run-a"] {
            let client = requester_client(&endpoint, Some(actor), Some("person/other")).unwrap();
            assert_eq!(client.person.as_deref(), Some(actor));
            assert_eq!(
                client.endpoint.unix_socket().unwrap().as_ref(), Path::new("/unused/jobs.sock"),
            );
        }
    }

    #[test]
    fn polling_is_caller_declared_and_exit_semantics_are_in_help() {
        assert!(Cli::try_parse_from(["fractal", "jobs", "wait", "job/x"]).is_err());
        assert!(Cli::try_parse_from([
            "fractal", "jobs", "wait", "job/x", "--poll-ms", "0",
        ]).is_err());
        let mut command = Cli::command();
        let jobs = command.find_subcommand_mut("jobs").unwrap();
        for name in ["submit", "wait", "cancel"] {
            let help = jobs.find_subcommand_mut(name).unwrap().render_long_help().to_string();
            assert!(help.contains("14 unknown-after-host-loss"));
            assert!(help.contains("NEVER rerun"));
        }
    }
}
