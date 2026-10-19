#!/usr/bin/env python3
"""Focused parser and invariant controls for r15-rename-proof."""
import importlib.machinery
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
loader = importlib.machinery.SourceFileLoader('r15_proof', str(ROOT / 'scripts/r15-rename-proof'))
spec = importlib.util.spec_from_loader(loader.name, loader)
proof = importlib.util.module_from_spec(spec)
loader.exec_module(proof)


class RenameProof(unittest.TestCase):
    def test_rust_lexing_keeps_raw_values_and_ignores_comments_and_lifetimes(self):
        source = '''// "ignored"
fn f<'a>(x: &'a str) { let a = r##"st3.v1"##; let b = b"ST_AGENT"; }
/* "ignored too" */'''
        self.assertEqual([value for value, _ in proof.literals(source, '.rs')], ['st3.v1', 'ST_AGENT'])

    def test_source_normalization_does_not_rename_runtime_names(self):
        crates = {'st3-client': 'fractal-client', 'st3': 'fractal'}
        self.assertEqual(proof.source_literal('crates/st3-client/src/lib.rs', crates, {}),
                         'crates/fractal-client/src/lib.rs')
        for runtime in ('st3.v1', 'st3-client.sock', '~/.config/st3', 'ST_AGENT', 'st3.replica-batch.v1'):
            self.assertEqual(proof.source_literal(runtime, crates, {}), runtime)

    def test_hunks_are_counted_once_and_pure_moves_have_no_content_hunks(self):
        patch = '''diff --git a/Cargo.toml b/Cargo.toml
@@ -1 +1 @@
-name = "st3"
+name = "fractal"
diff --git a/a.rs b/b.rs
similarity index 100%
rename from a.rs
rename to b.rs
diff --git a/c.rs b/c.rs
@@ -1 +1 @@
-use st3_client::Client;
+use fractal_client::Client;
diff --git a/README.md b/README.md
@@ -1 +1 @@
-old
+new
diff --git a/code.rs b/code.rs
@@ -1 +1 @@
-const OLD: u32 = 1;
+const NEW: u32 = 1;
'''
        self.assertEqual(proof.hunk_kinds(patch), {
            'Cargo.toml/package.json names': 1, 'use or import paths': 1, 'docs': 1, 'other': 1})

    def test_each_runtime_category_detects_an_actual_mutation(self):
        scratch = ROOT / 'tmp/r15'
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='proof-test-', dir=scratch) as directory:
            path = Path(directory) / 'sample.rs'
            original = '''const SCHEMA_NAME: &str = "st3.v1";
let env = std::env::var("ST_AGENT");
let socket = "/state/st3.sock";
'''
            path.write_text(original)
            baseline = proof.inventory(path, 'sample.rs', {}, {})
            for category, old, new in (
                ('serialized_constants', 'st3.v1', 'st3.v2'),
                ('wire_storage', 'st3.v1', 'st3.v2'),
                ('environment', 'ST_AGENT', 'FRACTAL_AGENT'),
                ('on_disk_paths', '/state/st3.sock', '/state/fractal.sock'),
            ):
                with self.subTest(category=category):
                    path.write_text(original.replace(old, new))
                    changed = proof.inventory(path, 'sample.rs', {}, {})
                    self.assertNotEqual(baseline[category], changed[category])


if __name__ == '__main__':
    unittest.main()
