//! Root's tile registry: which Unix user is which agent's tile, and what its tile sees.
//!
//! The registry is the only source of a tile's identity. A process is a tile's when the kernel
//! says it runs as the tile's Unix user inside the tile's cgroup, which only root can place it
//! in. Nothing a tile says about itself, such as `ST_AGENT` or `--as`, counts.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// Where root keeps the registry.
pub const DEFAULT_REGISTRY: &str = "/etc/st-tiles/tiles.toml";
/// The systemd slice every tile's slice lives under.
pub const TILES_SLICE: &str = "st_tiles.slice";

/// Root's registry of tiles. Root owns the file; nothing reads one anyone else can change.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// Each tile's private state: `STATE/NAME/home` and `STATE/NAME/work`, owned by the tile.
    #[serde(default = "default_state_root")]
    pub state_root: PathBuf,
    /// Each tile's runtime directories: its endpoint directory and the mount point of its root.
    #[serde(default = "default_runtime_root")]
    pub runtime_root: PathBuf,
    /// Immutable files and directories every tile sees read-only at the same path, such as the
    /// closures of approved tools. Each must be controlled by root. The host root is never bound.
    #[serde(default)]
    pub read_only: Vec<PathBuf>,
    /// `PATH` inside a tile.
    #[serde(default = "default_path")]
    pub path: Vec<PathBuf>,
    /// The sekrets gateway's socket, which every tile reaches at
    /// `/run/st-sekrets/gateway.sock`, the client's default. The gateway names a tile's caller
    /// by its Unix user and cgroup, as this registry does. Unset, a tile reaches no gateway.
    #[serde(default)]
    pub sekrets_socket: Option<PathBuf>,
    /// Tiles by name.
    #[serde(default)]
    pub tiles: BTreeMap<String, Tile>,
}

/// One tile: a durable Unix user that is one agent, never a person.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Tile {
    pub uid: u32,
    pub gid: u32,
    /// The agent this tile is, such as `agent/fleet/fixture-example/atlas`.
    pub agent: String,
    /// The person the agent works for.
    pub person: String,
    /// The Unix user whose st daemon serves the tile's endpoint, normally its person's.
    #[serde(default)]
    pub supervisor_uid: Option<u32>,
    /// Resource ceilings for the tile's tools, in systemd's syntax. Limits, not reservations.
    #[serde(default = "default_tasks_max")]
    pub tasks_max: u32,
    #[serde(default = "default_memory_max")]
    pub memory_max: String,
    #[serde(default)]
    pub memory_high: Option<String>,
    #[serde(default = "default_cpu_weight")]
    pub cpu_weight: u32,
}

