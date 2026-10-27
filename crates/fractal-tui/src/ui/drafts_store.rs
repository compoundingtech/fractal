//! What was typed and not yet sent, kept on this device so a crash, a restart or a reconnect
//! never loses it (Nathan, 2026-10-11: "I was just typing a big message and the tui crashed and
//! lost it"). One file beside the prefs, never synced through st: a draft is the person's own
//! words until they send them. Every change to the boxes is written at once, by key (the agent
//! whose box it is), and only the keys this Fractal TUI changed, so two stuis on one device do not
//! undo each other.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A draft nobody touched for this long is dropped when the file is next written.
const KEEP_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Saved {
    text: String,
    at: u64,
}

type File = BTreeMap<String, Saved>;

/// `$XDG_CACHE_HOME/st3/stui/drafts.json` (or under `~/.cache`). The cache, not the state, as
/// with the run files: a device that only pairs with a member must keep no st state, and typing
/// a message is not a reason to create it.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    base.is_absolute()
        .then(|| base.join("st3").join("stui").join("drafts.json"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn read(path: &Path) -> File {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The drafts a previous Fractal TUI left, by key.
pub fn load(path: &Path) -> HashMap<String, String> {
    read(path)
        .into_iter()
        .filter(|(_, saved)| !saved.text.trim().is_empty())
        .map(|(key, saved)| (key, saved.text))
        .collect()
}

/// Write what changed between `before` (what this Fractal TUI last wrote or read) and `now`: a key
/// with new text is set, a key gone or empty is removed. Other keys in the file stay as they are.
pub fn save(
    path: &Path,
    before: &HashMap<String, String>,
    current: &HashMap<String, String>,
) -> std::io::Result<()> {
    let live = |text: &String| !text.trim().is_empty();
    let mut file = read(path);
    let at = now();
    for (key, text) in current.iter().filter(|(_, text)| live(text)) {
        if before.get(key) != Some(text) {
            file.insert(
                key.clone(),
                Saved {
                    text: text.clone(),
                    at,
                },
            );
        }
    }
    for key in before.keys() {
        if !current.get(key).is_some_and(live) {
            file.remove(key);
        }
    }
    file.retain(|_, saved| saved.at + KEEP_SECS > at);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension(format!("json.{}.partial", std::process::id()));
    write_private(&partial, &serde_json::to_vec(&file)?)?;
    std::fs::rename(partial, path)
}

/// What typing has changed since the last write: the file, what it holds as this Fractal TUI last wrote
/// it, and the boxes now. Kept so a panic can write it on the way out (`flush_unsaved`).
type Unsaved = (PathBuf, HashMap<String, String>, HashMap<String, String>);
static UNSAVED: std::sync::Mutex<Option<Unsaved>> = std::sync::Mutex::new(None);

/// Tests that note or flush the unsaved boxes take this first: the box is process-wide.
#[cfg(test)]
pub(crate) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Remember the boxes that are not written yet, for a panic to write.
pub fn note_unsaved(
    path: &Path,
    before: &HashMap<String, String>,
    current: &HashMap<String, String>,
) {
    if let Ok(mut unsaved) = UNSAVED.lock() {
        *unsaved = Some((path.to_owned(), before.clone(), current.clone()));
    }
}

/// Nothing is waiting any more: the boxes were written.
pub fn clear_unsaved() {
    if let Ok(mut unsaved) = UNSAVED.lock() {
        *unsaved = None;
    }
}

/// Write what typing changed and no write has taken yet. Called by the panic hook, so it never
/// waits for a lock a panicking thread might hold.
pub fn flush_unsaved() {
    if let Ok(mut unsaved) = UNSAVED.try_lock()
        && let Some((path, before, current)) = unsaved.take()
    {
        let _ = save(&path, &before, &current);
    }
}

/// The file holds the person's unsent words: only they read it. Not flushed to disk: a crash of
/// Fractal TUI leaves the write with the operating system, and only a power cut could lose it.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, text)| ((*key).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn a_draft_survives_a_restart_and_a_missing_file_is_no_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("st3").join("stui").join("drafts.json");
        assert!(load(&path).is_empty());
        let typed = map(&[("agent/example/atlas", "a long message\nwith two lines")]);
        save(&path, &HashMap::new(), &typed).unwrap();
        assert_eq!(load(&path), typed);
    }

    #[test]
    fn a_sent_or_emptied_draft_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        let typed = map(&[("a", "one"), ("b", "two")]);
        save(&path, &HashMap::new(), &typed).unwrap();
        save(&path, &typed, &map(&[("a", "one"), ("b", "  ")])).unwrap();
        assert_eq!(load(&path), map(&[("a", "one")]));
        save(&path, &map(&[("a", "one")]), &HashMap::new()).unwrap();
        assert!(load(&path).is_empty());
    }

    #[test]
    fn two_tuis_do_not_undo_each_others_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        let first = map(&[("a", "from the first")]);
        save(&path, &HashMap::new(), &first).unwrap();
        // A second Fractal TUI, started before the first typed, writes only its own key.
        save(&path, &HashMap::new(), &map(&[("b", "from the second")])).unwrap();
        assert_eq!(
            load(&path),
            map(&[("a", "from the first"), ("b", "from the second")])
        );
    }

    #[test]
    fn an_old_draft_is_dropped_and_a_corrupt_file_is_no_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        let mut file = File::new();
        file.insert(
            "old".into(),
            Saved {
                text: "forgotten".into(),
                at: now() - KEEP_SECS - 10,
            },
        );
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        save(&path, &HashMap::new(), &map(&[("new", "kept")])).unwrap();
        assert_eq!(load(&path), map(&[("new", "kept")]));
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(load(&path).is_empty());
    }

    #[test]
    fn a_panic_writes_what_typing_has_not_yet() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        let typed = map(&[("a", "the last words")]);
        note_unsaved(&path, &HashMap::new(), &typed);
        flush_unsaved();
        assert_eq!(load(&path), typed);
        // Written once: a second flush has nothing to do.
        std::fs::remove_file(&path).unwrap();
        flush_unsaved();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drafts.json");
        save(&path, &HashMap::new(), &map(&[("a", "secret")])).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}
