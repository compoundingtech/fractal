#!/usr/bin/env python3
"""Focused parser and invariant controls for r15-rename-proof."""
import ast
import importlib.machinery
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
loader = importlib.machinery.SourceFileLoader('r15_proof', str(ROOT / 'scripts/r15-rename-proof'))
spec = importlib.util.spec_from_loader(loader.name, loader)
proof = importlib.util.module_from_spec(spec)
loader.exec_module(proof)


class RenameProof(unittest.TestCase):
    def test_indirect_directory_literals_follow_path_assignments_and_loops(self):
        spec = importlib.util.spec_from_file_location('script_paths', ROOT / 'scripts/r15-rename-scripts.py')
        scripts = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(scripts)
        source = ('for package in ("st3-client", "st3-views"):\n'
                  '    package_path = f"clients/typescript/{package}"\n'
                  '    source = (root / package_path / "BUCK").read_text()\n'
                  'label = "st3-client"\n')
        matches = scripts.path_directory_literals(source, scripts.moved_directory_names())
        self.assertEqual([row[3] for row in matches], ['st3-client', 'st3-views'])
        rewritten = scripts.rewrite_path_directory_literals(source)
        self.assertIn('("fractal-client", "fractal-views")', rewritten)
        self.assertIn('label = "st3-client"', rewritten)
        self.assertFalse(scripts.path_directory_literals(rewritten, scripts.moved_directory_names()))
        embedded = "python3 - <<'PY'\n" + source + "PY\n"
        self.assertEqual(len(scripts.path_directory_literals(embedded, scripts.moved_directory_names())), 2)
        assigned = ('package = "st3-client"\n'
                    'base = root / "clients" / "typescript"\n'
                    'path = base / package / "BUCK"\n')
        self.assertEqual(len(scripts.path_directory_literals(assigned, scripts.moved_directory_names())), 1)
        local = 'local = root / "assistant-ui"\npath = local / "st3-views"\n'
        self.assertFalse(scripts.path_directory_literals(local, scripts.moved_directory_names()))

    def test_documented_flake_selectors_resolve_local_and_remote_outputs(self):
        outputs = {
            'packages': {'x86_64-linux': ['st-chatgpt']},
            'apps': {}, 'checks': {'x86_64-linux': ['genie-freshness']},
            'devShells': {'x86_64-linux': ['web', 'genie', 'perf']},
        }
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            (tree / 'docs').mkdir()
            path = tree / 'docs/build.md'
            valid = ('nix build .#st-chatgpt\nnix develop .#web\n'
                     'nix build github:compoundingtech/smalltalk/<commit>#st-chatgpt\n'
                     'nix build .#checks.<system>.genie-freshness\n')
            path.write_text(valid)
            self.assertEqual(proof.documented_flake_selector_guard(tree, outputs)['checked'], 4)
            self.assertEqual(proof.documented_flake_selector_guard(tree, outputs)['result'], 'PASS')
            for invalid in ('nix build .#fractal-chatgpt', 'nix build .#web',
                            'nix build .#checks.x86_64-linux.suite',
                            'nix build github:compoundingtech/smalltalk/main#missing'):
                path.write_text(valid + invalid)
                self.assertEqual(proof.documented_flake_selector_guard(tree, outputs)['result'], 'FAIL')

    def test_docs_preserve_executables_and_command_guard_rejects_crate_names(self):
        spec = importlib.util.spec_from_file_location('rename_docs', ROOT / 'scripts/r15-rename-docs.py')
        docs = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(docs)
        old, new = 'st-fabric-adapter', 'fractal-' + 'fabric-adapter'
        text = f'{old} --once\n`{old}` is a separate process\n~/.local/state/{old}\ncargo test -p {old}\n'
        expected = text.replace(f'-p {old}', f'-p {new}')
        self.assertEqual(docs.rewrite(text), expected)
        self.assertEqual(docs.rewrite(expected), expected)
        for prefix in ('.#', 'github:compoundingtech/smalltalk/<commit>#'):
            self.assertEqual(docs.rewrite('nix build ' + prefix + 'fractal-chatgpt'),
                             'nix build ' + prefix + 'st-chatgpt')
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            (tree / 'scripts').mkdir()
            (tree / 'docs').mkdir()
            path = tree / 'docs/adapter.md'
            path.write_text(expected)
            self.assertEqual(proof.executable_command_guard(tree, {old}, {old: new})['result'], 'PASS')
            for command in (f'{new} --once', f'`{new} --status`',
                            f'subprocess.run(["{new}", "--once"])'):
                path.write_text(command)
                self.assertEqual(proof.executable_command_guard(tree, {old}, {old: new})['result'], 'FAIL')

    def test_staged_npm_authority_requires_declared_path_and_pinned_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            source = 'repos/member/packages/@fixture/widget'
            (tree / 'pnpm-source-inputs.json').write_text(json.dumps({'sourceInputPaths': [source]}))
            (tree / 'megarepo.lock').write_text(json.dumps({
                'members': {'member': {'commit': 'a' * 40}}}))
            workspace = tree / 'pnpm-workspace.yaml'
            workspace.write_text(
                'overrides:\n  "@fixture/widget": "file:.devenv/pnpm-source-inputs/current/' + source + '"\n')
            with patch.object(proof.subprocess, 'check_output',
                              return_value=json.dumps({'name': '@fixture/widget'})) as read:
                self.assertEqual(proof.staged_npm_packages(tree), {'@fixture/widget'})
                self.assertEqual(read.call_args.args[0][-1],
                                 'a' * 40 + ':packages/@fixture/widget/package.json')
            with patch.object(proof.subprocess, 'check_output',
                              return_value=json.dumps({'name': '@fixture/wrong'})):
                with self.assertRaisesRegex(ValueError, 'identity mismatch'):
                    proof.staged_npm_packages(tree)
            (tree / 'pnpm-source-inputs.json').write_text('{"sourceInputPaths":[]}')
            with self.assertRaisesRegex(ValueError, 'no declared source input'):
                proof.staged_npm_packages(tree)

    def test_embedded_cargo_fixture_keys_resolve_after_script_rewrite(self):
        loader = importlib.machinery.SourceFileLoader(
            'r15_scripts_test', str(ROOT / 'scripts/r15-rename-scripts.py'))
        spec = importlib.util.spec_from_loader(loader.name, loader)
        scripts = importlib.util.module_from_spec(spec)
        loader.exec_module(scripts)
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            crate = tree / 'crates/fractal-build-stamp'
            crate.mkdir(parents=True)
            (crate / 'Cargo.toml').write_text('[package]\nname="fractal-build-stamp"\n')
            (tree / 'scripts').mkdir()
            path = tree / 'scripts/fixture'
            manifest = ('[dependencies]\nst-build-stamp={path="crates/fractal-build-stamp"}\n'
                        'library={path="crates/library"}\n')
            source = "(work / 'Cargo.toml').write_text(" + repr(manifest) + ")\n"
            path.write_text(source)
            result = proof.embedded_cargo_guard(tree)
            self.assertEqual(result['result'], 'FAIL')
            self.assertEqual(result['checked'], 1)
            self.assertEqual(result['unresolved'][0]['dependency'], 'st-build-stamp')
            updated = scripts.rewrite(Path('scripts/ci-build-snapshot-test'), source)
            self.assertNotEqual(updated, source)
            self.assertEqual(scripts.rewrite(Path('scripts/ci-build-snapshot-test'), updated), updated)
            path.write_text(updated)
            self.assertEqual(proof.embedded_cargo_guard(tree)['result'], 'PASS')
            # Explicit Cargo renames resolve; an incorrect package field does not.
            for package, expected in (('fractal-build-stamp', 'PASS'), ('missing', 'FAIL')):
                manifest = ('[dev-dependencies]\nalias={path="./crates/fractal-build-stamp",'
                            'package="' + package + '"}\n')
                path.write_text("(work / 'Cargo.toml').write_text(" + repr(manifest) + ")\n")
                self.assertEqual(proof.embedded_cargo_guard(tree)['result'], expected)

    def test_client_rewrite_selects_non_ts_sources_but_not_recorded_data(self):
        source = ast.parse((ROOT / 'scripts/r15-rename-client.py').read_text())
        (ROOT / 'tmp/r15').mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / 'tmp/r15') as directory:
            tree = Path(directory)
            namespace = {'ROOT': tree, 'Path': Path}
            for node in source.body:
                if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name):
                    name = node.targets[0].id
                    if name in {'SOURCE_SUFFIXES', 'GENERATED', 'EXCLUDED_ROOTS'}:
                        namespace[name] = ast.literal_eval(node.value)
            selected = next(node for node in source.body
                            if isinstance(node, ast.FunctionDef) and node.name == 'selected')
            exec(compile(ast.Module(body=[selected], type_ignores=[]), '<client-selection>', 'exec'), namespace)
            for relative in ('apps/web/app.css', 'apps/web/index.html', 'apps/web/vite.config.ts',
                             'apps/web/tailwind.config.json', 'apps/web/tsconfig.json',
                             'apps/web/tsconfig.jsonc', 'apps/ios/scripts/fixture.py',
                             'apps/web/scripts/ci/fixtures-gate.sh'):
                with self.subTest(path=relative):
                    self.assertTrue(namespace['selected'](tree / relative))
            for relative in ('clients/typescript/fractal-scenarios/recordings/session.normalized.json',
                             'clients/typescript/fractal-scenarios/recordings/session.recording.jsonl',
                             'fixtures/scenarios/session.json',
                             'release-notes/source-package.json'):
                with self.subTest(path=relative):
                    self.assertFalse(namespace['selected'](tree / relative))

    def test_client_relative_package_rewrites_preserve_unmoved_local_folders(self):
        source = ast.parse((ROOT / 'scripts/r15-rename-client.py').read_text())
        with tempfile.TemporaryDirectory(dir=ROOT / 'tmp/r15') as directory:
            tree = Path(directory)
            namespace = {'ROOT': tree, 'Path': Path, 're': __import__('re')}
            for node in source.body:
                if (isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
                        and node.targets[0].id in {'PACKAGES', 'PATHS'}):
                    namespace[node.targets[0].id] = ast.literal_eval(node.value)
            rewrite = next(node for node in source.body
                           if isinstance(node, ast.FunctionDef) and node.name == 'rewrite')
            exec(compile(ast.Module(body=[rewrite], type_ignores=[]), '<client-rewrite>', 'exec'), namespace)
            local = tree / 'packages/fractal-ui/src/assistant-ui'
            (local / 'st3-views').mkdir(parents=True)
            (local / 'composition').mkdir()
            source = "import { SyncLine } from '../st3-views/SyncLine'"
            path = local / 'composition/Transcript.tsx'
            self.assertEqual(namespace['rewrite'](source, path), source)
            self.assertEqual(namespace['rewrite'](source.replace('st3-', 'fractal-'), path), source)
            canonical = tree / 'clients/typescript/fractal-views'
            canonical.mkdir(parents=True)
            client = tree / 'clients/typescript/fractal-client'
            client.mkdir()
            self.assertEqual(namespace['rewrite'](source, client / 'view.ts'),
                             source.replace('st3-', 'fractal-'))

    def test_rust_rewrite_changes_source_names_not_prompt_or_transcript_strings(self):
        source = ast.parse((ROOT / 'scripts/r15-rename-rust.py').read_text())
        namespace = {
            're': __import__('re'),
            'PACKAGES': {'st3': 'fractal', 'st2': 'fractal-legacy-runner', 'stui': 'fractal-tui'},
            'IDENTIFIERS': {'st3': 'fractal_daemon', 'st2': 'fractal_legacy_runner'},
        }
        names = {'SELF_ALIAS', 'IDENTIFIER_TOKEN', 'IMPORT_TOKEN', 'RUST_TEXT'}
        for node in source.body:
            if (isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
                    and node.targets[0].id in names) or (
                    isinstance(node, ast.FunctionDef) and node.name in {'rust_names', 'rust_source_names'}):
                exec(compile(ast.Module(body=[node], type_ignores=[]), '<rust-rewrite>', 'exec'), namespace)
        rewrite = namespace['rust_source_names']
        source = '''use st3::Client;
// cargo test -p stui
let prompt = "use st2 message and use st3 conversations";
let transcript = r##"$ cargo test -p stui"##;
let bytes = br#"use st3 conversations"#;
'''
        expected = source.replace('use st3::', 'use fractal_daemon::').replace(
            '// cargo test -p stui', '// cargo test -p fractal-tui')
        self.assertEqual(rewrite(source), expected)
        self.assertEqual(rewrite(expected), expected)

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
                'devShells': {},
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
                'name': 'root', 'workspaces': ['packages/*'], 'dependencies': {'react': '19.0.0'}}))
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

    def test_npm_recorded_and_synthetic_payloads_do_not_become_module_imports(self):
        old = {'@smalltalk/fractal-ui': '@fractal/ui'}
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            (tree / 'package.json').write_text(json.dumps({'name': '@fractal/ui'}))
            payload = "import { x } from '@atlas/api'; pnpm --filter @atlas/api test"
            row = {'kind': 'http', 'ordinal': 1, 'receive_ms': 0, 'socket_id': None,
                   'value': {'body': payload}}
            (tree / 'capture.jsonl').write_text(json.dumps(row) + '\n')
            (tree / 'normalized.json').write_text(json.dumps([row]))
            (tree / 'conversation.json').write_text(json.dumps({
                'format': 'st3.scenario.slice.v1', 'source': {'_tag': 'recorded'},
                'state': {'text': payload}}))
            (tree / 'script.json').write_text(json.dumps({
                'format_version': 2, 'agent': 'a', 'anchor': 'a', 'reads': [],
                'turns': [payload], 'world': 'w', 'id_map': {}}))
            world = tree / 'world.ts'
            world.write_text(
                "import { x } from '@fractal/ui';\n"
                "const command = 'pnpm --filter @atlas/api test';\n"
                'const snippet = "import { x } from \'@atlas/api\'";\n')
            (tree / 'module.wat').write_text(
                '(module (memory (export "memory") 1) (import "host" "alloc" (func)))')
            baseline = proof.npm_resolution_guard(tree, old)
            self.assertEqual(baseline['result'], 'PASS')
            self.assertEqual(baseline['recorded_payloads'], 4)
            original = world.read_text()
            for consumer in (
                "import { x } from '@unknown/actual';",
                "// source package: @smalltalk/fractal-ui",
                "execSync('pnpm --filter @unknown/actual test');",
                "const cmd = 'pnpm --filter @unknown/actual test'; execSync(cmd);",
            ):
                with self.subTest(consumer=consumer):
                    world.write_text(original + consumer)
                    self.assertEqual(proof.npm_resolution_guard(tree, old)['result'], 'FAIL')
            world.write_text(original)
            # Ordinary JSON is not a recording just because it contains a snippet.
            (tree / 'config.json').write_text(json.dumps({'source': "import '@unknown/actual';"}))
            self.assertEqual(proof.npm_resolution_guard(tree, old)['result'], 'FAIL')

    def test_npm_workspace_authority_does_not_leak_from_isolated_fixture_manifests(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = Path(directory)
            (tree / 'package.json').write_text(json.dumps({
                'name': 'root', 'workspaces': ['packages/*'],
                'devDependencies': {'@types/hast': '^3.0.0'}}))
            package = tree / 'packages/ui/package.json'
            package.parent.mkdir(parents=True)
            package.write_text(json.dumps({'name': '@fractal/ui'}))
            fixture = tree / 'evals/seed/package.json'
            fixture.parent.mkdir(parents=True)
            fixture.write_text(json.dumps({'name': '@acme/signal'}))
            (fixture.parent / 'index.js').write_text("import { x } from '@acme/signal';\n")
            source = tree / 'packages/ui/index.ts'
            source.write_text("import type { Root } from 'hast';\n")
            result = proof.npm_resolution_guard(tree, {})
            self.assertEqual(result['result'], 'PASS')
            self.assertEqual(result['workspace_names'], ['@fractal/ui', 'root'])
            self.assertEqual(result['isolated_manifests'], 1)
            for mutation in ("import { x } from '@acme/signal';", "import { x } from 'hast';"):
                with self.subTest(mutation=mutation):
                    source.write_text(mutation)
                    self.assertEqual(proof.npm_resolution_guard(tree, {})['result'], 'FAIL')

    def test_npm_historical_release_fragments_are_immutable_not_live_consumers(self):
        old = {'@smalltalk/fractal-ui': '@fractal/ui'}
        with tempfile.TemporaryDirectory() as directory:
            base, tree = Path(directory) / 'base', Path(directory) / 'head'
            for root in (base, tree):
                (root / 'release-notes').mkdir(parents=True)
                (root / 'release-notes/history.json').write_text(
                    json.dumps({'summary': 'Adopt `@smalltalk/fractal-ui`.'}))
            (tree / 'package.json').write_text(json.dumps({'name': '@fractal/ui'}))
            result = proof.npm_resolution_guard(tree, old, base)
            self.assertEqual(result['result'], 'PASS')
            self.assertEqual(result['immutable_release_fragments'], 1)
            historical = tree / 'release-notes/history.json'
            historical.write_text(json.dumps({'summary': 'Adopt `@fractal/ui`.'}))
            result = proof.npm_resolution_guard(tree, old, base)
            self.assertEqual(result['result'], 'FAIL')
            self.assertEqual(result['unresolved'][0]['reason'], 'historical release fragment changed')
            historical.unlink()
            result = proof.npm_resolution_guard(tree, old, base)
            self.assertEqual(result['unresolved'][0]['reason'], 'historical release fragment removed')
            historical.write_bytes((base / 'release-notes/history.json').read_bytes())
            (tree / 'release-notes/new.json').write_text(
                json.dumps({'summary': 'Adopt `@smalltalk/fractal-ui`.'}))
            self.assertEqual(proof.npm_resolution_guard(tree, old, base)['result'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
