//! No environment, argv, class or priority is admission evidence.
use super::*;
use std::path::{Component, Path as FsPath};

fn unavailable() -> ApiError {
    refused("guard-unavailable", StatusCode::SERVICE_UNAVAILABLE)
}

fn valid_path(path: &FsPath) -> bool {
    path.is_absolute()
        && path.components().skip(1).all(|part| matches!(part, Component::Normal(_)))
        && path != FsPath::new("/")
}

fn unified(contents: &str) -> Result<&FsPath, ApiError> {
    let mut paths = contents.lines().filter_map(|line| line.strip_prefix("0::"));
    let path = FsPath::new(paths.next().ok_or_else(unavailable)?);
    if paths.next().is_some() || !(path == FsPath::new("/") || valid_path(path)) {
        return Err(unavailable());
    }
    Ok(path)
}

fn check(
    config: &JobsConfig,
    membership: &str,
    cgroupfs: &FsPath,
    lookup: impl FnOnce(&str) -> Result<Option<String>, ApiError>,
) -> Result<(), ApiError> {
    let path = unified(membership)?;
    if config.keeper_cgroups.is_empty() {
        return Err(unavailable());
    }
    for root in &config.keeper_cgroups {
        if !valid_path(root) {
            return Err(unavailable());
        }
        let relative = root.strip_prefix("/").map_err(|_| unavailable())?;
        let events = std::fs::read_to_string(cgroupfs.join(relative).join("cgroup.events"))
            .map_err(|_| unavailable())?;
        if !events.lines().any(|line| matches!(line, "populated 0" | "populated 1")) {
            return Err(unavailable());
        }
    }
    let Some(relative) = config.keeper_cgroups.iter()
        .find_map(|root| path.strip_prefix(root).ok())
    else {
        return Ok(());
    };
    let mut parts = relative.components();
    let Some(Component::Normal(name)) = parts.next() else {
        return Err(unavailable());
    };
    if name == "idle" {
        return Ok(());
    }
    let job = lookup(name.to_str().ok_or_else(unavailable)?)?.ok_or_else(unavailable)?;
    let mut error = refused("recursive-heavy-job", StatusCode::CONFLICT);
    error.details.insert("job_id".into(), json!(job));
    Err(error)
}

pub(super) fn guard(
    state: &AppState,
    session: &client_v0::ClientSession,
    config: &JobsConfig,
) -> Result<(), ApiError> {
    #[cfg(target_os = "linux")]
    {
        // This accept-time pidfd witness comes from SO_PEERPIDFD, never request JSON.
        let peer = session.peer_process.as_ref().ok_or_else(unavailable)?;
        peer.validate().map_err(|_| unavailable())?;
        let path = format!("/proc/{}/cgroup", peer.pid());
        let membership = std::fs::read_to_string(&path).map_err(|_| unavailable())?;
        let decision = check(config, &membership, FsPath::new("/sys/fs/cgroup"), |name| {
            read(state, |tx| projection::job_for_cgroup(tx, &state.node, name))
        });
        let after = std::fs::read_to_string(&path).map_err(|_| unavailable())?;
        peer.validate().map_err(|_| unavailable())?;
        if membership != after {
            return Err(unavailable());
        }
        decision
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state, session, config);
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests;
