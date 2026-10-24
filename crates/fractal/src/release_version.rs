//! Installer-owned release identity; never changes the baked machine version.
use std::io::Read;
use std::path::{Path, PathBuf};

fn receipt_path(binary: &Path) -> Option<PathBuf> {
    let directory = binary.parent()?;
    if directory.file_name()? == "MacOS" && directory.parent()?.file_name()? == "Contents" {
        let mut path = directory.parent()?.parent()?.as_os_str().to_os_string();
        path.push(".RELEASE.json");
        Some(PathBuf::from(path))
    } else {
        Some(directory.join("RELEASE.json"))
    }
}

fn display(receipt: &Path, machine: &str) -> String {
    let release = (|| {
        if !std::fs::metadata(receipt).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(receipt).ok()?.take(8193).read_to_end(&mut bytes).ok()?;
        if bytes.len() > 8192 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let tag = value.get("tag")?.as_str()?;
        let commit = value.get("commit")?.as_str()?;
        // Compare exactly with the embedded revision, not a user-supplied prefix.
        let revision = machine.rsplit_once('+')?.1;
        if commit.len() < 7
            || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            || commit != revision
            || tag.is_empty()
            || !tag.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return None;
        }
        Some(format!("{tag} ({commit})"))
    })();
    release.unwrap_or_else(|| machine.to_owned())
}

pub(crate) fn print() {
    let machine = fractal_drivers::version::machine_version();
    let version = std::env::current_exe()
        .ok()
        .and_then(|binary| receipt_path(&binary))
        .map_or_else(|| machine.clone(), |path| display(&path, &machine));
    println!("fractal {version}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_release_receipt() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("RELEASE.json");
        std::fs::write(&path, r#"{"tag":"v0.3.18","commit":"abcdef123"}"#).unwrap();
        assert_eq!(display(&path, "0.3.17+abcdef123"), "v0.3.18 (abcdef123)");
    }

    #[test]
    fn mismatching_release_receipt() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("RELEASE.json");
        std::fs::write(&path, r#"{"tag":"v0.3.18","commit":"abcdef123"}"#).unwrap();
        assert_eq!(display(&path, "0.3.17+abcdef124"), "0.3.17+abcdef124");
        assert_eq!(display(&path, "0.3.17+abcdef123-dirty"), "0.3.17+abcdef123-dirty");
    }

    #[test]
    fn missing_or_corrupt_release_receipt() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("RELEASE.json");
        let fallback = "0.3.17+abcdef123";
        assert_eq!(display(&path, fallback), fallback);
        for bytes in ["not json", "{}", r#"{"tag":"v0.3.18\nforged","commit":"abcdef123"}"#] {
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(display(&path, fallback), fallback);
        }
    }

    #[test]
    fn macos_receipt_stays_outside_signed_bundle() {
        assert_eq!(receipt_path(Path::new("/Applications/SmallTalk.app/Contents/MacOS/st3")),
            Some(PathBuf::from("/Applications/SmallTalk.app.RELEASE.json")));
        assert_eq!(receipt_path(Path::new("/home/user/.local/bin/st3")),
            Some(PathBuf::from("/home/user/.local/bin/RELEASE.json")));
    }
}
