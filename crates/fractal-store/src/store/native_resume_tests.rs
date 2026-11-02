//! Native-resume tests that read external sessions or parse intents; they live here because native_resume sits below the store.

#![allow(unused_imports, dead_code)]

#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::native_resume_tests_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::native_resume_tests_shared_seats::{argv};
use crate::native_resume::*;
use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use sha2::{Digest as _, Sha256};
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::os::fd::AsRawFd as _;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;

#[test]
fn suspend_preflight_names_selectors_in_typed_launches() {
    for (driver, args, expected) in [
        ("claude", "\"--continue\"", Some("--continue")),
        (
            "claude",
            "\"--resume=native-copper\"",
            Some("--resume=native-copper"),
        ),
        (
            "pi",
            "\"--session\" \"/missing-transcript\"",
            Some("--session"),
        ),
        ("omp", "\"-c\"", Some("-c")),
        ("omp", "\"--no-session\"", Some("--no-session")),
        ("opencode", "\"-s\" \"native-copper\"", Some("-s")),
        ("codex", "\"resume\" \"native-copper\"", Some("resume")),
        ("codex", "\"fork\" \"native-copper\"", Some("fork")),
        ("omp", "\"--model\" \"copper\"", None),
        ("omp", "\"--\" \"--resume\"", None),
        ("codex", "\"--\" \"resume\"", None),
    ] {
        let source = format!(
            "version 2\nagent \"example\" {{ host \"amber\"; workspace \"/workspace\"; harness {driver:?} {{ args {args}; }} }}"
        );
        let member = crate::parse_intent(&source, "amber")
            .unwrap()
            .subjects
            .into_values()
            .find_map(|desired| desired.member)
            .unwrap();
        assert_eq!(
            authored_session_selection(&member).as_deref(),
            expected,
            "{driver}: {args}"
        );
    }
}

