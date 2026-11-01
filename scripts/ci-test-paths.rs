// Test-only relocation for same-source nextest archives. Ordinary cargo test retains its
// compiled paths. No production version or build-identity environment value is changed.
pub(crate) fn resolve(
    name: &str,
    compiled: &str,
    build: Option<&str>,
    current: Option<&str>,
    executable: Option<&str>,
    archive: bool,
) -> Result<String, &'static str> {
    if name == "CARGO_MANIFEST_DIR" {
        match (build, current) {
            (Some(build), Some(current)) => {
                let relative = std::path::Path::new(compiled)
                    .strip_prefix(build)
                    .map_err(|_| "manifest directory is outside the recorded build workspace")?;
                std::path::Path::new(current)
                    .join(relative)
                    .to_str()
                    .map(str::to_owned)
                    .ok_or("test checkout path is not UTF-8")
            }
            (None, None) if !archive => Ok(compiled.to_owned()),
            _ => Err("both archive workspace identities are required"),
        }
    } else if name.starts_with("CARGO_BIN_EXE_") || name == "CARGO_TARGET_TMPDIR" {
        match executable {
            Some(path) => Ok(path.to_owned()),
            None if !archive => Ok(compiled.to_owned()),
            None => Err("archive test executable must be supplied by nextest"),
        }
    } else {
        Err("only test path variables are supported")
    }
}

#[allow(unused_macros)]
macro_rules! test_env {
    ($name:expr) => {
        test_env!($name, "")
    };
    ($name:expr, $suffix:literal) => {{
        static VALUE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        VALUE
            .get_or_init(|| {
                let build = std::env::var("CI_TEST_BUILD_WORKSPACE").ok();
                let current = std::env::var("CI_TEST_WORKSPACE").ok();
                let executable = if $name == "CARGO_TARGET_TMPDIR" {
                    std::env::var("CI_TEST_TARGET_TMPDIR").ok()
                } else {
                    $name.strip_prefix("CARGO_BIN_EXE_").and_then(|name| {
                        std::env::var(format!("NEXTEST_BIN_EXE_{}", name.replace('-', "_")))
                            .or_else(|_| std::env::var(format!("NEXTEST_BIN_EXE_{name}")))
                            .or_else(|_| std::env::var($name))
                            .ok()
                    })
                };
                let value = crate::ci_test_paths::resolve(
                    $name,
                    env!($name),
                    build.as_deref(),
                    current.as_deref(),
                    executable.as_deref(),
                    std::env::var_os("CI_TEST_EXTRACTED").is_some(),
                )
                .expect("valid exact-source test archive paths");
                format!("{value}{}", $suffix)
            })
            .as_str()
    }};
}

#[allow(unused_macros)]
macro_rules! test_bin {
    ($name:literal) => {
        std::path::Path::new(test_env!(concat!("CARGO_BIN_EXE_", $name)))
    };
}

/// fractal beside `fixture`, where the fractal-cli package puts it in the same target directory (`cargo
/// test --workspace`, `-p fractal-cli`, or a nextest archive of both). A plain `cargo test -p fractal` does
/// not build it, so `verify` holds it to Cargo's own freshness rule: it fails when it is missing, when the
/// dep-info Cargo writes beside it (`fractal.d`) is missing, or when a source file that lists is newer than
/// it, as after a change to the CLI library that only the fixture and the tests were rebuilt for. An
/// extracted archive is not verified: one `cargo nextest archive` built its fractal with its tests, and its
/// dep-info names the build machine's sources.
#[allow(dead_code)]
pub(crate) fn fractal_beside(fixture: &std::path::Path, verify: bool) -> Result<String, String> {
    let fractal = fixture.with_file_name("fractal");
    let rerun = "the fractal-cli package builds it; run these tests with -p fractal-cli or --workspace";
    let built = fractal
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("{} is missing ({error}): {rerun}", fractal.display()))?;
    if verify {
        let dep_info = fractal.with_file_name("fractal.d");
        let listed = std::fs::read_to_string(&dep_info)
            .map_err(|error| format!("{} is missing ({error}), so {} cannot be checked against its sources: {rerun}",
                dep_info.display(), fractal.display()))?;
        if let Some(source) = dep_info_sources(&listed).into_iter().find(|source| {
            std::fs::metadata(source).and_then(|metadata| metadata.modified()).is_ok_and(|modified| modified > built)
        }) {
            return Err(format!("{} is older than {}, a source it was built from: {rerun}", fractal.display(), source.display()));
        }
    }
    fractal.to_str().map(str::to_owned).ok_or_else(|| "test target path is not UTF-8".to_owned())
}

