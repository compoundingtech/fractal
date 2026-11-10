use super::*;

#[derive(Subcommand)]
pub(super) enum ClaimsCommand {
    /// List claims by kind, exact subject or subject prefix, one line each.
    Ls(ClaimsLsArgs),
}

#[derive(Args)]
pub(super) struct ClaimsLsArgs {
    /// Only claims of this kind.
    #[arg(long)]
    kind: Option<String>,
    /// Only claims whose subject starts with this prefix, such as `custom/acme/change/`.
    #[arg(long, conflicts_with = "subject")]
    subject_prefix: Option<String>,
    /// Only claims of this exact subject.
    #[arg(long)]
    subject: Option<String>,
    /// At most this many claims (1 to 500).
    #[arg(long, default_value_t = 100)]
    limit: usize,
    /// Only claims after this store index.
    #[arg(long)]
    after_index: Option<u64>,
    /// Only claims before this store index.
    #[arg(long)]
    before_index: Option<u64>,
    /// Oldest first (`asc`) or newest first (`desc`).
    #[arg(long, value_enum, default_value_t = ClaimsOrder::Asc)]
    order: ClaimsOrder,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ClaimsOrder {
    Asc,
    Desc,
}

pub(super) async fn run_claims(client: &Client, command: ClaimsCommand, json_output: bool) -> Result<()> {
    let ClaimsCommand::Ls(args) = command;
    let descending = args.order == ClaimsOrder::Desc;
    let mut query = vec![
        format!("limit={}", args.limit),
        format!("order={}", if descending { "desc" } else { "asc" }),
    ];
    for (name, value) in [
        ("kind", &args.kind),
        ("subject_prefix", &args.subject_prefix),
        ("subject", &args.subject),
    ] {
        if let Some(value) = value {
            query.push(format!("{name}={}", urlencoding::encode(value)));
        }
    }
    if let Some(after) = args.after_index {
        query.push(format!("after_index={after}"));
    }
    if let Some(before) = args.before_index {
        query.push(format!("before_index={before}"));
    }
    let page: ClaimsPage = client.get(&format!("/v1/claims?{}", query.join("&"))).await?;
    if json_output {
        return print_value(&page, true);
    }
    write_claims_page(&mut cli_output::stdout(), &page, descending)
}

/// One line per claim, as `trace show` prints them, then the flag that reads the next page.
fn write_claims_page(output: &mut impl std::io::Write, page: &ClaimsPage, descending: bool) -> Result<()> {
    for claim in &page.claims {
        write_trace_claim(output, claim)?;
    }
    if let Some(cursor) = page.next_cursor {
        let flag = if descending { "--before-index" } else { "--after-index" };
        writeln!(output, "next: {flag} {cursor}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claims_page_prints_trace_lines_and_the_next_page_flag() {
        let claim: ClaimRecord = serde_json::from_value(json!({
            "id": "claim-1", "store_index": 7, "batch_id": "batch-1",
            "subject": "custom/acme/change/1", "kind": "custom.acme.change", "origin": "alder",
            "actor": null, "body": {"fields": {"status": "open"}}, "predecessors": [],
            "accepted_at_unix_ms": 0,
        }))
        .unwrap();
        let page = ClaimsPage { claims: vec![claim], next_cursor: Some(7) };
        for (descending, flag) in [(false, "--after-index"), (true, "--before-index")] {
            let mut output = Vec::new();
            write_claims_page(&mut output, &page, descending).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!("7\t1970-01-01T00:00:00.000Z\tcustom.acme.change\tcustom/acme/change/1\tstatus=open\nnext: {flag} 7\n"),
            );
        }
        let mut output = Vec::new();
        write_claims_page(&mut output, &ClaimsPage { claims: vec![], next_cursor: None }, false).unwrap();
        assert!(output.is_empty());
    }
}
