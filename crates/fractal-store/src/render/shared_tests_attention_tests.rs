#![allow(unused_imports)]
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use anyhow::{Context as _, Result};
use serde_json::Value;
use sha2::Digest as _;
use crate::model::DesiredSubject;
use crate::store::Store;
use crate::render::*;

pub fn git(workspace: &Path, arguments: &[&str]) {
        let status = crate::test_support::git()
            .args([
                "-c",
                "user.name=Example",
                "-c",
                "user.email=example@example.invalid",
            ])
            .args(arguments)
            .current_dir(workspace)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?}");
    }

pub fn harness_agent(workspace: &Path) -> crate::model::NormalizedIntent {
        crate::graph::parse_test_intent(
            &format!(
                "version 2\nagent \"one\" {{ workspace {:?}; harness \"codex\" {{}} }}\n",
                workspace.display().to_string()
            ),
            "node",
        )
        .unwrap()
    }
