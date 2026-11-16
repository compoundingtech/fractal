//! Domain ownership is discovered once, or restored from the write-ahead journal.
use anyhow::Result;
use std::process::Output;

#[derive(Debug)]
pub enum DomainError {
    Inspection { domain: String, detail: String },
    LegacyDaemonNotLoaded { domains: Vec<String> },
    AmbiguousLegacyDaemonDomain { jobs: Vec<String> },
    SplitLaunchdDomains { jobs: Vec<String> },
    InvalidJournalDomain(String),
    MissingJournalField,
}
impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inspection { domain, detail } => write!(f, "cannot inspect launchd domain {domain}: {detail}"),
            Self::LegacyDaemonNotLoaded { domains } => write!(f, "no recognized legacy daemon loaded in {domains:?}; expected com.compoundingtech.st3 or org.nix-community.home.smalltalk"),
            Self::AmbiguousLegacyDaemonDomain { jobs } => write!(f, "ambiguous legacy daemon ownership: {jobs:?}; stop and reconcile duplicate jobs before migration"),
            Self::SplitLaunchdDomains { jobs } => write!(f, "recognized jobs split across launchd domains: {jobs:?}; reconcile ownership before migration"),
            Self::InvalidJournalDomain(domain) => write!(f, "invalid journal launchd_domain {domain:?}: expected gui/<current uid> or user/<current uid>"),
            Self::MissingJournalField => write!(f, "journal is missing launchd_domain; no domain will be assumed. The old writer used gui/<uid> only. For manual recovery on that GUI host, preserve the journal, inspect its services/pairs/moves, stop target jobs in gui/<uid>, reverse recorded moves and restore legacy enablement/start state for rollback; if committed, do not reverse state: verify the target API and retire legacy jobs in gui/<uid>. Do not edit the journal or rerun migration until ownership is repaired"),
        }
    }
}
impl std::error::Error for DomainError {}
#[derive(Debug)]
pub enum Probe { Available(Vec<String>), Unavailable }
pub fn probe(domain: &str, output: Output) -> Result<Probe> {
    let text = String::from_utf8_lossy(&output.stdout);
    let detail = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(125) && detail.trim() == "Could not print domain: 125: Domain does not support specified action" {
        return Ok(Probe::Unavailable);
    }
    let failure = || DomainError::Inspection { domain: domain.into(), detail: format!("{}: {detail}; output: {text}", output.status) };
    if !output.status.success() { return Err(failure().into()); }
    let mut lines = text.lines();
    if lines.next().and_then(|line| line.strip_suffix(" = {")) != Some(domain) { return Err(failure().into()); }
    if !lines.by_ref().any(|line| line.trim() == "services = {") { return Err(failure().into()); }
    let mut jobs = Vec::new();
    for line in lines {
        if line.trim() == "}" { return Ok(Probe::Available(jobs)); }
        let mut fields = line.split_whitespace();
        if fields.next().is_none() || fields.next().is_none() { return Err(failure().into()); }
        let label = fields.next().ok_or_else(failure)?;
        if fields.next().is_some() { return Err(failure().into()); }
        jobs.push(label.to_owned());
    }
    Err(failure().into())
}

