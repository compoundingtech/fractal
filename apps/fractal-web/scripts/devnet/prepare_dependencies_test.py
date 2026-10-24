#!/usr/bin/env python3
"""Filesystem coverage for preparing snapshots from immutable dependency generations."""
import contextlib
import errno
import fcntl
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from dependency_cache import install_key, populate


HELPER = Path(__file__).with_name('prepare-dependencies.py').resolve()
SPEC = importlib.util.spec_from_file_location('prepare_dependencies', HELPER)
PREPARATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARATION)


class PrepareDependenciesTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='prepare-dependencies-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / 'source'
        self.snapshot = self.root / 'snapshot'
        self.cache = self.root / 'cache'
        self.manifest = self.root / 'manifest.json'
        self.source.mkdir()
        self.snapshot.mkdir()
        self.cache.mkdir()
        for name in ('pnpm-lock.yaml', 'pnpm-workspace.yaml', 'megarepo.kdl',
                     'megarepo.lock', 'pnpm-source-inputs.json'):
            (self.source / name).write_text(name + '\n')
        for root in (self.source, self.snapshot):
            workspace = root / 'packages/workspace'
            workspace.mkdir(parents=True)
            (workspace / 'index.js').write_text('export const workspace = true;\n')
        self.write_source('node_modules/fixture/index.js', 'immutable root dependency\n')
        self.write_source('apps/fractal-web/node_modules/nested/index.js', 'immutable nested dependency\n')
        self.write_source('apps/fractal-web/private/build-live.mjs', '// ignored overlay\n')
        self.write_source('apps/fractal-web/private/node_modules/tool/index.js', '// cached overlay dependency\n')
        self.write_source('apps/fractal-web/vendor/utils/index.js', '// vendor overlay\n')
        (self.source / 'apps/fractal-web/private/build-link.mjs').symlink_to('build-live.mjs')
        (self.source / 'node_modules/workspace').symlink_to('../packages/workspace')
        (self.source / 'apps/fractal-web/node_modules/workspace').symlink_to('../../../packages/workspace')
        (self.source / 'node_modules/absolute-workspace').symlink_to(self.source / 'packages/workspace')
        self.key = install_key(self.source)
        with (self.cache / '.cache.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            populate(self.source, self.cache, self.key)
        self.generation = self.cache / self.key

    def write_source(self, relative, content):
        path = self.source / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def prepare(self):
        PREPARATION.prepare(self.source, self.snapshot, self.manifest, self.generation, self.key)

    def test_cli_links_cache_files_and_preserves_overlay_manifest(self):
        source_file = self.source / 'node_modules/fixture/index.js'
        source_file.write_text('mutable source changed after cache population\n')
        result = subprocess.run([sys.executable, str(HELPER), str(self.source), str(self.snapshot),
                                 str(self.manifest), str(self.generation), self.key],
                                text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '')
        snapshot_file = self.snapshot / 'node_modules/fixture/index.js'
        cache_file = self.generation / 'node_modules/fixture/index.js'
        self.assertEqual(snapshot_file.read_text(), 'immutable root dependency\n')
        self.assertTrue(os.path.samefile(snapshot_file, cache_file))
        self.assertFalse(os.path.samefile(snapshot_file, source_file))
        for relative in ('node_modules/workspace', 'node_modules/absolute-workspace',
                         'apps/fractal-web/node_modules/workspace'):
            link = self.snapshot / relative
            self.assertTrue(link.is_symlink())
            self.assertEqual(link.resolve(), self.snapshot / 'packages/workspace')
        manifest = json.loads(self.manifest.read_text())
        self.assertEqual(manifest['dependencyKey'], self.key)
        self.assertEqual(manifest['dependencyFiles'], {'linked': 3, 'copied': 0})
        overlay = manifest['copiedIgnoredOverlayFiles']
        self.assertEqual(overlay['apps/fractal-web/private/build-live.mjs'], {
            'sha256': hashlib.sha256(b'// ignored overlay\n').hexdigest()})
        self.assertEqual(overlay['apps/fractal-web/private/build-link.mjs'], {'symlink': 'build-live.mjs'})
        self.assertIn('apps/fractal-web/vendor/utils/index.js', overlay)
        self.assertFalse(any('node_modules' in name for name in overlay))
        self.assertIn(f'key={self.key} linked=3 copied=0', result.stderr)

    def test_rejects_marker_mismatch_before_copying(self):
        (self.generation / '.fractal-dep-key').write_text('0' * 64 + '\n')
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(list(self.snapshot.iterdir()), [self.snapshot / 'packages'])
        self.assertFalse(self.manifest.exists())

    def test_rejects_changed_source_key_before_copying(self):
        (self.source / 'pnpm-lock.yaml').write_text('changed\n')
        with self.assertRaisesRegex(ValueError, 'source install key'):
            self.prepare()
        self.assertFalse((self.snapshot / 'node_modules').exists())
        self.assertFalse((self.snapshot / 'apps').exists())

    def test_rejects_unsafe_inventory(self):
        for inventory in (['../outside/node_modules'], ['/outside/node_modules'],
                          ['packages/../../outside/node_modules'], ['packages/workspace'],
                          {'node_modules': True}, [7]):
            with self.subTest(inventory=inventory):
                (self.generation / '.node-modules.json').write_text(json.dumps(inventory))
                with self.assertRaises(ValueError):
                    self.prepare()
                self.assertFalse((self.snapshot / 'node_modules').exists())

    def test_rejects_escaping_inventory_ancestor(self):
        outside = self.root / 'outside'
        (outside / 'node_modules').mkdir(parents=True)
        (self.generation / 'escape').symlink_to(outside, target_is_directory=True)
        (self.generation / '.node-modules.json').write_text(json.dumps(['escape/node_modules']))
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse((self.snapshot / 'escape').exists())

    def test_rejects_outer_overlay_symlink(self):
        private = self.source / 'apps/fractal-web/private'
        shutil.rmtree(private)
        outside = self.root / 'outside'
        outside.mkdir()
        (outside / 'secret').write_text('not an overlay\n')
        private.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'source overlay'):
            self.prepare()
        self.assertFalse((self.snapshot / 'apps').exists())

    def test_rejects_escaping_overlay_ancestor(self):
        apps = self.source / 'apps'
        apps.rename(self.root / 'outside-apps')
        apps.symlink_to(self.root / 'outside-apps', target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'source overlay'):
            self.prepare()
        self.assertFalse((self.snapshot / 'apps').exists())

    def test_rejects_destination_symlink_ancestor(self):
        outside = self.root / 'outside'
        outside.mkdir()
        (self.snapshot / 'apps').symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'destination'):
            self.prepare()
        self.assertEqual(list(outside.iterdir()), [])
        self.assertFalse((self.snapshot / 'node_modules').exists())

    def test_rejects_destination_root_symlink(self):
        (self.snapshot / 'node_modules').symlink_to(self.root / 'outside', target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'destination'):
            self.prepare()
        self.assertFalse((self.snapshot / 'apps').exists())

    def test_exdev_warns_before_copying_and_counts_fallback(self):
        errors = io.StringIO()
        copy2 = shutil.copy2
        copied = []

        def observed_copy(src, dst, *args, **kwargs):
            if Path(src).is_relative_to(self.generation):
                self.assertIn('WARNING: EXDEV dependency copy fallback', errors.getvalue())
                copied.append(Path(dst))
            return copy2(src, dst, *args, **kwargs)

        with contextlib.redirect_stderr(errors), \
                patch.object(PREPARATION.os, 'link', side_effect=OSError(errno.EXDEV, 'cross-device')), \
                patch.object(PREPARATION.shutil, 'copy2', side_effect=observed_copy):
            self.prepare()
        self.assertEqual(len(copied), 3)
        manifest = json.loads(self.manifest.read_text())
        self.assertEqual(manifest['dependencyFiles'], {'linked': 0, 'copied': 3})
        self.assertIn('linked=0 copied=3 WARNING: EXDEV copy fallback', errors.getvalue())
        self.assertEqual(errors.getvalue().count('WARNING: EXDEV dependency copy fallback'), 1)
        self.assertFalse(os.path.samefile(self.snapshot / 'node_modules/fixture/index.js',
                                         self.generation / 'node_modules/fixture/index.js'))

    def test_other_link_errors_propagate_without_fallback(self):
        errors = io.StringIO()
        copy2 = shutil.copy2
        dependency_copies = []

        def observed_copy(src, dst, *args, **kwargs):
            if Path(src).is_relative_to(self.generation):
                dependency_copies.append(Path(dst))
            return copy2(src, dst, *args, **kwargs)

        with contextlib.redirect_stderr(errors), \
                patch.object(PREPARATION.os, 'link', side_effect=OSError(errno.EACCES, 'denied')), \
                patch.object(PREPARATION.shutil, 'copy2', side_effect=observed_copy):
            with self.assertRaises(shutil.Error) as caught:
                self.prepare()
        self.assertIn('denied', str(caught.exception))
        self.assertEqual(dependency_copies, [])
        self.assertNotIn('EXDEV', errors.getvalue())
        self.assertFalse(self.manifest.exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
