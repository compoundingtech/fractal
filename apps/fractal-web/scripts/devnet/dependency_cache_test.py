#!/usr/bin/env python3
"""Networkless filesystem and lease coverage for immutable dependency caching."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import dependency_cache


HELPER = Path(__file__).with_name('dependency_cache.py').resolve()


class DependencyCacheTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='dependency-cache-')
        self.addCleanup(temporary.cleanup)
        self.fixture = Path(temporary.name)
        self.addCleanup(lambda: dependency_cache._make_directories_writable(self.fixture))
        self.source = self.fixture / 'source'
        self.source.mkdir()
        self.cache = self.fixture / 'cache'
        self.cache.mkdir()
        for index, name in enumerate(dependency_cache.INSTALL_INPUTS):
            (self.source / name).write_bytes(f'input {index}\n'.encode())
        self.dependency = self.source / 'node_modules/package/index.js'
        self.dependency.parent.mkdir(parents=True)
        self.dependency.write_text('original dependency\n')
        nested = self.source / 'apps/web/node_modules/package/index.js'
        nested.parent.mkdir(parents=True)
        nested.write_text('nested dependency\n')
        (self.source / 'node_modules/workspace').symlink_to('../apps/web')

    def populate(self):
        key = dependency_cache.install_key(self.source)
        with (self.cache / '.cache.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            dependency_cache.populate(self.source, self.cache, key)
        return key

    def change_key(self, value):
        (self.source / 'pnpm-lock.yaml').write_text(value)
        return dependency_cache.install_key(self.source)

    def generations(self):
        return {path.name for path in self.cache.iterdir()
                if path.is_dir() and not path.name.startswith('.')}

    def selection(self):
        return json.loads((self.cache / '.selection.json').read_text())

    def fingerprint(self, generation):
        result = {}
        for path in sorted(generation.rglob('*')):
            info = path.lstat()
            content = (os.readlink(path) if path.is_symlink() else
                       hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None)
            result[path.relative_to(generation).as_posix()] = (
                info.st_ino, info.st_mtime_ns, info.st_mode, content)
        return result

    def test_exact_install_key_order_and_cli_stdout(self):
        names = ('pnpm-lock.yaml', 'pnpm-workspace.yaml', 'megarepo.kdl',
                 'megarepo.lock', 'pnpm-source-inputs.json')
        expected = hashlib.sha256()
        for name in names:
            expected.update(name.encode())
            expected.update((self.source / name).read_bytes())
        self.assertEqual(dependency_cache.INSTALL_INPUTS, names)
        public_key = expected.hexdigest()
        expected_key = hashlib.sha256(
            b'fractal-dependency-key-v2\0' + public_key.encode()
            + b'\0' + hashlib.sha256(b'').hexdigest().encode()).hexdigest()
        self.assertEqual(dependency_cache.public_install_key(self.source), public_key)
        self.assertEqual(dependency_cache.install_key(self.source), expected_key)
        result = subprocess.run([sys.executable, str(HELPER), 'key', str(self.source)],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, expected_key + '\n')
        self.assertEqual(result.stderr, '')
        for name in names:
            original = (self.source / name).read_bytes()
            (self.source / name).write_bytes(original + b'changed')
            self.assertNotEqual(dependency_cache.install_key(self.source), expected_key)
            (self.source / name).write_bytes(original)

    def test_private_install_inputs_change_key_without_public_changes(self):
        original = dependency_cache.install_key(self.source)
        public = dependency_cache.public_install_key(self.source)
        private = self.source / 'apps/fractal-web/private'
        private.mkdir(parents=True)
        manifest = private / 'package.json'
        manifest.write_text('{"dependencies":{"tool":"1"}}\n')
        first = self.populate()
        self.assertNotEqual(first, original)
        manifest.write_text('{"dependencies":{"tool":"2"}}\n')
        second = self.populate()
        self.assertNotEqual(second, first)
        self.assertEqual(dependency_cache.public_install_key(self.source), public)
        self.assertEqual(self.generations(), {first, second})
        for name in ('pnpm-lock.yaml', '.npmrc', 'nested/package.json', 'patches/tool.patch'):
            path = private / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('{}\n' if path.name == 'package.json' else 'new private install input\n')
            changed = dependency_cache.install_key(self.source)
            self.assertNotEqual(changed, second)
            second = changed
        ignored = private / 'node_modules/tool/package.json'
        ignored.parent.mkdir(parents=True)
        ignored.write_text('installed output is not an input')
        self.assertEqual(dependency_cache.install_key(self.source), second)
        vendor = self.source / 'apps/fractal-web/vendor/tool/package.json'
        vendor.parent.mkdir(parents=True)
        vendor.write_text('{"dependencies":{"vendor-tool":"1"}}\n')
        self.assertNotEqual(dependency_cache.install_key(self.source), second)

    def test_private_scripts_local_closure_and_unrelated_files_invalidate(self):
        public = dependency_cache.public_install_key(self.source)
        private = self.source / 'apps/fractal-web/private'
        private.mkdir(parents=True)
        local = self.source / 'packages/local'
        local.mkdir(parents=True)
        nested = self.source / 'packages/nested'
        nested.mkdir(parents=True)
        workspace = self.source / 'packages/workspace'
        workspace.mkdir(parents=True)
        (private / 'package.json').write_text(json.dumps({
            'scripts': {'postinstall': 'node install.cjs'},
            'dependencies': {'local': 'file:../../../packages/local',
                             'workspace': 'workspace:*'}}))
        (local / 'package.json').write_text(json.dumps({
            'name': 'local', 'dependencies': {'nested': 'link:../nested'}}))
        (nested / 'package.json').write_text('{"name":"nested"}')
        (workspace / 'package.json').write_text('{"name":"workspace"}')
        paths = [private / 'install.cjs', local / 'index.js', nested / 'source.js',
                 workspace / 'index.js', private / 'unrelated.txt']
        for path in paths:
            path.write_text('original source\n')
        previous = self.populate()
        for path in paths:
            with self.subTest(path=path.relative_to(self.source)):
                path.write_text('changed source\n')
                refreshed = f'reinstalled after {path.relative_to(self.source)}\n'
                self.dependency.write_text(refreshed)
                current = self.populate()
                self.assertNotEqual(current, previous)
                self.assertEqual(dependency_cache.public_install_key(self.source), public)
                installed = self.cache / current / 'node_modules/package/index.js'
                self.assertEqual(installed.read_text(), refreshed)
                previous = current
        generated = private / 'build/generated.js'
        generated.parent.mkdir()
        generated.write_text('build paths are included unless proven safe to exclude')
        self.assertNotEqual(dependency_cache.install_key(self.source), previous)

    def test_private_local_input_escape_and_missing_workspace_fail_closed(self):
        private = self.source / 'apps/fractal-web/private'
        private.mkdir(parents=True)
        manifest = private / 'package.json'
        manifest.write_text(json.dumps({'dependencies': {
            'outside': 'file:' + str(self.fixture)}}))
        with self.assertRaisesRegex(ValueError, 'escapes source'):
            dependency_cache.install_key(self.source)
        manifest.write_text('{"dependencies":{"missing":"workspace:*"}}')
        with self.assertRaisesRegex(ValueError, 'resolve uniquely'):
            dependency_cache.install_key(self.source)

    def test_population_copies_source_and_reuse_is_immutable(self):
        key = self.populate()
        generation = self.cache / key
        installed = generation / 'node_modules/package/index.js'
        self.assertNotEqual(installed.stat().st_ino, self.dependency.stat().st_ino)
        self.assertEqual(dependency_cache.verify_generation(generation, key),
                         ['node_modules', 'apps/web/node_modules'])
        self.assertEqual(os.readlink(generation / 'node_modules/workspace'), '../apps/web')
        before = self.fingerprint(generation)
        self.dependency.write_text('mutated source dependency\n')
        self.assertEqual(installed.read_text(), 'original dependency\n')
        self.assertEqual(self.populate(), key)
        self.assertEqual(self.fingerprint(generation), before)
        result = subprocess.run([sys.executable, str(HELPER), 'populate', str(self.source),
                                 str(self.cache), key], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '')
        self.assertIn('reused key=' + key, result.stderr)
        self.assertEqual(self.fingerprint(generation), before)

    def test_publication_removes_write_bits_preserving_reads_and_execution(self):
        self.dependency.chmod(0o775)
        key = self.populate()
        generation = self.cache / key
        self.assertEqual((generation / 'node_modules/package/index.js').stat().st_mode & 0o777,
                         0o555)
        for path in (generation, *dependency_cache._entries(generation)):
            self.assertEqual(path.stat().st_mode & 0o222, 0, str(path))

    def test_clean_reuse_never_hashes_payload_and_accepts_snapshot_hardlinks(self):
        key = self.populate()
        installed = self.cache / key / 'node_modules/package/index.js'
        snapshot = self.fixture / 'snapshot-link'
        os.link(installed, snapshot)
        with mock.patch.object(dependency_cache, '_sha256',
                               side_effect=AssertionError('clean reuse hashed a file')):
            self.assertEqual(self.populate(), key)
            snapshot.unlink()
            self.assertEqual(self.populate(), key)

    def test_changed_metadata_rehashes_retires_and_rebuilds_from_source(self):
        for change in ('mode', 'mtime', 'content', 'inode', 'directory'):
            with self.subTest(change=change):
                key = self.populate()
                generation = self.cache / key
                installed = generation / 'node_modules/package/index.js'
                held = self.fixture / 'snapshot-link'
                os.link(installed, held)
                if change == 'mode':
                    installed.chmod(0o644)
                elif change == 'mtime':
                    info = installed.stat()
                    os.utime(installed, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
                elif change == 'content':
                    installed.chmod(0o644)
                    installed.write_text('corrupted cache bytes\n')
                    installed.chmod(0o444)
                elif change == 'inode':
                    installed.parent.chmod(0o755)
                    installed.unlink()
                    shutil.copy2(held, installed)
                    installed.parent.chmod(0o555)
                else:
                    installed.parent.chmod(0o755)
                old_mode = held.stat().st_mode
                self.dependency.write_text(f'fresh source for {change}\n')
                with mock.patch.object(dependency_cache, '_sha256',
                                       wraps=dependency_cache._sha256) as hashing:
                    self.assertEqual(self.populate(), key)
                # Diagnostic hashing reads the public generation, not just staging.
                self.assertTrue(any(Path(call.args[0]).is_relative_to(generation)
                                    for call in hashing.call_args_list))
                self.assertFalse(os.path.samefile(held, installed))
                self.assertEqual(held.stat().st_mode, old_mode,
                                 'garbage cleanup chmodded a payload hardlink')
                self.assertEqual(installed.read_text(), f'fresh source for {change}\n')
                self.assertEqual(dependency_cache.verify_generation(generation, key),
                                 ['node_modules', 'apps/web/node_modules'])
                self.assertEqual(self.selection(), {'current': key, 'previous': None})
                self.assertEqual(list((self.cache / '.garbage').iterdir()), [])
                held.unlink()

    def test_corrupt_generation_waits_for_lease_without_chmod_or_retirement(self):
        key = self.populate()
        generation = self.cache / key
        installed = generation / 'node_modules/package/index.js'
        info = installed.stat()
        os.utime(installed, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
        before = self.fingerprint(generation)
        selection = self.selection()
        flock = fcntl.flock
        lease_inode = (generation / '.lease').stat().st_ino

        def nonblocking_generation_lock(descriptor, operation):
            if (operation == fcntl.LOCK_EX
                    and os.fstat(descriptor.fileno()).st_ino == lease_inode):
                operation |= fcntl.LOCK_NB
            return flock(descriptor, operation)

        with (generation / '.lease').open('r') as live:
            flock(live, fcntl.LOCK_SH)
            with mock.patch.object(dependency_cache.fcntl, 'flock',
                                   side_effect=nonblocking_generation_lock):
                with self.assertRaises(BlockingIOError):
                    self.populate()
            self.assertEqual(self.fingerprint(generation), before)
            self.assertEqual(self.selection(), selection)
            self.assertFalse((self.cache / '.garbage').exists())
        self.assertEqual(self.populate(), key)

    def test_interrupted_corruption_retirement_rebuilds_on_next_population(self):
        key = self.populate()
        generation = self.cache / key
        installed = generation / 'node_modules/package/index.js'
        info = installed.stat()
        os.utime(installed, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
        rmtree = shutil.rmtree

        def interrupt_retired(path, *args, **kwargs):
            if Path(path).parent == self.cache / '.garbage':
                self.assertFalse(generation.exists())
                self.assertFalse((self.cache / '.selection.json').exists())
                (Path(path) / '.lease').unlink()
                raise RuntimeError('interrupted corrupt generation cleanup')
            return rmtree(path, *args, **kwargs)

        with mock.patch.object(dependency_cache.shutil, 'rmtree', side_effect=interrupt_retired):
            with self.assertRaisesRegex(RuntimeError, 'interrupted corrupt'):
                self.populate()
        self.assertEqual(self.generations(), set())
        self.assertEqual(self.populate(), key)
        self.assertEqual(list((self.cache / '.garbage').iterdir()), [])
        self.assertEqual(self.selection(), {'current': key, 'previous': None})

    def test_failed_retirement_rename_restores_generation_root_mode(self):
        key = self.populate()
        generation = self.cache / key
        installed = generation / 'node_modules/package/index.js'
        info = installed.stat()
        os.utime(installed, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
        original_mode = generation.stat().st_mode
        rename = Path.rename

        def failed_rename(path, destination):
            if path == generation:
                self.assertTrue(path.stat().st_mode & 0o200)
                self.assertEqual(installed.parent.stat().st_mode & 0o222, 0)
                self.assertEqual(installed.stat().st_mode & 0o222, 0)
                raise PermissionError('injected retirement rename failure')
            return rename(path, destination)

        with mock.patch.object(Path, 'rename', autospec=True, side_effect=failed_rename):
            with self.assertRaisesRegex(PermissionError, 'injected retirement'):
                self.populate()
        self.assertEqual(generation.stat().st_mode, original_mode)
        self.assertEqual(self.populate(), key)

    def test_publication_rehashes_every_captured_file_before_rename(self):
        normalize = dependency_cache._normalize_generation_links

        def mutate_capture(source, staging):
            normalize(source, staging)
            (staging / 'node_modules/package/index.js').write_text('changed after copy\n')

        with mock.patch.object(dependency_cache, '_normalize_generation_links',
                               side_effect=mutate_capture):
            with self.assertRaisesRegex(ValueError, 'changed during publication'):
                self.populate()
        self.assertEqual(self.generations(), set())
        self.assertEqual(list(self.cache.glob('.staging-*')), [])
        self.assertFalse((self.cache / '.selection.json').exists())

    def test_old_metadata_schema_requires_fresh_cache_root(self):
        key = self.populate()
        generation = self.cache / key
        generation.chmod(0o755)
        (generation / '.files.json').unlink()
        with self.assertRaisesRegex(ValueError, 'fresh cache root'):
            self.populate()
        self.assertTrue(generation.exists())

    def test_absolute_workspace_links_are_relative_and_reused_across_clones(self):
        workspace = self.source / 'packages/pkg'
        workspace.mkdir(parents=True)
        (workspace / 'index.js').write_text('clone A workspace\n')
        (self.source / 'node_modules/pkg').symlink_to(workspace)
        key = self.populate()
        generation = self.cache / key
        self.assertEqual(os.readlink(generation / 'node_modules/pkg'), '../packages/pkg')
        self.assertFalse((generation / 'node_modules/pkg').exists())
        before = self.fingerprint(generation)

        clone = self.fixture / 'clone-b'
        shutil.copytree(self.source, clone, symlinks=True)
        (clone / 'node_modules/pkg').unlink()
        (clone / 'node_modules/pkg').symlink_to(clone / 'packages/pkg')
        (clone / 'packages/pkg/index.js').write_text('clone B workspace\n')
        self.assertEqual(dependency_cache.install_key(clone), key)
        with (self.cache / '.cache.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            dependency_cache.populate(clone, self.cache, key)
        self.assertEqual(self.fingerprint(generation), before)
        for link in generation.rglob('*'):
            if link.is_symlink():
                self.assertFalse(Path(os.readlink(link)).is_absolute())
                self.assertNotIn(str(self.source), os.readlink(link))
        # The same stored topology points into whichever clone supplies files.
        (clone / 'node_modules/pkg').unlink()
        (clone / 'node_modules/pkg').symlink_to(os.readlink(generation / 'node_modules/pkg'))
        self.assertEqual((clone / 'node_modules/pkg/index.js').read_text(),
                         'clone B workspace\n')

    def test_normalization_preserves_logical_workspace_paths(self):
        outside = self.fixture / 'mounted-workspace'
        (outside / 'pkg').mkdir(parents=True)
        (self.source / 'packages').symlink_to(outside, target_is_directory=True)
        (self.source / 'node_modules/pkg').symlink_to(self.source / 'packages/pkg')
        key = self.populate()
        self.assertEqual(os.readlink(self.cache / key / 'node_modules/pkg'), '../packages/pkg')

    def test_resolved_source_targets_map_back_to_source_topology(self):
        workspace = self.source / 'packages/pkg'
        workspace.mkdir(parents=True)
        alias = self.fixture / 'source-alias'
        alias.symlink_to(self.source, target_is_directory=True)
        (self.source / 'node_modules/pkg').symlink_to(alias / 'packages/pkg')
        key = self.populate()
        self.assertEqual(os.readlink(self.cache / key / 'node_modules/pkg'), '../packages/pkg')

    def test_internal_pnpm_links_and_dangling_workspace_links_are_valid(self):
        store = self.source / 'node_modules/.pnpm/pkg@1/node_modules/pkg'
        store.mkdir(parents=True)
        (store / 'index.js').write_text('pnpm dependency\n')
        (self.source / 'node_modules/pkg').symlink_to('.pnpm/pkg@1/node_modules/pkg')
        scope = self.source / 'apps/web/node_modules/@scope'
        scope.mkdir()
        (scope / 'pkg').symlink_to('../../../../node_modules/.pnpm/pkg@1/node_modules/pkg')
        (self.source / 'node_modules/absent').symlink_to('../packages/absent')
        key = self.populate()
        generation = self.cache / key
        self.assertEqual((generation / 'node_modules/pkg/index.js').read_text(),
                         'pnpm dependency\n')
        self.assertEqual((generation / 'apps/web/node_modules/@scope/pkg/index.js').read_text(),
                         'pnpm dependency\n')
        self.assertFalse((generation / 'node_modules/absent').exists())
        self.assertEqual(dependency_cache.verify_generation(generation, key),
                         ['node_modules', 'apps/web/node_modules'])

    def test_external_source_links_fail_atomically(self):
        key = self.populate()
        generation = self.cache / key
        before = self.fingerprint(generation)
        selection = (self.cache / '.selection.json').read_bytes()
        self.change_key('different dependency inputs')
        outside = self.fixture / 'outside'
        outside.mkdir()
        for target in (outside, Path('../../outside'), self.fixture / 'missing'):
            with self.subTest(target=target):
                link = self.source / 'node_modules/external'
                link.symlink_to(target)
                with self.assertRaisesRegex(ValueError, 'symlink escapes source'):
                    self.populate()
                self.assertEqual(self.generations(), {key})
                self.assertEqual(self.fingerprint(generation), before)
                self.assertEqual((self.cache / '.selection.json').read_bytes(), selection)
                self.assertEqual(list(self.cache.glob('.staging-*')), [])
                link.unlink()

    def test_unsafe_published_links_refuse_reuse_without_repair(self):
        key = self.populate()
        generation = self.cache / key
        selection = (self.cache / '.selection.json').read_bytes()
        link = generation / 'node_modules/workspace'
        link.parent.chmod(0o755)
        for target in (self.source / 'apps/web', generation / 'apps/web',
                       self.fixture / 'outside', Path('../../outside')):
            with self.subTest(target=target):
                link.unlink()
                link.symlink_to(target)
                before = self.fingerprint(generation)
                with self.assertRaisesRegex(ValueError, 'symlink escapes generation'):
                    dependency_cache.verify_generation(generation, key)
                with self.assertRaisesRegex(ValueError, 'symlink escapes generation'):
                    self.populate()
                self.assertEqual(self.fingerprint(generation), before)
                self.assertEqual((self.cache / '.selection.json').read_bytes(), selection)
                self.assertEqual(list(self.cache.glob('.staging-*')), [])

    def test_marker_inventory_and_metadata_rejection(self):
        key = self.populate()
        generation = self.cache / key
        generation.chmod(0o755)
        marker = generation / '.fractal-dep-key'
        marker.chmod(0o644)
        marker.write_text('f' * 64 + '\n')
        with self.assertRaisesRegex(ValueError, 'marker'):
            dependency_cache.verify_generation(generation, key)
        marker.write_text(key + '\n')
        inventory = generation / '.node-modules.json'
        inventory.chmod(0o644)
        for invalid in ({}, 'node_modules', [None], [1], ['/node_modules'],
                        ['../node_modules'], ['apps/../../node_modules'],
                        ['./node_modules'], ['node_modules/'], ['node_modules', 'node_modules'],
                        ['node_modules', 'node_modules/package/node_modules']):
            with self.subTest(inventory=invalid):
                inventory.write_text(json.dumps(invalid))
                with self.assertRaises(ValueError):
                    dependency_cache.verify_generation(generation, key)
        inventory.write_text(json.dumps(['absent/node_modules']))
        with self.assertRaises(ValueError):
            dependency_cache.verify_generation(generation, key)
        inventory.unlink()
        external = self.fixture / 'inventory.json'
        external.write_text('[]')
        inventory.symlink_to(external)
        with self.assertRaisesRegex(ValueError, 'regular file'):
            dependency_cache.verify_generation(generation, key)
        inventory.unlink()
        inventory.write_text('[]')
        marker.unlink()
        marker.symlink_to(self.source / 'pnpm-lock.yaml')
        with self.assertRaisesRegex(ValueError, 'regular file'):
            dependency_cache.verify_generation(generation, key)

    def test_inventory_rejects_symlink_roots_and_ancestors(self):
        key = self.populate()
        generation = self.cache / key
        generation.chmod(0o755)
        (generation / '.node-modules.json').chmod(0o644)
        outside = self.fixture / 'outside'
        (outside / 'node_modules').mkdir(parents=True)
        for target in (outside, generation / 'apps/web'):
            with self.subTest(target=target):
                alias = generation / 'alias'
                alias.symlink_to(target, target_is_directory=True)
                (generation / '.node-modules.json').write_text(json.dumps(['alias/node_modules']))
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    dependency_cache.verify_generation(generation, key)
                alias.unlink()
        alias = generation / 'alias'
        alias.mkdir()
        (alias / 'node_modules').symlink_to(outside / 'node_modules', target_is_directory=True)
        (generation / '.node-modules.json').write_text(json.dumps(['alias/node_modules']))
        with self.assertRaisesRegex(ValueError, 'symlink'):
            dependency_cache.verify_generation(generation, key)
        generation_alias = self.cache / ('f' * 64)
        generation_alias.symlink_to(generation, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'path'):
            dependency_cache.verify_generation(generation_alias, generation_alias.name)

    def test_source_symlink_root_failure_publishes_no_partial_generation(self):
        outside = self.fixture / 'outside'
        outside.mkdir()
        for target in (outside, self.source / 'node_modules', self.fixture / 'missing'):
            with self.subTest(target=target):
                alias = self.source / 'zzz/node_modules'
                alias.parent.mkdir(exist_ok=True)
                alias.symlink_to(target, target_is_directory=True)
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    self.populate()
                self.assertEqual(self.generations(), set())
                self.assertEqual(list(self.cache.glob('.staging-*')), [])
                self.assertFalse((self.cache / '.selection.json').exists())
                alias.unlink()

    def test_wrong_source_key_rejected_before_publication(self):
        expected = dependency_cache.install_key(self.source)
        self.change_key('different')
        with self.assertRaisesRegex(ValueError, 'source install key'):
            with (self.cache / '.cache.lock').open('a') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                dependency_cache.populate(self.source, self.cache, expected)
        self.assertEqual(self.generations(), set())

    def test_failed_population_preserves_published_generation_and_selection(self):
        key = self.populate()
        before = self.fingerprint(self.cache / key)
        selection = (self.cache / '.selection.json').read_bytes()
        self.change_key('new install inputs')
        alias = self.source / 'zzz/node_modules'
        alias.parent.mkdir()
        alias.symlink_to(self.source / 'node_modules', target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.populate()
        self.assertEqual(self.generations(), {key})
        self.assertEqual(self.fingerprint(self.cache / key), before)
        self.assertEqual((self.cache / '.selection.json').read_bytes(), selection)
        self.assertEqual(list(self.cache.glob('.staging-*')), [])

    def test_selected_lineage_a_b_a_c_and_repeated_selection(self):
        original = (self.source / 'pnpm-lock.yaml').read_text()
        a = self.populate()
        self.change_key('B')
        b = self.populate()
        self.assertEqual(self.selection(), {'current': b, 'previous': a})
        self.change_key(original)
        self.assertEqual(self.populate(), a)
        self.assertEqual(self.selection(), {'current': a, 'previous': b})
        self.assertEqual(self.populate(), a)
        self.assertEqual(self.selection(), {'current': a, 'previous': b})
        self.change_key('C')
        c = self.populate()
        self.assertEqual(self.selection(), {'current': c, 'previous': a})
        self.assertEqual(self.generations(), {a, c})

    def _assert_interrupted_prune_recovers(self, partial):
        a = self.populate()
        self.change_key('B')
        b = self.populate()
        c = self.change_key('C')
        garbage = self.cache / '.garbage'
        original_rmtree = shutil.rmtree

        def interrupted_rmtree(path, *args, **kwargs):
            path = Path(path)
            if path.parent == garbage:
                # Removal has already left the public generation namespace.
                self.assertFalse((self.cache / a).exists())
                self.assertNotIn(a, self.selection().values())
                if partial:
                    (path / '.fractal-dep-key').unlink()
                    (path / 'node_modules/package/index.js').unlink()
                raise RuntimeError('simulated interrupted garbage deletion')
            return original_rmtree(path, *args, **kwargs)

        with mock.patch.object(dependency_cache.shutil, 'rmtree', side_effect=interrupted_rmtree):
            with self.assertRaisesRegex(RuntimeError, 'simulated interrupted'):
                self.populate()
        self.assertEqual(self.generations(), {b})
        self.assertEqual(self.selection(), {'current': b, 'previous': None})
        self.assertEqual(len(list(garbage.iterdir())), 1)
        # Reuse must clean even marker-less garbage without verifying it.
        self.change_key('B')
        with mock.patch.object(dependency_cache, 'verify_generation',
                               wraps=dependency_cache.verify_generation) as verify:
            self.assertEqual(self.populate(), b)
        self.assertTrue(all(Path(call.args[0]).parent == self.cache for call in verify.call_args_list))
        self.assertEqual(list(garbage.iterdir()), [])
        self.change_key('C')
        self.assertEqual(self.populate(), c)
        self.assertEqual(self.generations(), {b, c})
        self.assertEqual(self.selection(), {'current': c, 'previous': b})

    def test_interruption_after_retirement_before_rmtree_recovers(self):
        self._assert_interrupted_prune_recovers(partial=False)

    def test_interruption_mid_rmtree_recovers(self):
        self._assert_interrupted_prune_recovers(partial=True)

    def test_missing_selection_references_are_removed(self):
        a = self.populate()
        self.change_key('B')
        b = self.populate()
        garbage = self.cache / '.garbage'
        garbage.mkdir()
        (self.cache / a).chmod(0o755)
        (self.cache / a).rename(garbage / 'previous')
        self.assertEqual(self.populate(), b)
        self.assertEqual(self.selection(), {'current': b, 'previous': None})
        (self.cache / b).chmod(0o755)
        (self.cache / b).rename(garbage / 'current')
        self.assertEqual(self.populate(), b)
        self.assertEqual(self.selection(), {'current': b, 'previous': None})
        self.assertEqual(list(garbage.iterdir()), [])

    def test_invalid_selection_identities_fail_without_publication(self):
        key = dependency_cache.install_key(self.source)
        for invalid in ([], {}, {'current': '../escape', 'previous': None},
                        {'current': key, 'previous': '/escape'},
                        {'current': key, 'previous': []},
                        {'current': None, 'previous': None},
                        {'current': key, 'previous': key},
                        {'current': key, 'previous': None, 'extra': True}):
            with self.subTest(selection=invalid):
                (self.cache / '.selection.json').write_text(json.dumps(invalid))
                with self.assertRaisesRegex(ValueError, 'selection'):
                    self.populate()
                self.assertEqual(self.generations(), set())
                self.assertEqual(list(self.cache.glob('.staging-*')), [])

    def test_old_shared_lease_blocks_pruning_while_root_lock_is_held(self):
        a = self.populate()
        self.change_key('B')
        b = self.populate()
        c = self.change_key('C')
        # This descriptor models a live build that already dropped the root lock.
        with (self.cache / a / '.lease').open('r') as live:
            fcntl.flock(live, fcntl.LOCK_SH)
            script = '''
import fcntl, os, pathlib, sys
sys.path.insert(0, sys.argv[1])
import dependency_cache
source, cache, key = pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]), sys.argv[4]
old_inode = (cache / sys.argv[5] / '.lease').stat().st_ino
flock = fcntl.flock
def observed_flock(descriptor, operation):
    if operation == fcntl.LOCK_EX and os.fstat(descriptor.fileno()).st_ino == old_inode:
        try:
            flock(descriptor, operation | fcntl.LOCK_NB)
        except BlockingIOError:
            print('BLOCKED_ON_LIVE_LEASE', flush=True)
        else:
            raise AssertionError('old shared lease did not prevent exclusive pruning')
    return flock(descriptor, operation)
fcntl.flock = observed_flock
with (cache / '.cache.lock').open('a') as root:
    flock(root, fcntl.LOCK_EX)
    dependency_cache.populate(source, cache, key)
print('PUBLISHED', flush=True)
'''
            child = subprocess.Popen(
                [sys.executable, '-c', script, str(HELPER.parent), str(self.source),
                 str(self.cache), c, a], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True)
            try:
                self.assertTrue(select.select([child.stdout], [], [], 10)[0],
                                'pruner did not reach the old generation lease')
                self.assertEqual(child.stdout.readline().strip(), 'BLOCKED_ON_LIVE_LEASE')
                self.assertIsNone(child.poll())
                self.assertEqual(self.generations(), {a, b})
                self.assertFalse((self.cache / c).exists())
                with (self.cache / '.cache.lock').open('r') as root:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(root, fcntl.LOCK_EX | fcntl.LOCK_NB)
                # No root lock is needed to let the old build finish.
                fcntl.flock(live, fcntl.LOCK_UN)
                output, errors = child.communicate(timeout=10)
                self.assertEqual(child.returncode, 0, errors)
                self.assertEqual(output.strip(), 'PUBLISHED')
                self.assertEqual(self.generations(), {b, c})
                self.assertEqual(self.selection(), {'current': c, 'previous': b})
                self.assertEqual(list(self.cache.glob('.staging-*')), [])
            finally:
                if child.poll() is None:
                    child.kill()
                    child.communicate()


if __name__ == '__main__':
    unittest.main(verbosity=2)
