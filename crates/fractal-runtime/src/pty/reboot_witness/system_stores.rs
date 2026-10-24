use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const MAX_SYSTEM_STORE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SYSTEM_FILE_BYTES: usize = 1024 * 1024;
const MAX_SYSTEM_ENTRIES: usize = 16_384;
const SYSTEM_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

type Observations = BTreeMap<String, Vec<u8>>;

/// A complete, stable effective user-manager view is required, not an approximation
/// of unit-file precedence. All diagnostics deliberately omit command/file contents.
pub(super) fn scan_system_stores(home: &Path, operation: &str) -> Result<Vec<RebootWitnessScan>> {
    if !cfg!(target_os = "linux") {
        anyhow::bail!("system reboot stores require Linux");
    }
    if operation.is_empty() {
        anyhow::bail!("missing reboot operation label");
    }
    let first = snapshot(home, operation)?;
    let second = snapshot(home, operation)?;
    if first != second {
        anyhow::bail!("system reboot stores changed during inspection");
    }
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    for (key, value) in &first {
        hash.update((key.len() as u64).to_be_bytes());
        hash.update(key.as_bytes());
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
        bytes += value.len() as u64;
    }
    Ok(vec![RebootWitnessScan {
        store: "system-user-launch-stores".to_owned(),
        records: first.len() as u64,
        bytes,
        digest: format!("{:x}", hash.finalize()),
        exclusions: Vec::new(),
    }])
}

fn record(observations: &mut Observations, key: String, bytes: Vec<u8>, operation: &str) -> Result<()> {
    inspect(&bytes, operation)?;
    if observations.len() >= MAX_SYSTEM_ENTRIES
        || observations.values().map(Vec::len).sum::<usize>() + bytes.len() > MAX_SYSTEM_STORE_BYTES
    {
        anyhow::bail!("system reboot store inspection exceeds bounds");
    }
    if observations.insert(key, bytes).is_some() {
        anyhow::bail!("duplicate system reboot store observation");
    }
    Ok(())
}

fn inspect(bytes: &[u8], operation: &str) -> Result<()> {
    if bytes.windows(operation.len()).any(|window| window == operation.as_bytes()) {
        anyhow::bail!("revoked operation remains in a system reboot store");
    }
    Ok(())
}

fn text(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("invalid system reboot store encoding"))
}

fn normalized_lines(bytes: &[u8]) -> Result<Vec<u8>> {
    let value = text(bytes)?;
    if !value.is_empty() && !value.ends_with('\n') {
        anyhow::bail!("incomplete system manager output");
    }
    let mut lines: Vec<_> = value.lines().collect();
    lines.sort_unstable();
    Ok(lines.join("\n").into_bytes())
}

struct CommandResult {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn read_command_pipe(
    pipe: impl Read + Send + 'static,
    is_stdout: bool,
    sender: std::sync::mpsc::Sender<(bool, bool, Vec<u8>)>,
) {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe.take((MAX_SYSTEM_FILE_BYTES + 1) as u64).read_to_end(&mut bytes);
        let _ = sender.send((is_stdout, result.is_ok(), bytes));
    });
}

// Like pty::output_within, commands have a wall-clock deadline. Unlike the
// general helper, readers also cap allocation before collecting command output.
fn command(program: &str, args: &[&str]) -> Result<CommandResult> {
    let deadline = Instant::now() + SYSTEM_COMMAND_TIMEOUT;
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .env("SYSTEMD_COLORS", "0")
        .env("SYSTEMD_URLIFY", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| anyhow::anyhow!("system reboot inspection command unavailable"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("missing inspection output"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("missing inspection output"))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    read_command_pipe(stdout, true, sender.clone());
    read_command_pipe(stderr, false, sender);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                // Reaping must not extend the caller's inspection deadline.
                std::thread::spawn(move || { let _ = child.wait(); });
                anyhow::bail!("system reboot inspection command failed or timed out");
            }
        }
    };
    let mut stdout = None;
    let mut stderr = None;
    for _ in 0..2 {
        let (is_stdout, success, bytes) = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| anyhow::anyhow!("system reboot inspection output timed out"))?;
        if !success || bytes.len() > MAX_SYSTEM_FILE_BYTES {
            anyhow::bail!("system reboot inspection output unreadable or oversized");
        }
        if is_stdout { stdout = Some(bytes); } else { stderr = Some(bytes); }
    }
    Ok(CommandResult {
        status,
        stdout: stdout.ok_or_else(|| anyhow::anyhow!("missing inspection output"))?,
        stderr: stderr.ok_or_else(|| anyhow::anyhow!("missing inspection output"))?,
    })
}

