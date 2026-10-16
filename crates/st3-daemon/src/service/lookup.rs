//! Read both service namespaces without ever choosing between conflicting installs.
// arn:lmig:smalltalk:2026-10-16-service-labels-fractal
use super::*;

#[derive(Debug)]
pub struct ConflictingServices(String, String);

impl std::fmt::Display for ConflictingServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "conflicting service units: {} and {}; refusing to choose or change either", self.0, self.1)
    }
}

impl std::error::Error for ConflictingServices {}

fn select(mut statuses: Vec<ServiceStatus>) -> Result<ServiceStatus> {
    let mut selected: Option<usize> = None;
    for (index, status) in statuses.iter().enumerate() {
        if status.installed || status.running {
            if let Some(previous) = selected {
                return Err(ConflictingServices(statuses[previous].name.clone(), status.name.clone()).into());
            }
            selected = Some(index);
        }
    }
    Ok(statuses.swap_remove(selected.unwrap_or(0)))
}

pub(super) struct Services {
    pub daemon: ServiceStatus,
    pub replication: ServiceStatus,
}

impl Services {
    pub fn report(self) -> ServiceStatusReport {
        ServiceStatusReport {
            manager: if cfg!(target_os = "linux") { "systemd-user" } else { "launchd-user" },
            services: vec![self.daemon, self.replication],
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn native() -> Result<Services> {
    #[cfg(target_os = "linux")]
    let old = [service_name(), replication_service_name()];
    #[cfg(target_os = "macos")]
    let old = [service_label(), replication_service_label()];
    let resolve = |old: &str| -> Result<ServiceStatus> {
        let new = old.replacen("st3", "fractal", 1);
        #[cfg(target_os = "linux")]
        let mut names = vec![old.to_owned(), new];
        #[cfg(target_os = "macos")]
        let names = vec![old.to_owned(), new];
        // Home Manager writes smalltalk.service rather than the CLI's st3.service.
        #[cfg(target_os = "linux")]
        if crate::instance::current().is_none() && old == service_name() {
            names.push("smalltalk.service".into());
        }
        select(names.iter().map(|name| {
            #[cfg(target_os = "linux")]
            {
                let mut status = systemd_service_status(name)?;
                status.installed |= systemd_path(name)?.exists();
                Ok(status)
            }
            #[cfg(target_os = "macos")]
            {
                launchd_service_status(name, &launchd_path(name)?)
            }
        }).collect::<Result<Vec<_>>>()?)
    };
    Ok(Services { daemon: resolve(old[0])?, replication: resolve(old[1])? })
}

#[cfg(target_os = "linux")]
pub(super) fn systemd_path(name: &str) -> Result<PathBuf> {
    Ok(systemd_user_unit_path()?.with_file_name(name))
}

#[cfg(target_os = "macos")]
pub(super) fn launchd_path(label: &str) -> Result<PathBuf> {
    Ok(launch_agent_path()?.with_file_name(format!("{label}.plist")))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn before_install() -> Result<()> {
    let services = native()?;
    #[cfg(target_os = "linux")]
    let old = [service_name(), replication_service_name()];
    #[cfg(target_os = "macos")]
    let old = [service_label(), replication_service_label()];
    for (status, old) in [services.daemon, services.replication].iter().zip(old) {
        anyhow::ensure!(
            !(status.installed || status.running) || status.name == old,
            "{} already exists; this installer still writes {old}; refusing to create a second service",
            status.name
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, installed: bool, running: bool) -> ServiceStatus {
        ServiceStatus { name: name.into(), installed, running, state: String::new() }
    }

    #[test]
    fn service_namespace_presence_matrix() {
        for (old, new) in [("st3.service", "fractal.service"),
            ("st3-replication.service", "fractal-replication.service"),
            ("com.compoundingtech.st3", "com.compoundingtech.fractal"),
            ("com.compoundingtech.st3.replication", "com.compoundingtech.fractal.replication"),
            ("smalltalk.service", "fractal.service"),
            ("st3-instance-try.service", "fractal-instance-try.service")] {
            for (old_exists, new_exists, expected) in [
                (true, false, Some(old)), (false, true, Some(new)),
                (true, true, None), (false, false, Some(old)),
            ] {
                let result = select(vec![status(old, old_exists, false), status(new, new_exists, false)]);
                if let Some(expected) = expected {
                    let result = result.unwrap();
                    assert_eq!(result.name, expected);
                    assert_eq!(result.installed, old_exists || new_exists);
                } else {
                    assert!(result.unwrap_err().to_string().contains("conflicting service units"));
                }
            }
            // Loaded services remain conflicting after their files have disappeared.
            assert!(select(vec![status(old, false, true), status(new, false, true)]).is_err());
            assert!(select(vec![status(old, true, false), status(new, false, true)]).is_err());
        }
    }
}
