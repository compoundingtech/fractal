#!/usr/bin/env python3
"""Networkless filesystem and lease coverage for immutable dependency caching."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import unittest

import dependency_cache


HELPER = Path(__file__).with_name('dependency_cache.py').resolve()


class DependencyCacheTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='dependency-cache-')
        self.addCleanup(temporary.cleanup)
        self.fixture = Path(temporary.name)
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
        self.assertEqual(dependency_cache.install_key(self.source), expected.hexdigest())
        result = subprocess.run([sys.executable, str(HELPER), 'key', str(self.source)],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, expected.hexdigest() + '\n')
        self.assertEqual(result.stderr, '')
        for name in names:
            original = (self.source / name).read_bytes()
            (self.source / name).write_bytes(original + b'changed')
            self.assertNotEqual(dependency_cache.install_key(self.source), expected.hexdigest())
            (self.source / name).write_bytes(original)

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

    def test_marker_inventory_and_metadata_rejection(self):
        key = self.populate()
        generation = self.cache / key
        marker = generation / '.fractal-dep-key'
        marker.write_text('f' * 64 + '\n')
        with self.assertRaisesRegex(ValueError, 'marker'):
            dependency_cache.verify_generation(generation, key)
        marker.write_text(key + '\n')
        inventory = generation / '.node-modules.json'
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
