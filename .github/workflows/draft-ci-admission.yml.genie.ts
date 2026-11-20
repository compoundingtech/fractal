import { withoutForkPRJobs } from './ready-pr.ts'
import { readFileSync } from 'node:fs'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'

// workflow_run uses main's code even for a PR targeting an older stacked branch.
// Never check out PR code or download its artifacts with this write-scoped token.
// This main-owned metadata-only job never executes the triggering PR, including forks.
// Keep admission off the hosted dispatch queue it is meant to drain.
export default githubWorkflow(withoutForkPRJobs(auditCaches({
  name: 'Draft CI admission',
  on: {
    workflow_run: {
      workflows: ['Workspace CI', 'Performance', 'Public repository check', 'Public repository history', 'Nix', 'macOS CI', 'Fractal tag release', 'fractal-web'],
      types: ['requested'],
    },
  },
  permissions: { actions: 'write', 'pull-requests': 'read' },
  concurrency: { group: 'draft-ci-admission-${{ github.event.workflow_run.id }}', 'cancel-in-progress': false },
  actionlint: {
    ...defaultActionlintConfig,
    selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), 'ci1-priority'],
  },
  jobs: {
    admission: {
      if: "github.event.workflow_run.event == 'pull_request'",
      'runs-on': 'ci1-priority',
      'timeout-minutes': 3,
      steps: [{
        name: 'Qualify the owned control Python runtime',
        run: 'python3 -c "import sys; assert sys.version_info >= (3, 10), sys.version; print(sys.version)"',
      }, {
        name: 'Cancel only a currently draft PR run',
        env: { GH_TOKEN: '${{ github.token }}', REPOSITORY: '${{ github.repository }}', RUN_ID: '${{ github.event.workflow_run.id }}' },
        run: `python3 - <<'DRAFT_ADMISSION_PY'\n${readFileSync(new URL('../../scripts/ci-draft-admission', import.meta.url), 'utf8')}\nDRAFT_ADMISSION_PY`,
      }],
    },
  },
}, { admission: 'Reads PR/run metadata and requests draft cancellation; no checkout, build or cache.' })))