fn default_state_root() -> PathBuf {
    "/var/lib/st-tiles".into()
}
fn default_runtime_root() -> PathBuf {
    "/run/st-tiles".into()
}
fn default_path() -> Vec<PathBuf> {
    vec![
        "/run/current-system/sw/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ]
}
fn default_tasks_max() -> u32 {
    512
}
fn default_memory_max() -> String {
    "8G".into()
}
fn default_cpu_weight() -> u32 {
    100
}

impl Registry {
    /// Read the registry, refusing a file anyone but root can change.
    pub fn load(path: &Path) -> Result<Self> {
        root_controlled(path)?;
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parse {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let registry: Self = toml::from_str(text)?;
        registry.check()?;
        Ok(registry)
    }

    fn check(&self) -> Result<()> {
        for (label, root) in [
            ("state_root", &self.state_root),
            ("runtime_root", &self.runtime_root),
        ] {
            if !root.is_absolute() || root == Path::new("/") {
                bail!(
                    "{label}: {} must be an absolute directory, not /",
                    root.display()
                );
            }
        }
        for path in self
            .read_only
            .iter()
            .chain(&self.path)
            .chain(&self.sekrets_socket)
        {
            if !path.is_absolute() || path == Path::new("/") {
                bail!("{} must be an absolute path, not /", path.display());
            }
            if path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                bail!("{} must not contain `..`", path.display());
            }
        }
        for path in &self.read_only {
            for private in [&self.state_root, &self.runtime_root] {
                if path.starts_with(private) || private.starts_with(path) {
                    bail!(
                        "read_only: {} overlaps {}, where tiles keep private state",
                        path.display(),
                        private.display()
                    );
                }
            }
        }
        let mut uids = BTreeMap::new();
        for (name, tile) in &self.tiles {
            check_name(name)?;
            if tile.uid == 0 || tile.gid == 0 {
                bail!("tile {name}: a tile never runs as root");
            }
            if let Some(other) = uids.insert(tile.uid, name) {
                bail!("tiles {other} and {name} share Unix user {}", tile.uid);
            }
            if !tile.agent.starts_with("agent/") || tile.agent.len() <= "agent/".len() {
                bail!(
                    "tile {name}: `{}` is not an agent such as agent/fleet/fixture-example/atlas",
                    tile.agent
                );
            }
            if crate::sekrets::store::person_name(&tile.person).is_none() {
                bail!(
                    "tile {name}: `{}` is not a person such as person/ada",
                    tile.person
                );
            }
            if tile.supervisor_uid == Some(tile.uid) {
                bail!("tile {name}: a tile cannot supervise itself");
            }
            if tile.tasks_max == 0 || tile.cpu_weight == 0 || tile.cpu_weight > 10_000 {
                bail!("tile {name}: tasks_max must be positive and cpu_weight 1 to 10000");
            }
            for value in std::iter::once(&tile.memory_max).chain(&tile.memory_high) {
                if !memory_value(value) {
                    bail!("tile {name}: `{value}` is not a size such as 8G or 512M");
                }
            }
        }
        for tile in self.tiles.values() {
            if let Some(supervisor) = tile.supervisor_uid
                && let Some(name) = uids.get(&supervisor)
            {
                bail!("tile {name} cannot supervise another tile");
            }
        }
        Ok(())
    }

    pub fn tile(&self, name: &str) -> Result<&Tile> {
        self.tiles
            .get(name)
            .with_context(|| format!("no tile named {name} in the registry"))
    }

    /// The tile a Unix user is, if any.
    pub fn tile_for_uid(&self, uid: u32) -> Option<(&str, &Tile)> {
        self.tiles
            .iter()
            .find(|(_, tile)| tile.uid == uid)
            .map(|(name, tile)| (name.as_str(), tile))
    }

    /// The tile a process is, from the kernel's word alone: it runs as the tile's Unix user and
    /// inside the tile's slice, which only root can place it in.
    pub fn identify(&self, uid: u32, cgroup: &str) -> Option<(&str, &Tile)> {
        let (name, tile) = self.tile_for_uid(uid)?;
        cgroup
            .starts_with(&format!("{}/", slice_cgroup(name)))
            .then_some((name, tile))
    }

    pub fn tile_state(&self, name: &str) -> PathBuf {
        self.state_root.join(name)
    }

    pub fn tile_runtime(&self, name: &str) -> PathBuf {
        self.runtime_root.join(name)
    }

    /// The directory the tile's supervisor serves its endpoint in, bound into the tile.
    pub fn endpoint_dir(&self, name: &str) -> PathBuf {
        self.tile_runtime(name).join("endpoint")
    }
}

/// A tile's name: lower-case letters, digits and `_`, so it maps to systemd units unescaped.
pub fn check_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 32
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        bail!("`{name}` is not a tile name: up to 32 of a-z, 0-9 and _, starting with a letter");
    }
    Ok(())
}

fn memory_value(value: &str) -> bool {
    let digits = value.trim_end_matches(['K', 'M', 'G', 'T']);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && value.len() - digits.len() <= 1
        && digits.parse::<u64>().is_ok_and(|n| n > 0)
}

/// The tile's own slice, which holds each of its compartments.
pub fn slice_unit(name: &str) -> String {
    format!("st_tiles-{name}.slice")
}

/// The unit the tile's tools run in.
pub fn tools_unit(name: &str) -> String {
    format!("st-tile-{name}-tools.service")
}

pub fn slice_cgroup(name: &str) -> String {
    format!("/{TILES_SLICE}/{}", slice_unit(name))
}

pub fn tools_cgroup(name: &str) -> String {
    format!("{}/{}", slice_cgroup(name), tools_unit(name))
}

