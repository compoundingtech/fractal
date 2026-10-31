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
/// not build it: a missing fractal fails, and one older than the fixture may be stale and is warned about.
#[allow(dead_code)]
pub(crate) fn fractal_beside(fixture: &std::path::Path) -> String {
    let fractal = fixture.with_file_name("fractal");
    let modified = |path: &std::path::Path| path.metadata().and_then(|metadata| metadata.modified());
    match (modified(&fractal), modified(fixture)) {
        (Err(error), _) => panic!(
            "{} is missing ({error}): the fractal-cli package builds it; run these tests with -p fractal-cli or --workspace",
            fractal.display()
        ),
        (Ok(fractal_time), Ok(fixture_time)) if fractal_time < fixture_time => eprintln!(
            "warning: {} is older than fractal-fixture and may be stale; build it too with -p fractal-cli",
            fractal.display()
        ),
        _ => {}
    }
    fractal.to_str().expect("test target path is UTF-8").to_owned()
}

/// The fractal a parent test hands down in `NEXTEST_BIN_EXE_fractal` when it reruns itself with a cleared
/// environment (git_evidence), or else fractal beside `fixture`. In an extracted archive that rerun has
/// none of the archive variables, so the fixture resolves to its compiled path, with no fractal beside it.
#[allow(dead_code)]
pub(crate) fn fractal_handed_down_or_beside<'a>(
    handed_down: Option<String>,
    fixture: impl FnOnce() -> &'a std::path::Path,
) -> String {
    handed_down.unwrap_or_else(|| fractal_beside(fixture()))
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
    use super::{fractal_handed_down_or_beside, resolve};

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