pub fn resolve(uid: u32, recorded: Option<&str>, recovery: bool, mut inspect: impl FnMut(&str) -> Result<Probe>) -> Result<String> {
    let domains = [format!("gui/{uid}"), format!("user/{uid}")];
    if recovery {
        let recorded = recorded.ok_or(DomainError::MissingJournalField)?;
        if !domains.iter().any(|domain| domain == recorded) { return Err(DomainError::InvalidJournalDomain(recorded.into()).into()); }
        return Ok(recorded.into());
    }
    let mut jobs = Vec::new();
    let mut daemons = Vec::new();
    for domain in &domains {
        if let Probe::Available(labels) = inspect(domain)? {
            for label in labels {
                let daemon = matches!(label.as_str(), "com.compoundingtech.st3" | "org.nix-community.home.smalltalk");
                let recognized = daemon || matches!(label.as_str(), "com.compoundingtech.st3.replication" | "org.nix-community.home.smalltalk-replication" | "org.nix-community.home.smalltalk-apply" | "com.compoundingtech.fractal" | "com.compoundingtech.fractal.replication" | "com.compoundingtech.fractal-apply");
                if daemon { daemons.push(domain.clone()); }
                if recognized { jobs.push((domain.clone(), format!("{domain}/{label}"))); }
            }
        }
    }
    if daemons.is_empty() { return Err(DomainError::LegacyDaemonNotLoaded { domains: domains.into() }.into()); }
    if daemons.len() != 1 { return Err(DomainError::AmbiguousLegacyDaemonDomain { jobs: jobs.into_iter().map(|(_, job)| job).collect() }.into()); }
    let selected = daemons.remove(0);
    if jobs.iter().any(|(domain, _)| domain != &selected) { return Err(DomainError::SplitLaunchdDomains { jobs: jobs.into_iter().map(|(_, job)| job).collect() }.into()); }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;
    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output { status: std::process::ExitStatus::from_raw(code << 8), stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec() }
    }
    #[test]
    fn discovery_checks_both_domains_and_refuses_missing_duplicate_or_split_jobs() {
        for (gui, user, expected) in [
            (vec![], vec!["org.nix-community.home.smalltalk"], "user/501"),
            (vec!["com.compoundingtech.st3"], vec![], "gui/501"),
        ] {
            assert_eq!(resolve(501, None, false, |domain| Ok(Probe::Available(if domain.starts_with("gui") { &gui } else { &user }.iter().map(|s| (*s).into()).collect()))).unwrap(), expected);
        }
        let run = |gui: &[&str], user: &[&str]| resolve(501, None, false, |domain| Ok(Probe::Available(if domain.starts_with("gui") { gui } else { user }.iter().map(|s| (*s).into()).collect())));
        assert!(matches!(run(&[], &[]).unwrap_err().downcast_ref(), Some(DomainError::LegacyDaemonNotLoaded { .. })));
        for user in ["com.compoundingtech.st3", "org.nix-community.home.smalltalk"] {
            let error = run(&["com.compoundingtech.st3"], &[user]).unwrap_err();
            assert!(matches!(error.downcast_ref(), Some(DomainError::AmbiguousLegacyDaemonDomain { .. })));
            assert!(error.to_string().contains(&format!("user/501/{user}")));
        }
        for worker in ["org.nix-community.home.smalltalk-apply", "com.compoundingtech.fractal", "com.compoundingtech.st3.replication"] {
            assert!(matches!(run(&["com.compoundingtech.st3"], &[worker]).unwrap_err().downcast_ref(), Some(DomainError::SplitLaunchdDomains { .. })));
        }
    }
    #[test]
    fn domain_probe_distinguishes_unavailable_from_inspection_failure() {
        assert!(matches!(probe("gui/501", output(125, "", "Could not print domain: 125: Domain does not support specified action")).unwrap(), Probe::Unavailable));
        for code in [113, 125, 1] {
            assert!(matches!(probe("gui/501", output(code, "", "permission denied")).unwrap_err().downcast_ref(), Some(DomainError::Inspection { .. })));
        }
        assert!(probe("user/501", output(0, "unexpected output", "")).is_err());
        let parsed = probe("user/501", output(0, "user/501 = {\n services = {\n 42 - org.nix-community.home.smalltalk\n }\n}", "")).unwrap();
        assert!(matches!(parsed, Probe::Available(jobs) if jobs == ["org.nix-community.home.smalltalk"]));
        assert_eq!(resolve(501, None, false, |domain| if domain.starts_with("gui") { Ok(Probe::Unavailable) } else { Ok(Probe::Available(vec!["com.compoundingtech.st3".into()])) }).unwrap(), "user/501");
        for invalid in ["system", "user/502", "gui/501/other"] {
            assert!(matches!(resolve(501, Some(invalid), true, |_| panic!("recovery must not discover")).unwrap_err().downcast_ref(), Some(DomainError::InvalidJournalDomain(_))));
        }
    }
}
