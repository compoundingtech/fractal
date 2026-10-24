#!/usr/bin/env python3
"""Networkless concurrency smoke for immutable build snapshot leases."""
import fcntl
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import unittest


HELPER = Path(__file__).with_name('build-and-swap.sh').resolve()
ROOT = Path(__file__).resolve().parents[4]


class SnapshotLeaseTest(unittest.TestCase):
    def test_shared_snapshot_root_keeps_live_run(self):
        scratch = ROOT / 'tmp'
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='snapshot-lease-', dir=scratch) as temporary:
            fixture = Path(temporary)
            source = fixture / 'source'
            snapshot_root = fixture / 'snapshots'
            binary = fixture / 'bin'
            binary.mkdir()
            for relative in ('apps/fractal-web/private/build-live.mjs',
                             'apps/fractal-web/vendor/utils/src/node/cli-build-identity.js',
                             'node_modules/fixture/index.js'):
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('// synthetic fixture\n')
            git = binary / 'git'
            git.write_text('''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
root = pathlib.Path(os.environ['SNAPSHOT_LEASE_FIXTURE'])
args = sys.argv[3:]
with (root / 'git-calls.jsonl').open('a') as stream:
    stream.write(json.dumps(args) + '\\n')
if args[0] == 'rev-parse':
    print(os.environ['SNAPSHOT_LEASE_SHA'])
elif args[:2] == ['worktree', 'add']:
    (pathlib.Path(args[3]) / 'apps/fractal-web').mkdir(parents=True)
elif args[:2] == ['worktree', 'remove']:
    target = pathlib.Path(args[3])
    assert any(target.is_relative_to(root / name) for name in ('snapshots', 'relative-snapshot-root'))
    shutil.rmtree(target)
else:
    raise SystemExit(f'unexpected git arguments: {args}')
''')
            node = binary / 'node'
            node.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
if os.environ.get('SNAPSHOT_LEASE_HOLD') == '1':
    print('READY:' + os.getcwd(), flush=True)
    with pathlib.Path(os.environ['SNAPSHOT_LEASE_RELEASE']).open() as stream:
        stream.read(1)
build, sha = pathlib.Path(sys.argv[2]), sys.argv[4]
(build / 'web').mkdir(parents=True)
(build / 'web/index.html').write_text('<script src="/assets/fixture.js"></script>')
(build / 'launch.sh').write_text('#!/bin/sh\\n')
(build / 'build-receipt.json').write_text(json.dumps({
    'buildIdentity': {'rev': sha, 'dirty': False}, 'stackHead': sha}))
''')
            git.chmod(0o755)
            node.chmod(0o755)
            release = fixture / 'release'
            os.mkfifo(release)
            base_env = {
                **os.environ,
                'PATH': str(binary) + ':' + os.environ['PATH'],
                'SNAPSHOT_LEASE_FIXTURE': str(fixture),
                'SNAPSHOT_LEASE_RELEASE': str(release),
                'FRACTAL_BUILD_SNAPSHOT_ROOT': str(snapshot_root),
                'FRACTAL_BUILD_ADMISSION': 'fallback',
            }
            first_sha, second_sha = 'a' * 40, 'b' * 40
            first_env = {**base_env, 'FRACTAL_BUILD_ROOT': str(fixture / 'build-root-a'),
                         'SNAPSHOT_LEASE_SHA': first_sha, 'SNAPSHOT_LEASE_HOLD': '1'}
            first = subprocess.Popen(['bash', str(HELPER), str(source), first_sha, 'shared'],
                                     env=first_env, text=True, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, start_new_session=True)
            try:
                self.assertTrue(select.select([first.stdout], [], [], 10)[0], 'first build never became ready')
                ready = first.stdout.readline().strip()
                self.assertTrue(ready.startswith('READY:'), ready)
                live_snapshot = Path(ready.removeprefix('READY:')).parents[1]
                self.assertEqual(live_snapshot.parent, snapshot_root / 'shared')
                self.assertTrue(live_snapshot.is_dir())
                with Path(str(live_snapshot) + '.lock').open() as lease:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
                second_env = {**base_env, 'FRACTAL_BUILD_ROOT': str(fixture / 'build-root-b'),
                              'SNAPSHOT_LEASE_SHA': second_sha, 'SNAPSHOT_LEASE_HOLD': '0'}
                second = subprocess.run(['bash', str(HELPER), str(source), second_sha, 'shared'],
                                        env=second_env, capture_output=True, text=True, timeout=15)
                self.assertEqual(second.returncode, 0, second.stderr)
                self.assertTrue(live_snapshot.is_dir(), 'second build reclaimed the live first snapshot')
                calls = [json.loads(line) for line in (fixture / 'git-calls.jsonl').read_text().splitlines()]
                self.assertFalse(any(args[:2] == ['worktree', 'remove'] and args[3] == str(live_snapshot)
                                     for args in calls))
                with release.open('w') as stream:
                    stream.write('x')
                _, errors = first.communicate(timeout=15)
                self.assertEqual(first.returncode, 0, errors)
                self.assertFalse(live_snapshot.exists())
                self.assertTrue((fixture / 'build-root-a/shared/builds' / first_sha).is_dir())
                self.assertTrue((fixture / 'build-root-b/shared/builds' / second_sha).is_dir())
                relative_sha = 'c' * 40
                relative_env = {**base_env, 'FRACTAL_BUILD_ROOT': 'relative-build-root',
                                'FRACTAL_BUILD_SNAPSHOT_ROOT': 'relative-snapshot-root',
                                'SNAPSHOT_LEASE_SHA': relative_sha, 'SNAPSHOT_LEASE_HOLD': '0'}
                relative = subprocess.run(['bash', str(HELPER), str(source), relative_sha, 'shared'],
                                          cwd=fixture, env=relative_env, capture_output=True,
                                          text=True, timeout=15)
                self.assertEqual(relative.returncode, 0, relative.stderr)
                relative_build = fixture / 'relative-build-root/shared/builds' / relative_sha
                self.assertEqual((fixture / 'relative-build-root/shared/current').resolve(), relative_build)
                receipt = json.loads((relative_build / 'build-receipt.json').read_text())
                self.assertEqual(receipt['buildIdentity']['rev'], relative_sha)
                self.assertTrue((fixture / 'relative-build-root/shared/builds.jsonl').is_file())
                self.assertEqual(list((fixture / 'relative-snapshot-root/shared').iterdir()), [])
            finally:
                if first.poll() is None:
                    os.killpg(first.pid, signal.SIGTERM)
                    try:
                        first.communicate(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(first.pid, signal.SIGKILL)
                        first.communicate()


if __name__ == '__main__':
    unittest.main(verbosity=2)