fn systemctl(args: &[&str]) -> Result<Vec<u8>> {
    let output = command("systemctl", args)?;
    if !output.status.success() || !output.stderr.is_empty() {
        anyhow::bail!("effective systemd user-manager inspection failed");
    }
    Ok(output.stdout)
}

fn unit_name(value: &str) -> bool {
    !value.starts_with('-') && value.contains('.') && value.is_ascii()
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b":_.@\\-".contains(&byte))
}

fn inventory(bytes: &[u8], files: bool) -> Result<BTreeSet<String>> {
    let value = text(bytes)?;
    if !value.is_empty() && !value.ends_with('\n') {
        anyhow::bail!("incomplete systemd unit inventory");
    }
    let mut units = BTreeSet::new();
    for line in value.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        let valid = if files {
            (fields.len() == 2 || fields.len() == 3)
                && fields[1..].iter().all(|state| matches!(*state,
                    "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
                    | "masked" | "masked-runtime" | "static" | "disabled" | "indirect"
                    | "generated" | "transient" | "bad" | "-"))
        } else {
            fields.len() >= 5
                && matches!(fields[1], "loaded" | "not-found" | "bad-setting" | "error" | "masked" | "merged" | "stub")
                && matches!(fields[2], "active" | "reloading" | "inactive" | "failed" | "activating" | "deactivating" | "maintenance" | "refreshing")
        };
        if !valid || !unit_name(fields[0]) || !units.insert(fields[0].to_owned()) || units.len() > MAX_SYSTEM_ENTRIES {
            anyhow::bail!("unrecognized systemd unit inventory");
        }
    }
    Ok(units)
}

fn properties(bytes: &[u8]) -> Result<BTreeMap<&str, &str>> {
    let value = text(bytes)?;
    if !value.ends_with('\n') {
        anyhow::bail!("incomplete effective systemd properties");
    }
    let mut result = BTreeMap::new();
    for line in value.lines() {
        let (key, value) = line.split_once('=').ok_or_else(|| anyhow::anyhow!("malformed effective systemd properties"))?;
        if !matches!(key, "Environment" | "EnvironmentFiles" | "FragmentPath" | "SourcePath") || result.insert(key, value).is_some() {
            anyhow::bail!("unknown effective systemd property schema");
        }
    }
    if !result.contains_key("FragmentPath") || !result.contains_key("SourcePath")
        || result.contains_key("Environment") != result.contains_key("EnvironmentFiles")
    {
        anyhow::bail!("incomplete effective systemd properties");
    }
    Ok(result)
}

// systemctl's EnvironmentFiles property prints path (ignore_errors=yes/no).
// Refuse escaping, specifiers and ambiguous whitespace instead of guessing.
fn environment_files(value: &str) -> Result<Vec<&str>> {
    let mut remaining = value;
    let mut paths = Vec::new();
    while !remaining.is_empty() {
        let (path, rest) = remaining.split_once(" (ignore_errors=")
            .ok_or_else(|| anyhow::anyhow!("unknown systemd EnvironmentFiles schema"))?;
        let rest = rest.strip_prefix("yes)").or_else(|| rest.strip_prefix("no)"))
            .ok_or_else(|| anyhow::anyhow!("unknown systemd EnvironmentFiles schema"))?;
        if !safe_absolute_path(path) {
            anyhow::bail!("unsupported systemd environment file path");
        }
        paths.push(path);
        remaining = if rest.is_empty() { rest } else {
            rest.strip_prefix(' ').ok_or_else(|| anyhow::anyhow!("unknown systemd EnvironmentFiles schema"))?
        };
    }
    Ok(paths)
}

fn safe_absolute_path(path: &str) -> bool {
    path.starts_with('/') && !path.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '%' | '*' | '?' | '[' | ']' | '"' | '\''))
        && !Path::new(path).components().any(|part| matches!(part, std::path::Component::ParentDir))
}

