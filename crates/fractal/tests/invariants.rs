use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn raw_string_start(line: &str) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    for index in 0..bytes.len() {
        if index > 0
            && (bytes[index - 1].is_ascii_alphanumeric() || matches!(bytes[index - 1], b'_' | b'"'))
        {
            continue;
        }
        let mut cursor = match bytes[index..] {
            [b'r', ..] => index + 1,
            [b'b', b'r', ..] => index + 2,
            _ => continue,
        };
        let mut hashes = 0;
        while bytes.get(cursor) == Some(&b'#') {
            hashes += 1;
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'"') {
            return Some((cursor + 1, hashes));
        }
    }
    None
}

fn raw_string_closes(line: &str, hashes: usize) -> bool {
    line.as_bytes()
        .windows(hashes + 1)
        .any(|window| window.first() == Some(&b'"') && window[1..].iter().all(|byte| *byte == b'#'))
}

fn declared_tests(source: &str) -> BTreeSet<String> {
    let mut tests = BTreeSet::new();
    let mut modules = Vec::<(usize, String)>::new();
    let mut saw_test_attribute = false;
    let mut raw_string_hashes = None;

    for line in source.lines() {
        if let Some(hashes) = raw_string_hashes {
            if raw_string_closes(line, hashes) {
                raw_string_hashes = None;
            }
            continue;
        }
        if line.trim_start().starts_with("//") {
            continue;
        }
        let code = line;
        if let Some((content_start, hashes)) = raw_string_start(code) {
            if !raw_string_closes(&code[content_start..], hashes) {
                raw_string_hashes = Some(hashes);
            }
            continue;
        }
        let trimmed = code.trim();
        let indentation = code.len() - code.trim_start().len();
        if trimmed == "}" {
            while modules
                .last()
                .is_some_and(|(module_indentation, _)| *module_indentation >= indentation)
            {
                modules.pop();
            }
        }

        let declaration = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
        if let Some(rest) = declaration.strip_prefix("mod ")
            && let Some((name, _)) = rest.split_once('{')
        {
            modules.push((indentation, name.trim().to_owned()));
            saw_test_attribute = false;
            continue;
        }
        if trimmed.starts_with("#[") {
            if trimmed.contains("test") {
                saw_test_attribute = true;
            }
            continue;
        }
        if !saw_test_attribute || trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        let declaration = declaration.strip_prefix("async ").unwrap_or(declaration);
        if let Some(rest) = declaration.strip_prefix("fn ")
            && let Some((name, _)) = rest.split_once('(')
        {
            let name = name.trim();
            tests.insert(name.to_owned());
            if !modules.is_empty() {
                let mut qualified = modules
                    .iter()
                    .map(|(_, module)| module.as_str())
                    .collect::<Vec<_>>()
                    .join("::");
                qualified.push_str("::");
                qualified.push_str(name);
                tests.insert(qualified);
            }
        }
        saw_test_attribute = false;
    }

    tests
}

#[test]
fn declared_tests_preserve_module_qualification() {
    let source = r##"
#[test]
fn root_proof() {}

#[cfg(test)]
mod tests {
    fn fixture() -> &'static str {
        r#"
}
"#
    }

    #[test]
    fn module_proof() {}

    mod nested {
        #[test]
        fn nested_proof() {}
    }
}
"##;
    let tests = declared_tests(source);

    assert!(tests.contains("root_proof"));
    assert!(tests.contains("module_proof"));
    assert!(tests.contains("tests::module_proof"));
    assert!(tests.contains("tests::nested::nested_proof"));
    assert!(!tests.contains("wrong::module_proof"));
}

#[test]
fn qualified_proof_references_resolve() {
    let root = Path::new(test_env!("CARGO_MANIFEST_DIR")).join("../..");
    let invariants = include_str!("../../../docs/st3/invariants.md");
    let mut checked = 0;

    for (index, span) in invariants.split('`').enumerate() {
        if index % 2 == 0 {
            continue;
        }
        let Some((source_path, test_name)) = span.split_once(".rs::") else {
            if span.ends_with(".rs") {
                assert!(
                    root.join(span).is_file(),
                    "invariant proof source `{span}` does not exist"
                );
            }
            continue;
        };
        let relative_path = format!("{source_path}.rs");

        let source = fs::read_to_string(root.join(&relative_path))
            .unwrap_or_else(|error| panic!("proof source {relative_path} is unreadable: {error}"));
        let tests = declared_tests(&source);
        assert!(
            tests.contains(test_name),
            "invariant proof `{span}` does not name a declared test"
        );
        checked += 1;
    }

    assert!(
        checked >= 20,
        "expected a substantial qualified proof set, found only {checked}"
    );
}

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

fn visit(root: &Path, directory: &Path, prefix: &Path) {
    for entry in fs::read_dir(directory).expect("read workspace source directory") {
        let entry = entry.expect("read workspace source entry");
        let kind = entry.file_type().expect("read workspace source file type");
        let path = entry.path();
        if kind.is_dir() {
            if entry.file_name() != "target" && entry.file_name() != ".git" {
                visit(root, &path, prefix);
            }
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            guard(&prefix.join(path.strip_prefix(root).unwrap()), &fs::read_to_string(&path).expect("read Rust source"));
        }
    }
}

#[test]
fn workspace_transaction_resolution_hooks_belong_to_the_hub() {
    let root = Path::new(test_env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
    for member in manifest["workspace"]["members"].as_array().unwrap() {
        visit(&root, &root.join(member.as_str().unwrap()), Path::new(""));
    }
}
