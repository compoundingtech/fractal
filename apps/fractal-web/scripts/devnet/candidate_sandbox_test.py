#!/usr/bin/env python3
"""Runner unit tests; add --real to exercise the pinned Linux sandbox too."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import candidate_sandbox as sandbox

REAL = '--real' in sys.argv
if REAL:
    sys.argv.remove('--real')


class RunnerTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.worktree = self.root / 'candidate'
        self.worktree.mkdir()
        self.state = self.root / 'state'
        self.state.mkdir()
        self.store = self.root / 'pnpm'
        self.store.mkdir()
        self.binaries = {name: '/nix/store/test-tools/bin/' + name for name in ('bash', 'bwrap', 'env', 'git')}
        self.tools = patch.object(sandbox, 'tools', return_value=(self.binaries, self.state, self.store))
        self.tools.start()
        self.addCleanup(self.tools.stop)

    def test_default_networkless_and_environment_allowlist(self):
        with patch.dict(os.environ, {'GH_CONFIG_DIR': '/secret', 'ST_SECRET': 'bad', 'SCG_SECRET': 'bad'}), \
                patch.object(sandbox.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as execute:
            sandbox.run(['genie'], worktree=self.worktree)
        argv = execute.call_args.args[0]
        self.assertIn('--unshare-all', argv)
        self.assertNotIn('--share-net', argv)
        self.assertIn('--clearenv', argv)
        self.assertNotIn('env', execute.call_args.kwargs)
        self.assertNotIn('/secret', argv)
        self.assertNotIn('/run/user', argv)
        self.assertNotIn('/nix/var/nix/daemon-socket', argv)
        self.assertIn(str(self.worktree / '.devenv/candidate-members'), argv)

    def test_shell_definition_is_not_executed_on_host(self):
        responses = [subprocess.CompletedProcess([], 0, 'export SHELL_HOOK_MARKER=1\n'),
                     subprocess.CompletedProcess([], 0)]
        with patch.object(sandbox.subprocess, 'run', side_effect=responses) as execute:
            sandbox.run(['pnpm', 'install'], worktree=self.worktree, shell='web', network=True)
        bootstrap, confined = execute.call_args_list
        self.assertIn('print-dev-env', bootstrap.args[0])
        self.assertNotIn('GH_CONFIG_DIR', bootstrap.kwargs['env'])
        self.assertIn('--share-net', confined.args[0])
        self.assertIn('source /sandbox/environment.sh', ' '.join(confined.args[0]))

    def test_output_is_promoted_only_after_success(self):
        output = self.root / 'production/builds/revision'
        def complete(argv, **kwargs):
            stage = Path(argv[argv.index(str(output.parent)) - 1])
            (stage / output.name).mkdir()
            (stage / output.name / 'receipt').write_text('ok')
            return subprocess.CompletedProcess(argv, 0)
        with patch.object(sandbox.subprocess, 'run', side_effect=complete):
            sandbox.run(['node'], worktree=self.worktree, output=output)
        self.assertEqual((output / 'receipt').read_text(), 'ok')

    def test_output_symlink_is_rejected(self):
        output = self.root / 'production/builds/revision'
        def complete(argv, **kwargs):
            stage = Path(argv[argv.index(str(output.parent)) - 1])
            (stage / output.name).mkdir()
            (stage / output.name / 'escape').symlink_to('/etc/passwd')
            return subprocess.CompletedProcess(argv, 0)
        with patch.object(sandbox.subprocess, 'run', side_effect=complete), self.assertRaisesRegex(RuntimeError, 'without symlinks'):
            sandbox.run(['node'], worktree=self.worktree, output=output)
        self.assertFalse(output.exists())

    def test_nested_dependencies_are_readonly(self):
        modules = self.worktree / 'apps/web/node_modules'
        modules.mkdir(parents=True)
        with patch.object(sandbox.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as execute:
            sandbox.run(['node'], worktree=self.worktree, readonly_dependencies=True)
        argv = execute.call_args.args[0]
        position = argv.index(str(modules))
        self.assertEqual(argv[position - 1:position + 2], ['--ro-bind', str(modules), str(modules)])

    def test_working_directory_cannot_escape(self):
        with self.assertRaisesRegex(RuntimeError, 'working directory escapes'):
            sandbox.run(['node'], worktree=self.worktree, working_directory=self.root)


class ConfigurationTest(unittest.TestCase):
    def test_policy_cli_needs_no_configuration_or_nix(self):
        result = subprocess.run([sys.executable, str(Path(sandbox.__file__).resolve()), '--print-policy'],
                                env={'PATH': ''}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sandbox.DEPENDENCY_POLICY, 'bwrap-v1')
        self.assertEqual(result.stdout, sandbox.DEPENDENCY_POLICY + '\n')
        self.assertEqual(result.stderr, '')

    def test_normal_cli_still_requires_configuration(self):
        result = subprocess.run([sys.executable, str(Path(sandbox.__file__).resolve()), '--check-config'],
                                env={'PATH': ''}, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')
        self.assertIn('missing required input: FRACTAL_INTEGRATOR_SANDBOX', result.stderr)

    def test_mode_is_required_and_has_no_fallback(self):
        sandbox.tools.cache_clear()
        with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(RuntimeError, 'FRACTAL_INTEGRATOR_SANDBOX'):
            sandbox.validate_configuration()
        with patch.dict(os.environ, {'FRACTAL_INTEGRATOR_SANDBOX': 'none'}, clear=True), self.assertRaisesRegex(RuntimeError, 'no unsandboxed fallback'):
            sandbox.validate_configuration()


@unittest.skipUnless(REAL, 'use --real for pinned bubblewrap isolation smoke')
class IsolationTest(unittest.TestCase):
    def test_real_filesystem_environment_and_readonly_dependencies(self):
        # Sentinels deliberately live under the operator HOME, outside the clone.
        with tempfile.TemporaryDirectory(prefix='.fractal-sandbox-secret-', dir=Path.home()) as private, \
                tempfile.TemporaryDirectory(prefix='fractal-sandbox-fixture-') as fixture:
            private = Path(private)
            (private / 'ssh').write_text('dummy credential')
            (private / 'st3.sock').write_text('dummy socket sentinel')
            root = Path(fixture)
            worktree = root / 'candidate'
            worktree.mkdir()
            modules = worktree / 'apps/web/node_modules'
            modules.mkdir(parents=True)
            store = root / 'pnpm'
            store.mkdir()
            cached = store / 'dependency'
            cached.write_text('immutable')
            dependency = modules / 'dependency'
            os.link(cached, dependency)
            self.assertTrue(cached.samefile(dependency))
            lifecycle = worktree / 'lifecycle'
            lifecycle.write_text('#!/usr/bin/env bash\n/bin/sh -c \'printf lifecycle-ok\'\n')
            lifecycle.chmod(0o755)
            env = {'FRACTAL_INTEGRATOR_SANDBOX': 'bwrap',
                   'FRACTAL_INTEGRATOR_SANDBOX_STATE_ROOT': str(root / 'state'),
                   'FRACTAL_INTEGRATOR_PNPM_STORE': str(store),
                   'GH_CONFIG_DIR': str(private), 'ST_SECRET': 'dummy',
                   'FRACTAL_SECRET': 'dummy', 'AGENT_POLICY_SECRET': 'dummy',
                   'OP_PROXY_SECRET': 'dummy', 'SCG_SECRET': 'dummy'}
            sandbox.tools.cache_clear()
            with patch.dict(os.environ, env):
                binaries, _, _ = sandbox.tools()
                probe = ('[[ $HOME == /tmp/home && -d /nix/store && -d "$1" ]]\n'
                         '[[ ! -e "$2/ssh" && ! -e "$2/st3.sock" && ! -e /run/user ]]\n'
                         '[[ ! -e /nix/var/nix/daemon-socket/socket ]]\n'
                         'if printf changed > "$3/dependency" 2>/dev/null; then exit 9; fi\n'
                         'if printf changed > "$4/dependency" 2>/dev/null; then exit 10; fi\n'
                         'if printf new > "$4/new-entry" 2>/dev/null; then exit 11; fi\n'
                         '[[ $("$1/lifecycle") == lifecycle-ok ]]\n'
                         'printf allowed > "$1/sandbox-write"\n'
                         'env\n')
                result = sandbox.run([binaries['bash'], '-euc', probe, 'probe', str(worktree), str(private), str(modules), str(store)],
                                     worktree=worktree, readonly_dependencies=True, capture_output=True, text=True, check=True)
            names = {line.partition('=')[0] for line in result.stdout.splitlines()}
            forbidden = ('GH_', 'ST_', 'FRACTAL_', 'AGENT_POLICY_', 'OP_PROXY', 'SCG_', 'SSH_')
            self.assertFalse([name for name in names if name.startswith(forbidden)])
            self.assertEqual((modules / 'dependency').read_text(), 'immutable')
            self.assertEqual(cached.read_text(), 'immutable')
            self.assertTrue(cached.samefile(dependency))
            self.assertFalse((store / 'new-entry').exists())
            self.assertEqual((worktree / 'sandbox-write').read_text(), 'allowed')
            print('real sandbox: HOME/SSH/st3/daemon hidden; agent prefixes absent; nested dependencies and pnpm hardlink aliases readonly')
            sandbox.tools.cache_clear()


if __name__ == '__main__':
    unittest.main(verbosity=2)