fn effective_launch(bytes: &[u8]) -> Result<()> {
    let expected = BTreeSet::from(["ExecStart", "ExecStartPre", "ExecStartPost", "ExecCondition", "ExecReload", "ExecStop", "ExecStopPost"]);
    let value = text(bytes)?;
    let mut found = BTreeSet::new();
    for line in value.lines() {
        let (key, _) = line.split_once('=').ok_or_else(|| anyhow::anyhow!("malformed effective launch properties"))?;
        if !expected.contains(key) || !found.insert(key) {
            anyhow::bail!("unknown effective launch properties");
        }
    }
    if !value.ends_with('\n') || found != expected {
        anyhow::bail!("incomplete effective launch properties");
    }
    Ok(())
}

fn environment_file(bytes: &[u8]) -> Result<()> {
    for line in text(bytes)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') { continue; }
        let (key, value) = line.split_once('=').ok_or_else(|| anyhow::anyhow!("malformed systemd environment file"))?;
        let key = key.trim();
        if key.is_empty() || key.starts_with(|c: char| c.is_ascii_digit())
            || !key.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || value.contains('\0') || value.ends_with('\\')
        {
            anyhow::bail!("unsupported systemd environment file schema");
        }
        let value = value.trim();
        if let Some(quote) = value.chars().next().filter(|c| matches!(c, '\'' | '"')) {
            if value.len() < 2 || !value.ends_with(quote) {
                anyhow::bail!("unsupported multiline systemd environment file");
            }
        }
    }
    Ok(())
}

fn reject_symlink_components(path: &Path, allow_missing: bool) -> Result<()> {
    let mut prefix = std::path::PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            anyhow::bail!("unsafe system launch path");
        }
        prefix.push(component);
        let metadata = match std::fs::symlink_metadata(&prefix) {
            Ok(metadata) => metadata,
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => anyhow::bail!("system launch input is unreadable"),
        };
        if metadata.file_type().is_symlink() {
            anyhow::bail!("unsafe symlink in system launch input");
        }
    }
    Ok(())
}

fn open_launch_file(path: &Path) -> Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    if !path.is_absolute() { anyhow::bail!("unsupported system launch input path"); }
    let mut file = std::fs::File::open("/")
        .map_err(|_| anyhow::anyhow!("system launch input is unreadable"))?;
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            if matches!(component, std::path::Component::RootDir) { continue; }
            anyhow::bail!("unsafe system launch input path");
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| anyhow::anyhow!("unsupported system launch input path"))?;
        let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        if components.peek().is_some() { flags |= libc::O_DIRECTORY; }
        // Each component is opened relative to the already-open parent, so
        // symlink swaps cannot redirect the inspection between checks.
        let descriptor = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 { anyhow::bail!("system launch input is unreadable or unsafe"); }
        // A successful openat transfers this fresh descriptor to File.
        file = unsafe { std::fs::File::from_raw_fd(descriptor) };
    }
    Ok(file)
}

fn read_regular(path: &Path) -> Result<Vec<u8>> {
    let file = open_launch_file(path)?;
    let metadata = file.metadata().map_err(|_| anyhow::anyhow!("system launch input is unreadable"))?;
    if !metadata.is_file() || metadata.len() > MAX_SYSTEM_FILE_BYTES as u64 {
        anyhow::bail!("system launch input is not a bounded regular file");
    }
    let mut bytes = Vec::new();
    file.take((MAX_SYSTEM_FILE_BYTES + 1) as u64).read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("system launch input is unreadable"))?;
    if bytes.len() > MAX_SYSTEM_FILE_BYTES {
        anyhow::bail!("system launch input exceeds bounds");
    }
    Ok(bytes)
}

fn observe_file(observations: &mut Observations, path: &Path, operation: &str) -> Result<()> {
    let key = path.to_str().ok_or_else(|| anyhow::anyhow!("unsupported system launch path"))?.to_owned();
    if !observations.contains_key(&key) {
        record(observations, key, read_regular(path)?, operation)?;
    }
    Ok(())
}

