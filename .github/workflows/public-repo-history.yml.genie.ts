import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'
import { fullHistoryCheckoutSteps } from './workspace-ci.ts'

// Admission requires an organization ruleset's `workflows` rule selecting this
// path at an approved immutable SHA, not a shared Actions app status context.
// github.workflow_sha pins the scanner and policy to the same trusted revision.
// Candidate code is Git data only; the pinned revision must contain the scanner.
export default githubWorkflow(auditCaches({
  name: 'Public repository history',
  on: {
    pull_request: null,
    merge_group: null,
  },
  permissions: { contents: 'read' },
  concurrency: {
    group: 'public-repo-history-${{ github.run_id }}-${{ github.event_name }}',
    'cancel-in-progress': false,
  },
  jobs: {
    'public-repo-history': {
      name: 'public-repo-history',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 10,
      env: {
        BASE_SHA: '${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha }}',
        HEAD_SHA: '${{ github.event.pull_request.head.sha || github.event.merge_group.head_sha }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps({
          ref: '${{ github.workflow_sha }}',
          'persist-credentials': false,
        }, { name: 'Check out pinned workflow scanner and policy' }),
        {
          name: 'Fetch candidate history without checking it out',
          shell: 'bash',
          run: 'git fetch --no-tags origin "$BASE_SHA" "$HEAD_SHA"\ngit cat-file -e "$BASE_SHA^{commit}"\ngit cat-file -e "$HEAD_SHA^{commit}"\n',
        },
        {
          name: 'Check every introduced commit using pinned trusted policy',
          shell: 'bash',
          run: 'python3 "$PWD/scripts/check-public-repo" --repo "$PWD" --base "$BASE_SHA" --head "$HEAD_SHA"',
        },
      ],
    },
  },
}, {
  'public-repo-history': 'Reads candidate Git objects with the pinned workflow scanner and policy; never executes candidate code or signs evidence.',
}))
