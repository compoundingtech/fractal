//! Reserved fixture paths are derived from the instance, never from configurable overrides.
use std::fs;
use std::path::{Path, PathBuf};

use super::{CLIENT_CONTROL_FIXTURE, Config, STORE_DATABASE_FILES};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct FixturePathRefusal {
    pub code: &'static str,
    pub path: &'static str,
}

impl std::fmt::Display for FixturePathRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {} must be the fixture instance's own non-symlink path", self.code, self.path)
    }
}

impl std::error::Error for FixturePathRefusal {}

fn refusal(path: &'static str) -> FixturePathRefusal {
    FixturePathRefusal { code: "local-only-fixture-path-refused", path }
}

/// The active reserved instance is authoritative even when state_dir was overridden.
/// Recognizing an explicit reserved state path is only a fallback without that instance.
fn fixture_root(state_dir: &Path) -> Option<PathBuf> {
    if let Some(instance) = crate::instance::current()
        && instance.name() == CLIENT_CONTROL_FIXTURE
    {
        return Some(instance.root().to_owned());
    }
    state_dir.ancestors().find(|path| {
        path.file_name().is_some_and(|name| name == CLIENT_CONTROL_FIXTURE)
            && path.parent().and_then(Path::file_name).is_some_and(|name| {
                name == ".st-instance" || name == ".fractal-instance"
            })
    }).map(Path::to_owned)
}

// Validation must work before setup creates any fixture directories. Resolve the
// closest existing ancestor without creating files or following a missing leaf.
fn canonical_or_missing(path: &Path) -> std::io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(error)?;
            let leaf = path.file_name().ok_or_else(|| std::io::Error::other("missing path leaf"))?;
            Ok(canonical_or_missing(parent)?.join(leaf))
        }
        Err(error) => Err(error),
    }
}

fn exact_path(actual: &Path, expected: &Path, root: &Path, kind: &'static str) -> Result<(), FixturePathRefusal> {
    let fail = || refusal(kind);
    // Reject outside-root aliases even when they resolve back to the allowed path.
    if !root.is_absolute() || actual != expected {
        return Err(fail());
    }
    match fs::symlink_metadata(root.parent().ok_or_else(fail)?) {
        Ok(metadata) if metadata.file_type().is_symlink() => return Err(fail()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(fail()),
    }
    let relative = expected.strip_prefix(root).map_err(|_| fail())?;
    let mut prefix = root.to_owned();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            prefix.push(component);
        }
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(fail()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(fail()),
        }
    }
    // Resolve the parent separately: a socket/database leaf may not exist yet.
    let parent = actual.parent().ok_or_else(fail)?;
    let leaf = actual.file_name().ok_or_else(fail)?;
    let resolved = canonical_or_missing(parent).map_err(|_| fail())?.join(leaf);
    let allowed = canonical_or_missing(root).map_err(|_| fail())?.join(relative);
    if resolved != allowed {
        return Err(fail());
    }
    Ok(())
}

fn validate_at_root(config: &Config, root: &Path) -> Result<(), FixturePathRefusal> {
    let state = fractal_client::product_dirs::resolve(root.join("state/st3"), root.join("state/fractal"));
    exact_path(&config.state_dir, &state, root, "state_dir")?;
    exact_path(&super::store_dir(config), &state, root, "store_dir")?;
    exact_path(&config.store_path(), &state.join(STORE_DATABASE_FILES[0]), root, "store_path")?;
    for name in &STORE_DATABASE_FILES[1..] {
        exact_path(&state.join(name), &state.join(name), root, "store_path")?;
    }
    let socket = root.join("run/fractal-client.sock");
    exact_path(&config.client_gateway_socket, &socket, root, "client_gateway_socket")?;
    let bound = super::socket_binding_path(&socket);
    exact_path(&bound, &bound, root, "client_gateway_socket")
}

