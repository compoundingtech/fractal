//! Local Claude process ownership. Environment/session labels locate a witness; they never
//! replace the kernel's parent and birth-time proof. This is isolation from inherited context,
//! not a security boundary against a same-user process rewriting all of st's local state.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const FILE: &str = "claude-primary-process.json";
const LOCK: &str = "claude-primary-process.lock";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub birth: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Owner {
    version: u8,
    session: String,
    sequence: u64,
    runtime: String,
    wrapper: Process,
    provider: Option<Process>,
    closed: bool,
}

#[derive(Clone, Debug)]
pub enum Caller<'a> {
    Channel,
    Wrapper,
    Hook { trampoline: &'a str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Primary,
    Legacy,
}

pub fn recorded(dir: &Path) -> Result<bool> {
    Ok(dir.join(FILE).try_exists()?)
}

pub fn wrapper(dir: &Path) -> Result<Process> {
    Ok(serde_json::from_slice::<Owner>(&read_owner(dir)?)?.wrapper)
}

fn read_owner(dir: &Path) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let file = fs::File::open(dir.join(FILE))?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "Claude witness is not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 4096, "Claude witness exceeds the byte limit");
    Ok(bytes)
}

fn lock(dir: &Path, create: bool) -> Result<crate::flock::FileLock> {
    use crate::flock::{self, FileLock, Mode, Open};
    let file = flock::open(
        &dir.join(LOCK),
        if create { Open::Create } else { Open::Existing },
    )?;
    Ok(FileLock::hold_blocking(file, Mode::Exclusive)?)
}

fn write(dir: &Path, owner: &Owner) -> Result<()> {
    crate::harness_state::write_json_atomic(&dir.join(FILE), owner, dir, ".claude-primary-process")
}

/// Claim the harness and record its actual wrapper generation BEFORE spawning the provider.
/// Lock order is process-owner, then harness-state; helper publications use the same order.
pub(crate) fn claim(dir: &Path, identity: &str, runtime: &str, session: &str) -> Result<u64> {
    fs::create_dir_all(dir)?;
    let _guard = lock(dir, true)?;
    let wrapper = process(std::process::id())?;
    let sequence = crate::harness_state::claim(dir, identity, "claude", session)?;
    write(
        dir,
        &Owner {
            version: 1,
            session: session.into(),
            sequence,
            runtime: runtime.into(),
            wrapper,
            provider: None,
            closed: false,
        },
    )?;
    Ok(sequence)
}

/// The supervisor's spawned PID is authoritative, never a helper-provided value. An early
/// SessionStart may precede this write: only the kernel-proven direct child of this wrapper can
/// publish during that window. The wrapper performs no other spawns after claim and before this.
pub(crate) fn spawned(dir: &Path, session: &str, sequence: u64, pid: u32) -> Result<()> {
    let _guard = lock(dir, false)?;
    let mut owner: Owner = serde_json::from_slice(&read_owner(dir)?)?;
    anyhow::ensure!(
        owner.session == session && owner.sequence == sequence && !owner.closed,
        "Claude process ownership was superseded"
    );
    anyhow::ensure!(
        process(std::process::id())? == owner.wrapper,
        "Claude wrapper generation changed"
    );
    let child = process(pid)?;
    anyhow::ensure!(
        child.parent == owner.wrapper.pid,
        "Claude provider is not the wrapper's child"
    );
    owner.provider = Some(child);
    write(dir, &owner)
}

pub(crate) fn ended(dir: &Path, session: &str, sequence: u64) -> Result<()> {
    if !recorded(dir)? {
        return Ok(());
    } // A reexec of a legacy wrapper stays legacy.
    let _guard = lock(dir, false)?;
    let mut owner: Owner = serde_json::from_slice(&read_owner(dir)?)?;
    if owner.session == session && owner.sequence == sequence {
        owner.closed = true;
        write(dir, &owner)?;
    }
    Ok(())
}

/// A daemon supplies its independently captured physical wrapper. Local hooks have no daemon
/// connection, but still require the exact wrapper session/sequence and kernel relationship.
/// A missing record is explicitly legacy, never an implicit primary proof. Malformed records
/// and records for a different generation fail closed; they cannot downgrade to legacy.
#[allow(clippy::too_many_arguments)]
pub fn with_caller<T>(
    dir: &Path,
    session: &str,
    sequence: u64,
    pid: u32,
    expected_wrapper: Option<Process>,
    role: Caller<'_>,
    action: impl FnOnce(Admission) -> Result<T>,
) -> Result<T> {
    let path = dir.join(FILE);
    // A legacy wrapper never minted the lock or record. Do not create either from a helper.
    if !path.try_exists()? {
        return action(Admission::Legacy);
    }
    let _guard = lock(dir, false)?;
    let bytes = read_owner(dir)?;
    let owner: Owner = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        owner.version == 1
            && !owner.closed
            && owner.session == session
            && owner.sequence == sequence,
        "Claude primary ownership is closed or superseded"
    );
    if let Some(expected) = expected_wrapper {
        anyhow::ensure!(
            expected == owner.wrapper,
            "Claude ownership disagrees with the physical wrapper"
        );
    }
    crate::harness_state::with_current_ownership(dir, session, sequence, || Ok(()))?;
    let proof = prove(&owner, pid, &role, process, arguments)?;
    // Check before the publication and after capture; a reparented or reused process is not
    // admitted. Holding our lock prevents wrapper replacement during the actual publication.
    validate_proof(&proof, process)?;
    anyhow::ensure!(
        read_owner(dir)? == bytes,
        "Claude process witness changed during admission"
    );
    action(Admission::Primary)
}

