//! One configured registry per daemon. Only its named owner performs Git network
//! reads; an unavailable owner preserves graph reads and the existing PR path.
use super::config::{Config, Repository as RepositoryConfig};
use super::discovery::{Observation, Owner, SyncAdmission, Transport};
use super::transport::{Advertisement, Authentication, Repository};
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct AuthorizedTransport {
    repository: Repository,
    directory: PathBuf,
    remote: String,
    destination: String,
}

impl AuthorizedTransport {
    fn check_destination(&self) -> Result<()> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.directory).args([
            "config",
            "--get",
            &format!("remote.{}.url", self.remote),
        ]);
        for key in [
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_WORK_TREE",
            "GIT_NAMESPACE",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
        ] {
            command.env_remove(key);
        }
        let output = command.output()?;
        ensure!(
            output.status.success(),
            "configured Git remote is unavailable"
        );
        ensure!(
            String::from_utf8(output.stdout)?.trim() == self.destination,
            "Git destination changed from the operator-authorized configuration"
        );
        Ok(())
    }
}

impl Transport for AuthorizedTransport {
    fn discover(&self) -> Result<Advertisement> {
        self.check_destination()?;
        self.repository.discover(&self.remote)
    }

    fn sync(&self) -> Result<()> {
        self.check_destination()?;
        self.repository.sync(&self.remote)
    }
}

pub struct Entry {
    pub configuration: RepositoryConfig,
    pub owner: Option<Owner>,
    pub service_user: Option<String>,
    repository: Option<Repository>,
    unavailable: Option<String>,
    indexing: tokio::sync::Mutex<()>,
    pins: std::sync::Mutex<VecDeque<Pin>>,
}

struct Pin {
    id: String,
    observed: Observation,
    created: Instant,
}

pub struct Registry {
    entries: BTreeMap<String, Arc<Entry>>,
}

impl Registry {
    pub fn new(configuration: &Config, node: &str) -> Result<Self> {
        configuration.validate()?;
        let mut entries = BTreeMap::new();
        if !configuration.enabled {
            return Ok(Self { entries });
        }
        let mut destinations = BTreeSet::new();
        for configured in &configuration.repositories {
            ensure!(
                destinations.insert(configured.remote_url.clone()),
                "duplicate configured Git destination"
            );
            let mut entry = Entry {
                configuration: configured.clone(),
                owner: None,
                service_user: None,
                repository: None,
                unavailable: None,
                indexing: tokio::sync::Mutex::new(()),
                pins: std::sync::Mutex::new(VecDeque::new()),
            };
            if configured.reader_node != node {
                entry.unavailable = Some(format!(
                    "Git discovery is owned by node {}; use that node or the PR path",
                    configured.reader_node
                ));
            } else {
                entry.service_user = Command::new("id")
                    .arg("-un")
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .and_then(|output| String::from_utf8(output.stdout).ok())
                    .map(|user| user.trim().to_owned());
                let opened = (|| {
                    let directory = std::fs::canonicalize(&configured.transport_directory)?;
                    let repository = Repository::open(
                        &directory,
                        Authentication {
                            helper: configured.credential_helper.clone(),
                            ssh_command: configured.ssh_command.clone(),
                        },
                    )?;
                    let transport = AuthorizedTransport {
                        repository: repository.clone(),
                        directory,
                        remote: configured.remote.clone(),
                        destination: configured.remote_url.clone(),
                    };
                    transport.check_destination()?;
                    Ok::<_, anyhow::Error>((repository, transport))
                })();
                match opened {
                    Ok((repository, transport)) => {
                        entry.repository = Some(repository);
                        entry.owner = Some(Owner::new(Arc::new(transport)));
                    }
                    Err(error) => {
                        entry.unavailable = Some(format!(
                            "Git transport unavailable: {error}; use the PR path"
                        ))
                    }
                }
            }
            entries.insert(configured.name.clone(), Arc::new(entry));
        }
        Ok(Self { entries })
    }

    pub fn entry(&self, name: &str) -> Option<Arc<Entry>> {
        self.entries.get(name).cloned()
    }
}

impl Entry {
    pub async fn snapshot(
        &self,
        offline: bool,
        pin: Option<&str>,
    ) -> Result<(String, Observation)> {
        if let Some(pin) = pin {
            let pins = self
                .pins
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let pinned = pins
                .iter()
                .find(|entry| {
                    entry.id == pin
                        && entry.observed.offline == offline
                        && entry.created.elapsed() < Duration::from_secs(300)
                })
                .context("change cursor expired; restart the list")?;
            let mut observed = pinned.observed.clone();
            observed.reused = true;
            return Ok((pin.to_owned(), observed));
        }
        let observed = self.observation(offline).await?;
        // A pin names this bounded snapshot; hashing the full ref inventory would
        // repeat repository-sized work for every first-page request.
        let id = super::random_hex::<32>()?;
        let mut pins = self
            .pins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pins.retain(|pin| pin.created.elapsed() < Duration::from_secs(300));
        if !pins.iter().any(|pin| pin.id == id) {
            while pins.len() >= 32 {
                pins.pop_front();
            }
            pins.push_back(Pin {
                id: id.clone(),
                observed: observed.clone(),
                created: Instant::now(),
            });
        }
        Ok((id, observed))
    }