impl Config {
    /// Validate before any identity/store effects, again at client bind and Fabric exposure.
    pub fn validate_fixture_paths(&self) -> Result<(), FixturePathRefusal> {
        if !self.local_only() {
            return Ok(());
        }
        let root = fixture_root(&self.state_dir).ok_or_else(|| refusal("state_dir"))?;
        validate_at_root(self, &root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> Config {
        Config {
            state_dir: root.join("state/st3"),
            socket: root.join("run/fractal.sock"),
            client_gateway_socket: root.join("run/fractal-client.sock"),
            ..Config::default()
        }
    }

    #[test]
    fn only_own_fixture_paths_are_accepted_without_creating_them() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".st-instance/client-control-fixture");
        let config = fixture(&root);
        validate_at_root(&config, &root).unwrap();
        config.validate_fixture_paths().unwrap();
        assert!(!root.exists());
        let mut explicit = fixture(&root);
        explicit.store_dir = Some(explicit.state_dir.clone());
        validate_at_root(&explicit, &root).unwrap();
    }

    #[test]
    fn ordinary_instances_keep_their_configurable_paths() {
        let home = tempfile::tempdir().unwrap();
        let mut config = fixture(&home.path().join(".st-instance/ordinary"));
        config.store_dir = Some(home.path().join("external-store"));
        config.client_gateway_socket = home.path().join("external-client.sock");
        config.validate_fixture_paths().unwrap();
        assert!(!home.path().join("external-store").exists());
    }

    #[test]
    fn default_other_instance_and_outside_sockets_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".st-instance/client-control-fixture");
        for socket in [
            home.path().join("run/fractal.sock"),
            home.path().join("run/fractal-client.sock"),
            home.path().join(".st-instance/other/run/fractal.sock"),
            home.path().join(".st-instance/other/run/fractal-client.sock"),
            root.join("run/fractal.sock"),
            home.path().join("outside.sock"),
        ] {
            let mut config = fixture(&root);
            config.client_gateway_socket = socket;
            assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "client_gateway_socket");
        }
    }

    #[test]
    fn state_and_store_overrides_cannot_move_reserved_instance_authority() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".st-instance/client-control-fixture");
        let other = home.path().join(".st-instance/other/state/st3");
        fs::create_dir_all(&other).unwrap();
        let mut config = fixture(&root);
        config.store_dir = Some(other.clone());
        assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "store_dir");
        config.store_dir = None;
        config.state_dir = other;
        assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "state_dir");
        assert!(!root.exists());
    }

    #[test]
    fn symlinked_state_store_database_and_socket_are_refused() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".st-instance/client-control-fixture");
        let config = fixture(&root);
        let other = home.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::create_dir_all(config.state_dir.parent().unwrap()).unwrap();
        symlink(&other, &config.state_dir).unwrap();
        assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "state_dir");
        fs::remove_file(&config.state_dir).unwrap();
        fs::create_dir_all(&config.state_dir).unwrap();
        let alias = root.join("store-alias");
        symlink(&config.state_dir, &alias).unwrap();
        let mut aliased = fixture(&root);
        aliased.store_dir = Some(alias);
        assert_eq!(validate_at_root(&aliased, &root).unwrap_err().path, "store_dir");
        symlink(other.join("claims.sqlite3"), config.store_path()).unwrap();
        assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "store_path");
        fs::remove_file(config.store_path()).unwrap();
        fs::create_dir_all(config.client_gateway_socket.parent().unwrap()).unwrap();
        symlink(other.join("client.sock"), &config.client_gateway_socket).unwrap();
        assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "client_gateway_socket");
        fs::remove_file(&config.client_gateway_socket).unwrap();
        fs::remove_dir(config.client_gateway_socket.parent().unwrap()).unwrap();
        symlink(&other, root.join("run")).unwrap();
        assert_eq!(validate_at_root(&config, &root).unwrap_err().path, "client_gateway_socket");
    }
}
