use fractal_daemon::{cli_print as print, cli_println as println};

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand, ValueEnum};
use fractal_client::{
    AvailabilityDeadlineClock, AvailabilityDeadlineClockChange, AvailabilityInterruptions,
    AvailabilityInterruptionsChange, AvailabilitySet, AvailabilityStatus, AvailabilityStatusChange,
    AvailabilityStatusSource, PersonAvailability, PresenceReport,
};
use fractal_daemon::client::Endpoint;

#[derive(Args)]
#[command(
    args_conflicts_with_subcommands = true,
    after_help = "Availability informs and never grants authority. A person sees their own device presence; agents and other people see the effective status, interruptions and deadline clock. Only a person can change their own availability."
)]
pub(super) struct AvailabilityArgs {
    /// Act as this person or agent; defaults to the harness identity, else configured person.
    #[arg(long, global = true, value_parser = super::parse_actor_subject)]
    actor: Option<String>,
    #[command(flatten)]
    show: ShowArgs,
    #[command(subcommand)]
    command: Option<AvailabilityCommand>,
}

#[derive(Subcommand)]
enum AvailabilityCommand {
    /// Show availability: yours as a person, the people you work for as an agent.
    Show(ShowArgs),
    /// Set your status, interruptions or deadline clock. Persons only.
    Set(SetArgs),
    /// Return axes to their defaults: status from presence, interruptions all, deadline clock run.
    Clear {
        /// The axes to clear; all three when none is named.
        #[arg(value_enum)]
        axes: Vec<Axis>,
    },
    /// Report this device's state, as a paired client does on each transition. Persons only.
    Presence {
        /// active: in use; inactive: idle at the screen; locked; sleeping.
        #[arg(value_parser = ["active", "inactive", "locked", "sleeping"])]
        state: String,
        /// The device's last user input, UTC RFC3339; the daemon counts an active device present
        /// until ten minutes after it.
        #[arg(long, value_name = "RFC3339")]
        last_input_at: Option<String>,
        /// When the device observed the state, UTC RFC3339; defaults to now.
        #[arg(long, value_name = "RFC3339")]
        observed_at: Option<String>,
    },
}

#[derive(Args)]
struct ShowArgs {
    /// Show this person only; another person's presence is never shown.
    #[arg(long, value_parser = super::parse_person_subject)]
    person: Option<String>,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("axis").required(true).multiple(true))]
