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
