import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'

// Keep this separate from pull_request workflows: candidate code is Git data only.
// The accepted base must already contain the history scanner before this can pass.
export default githubWorkflow(auditCaches({
  name: 'Public repository history',
  on: {
    pull_request_target: { types: ['opened', 'synchronize', 'reopened', 'ready_for_review', 'edited'] },
    merge_group: null,
  },
  permissions: { contents: 'read' },
  concurrency: {
    group: 'public-repo-history-${{ github.event.pull_request.number || github.event.merge_group.head_sha }}',
    'cancel-in-progress': true,
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
        {
          name: 'Check out immutable accepted base and policy',
          uses: 'actions/checkout@v4',
          with: {
            ref: '${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha }}',
            'fetch-depth': 0,
            'persist-credentials': false,
          },
        },
        {
          name: 'Fetch candidate history without checking it out',
          shell: 'bash',
          run: 'git fetch --no-tags origin "$HEAD_SHA"\ngit cat-file -e "$HEAD_SHA^{commit}"\n',
        },
        {
          name: 'Check every introduced commit using accepted base policy',
          shell: 'bash',
          run: 'python3 "$PWD/scripts/check-public-repo" --repo "$PWD" --base "$BASE_SHA" --head "$HEAD_SHA"',
        },
      ],
    },
  },
}, {
  'public-repo-history': 'Reads candidate Git objects with the accepted base scanner and policy; never executes candidate code or signs evidence.',
}))
