import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { perfLoadJobs } from './perf-load.ts'
import { readyPRJobs } from './ready-pr.ts'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { readFileSync } from 'node:fs'
import { linuxRunner, linuxStageRunner, performanceRunner } from './workspace-ci.ts'

// Ignore unrelated label events before either the scope job or the load job starts.
// Give those events separate concurrency groups so they cannot cancel or replace PR work.
const irrelevantLabel = "github.event_name == 'pull_request' && (github.event.action == 'labeled' || github.event.action == 'unlabeled') && github.event.label.name != 'ci-perf'"
const relevantEvent = `!(${irrelevantLabel})`
const admitted = `!cancelled() && (${relevantEvent}) && (needs.scope.result != 'success' || needs.scope.outputs.run != 'false')`

// Performance by manual dispatch only, on any ref: the load jobs the daily long lanes run on main
// (daily-long.yml, docs/ci.md), from the same definitions (perf-load.ts). Main's daily runs seed the
// durable snapshots and real baseline reports a dispatched run compares with; Performance always stays
// on Namespace. Pushes to main no longer start it: newer pushes cancelled nearly every run.
export default githubWorkflow(scopeLinuxNixSandbox(readyPRJobs({
  name: 'Performance',
  on: {
    workflow_dispatch: {},
  },
  permissions: { contents: 'read', actions: 'read', 'pull-requests': 'read' },
  concurrency: {
    // Preserve the existing PR group; every dispatched run keeps its own group.
    group: `perf-\${{ (${irrelevantLabel}) && github.run_id || github.event.pull_request.number || github.run_id }}`,
    'cancel-in-progress': `\${{ (${relevantEvent}) && github.event_name == 'pull_request' }}`,
  },
  actionlint: {
    ...defaultActionlintConfig,
    selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), ...linuxRunner, ...linuxStageRunner, ...performanceRunner],
  },
  jobs: {
    scope: {
      name: 'Performance scope',
      if: relevantEvent,
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read', 'pull-requests': 'read' },
      outputs: { run: '${{ steps.scope.outputs.run }}' },
      steps: [{
        name: 'Admit main, nightly and selected PR Performance', id: 'scope',
        env: { GH_TOKEN: '${{ github.token }}' },
        // Workflow bytes from this run's ref (the PR merge ref for pull_request).
        // Metadata-only: no extra PR checkout or compiler; token is read-only.
        run: `python3 - <<'PERF_SCOPE_PY'\n${readFileSync(new URL('../../scripts/ci-perf-scope', import.meta.url), 'utf8')}\nPERF_SCOPE_PY`,
      }],
    },
    'perf-load': { name: 'perf-load', needs: ['scope'], if: admitted, ...perfLoadJobs['perf-load'] },
    // The hog runs cost a second performance runner and a macOS runner: manual dispatch only, never a
    // pull request even if PR triggers return. Here they report without failing the run; the daily
    // lanes fail on red, so a broken hog run alerts once a day.
    'perf-load-hog': {
      name: 'perf-load-hog',
      needs: ['scope'],
      if: `!cancelled() && github.event_name == 'workflow_dispatch' && (needs.scope.result != 'success' || needs.scope.outputs.run != 'false')`,
      'continue-on-error': true,
      ...perfLoadJobs['perf-load-hog'],
    },
    'perf-load-hog-macos': {
      name: 'perf-load-hog-macos',
      if: `github.event_name == 'workflow_dispatch'`,
      'continue-on-error': true,
      ...perfLoadJobs['perf-load-hog-macos'],
    },
  },
})))
