//! Bake this checkout's build identity, per the shared build-versioning contract. Only the final binaries (st2, st3) depend
//! on this crate and hand the identity to the libraries at startup (`fractal_drivers::version::set_build_stamps`), so a new commit
//! rebuilds those binaries and none of the libraries most of the workspace depends on.
//!
//! - `ST_BUILD_STAMP_LOCAL`: a LocalStamp for a plain `cargo build`, a private var distinct from the fleet's
//!   `CLI_BUILD_STAMP`, so the flake's authoritative NixStamp can never be overridden by this (see
//!   crates/fractal-drivers/src/version.rs). A hermetic Nix build has no `.git`, so this yields nothing there and the NixStamp is used.
//! - `AGENT_SPEC_REVISION`: the source revision st2 reports as `agentSpecRevision`. A supplied value (the flake, CI) must be a
//!   full SHA or an explicit `nix-dirty.*` identity; otherwise the clean HEAD, `local-dirty.<HEAD>` or `local.unknown`.
//!
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn is_sha(revision: &str) -> bool {
    revision.len() == 40
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Tell Cargo which Git files can change the stamp. The directory `refs` is NOT one of them: in a linked worktree
/// `git rev-parse --git-path refs` names the COMMON directory that every worktree of the repository shares, and Cargo
/// rescans a watched directory recursively, so any ref updated by anyone (a fetch, a push, a branch in another worktree)
/// re-ran this script and rebuilt everything that depends on this crate (a whole workspace, ~12 minutes) although this
/// worktree's HEAD had not moved. HEAD, the index and the ONE ref HEAD names are the inputs. While that ref is a loose file
/// only the file is watched; when it exists only in packed-refs (after `git gc` or `git pack-refs`) packed-refs is watched
/// instead, and so is the directory the loose file will appear in, because a new commit creates it there. A path that does
/// not exist is skipped: Cargo treats a missing watched file as always changed.
fn watch_git_inputs() {
    let path_of = |name: &str| git(&["rev-parse", "--git-path", name]);
    let watch = |path: &str| {
        if std::path::Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    };
    for name in ["HEAD", "index"] {
        if let Some(path) = path_of(name) {
            watch(&path);
        }
    }
    let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) else {
        return;
    };
    let Some(loose) = path_of(&branch) else {
        return;
    };
    if std::path::Path::new(&loose).exists() {
        watch(&loose);
        return;
    }
    if let Some(packed) = path_of("packed-refs") {
        watch(&packed);
    }
    if let Some(directory) = std::path::Path::new(&loose).parent().and_then(|p| p.to_str()) {
        watch(directory);
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=AGENT_SPEC_REVISION");
    let supplied = std::env::var("AGENT_SPEC_REVISION")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if let Some(value) = &supplied {
        assert!(
            is_sha(value) || value.starts_with("nix-dirty."),
            "AGENT_SPEC_REVISION must be a full source SHA or explicit nix-dirty identity"
        );
    }
    let full_rev = git(&["rev-parse", "HEAD"]);
    // `--no-optional-locks`: a plain status refreshes the index's stat data and writes the index back when a file was
    // touched but not changed (a fresh worktree, a restored checkout). The index is one of this script's inputs, so that
    // write made the next cargo command rerun this script and rebuild every package that depends on this crate.
    let dirty = full_rev.is_some()
        && git(&["--no-optional-locks", "status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    if let Some(full_rev) = &full_rev {
        let rev = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| full_rev.clone());
        let commit_ts = git(&["log", "-1", "--format=%ct"])
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        // Reject an invalid release identity during the build, before emitting a stamp.
        // This is the same local machine-version grammar as st-drivers::version.
        let version = std::env::var("CARGO_PKG_VERSION")
            .expect("Cargo must supply the package version to the build stamp generator");
        let suffix = if dirty { ".dirty" } else { "" };
        assert!(
            format!("{version}+local.{rev}{suffix}").len() <= 128,
            "build machine version exceeds 128 bytes; correct the package version or revision"
        );
        // Hand-assembled JSON: the short-sha is hex so no escaping is needed, and
        // this avoids a build-dependency just to serialize three fields.
        let stamp =
            format!(r#"{{"type":"local","rev":"{rev}","fullRev":"{full_rev}","commitTs":{commit_ts},"dirty":{dirty}}}"#);
        println!("cargo:rustc-env=ST_BUILD_STAMP_LOCAL={stamp}");
    }
    let revision = supplied.clone().unwrap_or_else(|| match &full_rev {
        None => "local.unknown".to_owned(),
        Some(head) if dirty => format!("local-dirty.{head}"),
        Some(head) => head.clone(),
    });
    println!("cargo:rustc-env=AGENT_SPEC_REVISION={revision}");
    // Performance supplies the real full SHA for a clean checkout. Its value changes with source identity, while fresh
    // index/ref timestamps do not affect this stamp, so only a new value reruns this. Keep deriving the stamp above from Git,
    // including its real commit time.
    if !dirty && supplied.is_some() && supplied == full_rev {
        return;
    }
    // Rebuild the stamp when HEAD moves or the working tree changes (dirty flag).
    // Git resolves the same worktree metadata for each package, including linked-worktree layouts.
    watch_git_inputs();
}