fn validate_proof(proof: &[Process], read: impl Fn(u32) -> Result<Process>) -> Result<()> {
    for captured in proof {
        anyhow::ensure!(
            read(captured.pid)? == *captured,
            "Claude process parent or birth changed during admission"
        );
    }
    Ok(())
}

fn prove(
    owner: &Owner,
    pid: u32,
    role: &Caller<'_>,
    read: impl Fn(u32) -> Result<Process>,
    args: impl Fn(u32) -> Result<Vec<String>>,
) -> Result<Vec<Process>> {
    let wrapper = read(owner.wrapper.pid)?;
    anyhow::ensure!(
        wrapper == owner.wrapper && wrapper.pid > 1,
        "Claude wrapper generation is not live"
    );
    let caller = read(pid)?;
    if matches!(role, Caller::Wrapper) {
        anyhow::ensure!(caller == wrapper, "this is not the primary Claude wrapper");
        return Ok(vec![wrapper]);
    }
    let mut proof = vec![wrapper, caller];
    let mut provider = read(caller.parent)?;
    if let Caller::Hook { trampoline } = role {
        let argv = args(provider.pid)?;
        if exact_trampoline(&argv, trampoline) {
            proof.push(provider);
            provider = read(provider.parent)?;
        }
    }
    anyhow::ensure!(
        provider.pid > 1 && provider.parent == wrapper.pid,
        "secondary Claude process cannot act as the launching seat"
    );
    if let Some(expected) = owner.provider {
        anyhow::ensure!(
            provider == expected,
            "Claude primary provider generation changed"
        );
    }
    proof.push(provider);
    Ok(proof)
}

fn exact_trampoline(argv: &[String], command: &str) -> bool {
    argv.len() == 3
        && argv[1] == "-c"
        && argv[2] == command
        && matches!(
            Path::new(&argv[0]).file_name().and_then(|p| p.to_str()),
            Some("sh" | "bash" | "dash" | "zsh")
        )
}

#[cfg(target_os = "linux")]
pub fn process(pid: u32) -> Result<Process> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, tail) = stat.rsplit_once(") ").context("malformed process stat")?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    anyhow::ensure!(
        fields.first().is_some_and(|s| !matches!(*s, "Z" | "X")),
        "process is terminal"
    );
    Ok(Process {
        pid,
        parent: fields.get(1).context("missing process parent")?.parse()?,
        birth: fields.get(19).context("missing process birth")?.parse()?,
    })
}

#[cfg(target_os = "macos")]
pub fn process(pid: u32) -> Result<Process> {
    let pid_arg = i32::try_from(pid)?;
    let mut info = std::mem::MaybeUninit::<libc::proc_taskallinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_taskallinfo>() as libc::c_int;
    let count = unsafe {
        libc::proc_pidinfo(
            pid_arg,
            libc::PROC_PIDTASKALLINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    anyhow::ensure!(count == size, "Darwin process generation is unavailable");
    let info = unsafe { info.assume_init() }.pbsd;
    anyhow::ensure!(
        info.pbi_pid == pid && info.pbi_status != 5,
        "Darwin process is terminal"
    );
    Ok(Process {
        pid,
        parent: info.pbi_ppid,
        birth: info
            .pbi_start_tvsec
            .checked_mul(1_000_000)
            .and_then(|v| v.checked_add(info.pbi_start_tvusec))
            .context("Darwin process birth overflow")?,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process(_pid: u32) -> Result<Process> {
    anyhow::bail!("process ownership is unsupported")
}

#[cfg(target_os = "linux")]
fn arguments(pid: u32) -> Result<Vec<String>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{pid}/cmdline"))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 1024 * 1024,
        "process arguments exceed the limit"
    );
    Ok(bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

#[cfg(target_os = "macos")]
fn arguments(pid: u32) -> Result<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, i32::try_from(pid)?];
    let mut size = 0;
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    anyhow::ensure!(
        result == 0 && size <= 1024 * 1024,
        "process arguments unavailable"
    );
    let mut bytes = vec![0u8; size];
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    anyhow::ensure!(result == 0, "process arguments changed");
    bytes.truncate(size);
    let argc = i32::from_ne_bytes(bytes.get(..4).context("missing argc")?.try_into()?);
    let tail = bytes.get(4..).context("missing argument buffer")?;
    let tail = tail
        .get(
            tail.iter()
                .position(|b| *b == 0)
                .context("missing executable")?
                + 1..,
        )
        .context("missing arguments")?;
    let padding = tail.iter().take_while(|b| **b == 0).count();
    let mut parts = tail[padding..].split(|b| *b == 0);
    (0..usize::try_from(argc)?)
        .map(|_| {
            parts
                .next()
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .context("truncated process arguments")
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn arguments(_pid: u32) -> Result<Vec<String>> {
    anyhow::bail!("process arguments unsupported")
}

#[cfg(test)]
mod tests;
