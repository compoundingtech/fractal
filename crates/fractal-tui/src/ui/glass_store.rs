//! A person's glasses on this device, until the graph keeps them. Alongside each glass's name
//! and splits of tab groups, this file keeps device-local identities, navigation history and
//! the last view; those are not part of the graph's structure.

use super::layout::{Group, Layout, Tab};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// 3 adds device-local navigation history; versions 1 and 2 still load.
const VERSION: u32 = 3;
const MAX_BYTES: u64 = 1 << 20;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    pub version: u32,
    /// The glass `fractal tui --glasses` opens here.
    pub last: Option<String>,
    pub glasses: Vec<StoredGlass>,
    /// Whether the Ctrl+S sidebar is shown here; unset, it is, as the old Fractal TUI's list was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<bool>,
    #[serde(default)]
    pub history: Vec<StoredPlace>,
    #[serde(default)]
    pub history_cursor: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredGlass {
    /// Made here once and never changed, renaming included: the graph keeps a glass under it.
    #[serde(default)]
    pub id: String,
    /// The graph's revision this glass last matched, kept here only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub name: String,
    /// Its splits and their tabs; Home, the first group's first tab, is not stored.
    pub layout: Layout,
    /// Where this device left it: the focused group and each group's shown tab. Kept on this
    /// device only; st keeps structure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<StoredView>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredView {
    pub focus: usize,
    /// Each group's shown tab as its strip counts them: in the first group, 0 is Home.
    pub current: Vec<usize>,
}

/// A content destination and the workspace location where it was shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredPlace {
    pub space: String,
    /// Device-local destination identity; empty only in old history awaiting migration.
    #[serde(default)]
    pub group_id: String,
    /// None for an empty group's Home, or in old history awaiting migration.
    #[serde(default)]
    pub tab_id: Option<String>,
    /// Legacy positions retained to migrate version 3 history on open.
    pub group: usize,
    pub tab: usize,
    pub pane: String,
    /// The Inbox list keeps its selected card without changing the tab's pane kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// Monotonic device-local visit order, including back/forward visits.
    #[serde(default)]
    pub visited: u64,
}

/// Version 1: a glass was tabs, and each tab a tree of split panes.
#[derive(Deserialize)]
struct StoredV1 {
    last: Option<String>,
    glasses: Vec<GlassV1>,
}

#[derive(Deserialize)]
struct GlassV1 {
    #[serde(default)]
    id: String,
    name: String,
    tabs: Vec<TabV1>,
}

#[derive(Deserialize)]
struct TabV1 {
    title: Option<String>,
    layout: LayoutV1,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LayoutV1 {
    Pane { pane: String },
    Split { children: Vec<LayoutV1> },
}

impl LayoutV1 {
    fn panes(self) -> Vec<String> {
        match self {
            LayoutV1::Pane { pane } => vec![pane],
            LayoutV1::Split { children } => {
                children.into_iter().flat_map(LayoutV1::panes).collect()
            }
        }
    }
}

/// A version 1 file in today's shape: every pane it held becomes a tab of one group, in order,
/// a tab's title going to its first pane. Nothing the person opened is lost; splits are.
fn from_v1(old: StoredV1) -> Stored {
    Stored {
        version: VERSION,
        sidebar: None,
        history: Vec::new(),
        history_cursor: 0,
        last: old.last,
        glasses: old
            .glasses
            .into_iter()
            .map(|glass| StoredGlass {
                id: glass.id,
                // The graph never kept the old shape, so no revision there matches it.
                revision: None,
                name: glass.name,
                layout: Layout::Group(Group {
                    tabs: glass
                        .tabs
                        .into_iter()
                        .flat_map(|tab| {
                            let mut title = tab.title;
                            tab.layout.panes().into_iter().map(move |pane| Tab {
                                title: title.take(),
                                ..Tab::pane(pane)
                            })
                        })
                        .collect(),
                    ..Group::default()
                }),
                view: None,
            })
            .collect(),
    }
}

/// Where this person's glasses live on this device.
pub fn path(person: &str) -> Option<PathBuf> {
    let (dir, key) = person_state(person)?;
    Some(dir.join(format!("glasses-{key}.json")))
}

/// The directory Fractal TUI keeps per-device state in, and a stable key for `person` within it.
pub(crate) fn person_state(person: &str) -> Option<(PathBuf, String)> {
    if person.is_empty() {
        return None;
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })?;
    if !base.is_absolute() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    person.hash(&mut hasher);
    Some((
        fractal_client::product_dirs::resolve(base.join("st3"), base.join("fractal")).join("stui"),
        format!("{:016x}", hasher.finish()),
    ))
}

/// What the file holds; nothing when it is missing, unreadable, too big or open to others.
pub fn load(path: &Path) -> Stored {
    let readable = fs::metadata(path).ok().filter(|metadata| {
        metadata.len() <= MAX_BYTES && metadata.permissions().mode() & 0o077 == 0
    });
    let Some(bytes) = readable.and_then(|_| fs::read(path).ok()) else {
        return Stored::default();
    };
    let version = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| value.get("version").and_then(serde_json::Value::as_u64));
    match version {
        Some(1) => serde_json::from_slice::<StoredV1>(&bytes)
            .map(from_v1)
            .unwrap_or_default(),
        Some(2 | 3) => serde_json::from_slice::<Stored>(&bytes).unwrap_or_default(),
        _ => Stored::default(),
    }
}

