#!/usr/bin/env python3
"""Focused parser and invariant controls for r15-rename-proof."""
import importlib.machinery
import importlib.util
import json
from pathlib import Path
import subprocess
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

    def test_annotated_raw_and_macro_sql_mutations_are_serialized_changes(self):
        sources = (
            'pub const SCHEMA: &str = /* sqlite-large-value: owner=fractal_store */ '
            '"CREATE INDEX st3_replica_lifecycle_unavailable ON records(id);";',
            '#[rustfmt::skip]\nconst RULES_SQL: &str = indoc::indoc! {r##"\n'
            'CREATE INDEX st3_replica_lifecycle_unavailable ON records(id);\n"##};',
            'const RULES_SQL: &str =\n concat!("CREATE INDEX ", '
            'r#"st3_replica_lifecycle_unavailable"#, " ON records(id);");',
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'schema.rs'
            for source in sources:
                with self.subTest(source=source):
                    path.write_text(source)
                    baseline = proof.inventory(path, 'schema.rs', {}, {})['serialized_constants']
                    self.assertTrue(any('st3_replica_lifecycle_unavailable' in key for key in baseline))
                    path.write_text(source.replace('st3_replica_lifecycle_unavailable', 'fractal_lifecycle_unavailable'))
                    changed = proof.inventory(path, 'schema.rs', {}, {})['serialized_constants']
                    self.assertNotEqual(baseline, changed)

    def test_wit_generated_identifiers_are_frozen_across_all_six_consumers(self):
        paths = [
            'components/github-issue/src/lib.rs', 'components/github-pr/src/lib.rs',
            'components/pty-stats/src/lib.rs', 'components/vista/src/lib.rs',
            'crates/st2-resource-wasip2/src/lib.rs', 'crates/st2-resource-wasip2/tests/executor.rs',
        ]
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            for relative in paths:
                path = tree / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                root = 'fixture' if relative.endswith('executor.rs') else 'crate::bindings::exports'
                path.write_text(f'use {root}::st2::resource_provider::provider_api;\n'
                                '// exports::ignored::resource_provider\n'
                                'const NOTE: &str = "fixture::ignored::resource_provider";\n')
            baseline = proof.wit_namespaces(tree, {'st2-resource-wasip2': 'fractal-resource-wasip2'}, {})
            self.assertEqual(sum(sum(values.values()) for values in baseline.values()), 6)
            for relative in paths:
                with self.subTest(path=relative):
                    path = tree / relative
                    original = path.read_text()
                    path.write_text(original.replace('::st2::', '::fractal_legacy_runner::'))
                    self.assertNotEqual(baseline, proof.wit_namespaces(
                        tree, {'st2-resource-wasip2': 'fractal-resource-wasip2'}, {}))
                    path.write_text(original)

    def test_wit_bytes_and_relative_symlink_targets_are_guarded(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            target = tree / 'provider.wit'
            target.write_text('package st2:resource-provider;\n')
            link = tree / 'nested/provider-api.wit'
            link.parent.mkdir()
            link.symlink_to('../provider.wit')
            baseline = proof.wit_artifacts(tree, {}, {})
            self.assertEqual(proof.symlink_guard(tree), {'checked': 1, 'dangling': []})
            target.write_text('package fractal:resource-provider;\n')
            self.assertNotEqual(baseline, proof.wit_artifacts(tree, {}, {}))
            link.unlink()
            link.symlink_to('../missing.wit')
            result = proof.symlink_guard(tree)
            self.assertEqual(result['checked'], 1)
            self.assertEqual(result['dangling'][0]['path'], 'nested/provider-api.wit')
            self.assertEqual(result['dangling'][0]['target'], '../missing.wit')

    def test_real_nix_evaluates_every_system_and_computed_output_name_without_building(self):
        # Deliberately no skip/mock: missing Nix or evaluation errors fail the guard.
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            subprocess.run(['git', 'init', '--quiet', str(tree)], check=True)

            def evaluate_committed_fixture():
                subprocess.run(['git', 'add', '--all'], cwd=tree, check=True)
                # Use the configured author and normal policy hooks, including
                # in isolated fixtures. Never replace the caller's identity.
                subprocess.run(['git', 'commit', '--quiet', '--allow-empty',
                                '-m', 'Record Nix proof fixture'], cwd=tree, check=True)
                revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=tree, text=True).strip()
                return proof.nix_output_names(tree, revision, repository=tree)

            (tree / 'flake.lock').write_text(
                '{"nodes":{"root":{}},"root":"root","version":7}\n')
            source = '''{
              outputs = { self }: let name = "stable" + "-output"; in {
                packages.x86_64-linux.${name} = throw "must not evaluate derivation";
                packages.aarch64-darwin.other = throw "must not build";
                apps.aarch64-linux.${name} = throw "must not evaluate app";
                checks.x86_64-darwin.${name} = throw "must not evaluate check";
              };
            }'''
            flake = tree / 'flake.nix'
            flake.write_text(source)
            baseline = evaluate_committed_fixture()
            self.assertEqual(baseline, {
                'packages': {'x86_64-linux': ['stable-output'], 'aarch64-darwin': ['other']},
                'apps': {'aarch64-linux': ['stable-output']},
                'checks': {'x86_64-darwin': ['stable-output']},
            })
            for kind, system in (
                ('packages', 'x86_64-linux'), ('apps', 'aarch64-linux'), ('checks', 'x86_64-darwin'),
            ):
                attribute = kind + '.' + system + '.${name}'
                for replacement in (kind + '.' + system + '.renamed', kind + '.riscv64-linux.${name}'):
                    with self.subTest(kind=kind, replacement=replacement):
                        flake.write_text(source.replace(attribute, replacement))
                        self.assertNotEqual(baseline[kind], evaluate_committed_fixture()[kind])
            flake.write_text('{ outputs = { self }: throw "intentional evaluation error"; }')
            with self.assertRaisesRegex(RuntimeError, 'Nix output evaluation failed'):
                evaluate_committed_fixture()
            (tree / 'flake.lock').unlink()
            with self.assertRaisesRegex(ValueError, 'flake.lock is missing'):
                evaluate_committed_fixture()

    def test_npm_reference_resolution_across_text_formats_and_dependency_selectors(self):
        old = {'@smalltalk/fractal-ui': '@fractal/ui'}
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            (tree / 'package.json').write_text(json.dumps({
                'name': 'root', 'dependencies': {'react': '19.0.0'}}))
            package = tree / 'packages/ui/package.json'
            package.parent.mkdir(parents=True)
            package.write_text(json.dumps({'name': '@fractal/ui'}))
            (tree / 'pnpm-lock.yaml').write_text(
                "lockfileVersion: '9.0'\npackages:\n"
                "  '@locked/transitive@1.0.0':\n"
                "    resolution: {integrity: sha512-YQ==}\n")
            cases = (
                ('app.css', "@import '@fractal/ui/tokens.css';", '@fractal/ui', '@smalltalk/fractal-ui'),
                ('index.html', "<script type='module'>import '@fractal/ui';</script>", '@fractal/ui', '@unknown/pkg'),
                ('entry.html', "<script type='module' src='@fractal/ui/web'></script>", '@fractal/ui', '@unknown/pkg'),
                ('tailwind.css', '@source "../../node_modules/@fractal/ui/src";', '@fractal/ui', '@unknown/pkg'),
                ('vite.config.ts', "export default { externalPackages: ['@fractal/ui'] }", '@fractal/ui', '@unknown/pkg'),
                ('build.config.js', "export default { dedupe: ['react'] }", 'react', 'unknown-config-package'),
                ('source.mjs', "export { x } from 'react';", 'react', 'unknown-bare-import'),
                ('module.vue', "<script>const x = import('react');</script>", 'react', '@unknown/pkg'),
                ('guide.md', "```js\nimport { x } from '@fractal/ui';\n```", '@fractal/ui', '@unknown/pkg'),
                ('run', 'pnpm --filter @fractal/ui build', '@fractal/ui', '@unknown/pkg'),
                ('receipt.json', '{"detail":"API comes from @fractal/ui/tokens"}', '@fractal/ui', '@smalltalk/fractal-ui'),
                ('fixture.py', "source = \"import '@fractal/ui';\"", '@fractal/ui', '@unknown/pkg'),
            )
            for relative, source, before, after in cases:
                with self.subTest(path=relative):
                    path = tree / relative
                    path.write_text(source)
                    self.assertEqual(proof.npm_resolution_guard(tree, old)['unresolved'], [])
                    path.write_text(source.replace(before, after))
                    result = proof.npm_resolution_guard(tree, old)
                    self.assertEqual(result['result'], 'FAIL')
                    self.assertTrue(any(item['path'] == relative for item in result['unresolved']))
                    path.unlink()
            for selector in ('workspace:*', 'file:../missing', 'link:../missing'):
                with self.subTest(selector=selector):
                    package.write_text(json.dumps({
                        'name': '@fractal/ui', 'dependencies': {'@unknown/missing': selector}}))
                    self.assertEqual(proof.npm_resolution_guard(tree, old)['result'], 'FAIL')
            package.write_text(json.dumps({
                'name': '@fractal/ui', 'dependencies': {'@smalltalk/fractal-ui': '^1.0.0'}}))
            self.assertEqual(proof.npm_resolution_guard(tree, old)['result'], 'FAIL')

    def test_npm_registry_aliases_builtins_and_nonconsumer_scopes(self):
        old = {'@smalltalk/fractal-ui': '@fractal/ui'}
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            (tree / 'package.json').write_text(json.dumps({
                'name': '@fractal/ui', 'dependencies': {'react': '^19.0.0'}}))
            (tree / 'pnpm-lock.yaml').write_text(
                "lockfileVersion: '9.0'\npackages:\n"
                "  '@locked/transitive@1.0.0':\n"
                "    resolution: {integrity: sha512-YQ==}\n")
            (tree / 'tsconfig.json').write_text(json.dumps({
                '$schema': 'https://example.invalid/tsconfig.json',
                'compilerOptions': {'paths': {'@local/*': ['./src/*']}}}))
            (tree / 'source.ts').write_text(
                "import fs from 'node:fs'; import path from 'path';\n"
                "import { x } from '@locked/transitive/subpath';\n"
                "import { y } from '@local/module';\n"
                "import { buildIdentity } from 'virtual:build-identity';\n")
            (tree / 'source.rs').write_text(
                'use exports::st2::resource_provider::provider_api;\n'
                '#[attribute]\nlet team = "@acme/gardeners";\n'
                'let email = "person@smalltalk/example.com";\n'
                'let url = "https://github.com/@smalltalk/fractal-ui";\n')
            scripts = tree / 'scripts'
            scripts.mkdir()
            (scripts / 'r15-rename-client.py').write_text(
                "PACKAGES = {'@smalltalk/fractal-ui': '@fractal/ui'}\n"
                'source = "import \'@unknown/real-consumer\';"\n')
            result = proof.npm_resolution_guard(tree, old)
            self.assertEqual(result['declaration_literals'], 1)
            self.assertEqual(len(result['unresolved']), 1)
            self.assertEqual(result['unresolved'][0]['reference'], '@unknown/real-consumer')
            (scripts / 'r15-rename-client.py').write_text(
                "PACKAGES = {'@smalltalk/fractal-ui': '@fractal/ui'}\n")
            self.assertEqual(proof.npm_resolution_guard(tree, old)['result'], 'PASS')


if __name__ == '__main__':
    unittest.main()
