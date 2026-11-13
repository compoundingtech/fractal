import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'
import { fullHistoryCheckoutSteps } from './workspace-ci.ts'
import { readyPRJobs, readyPullRequestTypes } from './ready-pr.ts'

// Ordinary content check until a separate trust anchor is activated.
// A future organization `workflows` rule can select this path at an approved SHA.
// Only then does github.workflow_sha pin scanner/policy to a trusted revision;
// without that rule a candidate can change its own workflow and scanner.
export default githubWorkflow(readyPRJobs(auditCaches({
  name: 'Public repository history',
  on: {
    pull_request: { types: [...readyPullRequestTypes] },
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
  'public-repo-history': 'Scans introduced Git content; trusted scanner identity requires the separately activated required-workflow rule.',
})))
