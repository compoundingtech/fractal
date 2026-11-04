//! Setup as one screen. This file decides what the checklist shows and does the work when the
//! person presses its one button; the screen itself is `fractal_tui::checklist`.
//!
//! The line-mode setup stays for pipes, `--yes` and anything that is not a terminal. Both modes
//! run the same stages (save the names, install st, start it, set up each agent, start the
//! Assistant); only who answers the questions differs.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use fractal_tui::checklist::{self, Facts, Field, Form, Outcome, Reporter, Row, Run, Tri};

use super::{
    PreparedSetup, SetupArgs, capture, check_login_path, daemon_ready, ensure_claude_channel,
    install_binaries, merge_config, said, start_daemon, validate_name,
};
use fractal_daemon::config::Config;
use fractal_daemon::environment::HARNESSES;

/// Everything the screen needs to know about this run of setup.
pub(super) struct Start {
    pub args: SetupArgs,
    pub path: PathBuf,
    pub config: Config,
    pub previous_node: String,
    pub existing_store: bool,
    pub default_person: String,
    pub default_node: String,
}

/// What the worker leaves behind for the caller.
#[derive(Default)]
struct Result_ {
    /// Whether the names were saved: setup went far enough to count.
    saved: bool,
    harness: Option<String>,
    started_assistant: bool,
    initial_subject: Option<String>,
    /// Lines setup said while the screen was up, for the terminal afterwards.
    log: Vec<String>,
    /// Harnesses that ended ready, in the order they did.
    ready: Vec<String>,
}

pub(super) async fn run(start: Start) -> Result<PreparedSetup> {
    println!("Looking for your agents…");
    let environment = fractal_daemon::environment::snapshot()?;
    let mut form = form(&start, &environment);
    if let Some(requested) = &start.args.harness {
        for row in &mut form.rows {
            row.selected = &row.id == requested && row.facts.found.is_some();
        }
        anyhow::ensure!(
            requested == "none" || form.rows.iter().any(|row| &row.id == requested && row.selected),
            "{requested} is not installed on the daemon's login PATH; install it and open a new login shell, then run fractal setup"
        );
    }

    // Where agents work, and so where a harness's sign-in handover starts; resolved once.
    let work_root = start.config.work_root();
    let shared = Mutex::new(Result_::default());
    let config = Mutex::new(start.config.clone());
    let executable = std::env::current_exe()?;
    let finished = checklist::run(
        form,
        &|field, value| {
            validate_name(value, field == Field::Machine).map_err(|error| error.to_string())
        },
        |run, reporter| work(&start, &shared, &config, &executable, run, reporter),
        |id: &str| match &work_root {
            Ok(root) => hand_over(id, root),
            Err(error) => Err(format!("the work folder cannot be used: {error:#}")),
        },
        &|| {
            fractal_daemon::environment::refresh();
            fractal_daemon::environment::snapshot()
                .map(|environment| rows(&environment))
                .unwrap_or_default()
        },
    )?;
    let outcome = shared.into_inner().unwrap_or_default();
    // The screen is gone; what setup said and how every row ended stays in the scrollback.
    for line in &outcome.log {
        println!("{line}");
    }
    for row in &finished.form.rows {
        if !row.selected {
            continue;
        }
        match &row.outcome {
            Some(Outcome::Ready(note)) => println!("{}: ready. {note}", row.name),
            Some(Outcome::NeedsLogin(note)) => println!("{}: needs you to sign in. {note}", row.name),
            Some(Outcome::Failed(error)) => println!("{}: failed. {error}", row.name),
            None => {}
        }
    }
    anyhow::ensure!(
        outcome.saved,
        "setup was cancelled; nothing was changed. Run fractal setup when you are ready"
    );
    println!("Your agents run without permission prompts inside their own workspaces.");
    Ok(PreparedSetup {
        config: config.into_inner().unwrap_or(start.config),
        harness: outcome.harness,
        initial_subject: outcome.initial_subject,
    })
}

// ---- what the screen shows ----------------------------------------------------------------

