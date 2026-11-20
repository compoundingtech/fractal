use super::*;
use std::process::{Command, Stdio};

#[cfg(not(target_os = "linux"))]
const COMMAND_BUDGET: Duration = Duration::from_secs(2);
const MAX_OUTPUT: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum EvidenceError {
    Deadline,
    OutputLimit,
    Failed(std::io::Error),
}
impl std::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deadline => f.write_str("OS session evidence deadline exceeded"),
            Self::OutputLimit => f.write_str("OS session evidence output limit exceeded"),
            Self::Failed(error) => write!(f, "OS session evidence: {error}"),
        }
    }
}
impl std::error::Error for EvidenceError {}
impl From<std::io::Error> for EvidenceError {
    fn from(error: std::io::Error) -> Self { Self::Failed(error) }
}

// A file, rather than a pipe reader thread, prevents inherited stdout descriptors from
// keeping a request alive after the subprocess has exited or been killed.
pub(super) fn output(command: &mut Command, budget: Duration) -> Result<Vec<u8>> {
    let mut output = tempfile::tempfile()?;
    command.stdout(Stdio::from(output.try_clone()?)).stderr(Stdio::null());
    let mut child = command.spawn().map_err(EvidenceError::from)?;
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait().map_err(EvidenceError::from)? {
            anyhow::ensure!(status.success(), "OS evidence subprocess failed: {status}");
            break;
        }
        let limit = output.metadata()?.len() > MAX_OUTPUT;
        if limit || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if limit { EvidenceError::OutputLimit } else { EvidenceError::Deadline }.into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if output.metadata()?.len() > MAX_OUTPUT { return Err(EvidenceError::OutputLimit.into()); }
    output.rewind()?;
    let mut bytes = Vec::new();
    output.take(MAX_OUTPUT).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[derive(Default)]
pub(super) struct OpenFiles {
    pub cwd: Option<PathBuf>,
    pub transcripts: BTreeSet<PathBuf>,
}

pub(super) fn open_files(pids: &[u32]) -> Result<BTreeMap<u32, OpenFiles>> {
    if pids.is_empty() { return Ok(BTreeMap::new()); }
    #[cfg(target_os = "linux")]
    {
        let mut found = BTreeMap::new();
        for pid in pids {
            let mut files = OpenFiles { cwd: fs::read_link(format!("/proc/{pid}/cwd")).ok(), ..Default::default() };
            if let Ok(entries) = fs::read_dir(format!("/proc/{pid}/fd")) {
                for entry in entries.flatten() {
                    if let Ok(path) = fs::read_link(entry.path())
                        && path.extension().is_some_and(|ext| ext == "jsonl")
                    { files.transcripts.insert(path); }
                }
            }
            found.insert(*pid, files);
        }
        Ok(found)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let pids = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let mut command = crate::environment::command("lsof")?;
        command.args(["-nP", "-a", "-p", &pids, "-Fpfnt"]);
        let bytes = output(&mut command, COMMAND_BUDGET)?;
        Ok(parse_lsof(&bytes))
    }
}

#[cfg(any(test, not(target_os = "linux")))]
fn parse_lsof(bytes: &[u8]) -> BTreeMap<u32, OpenFiles> {
    let mut found = BTreeMap::<u32, OpenFiles>::new();
    let mut pid = None;
    let mut cwd = false;
    let mut regular = false;
    for line in String::from_utf8_lossy(bytes).lines() {
        if let Some(value) = line.strip_prefix('p') { pid = value.parse().ok(); }
        else if let Some(value) = line.strip_prefix('f') { cwd = value == "cwd"; regular = false; }
        else if let Some(value) = line.strip_prefix('t') { regular = value == "REG" || value == "DIR"; }
        else if let Some(value) = line.strip_prefix('n') && let Some(pid) = pid {
            let files = found.entry(pid).or_default();
            let path = PathBuf::from(value);
            if cwd { files.cwd = Some(path); }
            else if regular && path.extension().is_some_and(|ext| ext == "jsonl") { files.transcripts.insert(path); }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stuck_subprocess_is_reaped_at_deadline() {
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 60"]);
        let start = Instant::now();
        let error = output(&mut command, Duration::from_millis(30)).unwrap_err();
        assert!(error.downcast_ref::<EvidenceError>().is_some_and(|error| matches!(error, EvidenceError::Deadline)));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn lsof_preserves_full_paths_and_process_boundaries() {
        let files = parse_lsof(b"p12\nfcwd\ntDIR\nn/work space\nf4\ntREG\nn/sessions/one.jsonl\np13\nf5\ntREG\nn/sessions/two.jsonl\n");
        assert_eq!(files[&12].cwd.as_deref(), Some(Path::new("/work space")));
        assert!(files[&12].transcripts.contains(Path::new("/sessions/one.jsonl")));
        assert!(!files[&12].transcripts.contains(Path::new("/sessions/two.jsonl")));
    }
}