/// The sources a Cargo dep-info file lists after its `target:`, with `\ ` read as a space and `\` line
/// continuations joined.
#[allow(dead_code)]
pub(crate) fn dep_info_sources(text: &str) -> Vec<std::path::PathBuf> {
    let text = text.replace("\\\n", " ");
    let mut sources = Vec::new();
    for line in text.lines() {
        let Some((_, listed)) = line.split_once(": ") else { continue };
        let mut current = String::new();
        let mut characters = listed.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '\\' if characters.peek() == Some(&' ') => current.push(characters.next().unwrap()),
                ' ' => {
                    if !current.is_empty() {
                        sources.push(std::path::PathBuf::from(std::mem::take(&mut current)));
                    }
                }
                other => current.push(other),
            }
        }
        if !current.is_empty() {
            sources.push(std::path::PathBuf::from(current));
        }
    }
    sources
}

/// The fractal a parent test hands down in `NEXTEST_BIN_EXE_fractal` when it reruns itself with a cleared
/// environment (git_evidence), or else fractal beside `fixture`. In an extracted archive that rerun has
/// none of the archive variables, so the fixture resolves to its compiled path, with no fractal beside it.
#[allow(dead_code)]
pub(crate) fn fractal_handed_down_or_beside<'a>(
    handed_down: Option<String>,
    fixture: impl FnOnce() -> &'a std::path::Path,
) -> String {
    handed_down.unwrap_or_else(|| {
        fractal_beside(fixture(), std::env::var_os("CI_TEST_EXTRACTED").is_none()).unwrap_or_else(|error| panic!("{error}"))
    })
}

/// The fractal executable, another package's binary, as a path string (see `fractal_handed_down_or_beside`).
#[allow(unused_macros)]
macro_rules! fractal_bin {
    () => {{
        static VALUE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        VALUE
            .get_or_init(|| {
                crate::ci_test_paths::fractal_handed_down_or_beside(
                    std::env::var("NEXTEST_BIN_EXE_fractal").ok(),
                    || test_bin!("fractal-fixture"),
                )
            })
            .as_str()
    }};
}

#[cfg(test)]
mod tests {
    use super::{dep_info_sources, fractal_beside, fractal_handed_down_or_beside, resolve};

    #[test]
    fn a_library_fixture_uses_its_own_relocated_manifest() {
        assert_eq!(
            resolve(
                "CARGO_MANIFEST_DIR",
                "/build/crates/fractal",
                Some("/build"),
                Some("/consumer"),
                None,
                true
            )
            .unwrap(),
            "/consumer/crates/fractal"
        );
    }

    #[test]
    fn nextest_supplies_the_relocated_executable() {
        assert_eq!(
            resolve(
                "CARGO_BIN_EXE_fractal-fixture",
                "/build/target/debug/fractal-fixture",
                Some("/build"),
                Some("/consumer"),
                Some("/extract/target/debug/fractal-fixture"),
                true
            )
            .unwrap(),
            "/extract/target/debug/fractal-fixture"
        );
    }

    #[test]
    fn a_rerun_uses_the_fractal_its_parent_handed_down() {
        // In an extracted archive, git_evidence's env-cleared rerun resolves the fixture to its
        // compiled path, and no fractal sits beside that; only the fractal its parent handed down exists.
        let compiled_fixture =
            std::path::Path::new("/absent-build-tree/target/debug/fractal-fixture");
        assert_eq!(
            fractal_handed_down_or_beside(Some("/archive/target/debug/fractal".to_owned()), || {
                compiled_fixture
            }),
            "/archive/target/debug/fractal"
        );
    }