    pub async fn observation(&self, offline: bool) -> Result<Observation> {
        if let Some(owner) = &self.owner {
            return owner.observe(offline).await;
        }
        let mut observation = Observation::default();
        observation.error = self.unavailable.clone();
        observation.offline = offline;
        observation.reused = true;
        Ok(observation)
    }

    /// Explicit, budgeted namespace fetch followed by finite local object validation.
    /// No SQLite read snapshot or writer is held during Git work.
    pub async fn sync(&self, store: Arc<Store>) -> Result<SyncAdmission> {
        let owner = self
            .owner
            .as_ref()
            .context("Git reader unavailable; use the PR path")?;
        let admission = owner.sync().await?;
        if admission != SyncAdmission::Completed {
            return Ok(admission);
        }
        let _indexing = self.indexing.lock().await;
        let repository = self
            .repository
            .clone()
            .context("Git repository unavailable")?;
        let configured = self.configuration.clone();
        tokio::task::spawn_blocking(move || import_tracking(&store, &repository, &configured))
            .await
            .context("Git index importer stopped")??;
        Ok(SyncAdmission::Completed)
    }
}

fn import_tracking(
    store: &Store,
    repository: &Repository,
    configuration: &RepositoryConfig,
) -> Result<()> {
    let namespace = format!(
        "refs/st/tracking/{}/",
        hex::encode(Sha256::digest(configuration.remote_url.as_bytes()))
    );
    let output = Command::new("git")
        .arg("--no-replace-objects")
        .arg("-C")
        .arg(&configuration.transport_directory)
        .args([
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            &namespace,
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .output()?;
    ensure!(
        output.status.success(),
        "cannot enumerate fetched change objects"
    );
    let inventory = String::from_utf8(output.stdout)?;
    ensure!(
        inventory.len() <= 1024 * 1024,
        "tracking inventory exceeds import budget"
    );
    let mut pending = Vec::new();
    for line in inventory.lines() {
        let (name, object) = line.split_once(' ').context("malformed tracking ref")?;
        if name.ends_with("/current") {
            super::oid(object)?;
            pending.push(object.to_owned());
        }
    }
    ensure!(
        pending.len() <= 5000,
        "tracking inventory exceeds change budget"
    );
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop() {
        let mut chain = Vec::new();
        let mut cursor = Some(current);
        while let Some(object) = cursor {
            if store
                .indexed_git_changes(&configuration.name, std::slice::from_ref(&object))?
                .contains_key(&object)
            {
                break;
            }
            ensure!(
                seen.insert(object.clone()),
                "metadata history cycle or repeated unindexed object"
            );
            ensure!(
                seen.len() <= 10000,
                "metadata history exceeds import job budget; use the PR path"
            );
            let revision = repository.load(&object)?;
            cursor = revision.record.previous_record.clone();
            chain.push(revision);
        }
        // Do not publish a current record before its otherwise unknown lineage validates.
        // Each receipt remains a separate short transaction, without a Git-held writer.
        for revision in chain.into_iter().rev() {
            store.index_git_change(&configuration.name, &revision)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> Config {
        Config {
            enabled: true,
            repositories: vec![RepositoryConfig {
                name: "fixture/garden".into(),
                reader_node: "fixture-owner".into(),
                transport_directory: PathBuf::from("/missing/fixture/transport"),
                remote: "origin".into(),
                remote_url: "https://example.invalid/fixture/garden.git".into(),
                pilot_started_at: "2026-01-01T00:00:00Z".into(),
                ..RepositoryConfig::default()
            }],
        }
    }

    #[tokio::test]
    async fn nonowner_and_missing_transport_preserve_unavailable_observations() {
        for node in ["fixture-other", "fixture-owner"] {
            let registry = Registry::new(&configured(), node).unwrap();
            let entry = registry.entry("fixture/garden").unwrap();
            assert!(entry.owner.is_none());
            let observed = entry.observation(false).await.unwrap();
            assert!(observed.advertisement().is_none());
            assert!(observed.error.as_deref().unwrap().contains("PR path"));
            let offline = entry.observation(true).await.unwrap();
            assert!(offline.offline);
            assert!(offline.advertisement().is_none());
        }
        assert!(
            Registry::new(&Config::default(), "fixture-owner")
                .unwrap()
                .entry("fixture/garden")
                .is_none()
        );
    }

    #[tokio::test]
    async fn observation_cursor_is_pinned_and_cannot_switch_offline_mode() {
        let registry = Registry::new(&configured(), "fixture-other").unwrap();
        let entry = registry.entry("fixture/garden").unwrap();
        let (pin, first) = entry.snapshot(false, None).await.unwrap();
        let (same, next) = entry.snapshot(false, Some(&pin)).await.unwrap();
        assert_eq!(same, pin);
        assert_eq!(next.attempted_at, first.attempted_at);
        assert_eq!(next.error, first.error);
        assert!(entry.snapshot(true, Some(&pin)).await.is_err());
        assert!(entry.snapshot(false, Some("missing")).await.is_err());
    }
    #[tokio::test]
    async fn explicit_fetch_validates_real_git_objects_and_indexes_exact_current() {
        use crate::changes::{Record, State, TrailerStatus, new_change_id};
        use std::path::Path;
        fn git(path: &Path, args: &[&str]) -> String {
            let output = Command::new("git")
                .arg("-C")
                .arg(path)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        }
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let remote = root.path().join("remote.git");
        let reader = root.path().join("reader");
        for directory in [&source, &remote, &reader] {
            std::fs::create_dir(directory).unwrap();
        }
        git(&source, &["init", "-q"]);
        git(&remote, &["init", "--bare", "-q"]);
        git(&reader, &["init", "-q"]);
        let destination = remote.to_str().unwrap();
        git(&source, &["remote", "add", "origin", destination]);
        git(&reader, &["remote", "add", "origin", destination]);
        git(
            &source,
            &["commit", "--allow-empty", "-qm", "Synthetic base"],
        );
        let base = git(&source, &["rev-parse", "HEAD"]);
        let id = new_change_id().unwrap();
        git(
            &source,
            &[
                "commit",
                "--allow-empty",
                "-qm",
                &format!("Synthetic change\n\nChange-Id: {id}"),
            ],
        );
        let repository = Repository::open(&source, Authentication::default()).unwrap();
        let (source_commit, source_tree) = repository.source("HEAD").unwrap();
        let revision = repository
            .prepare(Record {
                schema_version: 1,
                change_id: id.clone(),
                generation: 1,
                previous_record: None,
                source_commit,
                source_tree,
                base_commit: base,
                trailer_status: TrailerStatus::Present,
                state: State::Draft,
                next: None,
                depends_on: vec![],
                actor: "agent/fixture/builder".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                claim: None,
                mirror: None,
            })
            .unwrap();
        repository.publish("origin", None, &revision).unwrap();
        let mut configuration = configured();
        configuration.repositories[0].transport_directory = reader;
        configuration.repositories[0].remote_url = destination.to_owned();
        let registry = Registry::new(&configuration, "fixture-owner").unwrap();
        let entry = registry.entry("fixture/garden").unwrap();
        let store = Arc::new(Store::open_memory("fixture-owner").unwrap());
        assert_eq!(
            entry.sync(store.clone()).await.unwrap(),
            SyncAdmission::Completed
        );
        let indexed = store
            .indexed_git_changes("fixture/garden", std::slice::from_ref(&revision.oid))
            .unwrap();
        assert_eq!(indexed[&revision.oid].revision, revision);
        // Explicit fetch uses the same budget as discovery, so this immediate
        // observation must not start a second network operation.
        assert!(
            entry
                .observation(false)
                .await
                .unwrap()
                .advertisement()
                .is_none()
        );
        let observed =
            Observation::from_advertisement(repository.discover("origin").unwrap()).unwrap();
        let detail = super::super::view::show(&store, "fixture/garden", observed, &id).unwrap();
        assert!(matches!(
            detail.item.status,
            super::super::view::Status::Indexed
        ));
        assert_eq!(
            entry
                .owner
                .as_ref()
                .unwrap()
                .diagnostics()
                .await
                .counters
                .discoveries_started,
            0
        );
        assert_eq!(
            entry
                .owner
                .as_ref()
                .unwrap()
                .diagnostics()
                .await
                .counters
                .fetches_started,
            1
        );
        // A changed checkout URL cannot redirect an authorized subsequent operation.
        git(
            &configuration.repositories[0].transport_directory,
            &[
                "remote",
                "set-url",
                "origin",
                "https://example.invalid/other.git",
            ],
        );
        let guarded = AuthorizedTransport {
            repository: entry.repository.clone().unwrap(),
            directory: configuration.repositories[0].transport_directory.clone(),
            remote: "origin".into(),
            destination: destination.into(),
        };
        assert!(
            guarded
                .discover()
                .unwrap_err()
                .to_string()
                .contains("authorized")
        );
    }
}