struct SetArgs {
    /// Override the status your device presence would give.
    #[arg(long, group = "axis", value_parser = ["available", "away"])]
    status: Option<String>,
    /// Which asks should interrupt you.
    #[arg(long, group = "axis", value_parser = ["all", "urgent", "none"])]
    interruptions: Option<String>,
    /// Hold the deadlines of asks to you: routine asks only, or every ask.
    #[arg(long, group = "axis", value_parser = ["run", "hold-routine", "hold-all"])]
    deadline_clock: Option<String>,
    /// Keep every named axis for this long, such as `45m` or `2h`.
    #[arg(long = "for", value_name = "DURATION", conflicts_with = "until")]
    duration: Option<String>,
    /// Keep every named axis until this UTC RFC3339 time.
    #[arg(long, value_name = "RFC3339")]
    until: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
enum Axis {
    Status,
    Interruptions,
    DeadlineClock,
}

fn utc_millis(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_time(value: &str, flag: &str) -> Result<String> {
    let at = chrono::DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{flag} must be an RFC3339 time such as 2026-11-12T18:00:00Z"))?;
    Ok(utc_millis(at.with_timezone(&chrono::Utc)))
}

/// The `until` every named axis carries: `--for` from now, or `--until` as given.
fn until(args: &SetArgs, now: chrono::DateTime<chrono::Utc>) -> Result<Option<String>> {
    if let Some(duration) = args.duration.as_deref() {
        let ms = fractal_daemon::graph::parse_duration(duration, true)?;
        let ms = i64::try_from(ms).context("--for is too long")?;
        let at = now
            .checked_add_signed(chrono::TimeDelta::milliseconds(ms))
            .context("--for is too long")?;
        return Ok(Some(utc_millis(at)));
    }
    args.until.as_deref().map(|until| parse_time(until, "--until")).transpose()
}

/// One entry per named axis, each with the shared `until`.
fn set_request(args: &SetArgs, now: chrono::DateTime<chrono::Utc>) -> Result<AvailabilitySet> {
    let until = until(args, now)?;
    Ok(AvailabilitySet {
        status: args.status.clone().map(|value| AvailabilityStatusChange {
            value: Some(value),
            until: until.clone(),
        }),
        interruptions: args.interruptions.clone().map(|value| AvailabilityInterruptionsChange {
            value,
            until: until.clone(),
        }),
        deadline_clock: args.deadline_clock.clone().map(|value| AvailabilityDeadlineClockChange {
            value,
            until: until.clone(),
        }),
    })
}

/// Return `axes` (all three when empty) to their defaults.
fn clear_request(axes: &[Axis]) -> AvailabilitySet {
    let all = axes.is_empty();
    let named = |axis| all || axes.contains(&axis);
    AvailabilitySet {
        status: named(Axis::Status).then_some(AvailabilityStatusChange { value: None, until: None }),
        interruptions: named(Axis::Interruptions).then(|| AvailabilityInterruptionsChange {
            value: "all".into(),
            until: None,
        }),
        deadline_clock: named(Axis::DeadlineClock).then(|| AvailabilityDeadlineClockChange {
            value: "run".into(),
            until: None,
        }),
    }
}

/// One person's availability as an indented block; presence appears only on the person's own read.
pub(super) fn render_block(availability: &PersonAvailability) -> String {
    use std::fmt::Write as _;
    let at = |label: &str, time: &Option<String>| {
        time.as_deref().map(|time| format!(" {label} {time}")).unwrap_or_default()
    };
    let source = match availability.status_source {
        AvailabilityStatusSource::Override => "set",
        AvailabilityStatusSource::Presence => "from presence",
        AvailabilityStatusSource::Unreported => "no device reporting",
        AvailabilityStatusSource::Unknown => "source not known to this client",
    };
    let mut block = format!("{}\n", availability.person);
    let _ = writeln!(
        block,
        "  status          {} ({source}){}",
        availability.status,
        at("until", &availability.status_until)
    );
    if let Some(presence) = &availability.presence {
        let _ = writeln!(
            block,
            "  presence        {presence}{}",
            at("observed", &availability.presence_observed_at)
        );
    }
    let _ = writeln!(
        block,
        "  interruptions   {}{}",
        availability.interruptions,
        at("until", &availability.interruptions_until)
    );
    let _ = writeln!(
        block,
        "  deadline clock  {}{}{}",
        availability.deadline_clock,
        at("since", &availability.deadline_clock_since),
        at("until", &availability.deadline_clock_until)
    );
    block
}

/// Whether `fractal now` should name this availability: only when something departs from the
/// default. Any away shows, set or derived from presence: the asker should park. No device
/// reporting (`unreported`) is the default, available, and stays quiet. A word this client does
/// not know departs from the default and shows.
pub(super) fn worth_showing(availability: &PersonAvailability) -> bool {
    matches!(availability.status_source, AvailabilityStatusSource::Override | AvailabilityStatusSource::Unknown)
        || availability.status != AvailabilityStatus::Available
        || availability.interruptions != AvailabilityInterruptions::All
        || availability.deadline_clock != AvailabilityDeadlineClock::Run
}

/// One line for someone about to wait on the person: the status always, the rest only when it
/// differs from the default.
pub(super) fn summary_line(availability: &PersonAvailability) -> String {
    let at = |label: &str, time: &Option<String>| {
        time.as_deref().map(|time| format!(" {label} {time}")).unwrap_or_default()
    };
    let mut line = format!(
        "{} is {}{}",
        availability.person,
        availability.status,
        at("until", &availability.status_until)
    );
    if availability.interruptions != AvailabilityInterruptions::All {
        line.push_str(&format!(
            "; interruptions: {}{}",
            availability.interruptions,
            at("until", &availability.interruptions_until)
        ));
    }
    if availability.deadline_clock != AvailabilityDeadlineClock::Run {
        line.push_str(&format!(
            "; deadline clock: {}{}{}",
            availability.deadline_clock,
            at("since", &availability.deadline_clock_since),
            at("until", &availability.deadline_clock_until)
        ));
    }
    line
}

/// Whether the person's deadline clock holds an ask's deadline now: `hold-all` holds every ask,
/// `hold-routine` every ask that is not urgent. `None` for a clock word this client does not know.
pub(super) fn holds_deadline(availability: &PersonAvailability, urgent: bool) -> Option<bool> {
    match availability.deadline_clock {
        AvailabilityDeadlineClock::Run => Some(false),
        AvailabilityDeadlineClock::HoldRoutine => Some(!urgent),
        AvailabilityDeadlineClock::HoldAll => Some(true),
        AvailabilityDeadlineClock::Unknown => None,
    }
}

/// The asker's line about its own ask: whether it is urgent and, when the ask is known to have a
/// deadline (`--deadline` names one; a daemon with ask deadlines off refuses it) and the person's
/// availability was read, whether their deadline clock holds that deadline. Without `--deadline`
/// the client cannot tell whether the fleet gave the ask a default deadline, so it says nothing
/// about one.
pub(super) fn ask_line(urgent: bool, deadline: bool, availability: Option<&PersonAvailability>) -> String {
    let mut line = if urgent {
        "this ask is urgent (priority critical)".to_owned()
    } else {
        "this ask is not urgent".to_owned()
    };
    let Some(availability) = availability.filter(|_| deadline) else { return line };
    match holds_deadline(availability, urgent) {
        Some(true) => line.push_str(&format!("; {} holds its deadline", availability.deadline_clock)),
        Some(false) if availability.deadline_clock == AvailabilityDeadlineClock::HoldRoutine => {
            line.push_str("; its deadline runs through hold-routine")
        }
        Some(false) => {}
        None => line.push_str("; deadline clock not known to this client"),
    }
    line
}

pub(super) async fn run(
    endpoint: &Endpoint,
    configured_person: Option<&str>,
    args: AvailabilityArgs,
    json_output: bool,
) -> Result<()> {
    let actor = super::configured_actor(args.actor.as_deref(), configured_person, "availability")?;
    let writes = !matches!(args.command, None | Some(AvailabilityCommand::Show(_)));
    if let Some(own) = fractal_drivers::contracts::env("ST_AGENT").filter(|own| !own.is_empty()) {
        anyhow::ensure!(actor == own, "an agent must read availability as its own harness identity");
        anyhow::ensure!(!writes, "only a person outside an agent seat can change availability; an agent reads it with `fractal availability`");
    }
    if writes {
        anyhow::ensure!(
            actor.starts_with("person/"),
            "only a person can change their own availability; `{actor}` can only read it"
        );
        super::parse_person_subject(&actor).map_err(anyhow::Error::msg)?;
    }
    let client = super::generated_client(endpoint, Some(&actor))?;
    let now = chrono::Utc::now();
    let changed = match args.command {
        None | Some(AvailabilityCommand::Show(_)) => {
            let show = match args.command {
                Some(AvailabilityCommand::Show(show)) => show,
                _ => args.show,
            };
            let response = client.availability_list(show.person.as_deref()).await?.value;
            if json_output {
                return super::print_value(&response, true);
            }
            if response.people.is_empty() {
                println!("No availability to show for {actor}.");
            }
            let blocks = response.people.iter().map(render_block).collect::<Vec<_>>();
            print!("{}", blocks.join("\n"));
            return Ok(());
        }
        Some(AvailabilityCommand::Set(set)) => client
            .availability_set(&set_request(&set, now)?)
            .await
            .context("could not set your availability")?,
        Some(AvailabilityCommand::Clear { axes }) => client
            .availability_set(&clear_request(&axes))
            .await
            .context("could not clear your availability")?,
        Some(AvailabilityCommand::Presence { state, last_input_at, observed_at }) => {
            let reported = client
                .presence_report(&presence_request(state, last_input_at.as_deref(), observed_at.as_deref(), now)?)
                .await
                .context("could not report your presence")?
                .value;
            if json_output {
                return super::print_value(&reported, true);
            }
            print!("{}", render_block(&reported.availability));
            match &reported.report_again_at {
                Some(at) => println!("Report again at {at} if the state has not changed by then."),
                None => println!("Report again on the next state change or input."),
            }
            return Ok(());
        }
    }
    .value;
    if json_output {
        return super::print_value(&changed, true);
    }
    print!("{}", render_block(&changed));
    Ok(())
}

/// A presence report for `state`; times are normalized to UTC milliseconds, `observed_at`
/// defaulting to `now`.
fn presence_request(
    state: String,
    last_input_at: Option<&str>,
    observed_at: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PresenceReport> {
    Ok(PresenceReport {
        state,
        last_input_at: last_input_at.map(|at| parse_time(at, "--last-input-at")).transpose()?,
        observed_at: match observed_at {
            Some(at) => parse_time(at, "--observed-at")?,
            None => utc_millis(now),
        },
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
    use serde_json::json;
    use super::*;
    use fractal_client::AvailabilityPresence;

    fn parse(argv: &[&str]) -> AvailabilityArgs {
        let mut full = vec!["st", "availability"];
        full.extend(argv);
        let cli = super::super::Cli::try_parse_from(full).unwrap();
        let super::super::Command::Availability(args) = cli.command else { panic!("expected availability") };
        args
    }

    fn refused(argv: &[&str]) -> bool {
        let mut full = vec!["st", "availability"];
        full.extend(argv);
        super::super::Cli::try_parse_from(full).is_err()
    }

    fn at(time: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&chrono::Utc)
    }

    fn set_body(argv: &[&str]) -> Result<serde_json::Value> {
        let Some(AvailabilityCommand::Set(set)) = parse(argv).command else { panic!("expected set") };
        Ok(serde_json::to_value(set_request(&set, at("2026-11-12T17:00:00Z"))?)?)
    }

    #[test]
    fn availability_parser_reads_show_with_or_without_the_subcommand() {
        let bare = parse(&[]);
        assert!(bare.command.is_none() && bare.show.person.is_none() && bare.actor.is_none());
        let top = parse(&["--person", "person/ada", "--actor", "agent/helper"]);
        assert_eq!(top.show.person.as_deref(), Some("person/ada"));
        assert_eq!(top.actor.as_deref(), Some("agent/helper"));
        let show = parse(&["show", "--person", "person/ada", "--actor", "person/avery"]);
        let Some(AvailabilityCommand::Show(show_args)) = show.command else { panic!("expected show") };
        assert_eq!(show_args.person.as_deref(), Some("person/ada"));
        assert_eq!(show.actor.as_deref(), Some("person/avery"));
        assert!(refused(&["--person", "agent/helper"]));
        assert!(refused(&["--person", "person/ada", "clear"]));
        assert!(refused(&["set", "--status", "away", "--person", "person/ada"]));
    }

    #[test]
    fn availability_set_needs_an_axis_and_applies_one_until_to_each() {
        assert!(refused(&["set"]));
        assert!(refused(&["set", "--for", "1h"]));
        assert!(refused(&["set", "--status", "busy"]));
        assert!(refused(&["set", "--status", "away", "--for", "1h", "--until", "2099-01-01T00:00:00Z"]));
        assert_eq!(
            set_body(&["set", "--status", "away", "--interruptions", "urgent", "--deadline-clock", "hold-all", "--for", "1h"]).unwrap(),
            json!({
                "status": {"value": "away", "until": "2026-11-12T18:00:00.000Z"},
                "interruptions": {"value": "urgent", "until": "2026-11-12T18:00:00.000Z"},
                "deadline_clock": {"value": "hold-all", "until": "2026-11-12T18:00:00.000Z"},
            })
        );
        assert_eq!(
            set_body(&["set", "--deadline-clock", "hold-routine", "--until", "2099-01-01T01:00:00+01:00"]).unwrap(),
            json!({"deadline_clock": {"value": "hold-routine", "until": "2099-01-01T00:00:00.000Z"}})
        );
        assert_eq!(
            set_body(&["set", "--interruptions", "none"]).unwrap(),
            json!({"interruptions": {"value": "none"}})
        );
        assert!(set_body(&["set", "--status", "away", "--until", "tomorrow"]).is_err());
    }

    #[test]
    fn availability_clear_defaults_to_every_axis() {
        let Some(AvailabilityCommand::Clear { axes }) = parse(&["clear"]).command else { panic!("expected clear") };
        assert_eq!(
            serde_json::to_value(clear_request(&axes)).unwrap(),
            json!({"status": {"value": null}, "interruptions": {"value": "all"}, "deadline_clock": {"value": "run"}})
        );
        let Some(AvailabilityCommand::Clear { axes }) = parse(&["clear", "deadline-clock", "status"]).command else {
            panic!("expected clear")
        };
        assert_eq!(
            serde_json::to_value(clear_request(&axes)).unwrap(),
            json!({"status": {"value": null}, "deadline_clock": {"value": "run"}})
        );
        assert!(refused(&["clear", "presence"]));
    }

    #[test]
    fn availability_presence_reports_raw_device_states_with_utc_times() {
        let args = parse(&[
            "presence", "active", "--last-input-at", "2026-11-12T18:55:00+01:00", "--observed-at", "2026-11-12T17:00:00Z",
        ]);
        let Some(AvailabilityCommand::Presence { state, last_input_at, observed_at }) = args.command else {
            panic!("expected presence")
        };
        assert_eq!(
            serde_json::to_value(
                presence_request(state, last_input_at.as_deref(), observed_at.as_deref(), at("2026-11-12T17:01:00Z"))
                    .unwrap()
            )
            .unwrap(),
            json!({"state": "active", "last_input_at": "2026-11-12T17:55:00.000Z", "observed_at": "2026-11-12T17:00:00.000Z"})
        );
        assert_eq!(
            serde_json::to_value(presence_request("locked".into(), None, None, at("2026-11-12T17:01:00Z")).unwrap()).unwrap(),
            json!({"state": "locked", "observed_at": "2026-11-12T17:01:00.000Z"})
        );
        assert!(presence_request("active".into(), Some("soon"), None, at("2026-11-12T17:01:00Z")).is_err());
        for state in ["inactive", "sleeping"] {
            assert!(!refused(&["presence", state]), "{state}");
        }
        for derived in ["present", "idle", "unreported", "unknown"] {
            assert!(refused(&["presence", derived]), "{derived} is derived by the daemon, not reported");
        }
        assert!(refused(&["presence"]));
    }

    #[test]
    fn availability_block_shows_presence_only_when_the_read_carries_it() {
        let own: PersonAvailability = serde_json::from_value(json!({
            "person": "person/ada", "status": "away", "status_source": "override",
            "status_until": "2026-11-12T18:00:00.000Z",
            "presence": "idle", "presence_observed_at": "2026-11-12T17:00:00.000Z",
            "interruptions": "urgent", "interruptions_until": null,
            "deadline_clock": "hold-all", "deadline_clock_since": "2026-11-12T16:00:00.000Z", "deadline_clock_until": null,
        }))
        .unwrap();
        assert_eq!(
            render_block(&own),
            concat!(
                "person/ada\n",
                "  status          away (set) until 2026-11-12T18:00:00.000Z\n",
                "  presence        idle observed 2026-11-12T17:00:00.000Z\n",
                "  interruptions   urgent\n",
                "  deadline clock  hold-all since 2026-11-12T16:00:00.000Z\n",
            )
        );
        let other = PersonAvailability { presence: None, presence_observed_at: None, ..own };
        assert!(!render_block(&other).contains("presence"));
    }

    #[test]
    fn availability_block_names_an_unreported_or_unknown_source() {
        let unreported: PersonAvailability = serde_json::from_value(json!({
            "person": "person/ada", "status": "available", "status_source": "unreported", "status_until": null,
            "presence": "unreported", "presence_observed_at": null,
            "interruptions": "all", "interruptions_until": null,
            "deadline_clock": "run", "deadline_clock_since": null, "deadline_clock_until": null,
        }))
        .unwrap();
        let block = render_block(&unreported);
        assert!(block.contains("  status          available (no device reporting)\n"), "{block}");
        assert!(block.contains("  presence        unreported\n"), "{block}");
        // A word from a newer daemon still renders instead of failing the read.
        let newer: PersonAvailability = serde_json::from_value(json!({
            "person": "person/ada", "status": "in-a-meeting", "status_source": "calendar", "status_until": null,
            "presence": null, "presence_observed_at": null,
            "interruptions": "all", "interruptions_until": null,
            "deadline_clock": "run", "deadline_clock_since": null, "deadline_clock_until": null,
        }))
        .unwrap();
        assert!(render_block(&newer).contains("  status          unknown (source not known to this client)\n"));
    }

    #[test]
    fn availability_summary_names_status_always_and_only_non_default_rest() {
        let available = PersonAvailability {
            person: "person/ada".into(),
            status: AvailabilityStatus::Available,
            status_source: AvailabilityStatusSource::Presence,
            status_until: None,
            presence: None,
            presence_observed_at: None,
            interruptions: AvailabilityInterruptions::All,
            interruptions_until: None,
            deadline_clock: AvailabilityDeadlineClock::Run,
            deadline_clock_since: None,
            deadline_clock_until: None,
        };
        assert_eq!(summary_line(&available), "person/ada is available");
        assert!(!worth_showing(&available));
        let unreported = PersonAvailability {
            status_source: AvailabilityStatusSource::Unreported,
            presence: Some(AvailabilityPresence::Unreported),
            ..available.clone()
        };
        assert!(!worth_showing(&unreported), "no device reporting is the default: agents ask normally");
        assert_eq!(summary_line(&unreported), "person/ada is available");
        let presence_away = PersonAvailability { status: AvailabilityStatus::Away, ..available.clone() };
        assert!(worth_showing(&presence_away), "presence-derived away shows: the asker should park");
        assert!(worth_showing(&PersonAvailability { status: AvailabilityStatus::Unknown, ..available.clone() }));
        assert!(worth_showing(&PersonAvailability {
            status_source: AvailabilityStatusSource::Override,
            ..available.clone()
        }));
        assert!(worth_showing(&PersonAvailability {
            status_source: AvailabilityStatusSource::Unknown,
            ..available.clone()
        }));
        assert!(worth_showing(&PersonAvailability {
            interruptions: AvailabilityInterruptions::Urgent,
            ..available.clone()
        }));
        assert!(worth_showing(&PersonAvailability {
            deadline_clock: AvailabilityDeadlineClock::HoldRoutine,
            ..available.clone()
        }));
        let away = PersonAvailability {
            status: AvailabilityStatus::Away,
            status_source: AvailabilityStatusSource::Override,
            status_until: Some("2026-11-12T18:00:00.000Z".into()),
            interruptions: AvailabilityInterruptions::Urgent,
            interruptions_until: Some("2026-11-12T18:00:00.000Z".into()),
            deadline_clock: AvailabilityDeadlineClock::HoldAll,
            deadline_clock_since: Some("2026-11-12T17:00:00.000Z".into()),
            deadline_clock_until: Some("2026-11-12T18:00:00.000Z".into()),
            ..available
        };
        assert!(worth_showing(&away));
        assert_eq!(
            summary_line(&away),
            "person/ada is away until 2026-11-12T18:00:00.000Z; interruptions: urgent until 2026-11-12T18:00:00.000Z; deadline clock: hold-all since 2026-11-12T17:00:00.000Z until 2026-11-12T18:00:00.000Z"
        );
    }

    #[test]
    fn ask_line_says_whether_the_ask_is_urgent_and_whether_the_clock_holds_it() {
        let with_clock = |deadline_clock| PersonAvailability {
            person: "person/ada".into(),
            status: AvailabilityStatus::Available,
            status_source: AvailabilityStatusSource::Presence,
            status_until: None,
            presence: None,
            presence_observed_at: None,
            interruptions: AvailabilityInterruptions::All,
            interruptions_until: None,
            deadline_clock,
            deadline_clock_since: None,
            deadline_clock_until: None,
        };
        let routine = with_clock(AvailabilityDeadlineClock::HoldRoutine);
        assert_eq!(ask_line(false, true, Some(&routine)), "this ask is not urgent; hold-routine holds its deadline");
        assert_eq!(
            ask_line(true, true, Some(&routine)),
            "this ask is urgent (priority critical); its deadline runs through hold-routine"
        );
        let all = with_clock(AvailabilityDeadlineClock::HoldAll);
        assert_eq!(ask_line(true, true, Some(&all)), "this ask is urgent (priority critical); hold-all holds its deadline");
        let run = with_clock(AvailabilityDeadlineClock::Run);
        assert_eq!(holds_deadline(&run, false), Some(false));
        assert_eq!(ask_line(false, true, Some(&run)), "this ask is not urgent");
        let newer = with_clock(AvailabilityDeadlineClock::Unknown);
        assert_eq!(holds_deadline(&newer, false), None);
        assert_eq!(ask_line(false, true, Some(&newer)), "this ask is not urgent; deadline clock not known to this client");
        // Without a known deadline there is nothing to hold; without a read, only the urgency is known.
        assert_eq!(ask_line(false, false, Some(&all)), "this ask is not urgent");
        assert_eq!(ask_line(true, true, None), "this ask is urgent (priority critical)");
    }
}