    /// A target directory with fractal-fixture, fractal built at `built`, its dep-info listing one source
    /// modified at `source_modified`, and that source.
    fn target_directory(name: &str, built: std::time::SystemTime, source_modified: std::time::SystemTime) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ci-test-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("debug")).unwrap();
        std::fs::create_dir_all(root.join("crate dir")).unwrap();
        let source = root.join("crate dir/main.rs");
        std::fs::write(&source, "fn main() {}").unwrap();
        std::fs::File::options().write(true).open(&source).unwrap().set_modified(source_modified).unwrap();
        std::fs::write(root.join("debug/fractal-fixture"), "").unwrap();
        let fractal = root.join("debug/fractal");
        std::fs::write(&fractal, "").unwrap();
        std::fs::File::options().write(true).open(&fractal).unwrap().set_modified(built).unwrap();
        let escaped = source.display().to_string().replace(' ', "\\ ");
        std::fs::write(root.join("debug/fractal.d"), format!("{}: {escaped}\n", fractal.display())).unwrap();
        root
    }

    #[test]
    fn an_old_fractal_left_after_a_library_change_is_refused() {
        // `cargo test -p fractal` after a change to the CLI library rebuilds the fixture and the tests but not
        // fractal, which the fractal-cli package builds: its sources are newer than it.
        let now = std::time::SystemTime::now();
        let root = target_directory("stale", now - std::time::Duration::from_secs(600), now);
        let error = fractal_beside(&root.join("debug/fractal-fixture"), true).unwrap_err();
        assert!(error.contains("older than") && error.contains("main.rs") && error.contains("-p fractal-cli"), "{error}");
        // An extracted archive is not checked against sources: one archive run built fractal with its tests.
        assert!(fractal_beside(&root.join("debug/fractal-fixture"), false).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_fractal_newer_than_its_sources_is_used() {
        let now = std::time::SystemTime::now();
        let root = target_directory("fresh", now, now - std::time::Duration::from_secs(600));
        assert_eq!(
            fractal_beside(&root.join("debug/fractal-fixture"), true).unwrap(),
            root.join("debug/fractal").to_str().unwrap()
        );
        std::fs::remove_file(root.join("debug/fractal.d")).unwrap();
        let error = fractal_beside(&root.join("debug/fractal-fixture"), true).unwrap_err();
        assert!(error.contains("fractal.d is missing"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dep_info_sources_read_escaped_spaces_and_continuations() {
        assert_eq!(
            dep_info_sources("/t/debug/fractal: /a/b.rs /a/with\\ space.rs \\\n /c/d.rs\n"),
            ["/a/b.rs", "/a/with space.rs", "/c/d.rs"].map(std::path::PathBuf::from)
        );
    }

    #[test]
    fn ordinary_cargo_test_retains_the_original_paths() {
        for name in [
            "CARGO_MANIFEST_DIR",
            "CARGO_BIN_EXE_st2",
            "CARGO_TARGET_TMPDIR",
        ] {
            assert_eq!(
                resolve(name, "/original", None, None, None, false).unwrap(),
                "/original"
            );
        }
    }

    #[test]
    fn stale_or_partial_archive_paths_fail_closed() {
        assert!(
            resolve(
                "CARGO_MANIFEST_DIR",
                "/stale/crate",
                Some("/build"),
                Some("/consumer"),
                None,
                true
            )
            .is_err()
        );
        assert!(
            resolve(
                "CARGO_MANIFEST_DIR",
                "/build/crate",
                Some("/build"),
                None,
                None,
                true
            )
            .is_err()
        );
        assert!(resolve("CARGO_MANIFEST_DIR", "/build/crate", None, None, None, true).is_err());
        assert!(resolve("CARGO_BIN_EXE_st2", "/old/st2", None, None, None, true).is_err());
        assert!(resolve("CARGO_TARGET_TMPDIR", "/old/target", None, None, None, true).is_err());
        assert!(resolve("CARGO_PKG_VERSION", "1", None, None, None, false).is_err());
    }
}