fn form(start: &Start, environment: &BTreeMap<String, String>) -> Form {
    let rows = rows(environment);
    let codex = rows
        .iter()
        .any(|row| row.id == "codex" && row.facts.found.is_some())
        .then(|| start.config.codex_work_without_asking.unwrap_or(true));
    let home = home(environment);
    let login_note = if cfg!(target_os = "macos") {
        format!(
            "Installs a launch agent for Fractal in {}/Library/LaunchAgents. No admin rights. Remove it with {} service uninstall.",
            home.display(), start.args.program()
        )
    } else {
        let config_home = environment
            .get("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        format!(
            "Installs a systemd user service at {}/systemd/user/st3.service. No admin rights. Remove it with {} service uninstall.",
            config_home.display(), start.args.program()
        )
    };
    Form {
        name: start
            .args
            .person
            .clone()
            .unwrap_or_else(|| start.default_person.clone()),
        machine: start
            .args
            .node
            .clone()
            .unwrap_or_else(|| start.default_node.clone()),
        login_label: "Start Fractal when I log in".into(),
        login: start.args.service.unwrap_or(true),
        login_note,
        linger: lingering_offered(),
        work_root: start
            .config
            .work_root()
            .map(|root| root.display().to_string())
            .unwrap_or_default(),
        rows,
        codex,
    }
}

