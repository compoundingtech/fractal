use std::{fs, path::Path};

// Deliberately a source guard, matching the workspace's raw-writer guard convention.
// Whitespace compaction also catches method calls split across lines. Both installation
// and removal bypass the hub; only its owner and the explicit loss probe may do either.
fn guard(relative: &Path, source: &str) {
    if relative == Path::new("crates/smallclaims/src/sqlite/writer_observer.rs") {
        return;
    }
    let mut compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    if relative == Path::new("crates/smallclaims/src/sqlite/commit_scope.rs") {
        let probe = "fn replacement_of_each_native_coverage_hook_invalidates_dependencies_fail_closed()";
        let body = source.split_once(probe).expect("ownership-loss probe must remain").1
            .split_once("\n    }\n").expect("ownership-loss probe must end").0;
        let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        // This is an ownership-coverage probe, not a consumer registration exception:
        // permit exactly the two removals, inside this one named fail-closed test.
        for (hook, signature) in [("commit", "fn()->bool"), ("rollback", "fn()")] {
            let removal = format!("connection.{hook}_hook(None::<{signature}>);");
            assert_eq!(body.matches(&removal).count(), 1, "loss probe must remove {hook} coverage once");
            assert_eq!(compact.matches(&removal).count(), 1, "loss probe exception must not spread");
            compact = compact.replacen(&removal, "", 1);
        }
    }
    for hook in ["commit", "rollback"] {
        for token in [
            format!(".{hook}_hook("),
            format!(".{hook}_hook::<"),
            format!("::{hook}_hook("),
            format!("::{hook}_hook::<"),
            format!("sqlite3_{hook}_hook"),
        ] {
            assert!(!compact.contains(&token),
                "{}: raw {hook} hook bypasses the owner; use smallclaims::sqlite::observe_transaction_resolution",
                relative.display());
        }
    }
}

fn visit(root: &Path, directory: &Path) {
    for entry in fs::read_dir(directory).expect("read workspace source directory") {
        let entry = entry.expect("read workspace source entry");
        let kind = entry.file_type().expect("read workspace source file type");
        let path = entry.path();
        if kind.is_dir() {
            if entry.file_name() != "target" && entry.file_name() != ".git" {
                visit(root, &path);
            }
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            guard(path.strip_prefix(root).unwrap(), &fs::read_to_string(&path).expect("read Rust source"));
        }
    }
}

#[test]
fn workspace_transaction_resolution_hooks_belong_to_the_hub() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
    for member in manifest["workspace"]["members"].as_array().unwrap() {
        visit(root, &root.join(member.as_str().unwrap()));
    }
}
