use super::*;
use fractal_daemon::cli_println as println;
use fractal_client::{DeviceCommandRequest, DeviceCommandState, PhoneCommand, PhoneNavigateArgs};

#[derive(Args)]
pub(super) struct CommandArgs {
    #[arg(add = ArgValueCompleter::new(Complete(Entity::Device)))]
    device: String,
    /// Lifetime in milliseconds, 1..=60000 (default: 30000). No replay after expiry.
    #[arg(long, default_value_t = 30_000, value_parser = clap::value_parser!(u64).range(1..=60_000), global = true)]
    ttl: u64,
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Inspect the foreground phone's current route and connection.
    Inspect,
    /// Navigate using the phone's typed screen arguments.
    Navigate {
        #[command(subcommand)]
        screen: Screen,
    },
    /// Run the phone's existing mark-read handler for an attention card.
    MarkRead { attention_id: String },
}

#[derive(Subcommand)]
#[command(rename_all = "verbatim")]
enum Screen {
    NowRoot,
    Agents,
    Missions,
    Usage,
    History,
    Mission { id: String },
    Attention { id: String },
    Launch { id: String },
    Conversation {
        target: String,
        #[arg(long)]
        session_id: Option<String>,
    },
    Terminal { terminal_id: String },
    Documents {
        #[arg(long)]
        prefix: Option<String>,
    },
    Document {
        reference: String,
        #[arg(long)]
        trail: Vec<String>,
    },
}

impl From<Screen> for PhoneNavigateArgs {
    fn from(screen: Screen) -> Self {
        use fractal_client::{PhoneConversationParams, PhoneDocumentParams, PhoneDocumentsParams, PhoneIdParams, PhoneTerminalParams};
        match screen {
            Screen::NowRoot => Self::NowRoot,
            Screen::Agents => Self::Agents,
            Screen::Missions => Self::Missions,
            Screen::Usage => Self::Usage,
            Screen::History => Self::History,
            Screen::Mission { id } => Self::Mission(PhoneIdParams { id }),
            Screen::Attention { id } => Self::Attention(PhoneIdParams { id }),
            Screen::Launch { id } => Self::Launch(PhoneIdParams { id }),
            Screen::Conversation { target, session_id } => Self::Conversation(PhoneConversationParams { target, session_id }),
            Screen::Terminal { terminal_id } => Self::Terminal(PhoneTerminalParams { terminal_id }),
            Screen::Documents { prefix } => Self::Documents(prefix.map(|prefix| PhoneDocumentsParams { prefix: Some(prefix) })),
            Screen::Document { reference, trail } => Self::Document(PhoneDocumentParams { reference, trail }),
        }
    }
}

pub(super) async fn run(client: &GeneratedClient, args: CommandArgs, json_output: bool) -> Result<()> {
    let command = match args.verb {
        Verb::Inspect => PhoneCommand::Inspect(fractal_client::PhoneInspectArgs::default()),
        Verb::Navigate { screen } => PhoneCommand::Navigate(screen.into()),
        Verb::MarkRead { attention_id } => PhoneCommand::MessageRead(fractal_client::PhoneMessageReadArgs { attention_id }),
    };
    let issued = client.device_command(&DeviceCommandRequest {
        api_version: CLIENT_V0_API_VERSION.into(), device: args.device,
        command, ttl_ms: Some(args.ttl),
    }).await?;
    // One held read; the daemon owns the deadline and result precedence.
    let result = client.device_command_get(&issued.value.header.id, true).await?;
    let (state, code) = match result.value.state {
        DeviceCommandState::Completed => ("completed", 0),
        DeviceCommandState::Refused => ("refused", 3),
        DeviceCommandState::Expired => ("expired", 4),
        DeviceCommandState::NotForeground => ("not-foreground", 5),
        DeviceCommandState::Failed => ("failed", 1),
        DeviceCommandState::Pending | DeviceCommandState::Delivered =>
            anyhow::bail!("held phone command read returned a nonterminal state"),
    };
    if json_output {
        print_client_value(&result, true)?;
    } else {
        println!("{}  {state}  {}", result.value.header.id, result.value.device_id);
        if let Some(error) = &result.value.error_code { println!("  {error}"); }
        if let Some(value) = &result.value.result { println!("{}", serde_json::to_string_pretty(value)?); }
    }
    if code == 0 { Ok(()) } else { Err(CommandExit(code).into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_command_cli_parses_typed_screens_and_bounded_ttl() {
        for screen in [
            vec!["NowRoot"], vec!["Agents"], vec!["Missions"], vec!["Usage"],
            vec!["History"], vec!["Mission", "mission/a"], vec!["Attention", "attention/a"],
            vec!["Launch", "launch/a"], vec!["Conversation", "agent/a", "--session-id", "s"],
            vec!["Terminal", "terminal/a"], vec!["Documents", "--prefix", "docs/"],
            vec!["Document", "doc/a", "--trail", "doc/root"],
        ] {
            let mut args = vec!["fractal", "devices", "--as", "person/alex", "command", "device/a", "navigate"];
            args.extend(screen);
            let cli = Cli::try_parse_from(args).unwrap();
            let Command::Devices(DevicesArgs { command: Some(DevicesCommand::Command(args)), .. }) = cli.command else { panic!("not a phone command") };
            let Verb::Navigate { screen } = args.verb else { panic!("not navigation") };
            let value = serde_json::to_value(PhoneNavigateArgs::from(screen)).unwrap();
            assert!(value["screen"].is_string());
            assert_eq!(args.ttl, 30_000);
        }
        for tail in [
            vec!["--ttl", "0", "inspect"], vec!["--ttl", "60001", "inspect"],
            vec!["navigate", "Conversation"], vec!["navigate", "NowRoot", "unexpected"],
            vec!["navigate", "Unknown"], vec!["mark-read"],
        ] {
            let mut args = vec!["fractal", "devices", "command", "device/a"];
            args.extend(tail);
            assert!(Cli::try_parse_from(args).is_err());
        }
        assert!(Cli::try_parse_from(["fractal", "devices", "--as", "agent/worker",
            "command", "device/a", "inspect"]).is_err());
        for ttl in ["1", "60000"] {
            assert!(Cli::try_parse_from(["fractal", "devices", "command", "device/a",
                "--ttl", ttl, "inspect"]).is_ok());
        }
    }
}