fn desktop(bytes: &[u8]) -> Result<()> {
    let mut section = None;
    let mut keys = BTreeSet::new();
    let mut has_type = false;
    let mut has_exec = false;
    for line in text(bytes)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            if name != "Desktop Entry" && !name.starts_with("Desktop Action ") {
                anyhow::bail!("unknown XDG autostart section");
            }
            section = Some(name);
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| anyhow::anyhow!("malformed XDG autostart entry"))?;
        let section = section.ok_or_else(|| anyhow::anyhow!("missing XDG desktop section"))?;
        let base = key.split('[').next().unwrap_or(key);
        if !matches!(base, "Type" | "Version" | "Name" | "GenericName" | "NoDisplay" | "Comment" | "Icon" | "Hidden" | "OnlyShowIn" | "NotShowIn" | "DBusActivatable" | "TryExec" | "Exec" | "Path" | "Terminal" | "Actions" | "MimeType" | "Categories" | "Implements" | "Keywords" | "StartupNotify" | "StartupWMClass" | "URL")
            && !base.starts_with("X-")
        {
            anyhow::bail!("unknown XDG autostart key");
        }
        if !keys.insert((section, key)) || value.contains('\0') {
            anyhow::bail!("malformed XDG autostart entry");
        }
        if section == "Desktop Entry" && key == "Type" {
            if value != "Application" { anyhow::bail!("unsupported XDG autostart type"); }
            has_type = true;
        }
        if section == "Desktop Entry" && key == "Exec" { has_exec = !value.is_empty(); }
        if matches!(base, "Hidden" | "NoDisplay" | "DBusActivatable" | "Terminal" | "StartupNotify") && !matches!(value, "true" | "false") {
            anyhow::bail!("malformed XDG autostart boolean");
        }
    }
    // DBus activation without an explicit Exec requires another restore schema.
    if !has_type || !has_exec { anyhow::bail!("incomplete XDG autostart launch schema"); }
    Ok(())
}

fn directory(observations: &mut Observations, path: &Path, generators: bool, operation: &str) -> Result<()> {
    reject_symlink_components(path, true)?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return record(observations, format!("directory:{}", path.to_string_lossy()), b"absent".to_vec(), operation);
        }
        Err(_) => anyhow::bail!("system launch directory is unreadable"),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("unsafe system launch directory");
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|_| anyhow::anyhow!("system launch directory is unreadable"))? {
        let entry = entry.map_err(|_| anyhow::anyhow!("system launch directory enumeration failed"))?;
        if entries.len() >= MAX_SYSTEM_ENTRIES { anyhow::bail!("system launch directory exceeds bounds"); }
        entries.push(entry.path());
    }
    entries.sort();
    record(observations, format!("directory:{}", path.to_string_lossy()),
        entries.iter().map(|entry| entry.to_str().ok_or_else(|| anyhow::anyhow!("unsupported system launch path")))
            .collect::<Result<Vec<_>>>()?.join("\n").into_bytes(), operation)?;
    for entry in entries {
        if generators { anyhow::bail!("unknown systemd user generator output"); }
        if entry.extension().and_then(|value| value.to_str()) != Some("desktop") {
            anyhow::bail!("unknown XDG autostart content");
        }
        let bytes = read_regular(&entry)?;
        inspect(&bytes, operation)?;
        desktop(&bytes)?;
        record(observations, entry.to_str().ok_or_else(|| anyhow::anyhow!("unsupported system launch path"))?.to_owned(), bytes, operation)?;
    }
    Ok(())
}

fn cron_output(success: bool, code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<Vec<u8>> {
    if success && stderr.is_empty() {
        text(stdout)?;
        return Ok(stdout.to_vec());
    }
    // Vixie/Cronie's conventional absence is exit 1, empty stdout and precisely
    // one diagnostic naming the invoking account. No generic exit-1 fallback.
    let diagnostic = text(stderr)?;
    let account = diagnostic.strip_prefix("no crontab for ").and_then(|value| value.strip_suffix('\n'));
    if code == Some(1) && stdout.is_empty() && account.is_some_and(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))) {
        return Ok(Vec::new());
    }
    anyhow::bail!("user crontab inspection failed or absence is ambiguous")
}

