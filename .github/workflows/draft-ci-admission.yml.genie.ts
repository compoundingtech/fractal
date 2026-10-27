import { readFileSync } from 'node:fs'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'

// workflow_run uses main's code even for a PR targeting an older stacked branch.
// Never check out PR code or download its artifacts with this write-scoped token.
export default githubWorkflow(auditCaches({
  name: 'Draft CI admission',
  on: {
    workflow_run: {
      workflows: ['Workspace CI', 'Performance', 'Public repository check', 'Nix', 'macOS CI', 'Fractal tag release', 'fractal-web'],
      types: ['requested'],
    },
  },
  permissions: { actions: 'write', 'pull-requests': 'read' },
  concurrency: { group: 'draft-ci-admission-${{ github.event.workflow_run.id }}', 'cancel-in-progress': false },
  actionlint: defaultActionlintConfig,
  jobs: {
    admission: {
      if: "github.event.workflow_run.event == 'pull_request'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 3,
      steps: [{
        name: 'Cancel only a currently draft PR run',
        env: { GH_TOKEN: '${{ github.token }}', REPOSITORY: '${{ github.repository }}', RUN_ID: '${{ github.event.workflow_run.id }}' },
        run: `python3 - <<'DRAFT_ADMISSION_PY'\n${readFileSync(new URL('../../scripts/ci-draft-admission', import.meta.url), 'utf8')}\nDRAFT_ADMISSION_PY`,
      }],
    },
  },
}, { admission: 'Reads PR/run metadata and requests draft cancellation; no checkout, build or cache.' }))