/// Replace the file at once, readable by this user only.
pub fn save(path: &Path, stored: &Stored) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&Stored {
        version: VERSION,
        ..stored.clone()
    })?;
    if bytes.len() as u64 > MAX_BYTES {
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::Side;

    #[test]
    fn glasses_round_trip_through_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("st3").join("stui").join("glasses.json");
        assert_eq!(load(&path), Stored::default(), "nothing saved yet");
        let mut layout = Layout::Group(Group::of(Tab::pane("agent:agent/example/harbor")));
        layout.split(
            0,
            Side::Right,
            Group::of(Tab {
                title: Some("the audit".into()),
                ..Tab::pane("mission:mission/example/audit")
            }),
        );
        let empty = Group::default();
        let stored = Stored {
            version: VERSION,
            sidebar: None,
            history: vec![
                StoredPlace {
                    space: "0190a1b2-0000-7000-8000-000000000002".into(),
                    group_id: layout.groups()[1].id.clone(),
                    tab_id: Some(layout.groups()[1].tabs[0].id.clone()),
                    group: 1,
                    tab: 0,
                    pane: "mission:mission/example/audit".into(),
                    selection: None,
                    visited: 1,
                },
                StoredPlace {
                    space: "0190a1b2-0000-7000-8000-000000000001".into(),
                    group_id: empty.id.clone(),
                    tab_id: None,
                    group: 0,
                    tab: 0,
                    pane: "home:inbox".into(),
                    selection: None,
                    visited: 2,
                },
            ],
            history_cursor: 0,
            last: Some("review".into()),
            glasses: vec![
                StoredGlass {
                    id: "0190a1b2-0000-7000-8000-000000000001".into(),
                    revision: None,
                    name: "main".into(),
                    layout: Layout::Group(empty),
                    view: None,
                },
                StoredGlass {
                    id: "0190a1b2-0000-7000-8000-000000000002".into(),
                    revision: Some("r2".into()),
                    name: "review".into(),
                    layout,
                    view: Some(StoredView {
                        focus: 1,
                        current: vec![0, 1],
                    }),
                },
            ],
        };
        save(&path, &stored).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded, stored);
        // Structural equality deliberately ignores IDs, so check persistence explicitly.
        for (before, after) in stored.glasses.iter().zip(&loaded.glasses) {
            for (before, after) in before.layout.groups().iter().zip(after.layout.groups()) {
                assert_eq!(after.id, before.id);
                for (before, after) in before.tabs.iter().zip(&after.tabs) {
                    assert_eq!(after.id, before.id);
                }
            }
        }
        assert_eq!(
            loaded.history[0].group_id,
            loaded.glasses[1].layout.groups()[1].id
        );
        assert_eq!(
            loaded.history[0].tab_id.as_deref(),
            Some(loaded.glasses[1].layout.groups()[1].tabs[0].id.as_str())
        );
        assert_eq!(loaded.history[1].tab_id, None);
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // A file others can read is not trusted.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(load(&path), Stored::default());
    }

    #[test]
    fn a_version_1_file_keeps_every_pane_as_a_tab() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glasses.json");
        let old = serde_json::json!({"version": 1, "last": "main", "glasses": [{
        "id": "0190a1b2-0000-7000-8000-000000000003", "revision": "r9", "name": "main",
        "tabs": [
            {"title": "audit work", "layout": {"split": "right", "children": [
                {"pane": "mission:mission/example/audit"}, {"pane": "machine:machine/harbor"}]}},
            {"layout": {"pane": "agent:agent/example/keeper"}}
        ]}]});
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let stored = load(&path);
        assert_eq!(stored.version, VERSION);
        assert_eq!(stored.last.as_deref(), Some("main"));
        let glass = &stored.glasses[0];
        assert_eq!(glass.revision, None, "the graph never kept the old shape");
        let tabs = &glass.layout.groups()[0].tabs;
        assert_eq!(
            tabs.iter()
                .map(|tab| (tab.title.as_deref(), tab.pane.as_str()))
                .collect::<Vec<_>>(),
            [
                (Some("audit work"), "mission:mission/example/audit"),
                (None, "machine:machine/harbor"),
                (None, "agent:agent/example/keeper"),
            ]
        );
    }

    #[test]
    fn old_layouts_and_positional_history_load_before_identity_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glasses.json");
        for version in [2, 3] {
            let mut old = serde_json::json!({
                "version": version, "last": "main",
                "glasses": [{
                    "id": "glass-main", "name": "main",
                    "layout": {"tabs": [{"pane": "agent:a"}]}
                }]
            });
            if version == 3 {
                old["history"] = serde_json::json!([{
                    "space": "glass-main", "group": 0, "tab": 0, "pane": "agent:a"
                }]);
                old["history_cursor"] = serde_json::json!(0);
            }
            fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let loaded = load(&path);
            assert_eq!(loaded.glasses.len(), 1);
            let group = loaded.glasses[0].layout.groups()[0];
            assert_eq!(
                uuid::Uuid::parse_str(&group.id).unwrap().get_version_num(),
                7
            );
            assert_eq!(
                uuid::Uuid::parse_str(&group.tabs[0].id)
                    .unwrap()
                    .get_version_num(),
                7
            );
            if version == 3 {
                let place = &loaded.history[0];
                assert!(place.group_id.is_empty());
                assert_eq!(place.tab_id, None);
                assert_eq!((place.group, place.tab), (0, 0));
                assert_eq!(place.pane, "agent:a");
            } else {
                assert!(loaded.history.is_empty());
            }
            save(&path, &loaded).unwrap();
            let reloaded = load(&path);
            let restored = reloaded.glasses[0].layout.groups()[0];
            assert_eq!(restored.id, group.id);
            assert_eq!(restored.tabs[0].id, group.tabs[0].id);
        }
    }
}