fn snapshot(home: &Path, operation: &str) -> Result<Observations> {
    let mut observations = Observations::new();
    let listed = systemctl(&["--user", "list-units", "--all", "--plain", "--no-legend", "--no-pager"])?;
    let files = systemctl(&["--user", "list-unit-files", "--no-legend", "--no-pager"])?;
    let mut units = inventory(&listed, false)?;
    units.extend(inventory(&files, true)?);
    record(&mut observations, "systemd:list-units".to_owned(), normalized_lines(&listed)?, operation)?;
    record(&mut observations, "systemd:list-unit-files".to_owned(), normalized_lines(&files)?, operation)?;
    let manager = systemctl(&["--user", "show-environment"])?;
    for line in text(&manager)?.lines() {
        let (name, _) = line.split_once('=').ok_or_else(|| anyhow::anyhow!("malformed manager environment"))?;
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
            anyhow::bail!("malformed manager environment");
        }
    }
    record(&mut observations, "systemd:manager-environment".to_owned(), normalized_lines(&manager)?, operation)?;
    for unit in units {
        let output = systemctl(&["--user", "show", "--all", "--no-pager", "--property=Environment,EnvironmentFiles,FragmentPath,SourcePath", "--", &unit])?;
        inspect(&output, operation)?;
        let properties = properties(&output)?;
        if !properties.contains_key("Environment") && ![".target", ".timer", ".path", ".slice", ".device", ".automount"].iter().any(|suffix| unit.ends_with(*suffix)) {
            anyhow::bail!("missing effective unit environment schema");
        }
        // Exec directives can come from drop-ins, not just FragmentPath.
        if unit.ends_with(".service") {
            let launch = systemctl(&["--user", "show", "--all", "--no-pager", "--property=ExecStart,ExecStartPre,ExecStartPost,ExecCondition,ExecReload,ExecStop,ExecStopPost", "--", &unit])?;
            effective_launch(&launch)?;
            record(&mut observations, format!("systemd:launch:{unit}"), normalized_lines(&launch)?, operation)?;
        }
        for path in environment_files(properties.get("EnvironmentFiles").copied().unwrap_or(""))? {
            let bytes = read_regular(Path::new(path))?;
            inspect(&bytes, operation)?;
            environment_file(&bytes)?;
            if !observations.contains_key(path) {
                record(&mut observations, path.to_owned(), bytes, operation)?;
            }
        }
        for property in ["FragmentPath", "SourcePath"] {
            let path = properties[property];
            if path.is_empty() { continue; }
            if !safe_absolute_path(path) { anyhow::bail!("unsupported systemd launch source path"); }
            if property == "SourcePath" && !path.ends_with(".desktop") && !path.ends_with(".service") {
                anyhow::bail!("unknown systemd launch source schema");
            }
            observe_file(&mut observations, Path::new(path), operation)?;
            if path.ends_with(".desktop") { desktop(&read_regular(Path::new(path))?)?; }
        }
        record(&mut observations, format!("systemd:unit:{unit}"), normalized_lines(&output)?, operation)?;
    }
    // Inspect both the canonical owner-local runtime root and an explicit XDG
    // root; an inherited override must not hide the manager's default output.
    let canonical_runtime = std::path::PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }));
    let mut runtimes = BTreeSet::from([canonical_runtime]);
    if let Some(value) = std::env::var_os("XDG_RUNTIME_DIR") {
        runtimes.insert(std::path::PathBuf::from(value));
    }
    for runtime in runtimes {
        if !runtime.is_absolute() { anyhow::bail!("unsupported user runtime directory"); }
        for suffix in ["generator", "generator.early", "generator.late"] {
            directory(&mut observations, &runtime.join("systemd").join(suffix), true, operation)?;
        }
    }
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let mut roots = BTreeSet::from([home.join(".config/autostart"), config_home.join("autostart"), std::path::PathBuf::from("/etc/xdg/autostart")]);
    if let Some(value) = std::env::var_os("XDG_CONFIG_DIRS") {
        for root in std::env::split_paths(&value) {
            if !root.is_absolute() { anyhow::bail!("unsupported XDG configuration root"); }
            roots.insert(root.join("autostart"));
        }
    }
    for root in roots {
        if !root.is_absolute() { anyhow::bail!("unsupported XDG configuration root"); }
        directory(&mut observations, &root, false, operation)?;
    }
    let cron = command("crontab", &["-l"])?;
    let entries = cron_output(cron.status.success(), cron.status.code(), &cron.stdout, &cron.stderr)?;
    record(&mut observations, "user:crontab".to_owned(), entries, operation)?;
    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPERATION: &str = "c53c5ee8-3628-4bd5-86fd-e67f562103d4";

    #[test]
    fn unit_environment_and_manager_environment_labels_refuse() {
        for alias in ["ST3_START_OPERATION", "FRACTAL_START_OPERATION"] {
            let value = format!("Environment={alias}={OPERATION}\nEnvironmentFiles=\nFragmentPath=\nSourcePath=\n");
            assert!(properties(value.as_bytes()).is_ok());
            assert!(inspect(value.as_bytes(), OPERATION).is_err());
            assert!(inspect(format!("{alias}={OPERATION}\n").as_bytes(), OPERATION).is_err());
        }
    }

    #[test]
    fn environment_files_are_complete_and_labels_refuse() {
        assert_eq!(environment_files("/tmp/a (ignore_errors=no) /tmp/b (ignore_errors=yes)").unwrap(), vec!["/tmp/a", "/tmp/b"]);
        assert!(environment_files("/tmp/a").is_err());
        assert!(environment_files("/tmp/a\\x20b (ignore_errors=no)").is_err());
        assert!(inspect(format!("ST3_START_OPERATION={OPERATION}\n").as_bytes(), OPERATION).is_err());
    }

    #[test]
    fn incomplete_manager_output_refuses() {
        assert!(inventory(b"a.service loaded active running Example", false).is_err());
        assert!(inventory(b"a.service enabled\ntruncated\n", true).is_err());
        assert!(properties(b"Environment=\nEnvironmentFiles=\nFragmentPath=\n").is_err());
        assert!(properties(b"Environment=\nEnvironmentFiles=\nFragmentPath=\nSourcePath=").is_err());
        assert!(inventory(b"a.service loaded active running Example\n", false).is_ok());
    }

    #[test]
    fn cron_labels_and_ambiguous_absence_refuse() {
        let bytes = format!("@reboot ST3_START_OPERATION={OPERATION} launch\n");
        assert!(inspect(&cron_output(true, Some(0), bytes.as_bytes(), b"").unwrap(), OPERATION).is_err());
        assert!(cron_output(false, Some(1), b"", b"no crontab for alice\n").unwrap().is_empty());
        for diagnostic in [b"".as_slice(), b"permission denied\n", b"no crontab for alice\nextra\n"] {
            assert!(cron_output(false, Some(1), b"", diagnostic).is_err());
        }
        assert!(cron_output(false, Some(2), b"", b"no crontab for alice\n").is_err());
    }

    #[test]
    fn desktop_labels_and_unknown_schema_refuse() {
        assert!(desktop(b"[Desktop Entry]\nType=Application\nName=Example\nExec=/bin/true\n").is_ok());
        let bytes = format!("[Desktop Entry]\nType=Application\nExec=launch {OPERATION}\n");
        assert!(desktop(bytes.as_bytes()).is_ok());
        assert!(inspect(bytes.as_bytes(), OPERATION).is_err());
        for bytes in [b"Exec=/bin/true\n".as_slice(), b"[Desktop Entry]\nType=Application\n", b"[Unknown]\nExec=true\n", b"[Desktop Entry]\nType=Application\nExec=true\nHidden=maybe\n"] {
            assert!(desktop(bytes).is_err());
        }
    }

    #[test]
    fn generator_and_unreadable_launch_fixtures_refuse() {
        let root = tempfile::tempdir().unwrap();
        let generator = root.path().join("generator");
        std::fs::create_dir(&generator).unwrap();
        std::fs::write(generator.join("unknown.service"), b"[Service]\nExecStart=true\n").unwrap();
        assert!(directory(&mut Observations::new(), &generator, true, OPERATION).is_err());
        assert!(read_regular(&root.path().join("missing-environment-file")).is_err());
        assert!(read_regular(root.path()).is_err());
        let autostart = root.path().join("autostart");
        std::fs::create_dir(&autostart).unwrap();
        std::fs::create_dir(autostart.join("unreadable.desktop")).unwrap();
        assert!(directory(&mut Observations::new(), &autostart, false, OPERATION).is_err());
        let environment = root.path().join("environment");
        std::fs::write(&environment, format!("FRACTAL_START_OPERATION={OPERATION}\n")).unwrap();
        assert!(observe_file(&mut Observations::new(), &environment, OPERATION).is_err());
        std::fs::remove_dir(autostart.join("unreadable.desktop")).unwrap();
        std::fs::write(autostart.join("label.desktop"),
            format!("[Desktop Entry]\nType=Application\nExec=launch {OPERATION}\n")).unwrap();
        assert!(directory(&mut Observations::new(), &autostart, false, OPERATION).is_err());
    }

    #[test]
    fn malformed_environment_file_and_effective_launch_refuse() {
        assert!(environment_file(b"ST3_START_OPERATION=ok\n").is_ok());
        assert!(environment_file(b"broken known schema\n").is_err());
        assert!(environment_file(b"KEY=\"unterminated\n").is_err());
        assert!(effective_launch(b"ExecStart={ path=/bin/true ; }\n").is_err());
    }
}
