#!/usr/bin/env python3
"""Filesystem/API-boundary coverage for bounded, cached PR selection."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

HELPER = Path(__file__).with_name('integrator.py')
SCRATCH = HELPER.resolve().parents[4] / 'tmp' / 'integrator-selection-tests'


class SelectionTest(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=SCRATCH)
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        values = {f'FRACTAL_INTEGRATOR_{key}': str(self.root / key.lower()) for key in (
            'STATE_ROOT', 'CLONE', 'PROD_ROOT', 'SOURCE_CLONE', 'PRIVATE_OVERLAY', 'VENDOR', 'BUILD_SCRIPT')}
        values.update(FRACTAL_INTEGRATOR_PURPOSE='test', FRACTAL_INTEGRATOR_PRIORITY_JSON='[]',
                      FRACTAL_INTEGRATOR_TRUST_MODE='authors', FRACTAL_INTEGRATOR_TRUSTED_AUTHORS='[]',
                      FRACTAL_INTEGRATOR_SELECTION_BUDGET_SECONDS='60')
        with patch.dict(os.environ, values):
            spec = importlib.util.spec_from_file_location('selection_integrator', HELPER)
            self.module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(self.module)
        self.module.L.mkdir(parents=True)
        self.pr = {'number': 1, 'headRefOid': 'a' * 40, 'baseRefOid': 'b' * 40,
                   'changedFiles': 2, 'isDraft': False, 'isCrossRepository': False,
                   'headRepositoryOwner': {'login': 'compoundingtech'}, 'author': {'login': 'writer'}}
        self.files = ['apps/fractal-web/index.ts', 'README.md']

    def response(self, value):
        return subprocess.CompletedProcess([], 0, json.dumps(value), '')

    def responses(self, files=None, fence=None):
        names = self.files if files is None else files
        responses = [self.response({'files': [{'path': name} for name in names]})]
        if len(names) != self.pr['changedFiles']:
            responses.append(self.response([[{'filename': name} for name in names]]))
        return responses + [self.response(self.pr if fence is None else fence)]

    def cache_entries(self):
        return list((self.module.L / 'metadata-cache' / 'pr-files').glob('*.json'))

    def activation_builds(self):
        for sha in ('a' * 40, 'b' * 40):
            build = self.module.P / 'builds' / sha
            build.mkdir(parents=True)
            (build / 'build-receipt.json').write_text(json.dumps({'buildIdentity': {'rev': sha}}))
        self.module.recovery_select_build('b' * 40)
        return str(self.module.P / 'builds' / ('a' * 40))

    def test_activation_replaces_serving_generation_without_down(self):
        previous = self.activation_builds()
        for rollback, observations in (
                (previous, [None, 'https://new.invalid']),
                (None, [None, 'https://new.invalid'])):
            with self.subTest(previous=rollback), \
                    patch.object(self.module, 'current_server_url', side_effect=observations), \
                    patch.object(self.module, 'registration_not_ready', return_value=False), \
                    patch.object(self.module.subprocess, 'run') as run, \
                    patch.object(self.module, 'recovery_retain_builds') as retain:
                self.assertEqual(self.module.activate(rollback), 'https://new.invalid')
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual(commands, [
                    ['devnet', 'up', '--replace', 'test', '--ready-path', '/', '--',
                     str(self.module.P / 'current/launch.sh'), 'test', str(self.module.P / 'current')],
                    ['devnet', 'url', 'test', '--remote', '--wait']])
                retain.assert_called_once_with(rollback)

    def test_activation_clears_nonserving_registration_before_replace(self):
        previous = self.activation_builds()
        with patch.object(self.module, 'current_server_url',
                          side_effect=[None, 'https://new.invalid']), \
                patch.object(self.module, 'registration_not_ready', return_value=True), \
                patch.object(self.module.subprocess, 'run') as run, \
                patch.object(self.module, 'recovery_retain_builds'):
            self.module.activate(previous)
        self.assertEqual([call.args[0][:3] for call in run.call_args_list],
                         [['devnet', 'down', 'test'], ['devnet', 'up', '--replace'],
                          ['devnet', 'url', 'test']])

    def test_failed_replace_preserves_devnet_recovery_and_restores_build_pointer(self):
        previous = self.activation_builds()
        for message in ('candidate readiness failed', 'ReplacementRecoveryRequired'):
            self.module.recovery_select_build('b' * 40)
            error = subprocess.CalledProcessError(1, ['devnet', 'up', '--replace'], stderr=message)
            with self.subTest(error=message), \
                    patch.object(self.module, 'current_server_url',
                                 side_effect=[None, None, 'https://old.invalid']), \
                    patch.object(self.module, 'registration_not_ready', return_value=False), \
                    patch.object(self.module.subprocess, 'run', side_effect=error) as run, \
                    patch.object(self.module, 'recovery_retain_builds') as retain:
                with self.assertRaises(subprocess.CalledProcessError) as raised:
                    self.module.activate(previous)
                self.assertIs(raised.exception, error)
                self.assertEqual(run.call_count, 1)
                self.assertEqual(run.call_args.args[0][:3], ['devnet', 'up', '--replace'])
                self.assertEqual((self.module.P / 'current').resolve(), Path(previous))
                retain.assert_not_called()

    def test_publication_failure_after_replace_still_relaunches_previous_build(self):
        previous = self.activation_builds()
        with patch.object(self.module, 'current_server_url',
                          side_effect=[None, None]), \
                patch.object(self.module, 'registration_not_ready', return_value=False), \
                patch.object(self.module.subprocess, 'run') as run, \
                patch.object(self.module, 'recovery_retain_builds') as retain:
            with self.assertRaisesRegex(RuntimeError, 'previous build rolled back'):
                self.module.activate(previous)
        self.assertEqual([call.args[0][:3] for call in run.call_args_list],
                         [['devnet', 'up', '--replace'], ['devnet', 'url', 'test'],
                          ['devnet', 'down', 'test'], ['devnet', 'up', 'test']])
        self.assertEqual((self.module.P / 'current').resolve(), Path(previous))
        retain.assert_not_called()

    def test_registration_readiness_requires_positive_status_evidence(self):
        cases = [
            ({'status': 'running', 'tags': {'run.lifecycle': json.dumps({'_tag': 'ready'})}}, False),
            ({'status': 'running', 'tags': {'run.lifecycle': json.dumps({'_tag': 'starting'})}}, True),
            ({'status': 'running', 'tags': {'run.lifecycle': json.dumps({'_tag': 'terminal'})}}, True),
            ({'status': 'exited'}, True),
            ({'status': 'running', 'tags': {'run.lifecycle': 'invalid'}}, False),
            ({}, False),
        ]
        for row, expected in cases:
            with self.subTest(row=row), \
                    patch.object(self.module.subprocess, 'run', return_value=self.response(row)):
                self.assertEqual(self.module.registration_not_ready({}), expected)
        for observation in (subprocess.TimeoutExpired(['devnet', 'status'], 30),
                            subprocess.CompletedProcess([], 1, '', ''),
                            subprocess.CompletedProcess([], 0, 'invalid JSON', '')):
            with self.subTest(observation=observation):
                kwargs = ({'side_effect': observation} if isinstance(observation, Exception)
                          else {'return_value': observation})
                with patch.object(self.module.subprocess, 'run', **kwargs):
                    self.assertFalse(self.module.registration_not_ready({}))

    def test_inconclusive_registration_probe_does_not_stop_server(self):
        previous = self.activation_builds()
        for status in (subprocess.TimeoutExpired(['devnet', 'status'], 30),
                       subprocess.CompletedProcess([], 1, '', 'status unavailable')):
            with self.subTest(status=status), \
                    patch.object(self.module, 'current_server_url',
                                 side_effect=[None, 'https://new.invalid']), \
                    patch.object(self.module.subprocess, 'run',
                                 side_effect=[status, self.response({}), self.response({})]) as run, \
                    patch.object(self.module, 'recovery_retain_builds'):
                self.module.activate(previous)
                self.assertEqual([call.args[0][:3] for call in run.call_args_list],
                                 [['devnet', 'status', 'test'], ['devnet', 'up', '--replace'],
                                  ['devnet', 'url', 'test']])

    def test_failed_replace_preserves_candidate_or_unknown_publication_pointer(self):
        previous = self.activation_builds()
        for message in ('old generation retirement failed', 'ReplacementRecoveryRequired'):
            for published in ('https://candidate.invalid', None):
                error = subprocess.CalledProcessError(1, ['devnet', 'up', '--replace'], stderr=message)
                observations = [None, published] + ([] if published else [None])
                with self.subTest(error=message, published=published), \
                        patch.object(self.module, 'registration_not_ready', return_value=False), \
                        patch.object(self.module, 'current_server_url', side_effect=observations) as probe, \
                        patch.object(self.module.subprocess, 'run', side_effect=error) as run, \
                        patch.object(self.module, 'recovery_retain_builds') as retain:
                    with self.assertRaises(subprocess.CalledProcessError) as raised:
                        self.module.activate(previous)
                    self.assertIs(raised.exception, error)
                    self.assertEqual(run.call_count, 1)
                    self.assertEqual(probe.call_args_list[1].args[1], 'b' * 40)
                    if not published:
                        self.assertEqual(probe.call_args_list[2].args[1], 'a' * 40)
                    self.assertEqual((self.module.P / 'current').resolve().name, 'b' * 40)
                    retain.assert_not_called()

    def test_miss_fetches_complete_paths_then_hit_does_not_call_api(self):
        with patch.object(self.module, 'run', side_effect=self.responses()) as run:
            self.module.load_pr_files(self.pr)
        self.assertEqual(run.call_count, 2)
        self.assertEqual([entry['path'] for entry in self.pr['files']], self.files)
        self.assertEqual(len(self.cache_entries()), 1)
        self.assertFalse(list(self.cache_entries()[0].parent.glob('*.tmp')))
        with patch.object(self.module, 'run', side_effect=AssertionError('cache hit called API')):
            self.module.load_pr_files(dict(self.pr))

    def test_base_move_invalidates_even_with_identical_head(self):
        with patch.object(self.module, 'run', side_effect=self.responses()):
            self.module.load_pr_files(self.pr)
        self.pr['baseRefOid'] = 'c' * 40
        with patch.object(self.module, 'run', side_effect=self.responses()) as run:
            self.module.load_pr_files(self.pr)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(len(self.cache_entries()), 2)

    def test_head_move_invalidates(self):
        with patch.object(self.module, 'run', side_effect=self.responses()):
            self.module.load_pr_files(self.pr)
        self.pr['headRefOid'] = 'd' * 40
        with patch.object(self.module, 'run', side_effect=self.responses()) as run:
            self.module.load_pr_files(self.pr)
        self.assertEqual(run.call_count, 2)

    def test_partial_pagination_is_never_cached(self):
        with patch.object(self.module, 'run', side_effect=self.responses(files=self.files[:1])):
            with self.assertRaisesRegex(ValueError, 'incomplete file pagination'):
                self.module.load_pr_files(self.pr)
        self.assertEqual(self.cache_entries(), [])

    def test_multiple_pages_are_cached_only_when_complete(self):
        self.files = [f'apps/fractal-web/{n}.ts' for n in range(101)]
        self.pr['changedFiles'] = len(self.files)
        pages = [[{'filename': name} for name in self.files[:100]],
                 [{'filename': name} for name in self.files[100:]]]
        initial = self.response({'files': [{'path': name} for name in self.files[:100]]})
        with patch.object(self.module, 'run', side_effect=[initial, self.response(pages), self.response(self.pr)]):
            self.module.load_pr_files(self.pr)
        self.assertEqual(len(self.pr['files']), 101)
        self.assertEqual(len(self.cache_entries()), 1)

    def test_api_failure_does_not_cache_partial_stdout(self):
        error = subprocess.CalledProcessError(1, ['gh', 'api'], output='[[{"filename":"partial"}]]')
        with patch.object(self.module, 'run', side_effect=error):
            with self.assertRaises(subprocess.CalledProcessError):
                self.module.load_pr_files(self.pr)
        self.assertEqual(self.cache_entries(), [])

    def test_post_query_head_and_base_fences_refuse_cache_publication(self):
        for field in ('headRefOid', 'baseRefOid', 'changedFiles'):
            with self.subTest(field=field):
                fence = dict(self.pr, **{field: 3 if field == 'changedFiles' else 'e' * 40})
                with patch.object(self.module, 'run', side_effect=self.responses(fence=fence)):
                    with self.assertRaisesRegex(ValueError, 'moved during file selection'):
                        self.module.load_pr_files(self.pr)
                self.assertEqual(self.cache_entries(), [])

    def test_trust_is_rechecked_before_using_cached_files(self):
        with patch.object(self.module, 'run', side_effect=self.responses()):
            self.module.load_pr_files(self.pr)
        self.module.TRUST_MODE = 'collaborator-permission'
        row = {'skipped': []}
        with patch.object(self.module, 'run', side_effect=[self.response({'permission': 'write'}),
                                                         self.response({'permission': 'read'})]) as run:
            self.assertTrue(self.module.trusted_pr(self.pr, row, {}))
            self.assertFalse(self.module.trusted_pr(self.pr, row, {}))
        self.assertEqual(run.call_count, 2)
        self.assertEqual(row['skipped'][0]['reason'], 'untrusted-author')

    def test_expired_budget_does_not_read_cache_or_call_api(self):
        self.module.SELECTION_DEADLINE = 9
        with patch.object(self.module.time, 'monotonic', return_value=10), \
                patch.object(self.module, 'run', side_effect=AssertionError('expired API call')):
            with self.assertRaises(self.module.SelectionDeadline):
                self.module.load_pr_files(self.pr)
        self.assertEqual(self.cache_entries(), [])

    def test_subprocess_timeout_uses_remaining_total_budget(self):
        self.module.SELECTION_DEADLINE = 12
        error = subprocess.TimeoutExpired(['gh'], 2)
        with patch.object(self.module.time, 'monotonic', side_effect=[10, 10, 12]), \
                patch.object(self.module.subprocess, 'run', side_effect=error) as run:
            with self.assertRaises(self.module.SelectionDeadline):
                self.module.run(['gh', 'api', 'test'], capture=True, timeout=120)
        self.assertEqual(run.call_args.kwargs['timeout'], 2)

    def test_successful_gh_call_still_checks_selection_deadline(self):
        self.module.SELECTION_DEADLINE = 12
        with patch.object(self.module.time, 'monotonic', side_effect=[10, 10, 12]), \
                patch.object(self.module.subprocess, 'run', return_value=self.response({})) as run, \
                patch.object(self.module.candidate_sandbox, 'run',
                             side_effect=AssertionError('selection must use bounded host route')):
            with self.assertRaises(self.module.SelectionDeadline):
                self.module.run(['gh', 'api', 'test'], capture=True, timeout=120)
        self.assertEqual(run.call_args.kwargs['timeout'], 2)

    def test_merged_signature_version_invalidates_both_parent_versions(self):
        self.assertGreater(self.module.INTEGRATOR_VERSION, 8)

    def test_deadline_propagates_through_permission_check(self):
        self.module.TRUST_MODE = 'collaborator-permission'
        with patch.object(self.module, 'run', side_effect=self.module.SelectionDeadline('expired')):
            with self.assertRaises(self.module.SelectionDeadline):
                self.module.trusted_pr(self.pr, {'skipped': []}, {})

    def test_deadline_tick_logs_selection_outcome_without_mutating_candidate(self):
        self.module.P.mkdir(parents=True)
        current = self.module.P / 'current-sha'
        current.write_text('f' * 40)
        def observed(row):
            row.update(servedSha=current.read_text(), current=False)
        with patch.object(self.module, 'run', side_effect=self.module.SelectionDeadline('expired')), \
                patch.object(self.module, 'observe_served', side_effect=observed), \
                patch.object(self.module, 'prepare_clone', side_effect=AssertionError('candidate mutated')), \
                patch.object(self.module.candidate_sandbox, 'validate_configuration'), \
                contextlib.redirect_stdout(io.StringIO()):
            self.module.tick()
        row = json.loads((self.module.L / 'live-r3.jsonl').read_text())
        self.assertEqual(row['build'], 'skipped')
        self.assertEqual(row['reason'], 'selection-deadline')
        self.assertTrue(row['selectionFailure'])
        self.assertFalse(row['selectionComplete'])
        self.assertEqual(current.read_text(), 'f' * 40)
        self.assertIsNone(self.module.SELECTION_DEADLINE)
        self.assertFalse(self.module.C.exists())

    def test_clone_config_refusal_is_preserved_in_tick_receipt(self):
        subprocess.run(['git', 'init', '-q', str(self.module.C)], check=True)
        config = self.module.C / '.git/config'
        original = config.read_bytes()
        real_run = self.module.run
        def run(args, **kwargs):
            return self.response([]) if args[0] == 'gh' else real_run(args, **kwargs)
        cases = [
            ('gc.rerereunresolved', 'gc.rerereunresolved'),
            ('filter.x.clean', 'filter.x.clean'),
            ('url.https://fixture-user@example.invalid/repo?access=fixture-query.insteadOf',
             'url.<redacted-subsection>.insteadof'),
            ('http.https://fixture-user@example.invalid/repo?access=fixture-query.extraheader',
             'http.<redacted-subsection>.extraheader'),
        ]
        for key, expected in cases:
            with self.subTest(key=key):
                config.write_bytes(original)
                subprocess.run(['git', 'config', '--file', str(config), key, 'forbidden'], check=True)
                before = config.read_bytes()
                log = io.StringIO()
                with patch.object(self.module, 'run', side_effect=run), \
                        patch.object(self.module, 'observe_served'), \
                        patch.object(self.module.candidate_sandbox, 'validate_configuration'), \
                        contextlib.redirect_stdout(log):
                    self.module.tick()
                row = json.loads((self.module.L / 'live-r3.jsonl').read_text().splitlines()[-1])
                self.assertEqual(row['build'], 'failed')
                self.assertIn('non-allowlisted', row['detail'])
                self.assertIn(expected, row['detail'])
                self.assertIn(expected, log.getvalue())
                for secret in ('fixture-user', 'fixture-query', 'forbidden'):
                    self.assertNotIn(secret, log.getvalue())
                self.assertEqual(config.read_bytes(), before)

    def test_clone_preparation_rejects_worktree_identity_override(self):
        subprocess.run(['git', 'init', '-q', str(self.module.C)], check=True)
        subprocess.run(['git', '-C', str(self.module.C), 'config',
                        'extensions.worktreeConfig', 'true'], check=True)
        worktree_config = self.module.C / '.git/config.worktree'
        worktree_config.write_text('[user]\n name = Not the launcher\n email = override@example.invalid\n')
        before = worktree_config.read_bytes()
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            self.module.prepare_clone()
        self.assertIn('user.name', raised.exception.stderr)
        self.assertIn('user.email', raised.exception.stderr)
        self.assertNotIn('override@example.invalid', raised.exception.stderr)
        self.assertEqual(worktree_config.read_bytes(), before)

    def test_uncaptured_host_git_failure_retains_stderr(self):
        subprocess.run(['git', 'init', '-q', str(self.module.C)], check=True)
        with contextlib.redirect_stderr(io.StringIO()), \
                self.assertRaises(subprocess.CalledProcessError) as raised:
            self.module.git('checkout', 'missing-branch')
        self.assertIn('missing-branch', raised.exception.stderr)
        real_run = self.module.run
        def run(args, **kwargs):
            return self.response([]) if args[0] == 'gh' else real_run(args, **kwargs)
        with patch.object(self.module, 'run', side_effect=run), \
                patch.object(self.module, 'prepare_clone',
                             side_effect=lambda: self.module.git('checkout', 'missing-branch')), \
                patch.object(self.module, 'observe_served'), \
                patch.object(self.module.candidate_sandbox, 'validate_configuration'), \
                contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            self.module.tick()
        row = json.loads((self.module.L / 'live-r3.jsonl').read_text())
        self.assertEqual(row['build'], 'failed')
        self.assertIn('missing-branch', row['detail'])

    def test_fresh_clone_preparation_can_commit_without_global_identity(self):
        # The tick runs the real clone/preparation and host-git paths. Stop after
        # a commit at the fetch boundary; no candidate build or network is needed.
        source = self.module.SOURCE_CLONE
        subprocess.run(['git', 'init', '-q', str(source)], check=True)
        subprocess.run(['git', '-C', str(source), '-c', 'user.name=Fixture',
                        '-c', 'user.email=fixture@example.invalid', '-c', 'core.hooksPath=/dev/null',
                        'commit', '--allow-empty', '-qm', 'initial'], check=True)
        subprocess.run(['git', '-C', str(source), 'branch', '-M', 'main'], check=True)
        real_run = self.module.run
        committed = []
        class CommitComplete(Exception):
            pass
        def run(args, **kwargs):
            if args[0] == 'gh':
                return self.response([])
            if args[0] == 'git' and 'fetch' in args:
                self.module.git('commit', '--allow-empty', '-m', 'integrator fixture')
                committed.append(self.module.text('log', '-1', '--format=%an <%ae>|%cn <%ce>'))
                raise CommitComplete('commit proof complete')
            return real_run(args, **kwargs)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(('GIT_', 'FRACTAL_INTEGRATOR_GIT_USER_'))}
        env.update(GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1',
                   FRACTAL_INTEGRATOR_SANDBOX='bwrap')
        with patch.dict(os.environ, env, clear=True), \
                patch.object(self.module, 'run', side_effect=run), \
                patch.object(self.module, 'observe_served'), \
                patch.object(self.module.candidate_sandbox, 'validate_configuration'), \
                contextlib.redirect_stdout(io.StringIO()):
            self.module.tick()
        identity = 'Fractal live-r3 integrator <live-r3@fractal-integrator.invalid>'
        self.assertEqual(committed, [identity + '|' + identity])
        row = json.loads((self.module.L / 'live-r3.jsonl').read_text())
        self.assertEqual(row['error'], 'commit proof complete')
        with patch.dict(os.environ, {'FRACTAL_INTEGRATOR_GIT_USER_NAME': 'Deployment Bot',
                                     'FRACTAL_INTEGRATOR_GIT_USER_EMAIL': 'bot@example.invalid'}):
            self.module.prepare_clone()
        self.assertEqual(self.module.text('config', '--local', 'user.name'), 'Deployment Bot')
        self.assertEqual(self.module.text('config', '--local', 'user.email'), 'bot@example.invalid')

    def test_oversized_pr_is_skipped_without_api_or_cache_and_selection_continues(self):
        oversized = dict(self.pr, changedFiles=4058)
        row = {'skipped': [], 'selection': []}
        with patch.object(self.module, 'run', side_effect=AssertionError('known oversized query')):
            self.assertFalse(self.module.load_selected_pr_files(oversized, row))
        self.assertEqual(row['skipped'][0]['reason'], 'files-incomplete')
        self.assertEqual(row['skipped'][0]['changedFiles'], 4058)
        self.assertEqual(row['skipped'][0]['fetchedFiles'], 0)
        self.assertEqual(row['selection'][0]['reason'], 'files-incomplete')
        self.assertEqual(self.cache_entries(), [])
        with patch.object(self.module, 'run', side_effect=self.responses()):
            self.assertTrue(self.module.load_selected_pr_files(self.pr, row))
        self.assertEqual(len(self.cache_entries()), 1)

    def test_partial_paths_are_skipped_in_selection_receipt_without_cache(self):
        row = {'ts': 'test', 'main': None, 'included': [], 'skipped': [], 'selection': []}
        with patch.object(self.module, 'run', side_effect=self.responses(files=self.files[:1])):
            self.assertFalse(self.module.load_selected_pr_files(self.pr, row))
        self.module.publish_skips(row)
        receipt = json.loads((self.module.S / 'steward/last-skips.json').read_text())
        self.assertEqual(receipt['skipped'][0]['fetchedFiles'], 1)
        self.assertEqual(receipt['selection'][0]['changedFiles'], 2)
        self.assertEqual(receipt['selection'][0]['reason'], 'files-incomplete')
        self.assertEqual(self.cache_entries(), [])


if __name__ == '__main__':
    unittest.main()
