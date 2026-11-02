//! This build's identity, by the shared build-versioning contract: the same stamp and
//! formatting as st2 and st-drivers (`crates/fractal-drivers/src/version.rs`, compiled here as is),
//! fed at startup by the binary that links this library (`set_build_stamps`) with the stamps
//! crates/fractal-build-stamp bakes: the LocalStamp for a plain `cargo build`, the flake's
//! `CLI_BUILD_STAMP` for a Nix build.

#[allow(dead_code)]
#[path = "../../fractal-drivers/src/version.rs"]
mod shared;

pub use shared::{display_version, set_build_stamps};

/// For the footer: the machineVersion and how long ago its commit was, `0.1.0+local.ab12cd3 ·
/// 3 hours ago`.
pub fn short(now: u64) -> String {
    let identity = shared::build_identity();
    match shared::relative_time(identity.commit_unix, now) {
        Some(age) => format!("{} · {age}", shared::machine_version()),
        None => shared::machine_version(),
    }
}

/// This build as the client header names it: `stui 0.1.0+local.ab12cd3`.
pub fn client_name() -> String {
    client_name_with(&|name| std::env::var(name).ok())
}

// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read — DELETE at contraction — https://github.com/compoundingtech/fractal/issues/3032
fn client_name_with(get: &dyn Fn(&str) -> Option<String>) -> String {
    let label = get("FRACTAL_CLIENT_LABEL").or_else(|| get("STUI_CLIENT_LABEL"));
    client_name_labelled(label.as_deref())
}
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read

/// A test client names itself apart (`FRACTAL_CLIENT_LABEL=measure`) so a profile can tell its
/// connection from a person's; the label is never identity or authority.
fn client_name_labelled(label: Option<&str>) -> String {
    let label = label
        .map(|label| label.chars().filter(|c| c.is_ascii_graphic()).take(32).collect::<String>())
        .filter(|label| !label.is_empty());
    match label {
        Some(label) => format!("stui {} [{label}]", shared::machine_version()),
        None => format!("stui {}", shared::machine_version()),
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn client_label_prefers_fractal_and_empty_masks_legacy() {
        for (legacy, fractal, expected) in [
            (None, None, None),
            (Some("legacy"), None, Some("legacy")),
            (None, Some("fractal"), Some("fractal")),
            (Some("legacy"), Some("fractal"), Some("fractal")),
            (Some("legacy"), Some(""), None),
        ] {
            let name = super::client_name_with(&|key| match key {
                "STUI_CLIENT_LABEL" => legacy.map(str::to_owned),
                "FRACTAL_CLIENT_LABEL" => fractal.map(str::to_owned),
                _ => None,
            });
            assert_eq!(name, super::client_name_labelled(expected));
        }
    }

    #[test]
    fn a_test_client_labels_its_name_and_a_person_does_not() {
        assert!(!super::client_name_labelled(None).contains('['));
        assert!(super::client_name_labelled(Some("measure")).ends_with(" [measure]"));
        assert!(!super::client_name_labelled(Some(" \n")).contains('['));
    }

    #[test]
    fn the_footer_names_the_shared_machine_version() {
        assert!(super::short(super::now()).starts_with(&super::shared::machine_version()));
    }
}