/// "Keep running after I log out" is offered only where it can be turned on without sudo.
fn lingering_offered() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let user = std::env::var("USER").ok().filter(|user| !user.is_empty())?;
        let shown = Command::new("loginctl")
            .args(["show-user", &user, "--property=Linger", "--value"])
            .stdin(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        (String::from_utf8_lossy(&shown.stdout).trim() == "no").then_some(true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn home(environment: &BTreeMap<String, String>) -> PathBuf {
    environment
        .get("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("~"))
}

/// A row for every harness st supports, found or not.
pub(super) fn rows(environment: &BTreeMap<String, String>) -> Vec<Row> {
    let home = home(environment);
    HARNESSES
        .iter()
        .map(|&id| {
            let found = fractal_runtime::resolve_executable(id, environment).ok();
            let state = signin(id, environment, &home);
            let (name, sets_up, install_hint) = describe(id, environment, &home);
            Row {
                id: id.into(),
                name: name.into(),
                facts: Facts {
                    found: found.as_ref().map(|path| path.display().to_string()),
                    // Nothing is installed for it yet, and nothing has checked.
                    integration: Tri::Unknown,
                    signed_in: state.signed_in,
                },
                sets_up,
                install_hint,
                selected: found.is_some(),
                outcome: None,
            }
        })
        .collect()
}

/// A harness's display name, what setup does for it with the exact paths, and how to get it.
fn describe(
    id: &str,
    environment: &BTreeMap<String, String>,
    home: &Path,
) -> (&'static str, String, String) {
    let data = environment
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let dir = |variable: &str, default: &str| {
        environment
            .get(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(default))
    };
    match id {
        "claude" => (
            "Claude",
            format!(
                "Adds the Fractal channel plugin to Claude through its own plugin command (user scope, no admin rights). Writes the plugin files under {} and registers them in {}.",
                data.join("st2/claude-channel/marketplace").display(),
                dir("CLAUDE_CONFIG_DIR", ".claude").display()
            ),
            "Install Claude Code with its own installer (npm install -g @anthropic-ai/claude-code), open a new terminal, then press r to look again.".into(),
        ),
        "codex" => (
            "Codex",
            format!(
                "Checks that Codex starts and is signed in. Writes nothing under {}: seats start Codex with Fractal's own launch arguments.",
                dir("CODEX_HOME", ".codex").display()
            ),
            "Install Codex with its own installer (npm install -g @openai/codex), open a new terminal, then press r to look again.".into(),
        ),
        "opencode" => (
            "OpenCode",
            format!(
                "Checks that OpenCode starts and is signed in. Writes nothing under {}: Fractal starts its server for each seat.",
                home.join(".config/opencode").display()
            ),
            "Install OpenCode with its own installer (npm install -g opencode-ai), open a new terminal, then press r to look again.".into(),
        ),
        "pi" => (
            "Pi",
            "Checks that Pi starts. Writes nothing globally: Fractal loads its bundled extension for each seat.".into(),
            "Install Pi with its own installer so it is on your login PATH, open a new terminal, then press r to look again.".into(),
        ),
        _ => (
            "Omp",
            "Checks that Omp starts. Writes nothing globally: Fractal loads its bundled extension for each seat.".into(),
            "Install Omp with its own installer so it is on your login PATH, open a new terminal, then press r to look again.".into(),
        ),
    }
}

// ---- what is true of a harness -----------------------------------------------------------

/// Two facts about a harness's own first run, read from the files it keeps (never printed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Signin {
    /// Its first launch (theme, welcome) is finished.
    pub first_run: Tri,
    pub signed_in: Tri,
}

fn read_small(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(8 << 20)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

pub(super) fn signin(id: &str, environment: &BTreeMap<String, String>, home: &Path) -> Signin {
    let unknown = Signin {
        first_run: Tri::Unknown,
        signed_in: Tri::Unknown,
    };
    let variable = |name: &str| environment.get(name).is_some_and(|value| !value.is_empty());
    match id {
        "claude" => {
            let file = environment
                .get("CLAUDE_CONFIG_DIR")
                .filter(|dir| !dir.is_empty())
                .map(|dir| PathBuf::from(dir).join(".claude.json"))
                .unwrap_or_else(|| home.join(".claude.json"));
            let key = variable("ANTHROPIC_API_KEY") || variable("ANTHROPIC_AUTH_TOKEN");
            let Some(text) = read_small(&file) else {
                // No config at all: Claude has never run here.
                return Signin {
                    first_run: Tri::No,
                    signed_in: if key { Tri::Unknown } else { Tri::No },
                };
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                return unknown;
            };
            let first_run = json
                .get("hasCompletedOnboarding")
                .and_then(serde_json::Value::as_bool)
                .map_or(Tri::No, |done| if done { Tri::Yes } else { Tri::No });
            let signed_in = if json.get("oauthAccount").is_some_and(serde_json::Value::is_object) {
                Tri::Yes
            } else if key {
                Tri::Unknown
            } else {
                Tri::No
            };
            Signin { first_run, signed_in }
        }
        "codex" => {
            let dir = environment
                .get("CODEX_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            let has_login = read_small(&dir.join("auth.json")).is_some_and(|text| text.trim().len() > 2);
            Signin {
                first_run: Tri::Unknown,
                signed_in: if has_login {
                    Tri::Yes
                } else if variable("OPENAI_API_KEY") {
                    Tri::Unknown
                } else {
                    Tri::No
                },
            }
        }
        "opencode" => {
            let data = environment
                .get("XDG_DATA_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"));
            let has_login = read_small(&data.join("opencode/auth.json"))
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|json| json.as_object().map(|object| !object.is_empty()))
                .unwrap_or(false);
            Signin {
                first_run: Tri::Unknown,
                signed_in: if has_login { Tri::Yes } else { Tri::No },
            }
        }
        _ => unknown,
    }
}

fn needs_you(state: Signin) -> bool {
    state.first_run == Tri::No || state.signed_in == Tri::No
}

/// What setup says when a harness is installed but still not signed in after the handover.
///
/// For an instance the sign-in belongs to the instance's own Claude, Codex or OpenCode account, so
/// a plain `claude` or `codex login` typed in the person's own terminal would sign in their
/// default account and leave this check unsatisfied: the words name setup's own retry instead.
/// Without an instance the harness's own command in the person's terminal is the right one.
fn sign_in_words(id: &str, instance: Option<&str>) -> String {
    let (name, command) = match id {
        "claude" => ("Claude", "claude"),
        "codex" => ("Codex", "codex login"),
        _ => ("OpenCode", "opencode auth login"),
    };
    match (instance, id) {
        (Some(instance), "claude") => format!(
            "{name} is installed, but its sign-in is not finished for this instance. Run fractal --instance {instance} setup to try the sign-in again."
        ),
        (Some(instance), _) => format!(
            "{name} is installed, but not signed in for this instance. Run fractal --instance {instance} setup to try the sign-in again."
        ),
        (None, "claude") => format!(
            "{name} is installed, but its first launch or sign-in is not finished. Run {command} in your own terminal, sign in, then retry."
        ),
        (None, _) => format!("{name} is installed, but not signed in. Run {command} in your own terminal, then retry."),
    }
}

/// Run a harness's own first launch in this terminal, so the person can sign in, and return when
/// they leave it. The harness starts in the work folder, never in the folder setup was run from
/// (usually the home folder), and its own first-run state is written first, so its first screen is
/// the sign-in and not a theme chooser or a trust question (see [`admit_handover_folder`]).
fn hand_over(id: &str, work_root: &Path) -> Result<(), String> {
    let environment = fractal_daemon::environment::snapshot().map_err(|error| format!("{error:#}"))?;
    let program = fractal_runtime::resolve_executable(id, &environment).map_err(|error| format!("{error:#}"))?;
    let Some((arguments, note)) = handover_plan(id) else {
        return Ok(());
    };
    let work = admit_handover_folder(id, &environment, &home(&environment), work_root)?;
    println!("\n{note}\n");
    start_handover(&program, arguments, &environment, &work)
}

/// What each harness's sign-in handover runs, and what setup says before it.
fn handover_plan(id: &str) -> Option<(&'static [&'static str], &'static str)> {
    match id {
        "claude" => Some((
            &[],
            "Claude is starting so you can sign in. Leave it with /exit when you are done, and setup carries on.",
        )),
        "codex" => Some((&["login"], "Codex is asking you to sign in. Setup carries on when it finishes.")),
        "opencode" => Some((
            &["auth", "login"],
            "OpenCode is asking you to sign in. Setup carries on when it finishes.",
        )),
        _ => None,
    }
}

/// Make the work folder exist and write the first-run state Claude and Codex read at their first
/// start, in the config the harness will read (the one in `environment`, which is the instance's
/// own for an instance and the person's own otherwise): Claude's onboarding flag and the folder's
/// trust, Codex's trust entry. The writers keep their rules: only absent keys, idempotent, and no
/// login or key file is read or written. Returns the folder as the harness will see it.
fn admit_handover_folder(
    id: &str,
    environment: &BTreeMap<String, String>,
    home: &Path,
    work_root: &Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(work_root)
        .map_err(|error| format!("could not create the work folder {}: {error}", work_root.display()))?;
    let work = std::fs::canonicalize(work_root)
        .map_err(|error| format!("could not resolve the work folder {}: {error}", work_root.display()))?;
    let variable = |name: &str| environment.get(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    match id {
        "claude" => {
            let config = variable("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.to_path_buf()).join(".claude.json");
            fractal_drivers::pretrust::pretrust_at(&config, std::slice::from_ref(&work))
                .map(drop)
                .map_err(|error| format!("could not prepare Claude's first start: {error:#}"))?;
        }
        "codex" => {
            let config = variable("CODEX_HOME").unwrap_or_else(|| home.join(".codex")).join("config.toml");
            fractal_drivers::pretrust::pretrust_codex_at(&config, std::slice::from_ref(&work))
                .map(drop)
                .map_err(|error| format!("could not prepare Codex's first start: {error:#}"))?;
        }
        _ => {}
    }
    Ok(work)
}

/// Run the handover program with the login shell's environment, in `work`, until the person leaves it.
fn start_handover(
    program: &Path,
    arguments: &[&str],
    environment: &BTreeMap<String, String>,
    work: &Path,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(arguments).current_dir(work);
    for (name, value) in environment {
        // The terminal in front of the person describes itself better than a login shell does.
        if !matches!(name.as_str(), "TERM" | "COLUMNS" | "LINES") {
            command.env(name, value);
        }
    }
    command
        .status()
        .map(|_| ())
        .map_err(|error| format!("could not start {}: {error}", program.display()))
}

/// Start a program with `--version` and say what it answered; a program that cannot start is the
/// error, whole.
fn starts(program: &Path, environment: &BTreeMap<String, String>) -> Result<String, String> {
    let mut child = Command::new(program)
        .arg("--version")
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{} could not be started: {error}", program.display()))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} did not answer --version within 15 seconds", program.display()));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    if output.status.success() {
        Ok(text(&output.stdout).lines().next().unwrap_or_default().to_owned())
    } else {
        Err(format!(
            "{} --version failed ({}): {}{}",
            program.display(),
            output.status,
            text(&output.stderr),
            text(&output.stdout)
        ))
    }
}

// ---- what pressing the button does -------------------------------------------------------

fn work(
    start: &Start,
    shared: &Mutex<Result_>,
    config: &Mutex<Config>,
    executable: &Path,
    run: Run,
    reporter: &Reporter,
) -> Result<(), String> {
    // Everything setup says while the screen is up goes to its footer and then the scrollback.
    let log = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let sink_log = log.clone();
    let sink_reporter = reporter.clone();
    capture(Some(Box::new(move |line| {
        sink_reporter.step(line.lines().next().unwrap_or_default().to_owned());
        if let Ok(mut log) = sink_log.lock() {
            log.push(line);
        }
    })));
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
        .and_then(|runtime| {
            runtime
                .block_on(stages(start, shared, config, executable, run, reporter))
                .map_err(|error| format!("{error:#}"))
        });
    capture(None);
    if let (Ok(mut said), Ok(mut shared)) = (log.lock(), shared.lock()) {
        shared.log.append(&mut said);
    }
    result
}

async fn stages(
    start: &Start,
    shared: &Mutex<Result_>,
    config: &Mutex<Config>,
    executable: &Path,
    run: Run,
    reporter: &Reporter,
) -> Result<()> {
    let form = &run.form;
    validate_name(&form.name, false).context("your name")?;
    validate_name(&form.machine, true).context("this machine's name")?;
    anyhow::ensure!(
        !start.existing_store || form.machine == start.previous_node,
        "this store belongs to `{}`; setup cannot rename its machine",
        start.previous_node
    );
    let mut next = start.config.clone();
    next.person = Some(format!("person/{}", form.name));
    next.node = form.machine.clone();
    next.codex_work_without_asking = form.codex;
    next.validate()?;
    let should_start = !daemon_ready(&next).await && start.args.start.unwrap_or(true);
    fractal_daemon::service::preflight(form.login && should_start)?;

    reporter.step("Saving your name and this machine's name");
    merge_config(&start.path, &next)?;
    *config.lock().expect("config lock") = next.clone();
    shared.lock().expect("result lock").saved = true;
    said(format!("Saved {}", start.path.display()));
    if let Some(kib) = fractal_daemon::read_cache::override_kib() {
        said(format!("Read cache: {kib} KiB per reader."));
    }

    let executable = if start.args.install.unwrap_or(true) {
        reporter.step("Installing Fractal");
        install_binaries(executable)?
    } else {
        executable.to_owned()
    };
    check_login_path(&executable);

    if should_start {
        reporter.step("Starting Fractal");
        start_daemon(
            &next,
            &executable,
            &start.path,
            form.login,
            form.linger.unwrap_or(false),
            start.args.program(),
        )
        .await?;
    } else if !daemon_ready(&next).await {
        said("Configuration saved; the daemon remains stopped.");
    }
    if run.skip_agents {
        return Ok(());
    }

    let wanted = |row: &Row| {
        row.selected
            && row.facts.found.is_some()
            && run.only.as_ref().is_none_or(|only| only.contains(&row.id))
    };
    for row in form.rows.iter().filter(|row| wanted(row)) {
        let executable = executable.clone();
        let (id, name) = (row.id.clone(), row.name.clone());
        let worker = reporter.clone();
        reporter.step(format!("Setting up {name}"));
        let outcome = tokio::task::spawn_blocking(move || set_up_row(&id, &executable, &worker))
            .await
            .map_err(|error| anyhow::anyhow!("{name}: {error}"))?;
        if matches!(outcome, Outcome::Ready(_)) {
            shared.lock().expect("result lock").ready.push(row.id.clone());
        }
        reporter.row(&row.id, outcome);
    }

    // The Assistant starts while the screen is still up, once something is ready to run it.
    let (harness, ready, started) = {
        let shared = shared.lock().expect("result lock");
        (shared.ready.first().cloned(), shared.ready.clone(), shared.started_assistant)
    };
    if let (Some(harness), false) = (harness, started) {
        if daemon_ready(&next).await {
            reporter.step("Starting the Assistant");
            let subject =
                crate::onboarding::start(&next, &harness, &ready, start.args.onboarding, start.args.program()).await?;
            let mut shared = shared.lock().expect("result lock");
            shared.started_assistant = true;
            shared.harness = Some(harness);
            shared.initial_subject = subject;
        } else {
            anyhow::ensure!(
                !start.args.onboarding,
                "rerunning onboarding needs a running daemon; pass --start true"
            );
            said(format!("Start the Fractal daemon and run {} setup to begin onboarding.", start.args.program()));
        }
    }
    Ok(())
}

/// Set up one agent and say how it ended. Runs on its own thread: it starts programs and may hand
/// the terminal to the person.
fn set_up_row(id: &str, executable: &Path, reporter: &Reporter) -> Outcome {
    let environment = match fractal_daemon::environment::snapshot() {
        Ok(environment) => environment,
        Err(error) => return Outcome::Failed(format!("{error:#}")),
    };
    let home = home(&environment);
    let Ok(program) = fractal_runtime::resolve_executable(id, &environment) else {
        return Outcome::Failed(format!("{id} is no longer on the login PATH"));
    };
    let mut facts = Facts {
        found: Some(program.display().to_string()),
        integration: Tri::Unknown,
        signed_in: Tri::Unknown,
    };
    let mut note = String::new();
    if id == "claude" {
        if let Err(error) = ensure_claude_channel(executable) {
            return Outcome::Failed(format!("The Fractal channel plugin could not be installed: {error}"));
        }
        facts.integration = Tri::Yes;
        note.push_str(if fractal_drivers::claude_channel::fractal_policy_available() {
            "The Fractal channel is installed and the managed channel policy is present."
        } else {
            "The Fractal channel is installed. Seats start with Claude's development-channel flag, and Fractal accepts its one-time dialog."
        });
    } else {
        match starts(&program, &environment) {
            Ok(version) => {
                facts.integration = Tri::Yes;
                note.push_str(&format!("{} starts{}.", id, if version.is_empty() { String::new() } else { format!(" ({version})") }));
            }
            Err(error) => return Outcome::Failed(error),
        }
    }
    let mut state = signin(id, &environment, &home);
    if needs_you(state) && matches!(id, "claude" | "codex" | "opencode") {
        reporter.facts(id, Facts { signed_in: state.signed_in, ..facts.clone() });
        reporter.handover(id);
        fractal_daemon::environment::refresh();
        state = signin(id, &environment, &home);
    }
    facts.signed_in = state.signed_in;
    reporter.facts(id, facts);
    if needs_you(state) {
        Outcome::NeedsLogin(sign_in_words(id, fractal_daemon::instance::current().as_ref().map(|instance| instance.name())))
    } else {
        let tail = match state.signed_in {
            Tri::Yes => " You are signed in.",
            _ => " Sign-in is checked when the first agent starts.",
        };
        Outcome::Ready(format!("{note}{tail}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(home: &Path) -> BTreeMap<String, String> {
        BTreeMap::from([("HOME".to_owned(), home.display().to_string())])
    }

    #[test]
    fn claude_is_signed_in_only_when_its_config_says_so() {
        let home = tempfile::tempdir().unwrap();
        let env = environment(home.path());
        let state = signin("claude", &env, home.path());
        assert_eq!((state.first_run, state.signed_in), (Tri::No, Tri::No), "never run");
        std::fs::write(home.path().join(".claude.json"), r#"{"hasCompletedOnboarding":true}"#).unwrap();
        let state = signin("claude", &env, home.path());
        assert_eq!((state.first_run, state.signed_in), (Tri::Yes, Tri::No), "first run done, no account");
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"hasCompletedOnboarding":true,"oauthAccount":{"emailAddress":"ada@example.com"}}"#,
        )
        .unwrap();
        let state = signin("claude", &env, home.path());
        assert_eq!((state.first_run, state.signed_in), (Tri::Yes, Tri::Yes));
        assert!(!needs_you(state));
        std::fs::write(home.path().join(".claude.json"), "{ not json").unwrap();
        assert_eq!(signin("claude", &env, home.path()).signed_in, Tri::Unknown, "unreadable is unknown, not signed out");
    }

    #[test]
    fn an_api_key_in_the_login_environment_is_not_called_signed_out() {
        let home = tempfile::tempdir().unwrap();
        let mut env = environment(home.path());
        env.insert("ANTHROPIC_API_KEY".into(), "sk-example".into());
        assert_eq!(signin("claude", &env, home.path()).signed_in, Tri::Unknown);
        let mut env = environment(home.path());
        env.insert("OPENAI_API_KEY".into(), "sk-example".into());
        assert_eq!(signin("codex", &env, home.path()).signed_in, Tri::Unknown);
    }

    #[test]
    fn claude_config_dir_moves_where_the_config_is_read() {
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(
            other.path().join(".claude.json"),
            r#"{"hasCompletedOnboarding":true,"oauthAccount":{}}"#,
        )
        .unwrap();
        let mut env = environment(home.path());
        env.insert("CLAUDE_CONFIG_DIR".into(), other.path().display().to_string());
        assert_eq!(signin("claude", &env, home.path()).signed_in, Tri::Yes);
    }

    #[test]
    fn codex_and_opencode_read_their_own_login_files() {
        let home = tempfile::tempdir().unwrap();
        let env = environment(home.path());
        assert_eq!(signin("codex", &env, home.path()).signed_in, Tri::No);
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/auth.json"), r#"{"tokens":{}}"#).unwrap();
        assert_eq!(signin("codex", &env, home.path()).signed_in, Tri::Yes);
        assert_eq!(signin("opencode", &env, home.path()).signed_in, Tri::No);
        std::fs::create_dir_all(home.path().join(".local/share/opencode")).unwrap();
        std::fs::write(home.path().join(".local/share/opencode/auth.json"), r#"{"example":{"type":"api"}}"#).unwrap();
        assert_eq!(signin("opencode", &env, home.path()).signed_in, Tri::Yes);
        // Pi and Omp keep sign-in in ways st does not read: it says it does not know.
        assert_eq!(signin("pi", &env, home.path()).signed_in, Tri::Unknown);
        assert_eq!(signin("omp", &env, home.path()).signed_in, Tri::Unknown);
    }

    #[test]
    fn every_supported_harness_has_a_row_and_only_found_ones_start_selected() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["claude", "omp"] {
            let path = bin.join(name);
            std::fs::write(&path, "#!/bin/sh\necho 1.0\n").unwrap();
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        let mut env = environment(home.path());
        env.insert("PATH".into(), bin.display().to_string());
        let rows = rows(&env);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["claude", "codex", "opencode", "pi", "omp"]
        );
        let found = rows.iter().filter(|row| row.facts.found.is_some()).map(|row| row.id.as_str()).collect::<Vec<_>>();
        assert_eq!(found, ["claude", "omp"]);
        assert!(rows.iter().all(|row| row.selected == row.facts.found.is_some()));
        assert!(rows.iter().all(|row| row.facts.integration == Tri::Unknown), "nothing has checked an integration yet");
        let claude = &rows[0];
        assert!(claude.sets_up.contains("claude-channel/marketplace"), "{}", claude.sets_up);
        assert!(claude.sets_up.contains(&format!("{}/.claude", home.path().display())), "{}", claude.sets_up);
        assert!(rows[1].install_hint.contains("press r"), "a missing agent says how to get it");
    }

    #[test]
    fn a_program_that_cannot_start_is_reported_whole() {
        let home = tempfile::tempdir().unwrap();
        let broken = home.path().join("codex");
        std::fs::write(&broken, "#!/bin/sh\necho 'cannot load libexample.so' >&2\nexit 3\n").unwrap();
        std::fs::set_permissions(&broken, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let error = starts(&broken, &BTreeMap::new()).unwrap_err();
        assert!(error.contains("cannot load libexample.so"), "{error}");
        assert!(error.contains("exit status: 3") || error.contains("3"), "{error}");
        let good = home.path().join("good");
        std::fs::write(&good, "#!/bin/sh\necho 'good 2.5'\n").unwrap();
        std::fs::set_permissions(&good, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert_eq!(starts(&good, &BTreeMap::new()).unwrap(), "good 2.5");
        assert!(starts(&home.path().join("absent"), &BTreeMap::new()).unwrap_err().contains("could not be started"));
    }

    /// A person's own default folders, with files that only they would write.
    fn default_folders(home: &Path) -> (Vec<u8>, Vec<u8>) {
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".claude/settings.json"), b"{\"note\":\"Avery's own Claude settings\"}").unwrap();
        std::fs::write(home.join(".codex/config.toml"), b"# Avery's own Codex config\n").unwrap();
        (
            std::fs::read(home.join(".claude/settings.json")).unwrap(),
            std::fs::read(home.join(".codex/config.toml")).unwrap(),
        )
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_claude_handover_writes_first_run_state_in_the_instance_config_only() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let (settings, _) = default_folders(&home);
        let instance = tmp.path().join("instance");
        let work_root = instance.join("work");
        let mut env = environment(&home);
        env.insert("CLAUDE_CONFIG_DIR".into(), instance.join("claude").display().to_string());

        let work = admit_handover_folder("claude", &env, &home, &work_root).unwrap();

        assert_eq!(work, std::fs::canonicalize(&work_root).unwrap(), "the work folder was not created");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(instance.join("claude/.claude.json")).unwrap()).unwrap();
        assert_eq!(config["hasCompletedOnboarding"], true);
        assert_eq!(config["projects"][work.to_str().unwrap()]["hasTrustDialogAccepted"], true);
        assert!(config.get("oauthAccount").is_none(), "setup must not write a login");
        // The person's default Claude folder and config are untouched.
        assert_eq!(std::fs::read(home.join(".claude/settings.json")).unwrap(), settings);
        assert_eq!(listing(&home.join(".claude")), ["settings.json"]);
        assert!(!home.join(".claude.json").exists(), "the default ~/.claude.json was written");
        // A second preparation changes nothing.
        let first = std::fs::read(instance.join("claude/.claude.json")).unwrap();
        admit_handover_folder("claude", &env, &home, &work_root).unwrap();
        assert_eq!(std::fs::read(instance.join("claude/.claude.json")).unwrap(), first);
    }

    #[test]
    fn the_codex_handover_trusts_the_work_folder_in_the_instance_codex_home_only() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let (_, codex) = default_folders(&home);
        let instance = tmp.path().join("instance");
        let work_root = instance.join("work");
        let mut env = environment(&home);
        env.insert("CODEX_HOME".into(), instance.join("codex").display().to_string());

        let work = admit_handover_folder("codex", &env, &home, &work_root).unwrap();

        let written = std::fs::read_to_string(instance.join("codex/config.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&written).unwrap();
        assert_eq!(parsed["projects"][work.to_str().unwrap()]["trust_level"].as_str(), Some("trusted"));
        assert_eq!(listing(&instance.join("codex")), ["config.toml"], "a login file was created");
        // The person's default Codex folder is untouched.
        assert_eq!(std::fs::read(home.join(".codex/config.toml")).unwrap(), codex);
        assert_eq!(listing(&home.join(".codex")), ["config.toml"]);
        // A second preparation changes nothing.
        admit_handover_folder("codex", &env, &home, &work_root).unwrap();
        assert_eq!(std::fs::read_to_string(instance.join("codex/config.toml")).unwrap(), written);
    }

    #[test]
    fn a_harness_with_no_first_run_state_to_write_still_gets_the_work_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let work_root = tmp.path().join("work");

        let work = admit_handover_folder("opencode", &environment(&home), &home, &work_root).unwrap();

        assert_eq!(work, std::fs::canonicalize(&work_root).unwrap());
        assert_eq!(listing(&home), Vec::<String>::new());
    }

    #[test]
    fn the_handover_starts_in_the_work_folder_after_the_state_is_written() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        default_folders(&home);
        let instance = tmp.path().join("instance");
        let mut env = environment(&home);
        env.insert("CLAUDE_CONFIG_DIR".into(), instance.join("claude").display().to_string());
        // Stands in for `claude`: records where it started and what its config held at that moment.
        let program = tmp.path().join("claude");
        let seen = tmp.path().join("seen");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\npwd -P > '{seen}.cwd'\ncp \"$CLAUDE_CONFIG_DIR/.claude.json\" '{seen}.config'\n",
                seen = seen.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        let work = admit_handover_folder("claude", &env, &home, &instance.join("work")).unwrap();
        start_handover(&program, &[], &env, &work).unwrap();

        let started_in = std::fs::read_to_string(format!("{}.cwd", seen.display())).unwrap();
        assert_eq!(Path::new(started_in.trim()), work, "the handover did not start in the work folder");
        assert_ne!(Path::new(started_in.trim()), std::fs::canonicalize(&home).unwrap());
        let held: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{}.config", seen.display())).unwrap()).unwrap();
        assert_eq!(held["hasCompletedOnboarding"], true, "the state was not written before the handover");
    }

    #[test]
    fn the_handover_words_ask_only_for_a_sign_in() {
        let (arguments, note) = handover_plan("claude").unwrap();
        assert!(arguments.is_empty());
        assert!(note.contains("sign in"), "{note}");
        assert!(!note.to_lowercase().contains("theme"), "{note}");
        assert_eq!(handover_plan("codex").unwrap().0, ["login"]);
        assert_eq!(handover_plan("opencode").unwrap().0, ["auth", "login"]);
        assert!(handover_plan("pi").is_none());
    }

    #[test]
    fn an_instances_closing_words_name_setups_retry_and_never_the_default_folders() {
        for id in ["claude", "codex", "opencode"] {
            let words = sign_in_words(id, Some("trial"));
            assert!(words.contains("fractal --instance trial setup"), "{id}: {words}");
            assert!(!words.to_lowercase().contains("theme"), "{id}: {words}");
            // A plain harness command in the person's terminal would sign in their default account.
            assert!(!words.contains("your own terminal"), "{id}: {words}");
            assert!(!words.contains("Run claude") && !words.contains("Run codex") && !words.contains("Run opencode"), "{id}: {words}");
        }
        assert!(sign_in_words("claude", Some("trial")).starts_with("Claude is installed"));
        assert!(sign_in_words("codex", Some("trial")).starts_with("Codex is installed"));
        assert!(sign_in_words("opencode", Some("trial")).starts_with("OpenCode is installed"));
    }

    #[test]
    fn without_an_instance_the_closing_words_name_the_harness_command_in_the_persons_terminal() {
        let claude = sign_in_words("claude", None);
        assert!(claude.contains("Run claude in your own terminal"), "{claude}");
        assert!(!claude.to_lowercase().contains("theme"), "no theme to pick once the first-run state is written: {claude}");
        assert!(sign_in_words("codex", None).contains("Run codex login in your own terminal"));
        assert!(sign_in_words("opencode", None).contains("Run opencode auth login in your own terminal"));
        for id in ["claude", "codex", "opencode"] {
            assert!(!sign_in_words(id, None).contains("--instance"), "{id}");
        }
    }
}
