#!/usr/bin/env python3
"""Host publication must consume candidate data without candidate code discovery."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class HostBoundaryTest(unittest.TestCase):
    def test_isolated_validator_ignores_candidate_modules_and_python_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / 'snapshot'
            cwd = snapshot / 'apps/fractal-web'
            cwd.mkdir(parents=True)
            marker = root / 'host-imported-candidate'
            for name in ('json.py', 're.py', 'sitecustomize.py', 'usercustomize.py'):
                (cwd / name).write_text(f'open({str(marker)!r}, "w").write("executed")\nraise RuntimeError("candidate import")\n')
            build = root / 'build'
            (build / 'web').mkdir(parents=True)
            sha = 'a' * 40
            (build / 'build-receipt.json').write_text(json.dumps({
                'buildIdentity': {'rev': sha, 'dirty': False}, 'stackHead': sha}))
            (build / 'web/index.html').write_text('<script src="/assets/fixture.js"></script>')
            (build / 'launch.sh').write_text('#!/bin/sh\n')
            manifest = root / 'manifest.json'
            manifest.write_text('{"dependencyKey":"fixture","dependencyFiles":{"linked":1,"copied":0}}')
            result = subprocess.run([sys.executable, '-I', str(SCRIPTS / 'validate-candidate-output.py'),
                                     str(build), sha, str(manifest), str(snapshot)], cwd=cwd,
                                    env={**os.environ, 'PYTHONPATH': str(cwd), 'PYTHONHOME': str(cwd)},
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(marker.exists())
            self.assertEqual(json.loads((build / 'build-receipt.json').read_text())['dependencyKey'], 'fixture')

    def test_host_git_ignores_ambient_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / 'repo'
            subprocess.run(['git', 'init', '-q', str(repo)], check=True)
            marker = root / 'executed'
            attack = root / 'attack'
            attack.write_text(f'#!/bin/sh\ntouch "{marker}"\n')
            attack.chmod(0o755)
            (root / 'pre-commit').symlink_to(attack)
            (repo / '.gitattributes').write_text('*.dat filter=fixture\n')
            (repo / 'input.dat').write_text('candidate bytes')
            env = {**os.environ, 'GIT_CONFIG_COUNT': '1', 'GIT_CONFIG_KEY_0': 'filter.fixture.clean',
                   'GIT_CONFIG_VALUE_0': str(attack)}
            for args in (['add', '.'], ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                                      'commit', '-qm', 'fixture']):
                result = subprocess.run([sys.executable, '-I', str(SCRIPTS / 'host-git.py'),
                                         '-C', str(repo), *args], cwd=repo, env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(marker.exists())

    def test_host_git_refuses_every_non_allowlisted_local_key(self):
        keys = (
            'remote.origin.uploadpack', 'remote.origin.receivepack', 'uploadpack.packObjectsHook',
            'uploadpack.allowFilter', 'receive.advertisePushOptions',
            'core.sshCommand', 'core.gitProxy', 'core.alternateRefsCommand', 'core.askPass',
            'credential.helper', 'gpg.program', 'gpg.ssh.program', 'gpg.openpgp.program',
            'core.pager', 'pager.log', 'core.editor', 'sequence.editor',
            'protocol.ext.allow', 'url.fixture.insteadOf', 'http.proxy', 'http.proxyAuthMethod',
            'include.path', 'includeIf.gitdir:fixture.path', 'alias.fixture',
            'filter.fixture.clean', 'merge.fixture.driver', 'diff.fixture.command', 'diff.fixture.textconv',
            'core.hooksPath', 'core.fsmonitor', 'future.unknown', 'gc.rerereunresolved',
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / 'repo'
            subprocess.run(['git', 'init', '-q', str(repo)], check=True)
            config = repo / '.git/config'
            original = config.read_bytes()
            marker = root / 'executed'
            attack = root / 'attack'
            attack.write_text(f'#!/bin/sh\ntouch "{marker}"\n')
            attack.chmod(0o755)
            for key in keys:
                with self.subTest(key=key):
                    config.write_bytes(original)
                    subprocess.run(['git', 'config', '--file', str(config), key, str(attack)], check=True)
                    result = subprocess.run([sys.executable, '-I', str(SCRIPTS / 'host-git.py'),
                                             '-C', str(repo), 'status', '--porcelain'],
                                            capture_output=True, text=True, timeout=10)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn('non-allowlisted', result.stderr)
                    expected = ('includeif.<redacted-subsection>.path'
                                if key.startswith('includeIf.') else key.lower())
                    self.assertIn(expected, result.stderr)
                    self.assertFalse(marker.exists())
            # Per-worktree config is validated before Git can activate it too.
            config.write_bytes(original)
            (repo / '.git/config.worktree').write_text('[credential]\n helper = forbidden\n')
            result = subprocess.run([sys.executable, '-I', str(SCRIPTS / 'host-git.py'),
                                     '-C', str(repo), 'status', '--porcelain'], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('non-allowlisted', result.stderr)
            self.assertIn('credential.helper', result.stderr)

    def test_host_git_names_all_rejected_keys_across_both_configs(self):
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary) / 'repo'
            subprocess.run(['git', 'init', '-q', str(repo)], check=True)
            config = repo / '.git/config'
            with config.open('a') as stream:
                stream.write('[gc]\n rerereunresolved = 15\n[core]\n pager = forbidden\n')
            (repo / '.git/config.worktree').write_text('[filter "x"]\n clean = forbidden\n')
            result = subprocess.run([sys.executable, '-I', str(SCRIPTS / 'host-git.py'),
                                     '-C', str(repo), 'status', '--porcelain'],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            for key in ('gc.rerereunresolved', 'core.pager', 'filter.x.clean'):
                self.assertIn(key, result.stderr)


if __name__ == '__main__':
    unittest.main()