/// Whether `path` and every directory above it can be changed only by root.
pub fn root_controlled(path: &Path) -> Result<()> {
    let mut current = Some(path);
    while let Some(entry) = current {
        let metadata =
            fs::symlink_metadata(entry).with_context(|| format!("inspect {}", entry.display()))?;
        if metadata.file_type().is_symlink() {
            bail!(
                "{} is a symbolic link; name the file itself",
                entry.display()
            );
        }
        let mode = metadata.permissions().mode();
        if metadata.uid() != 0 || (mode & 0o022 != 0 && !(metadata.is_dir() && mode & 0o1000 != 0))
        {
            bail!(
                "{} can be changed by someone other than root (owner {}, mode {:o})",
                entry.display(),
                metadata.uid(),
                mode & 0o7777
            );
        }
        current = entry
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = r#"
state_root = "/var/lib/st-tiles"
runtime_root = "/run/st-tiles"
read_only = ["/nix/store/example-tools"]

[tiles.atlas]
uid = 61001
gid = 61001
agent = "agent/fleet/fixture-example/atlas"
person = "person/ada"
supervisor_uid = 1000

[tiles.birch]
uid = 61002
gid = 61002
agent = "agent/fleet/fixture-example/birch"
person = "person/ada"
"#;

    #[test]
    fn a_registry_names_each_tile_by_uid_and_slice() {
        let registry = Registry::parse(REGISTRY).unwrap();
        let atlas_tools = tools_cgroup("atlas");
        assert_eq!(
            atlas_tools,
            "/st_tiles.slice/st_tiles-atlas.slice/st-tile-atlas-tools.service"
        );
        assert_eq!(
            registry.identify(61001, &atlas_tools).map(|(name, _)| name),
            Some("atlas")
        );
        // Another tile's user in this tile's cgroup, or this user outside it, is no one.
        assert!(registry.identify(61002, &atlas_tools).is_none());
        assert!(registry.identify(61001, &tools_cgroup("birch")).is_none());
        assert!(
            registry
                .identify(61001, "/user.slice/user-61001.slice/session-3.scope")
                .is_none()
        );
        assert!(registry.identify(61001, &slice_cgroup("atlas")).is_none());
        assert!(
            registry
                .identify(61001, "/st_tiles.slice/st_tiles-atlas.slicex/a")
                .is_none()
        );
        assert!(registry.identify(1000, &atlas_tools).is_none());
    }

    #[test]
    fn a_registry_refuses_ambiguous_or_unsafe_tiles() {
        let shared_uid = REGISTRY.replace("uid = 61002", "uid = 61001");
        assert!(Registry::parse(&shared_uid).is_err());
        let root = REGISTRY.replace("uid = 61002", "uid = 0");
        assert!(Registry::parse(&root).is_err());
        let person = REGISTRY.replace("agent/fleet/fixture-example/birch", "person/ada");
        assert!(Registry::parse(&person).is_err());
        let supervised_by_tile =
            REGISTRY.replace("supervisor_uid = 1000", "supervisor_uid = 61002");
        assert!(Registry::parse(&supervised_by_tile).is_err());
        let private = REGISTRY.replace("/nix/store/example-tools", "/var/lib/st-tiles/birch");
        assert!(Registry::parse(&private).is_err());
        let host_root = REGISTRY.replace("/nix/store/example-tools", "/");
        assert!(Registry::parse(&host_root).is_err());
        let dashed = REGISTRY.replace("[tiles.birch]", "[tiles.bi-rch]");
        assert!(Registry::parse(&dashed).is_err());
        let size = REGISTRY.replace(
            "person = \"person/ada\"\nsupervisor_uid",
            "person = \"person/ada\"\nmemory_max = \"lots\"\nsupervisor_uid",
        );
        assert!(Registry::parse(&size).is_err());
        let unknown = format!("{REGISTRY}\nextra = 1\n");
        assert!(Registry::parse(&unknown).is_err());
    }

    #[test]
    fn tile_names_map_to_unescaped_units() {
        for name in ["atlas", "web_builder2"] {
            assert!(check_name(name).is_ok());
        }
        for name in [
            "",
            "Atlas",
            "2atlas",
            "at-las",
            "at.las",
            "at/las",
            &"a".repeat(33),
        ] {
            assert!(check_name(name).is_err(), "{name}");
        }
    }
}
