import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'

const publishHistoryStatus = `
await github.rest.repos.createCommitStatus({
  ...context.repo,
  sha: process.env.HEAD_SHA,
  context: 'public-repo-history',
  state: process.env.HISTORY_STATE,
  target_url: \`\${context.serverUrl}/\${context.repo.owner}/\${context.repo.repo}/actions/runs/\${context.runId}\`,
  description: process.env.HISTORY_STATE === 'pending'
    ? 'Checking introduced history with accepted base policy'
    : process.env.HISTORY_STATE === 'success'
      ? 'Introduced history passed accepted base policy'
      : 'History check failed or did not complete',
})
`

// Keep this separate from pull_request workflows: candidate code is Git data only.
// The accepted base must already contain the history scanner before this can pass.
// Status write access only binds the required result to the candidate SHA: the
// pull_request_target workflow run itself is associated with the accepted base.
export default githubWorkflow(auditCaches({
  name: 'Public repository history',
  on: {
    pull_request_target: { types: ['opened', 'synchronize', 'reopened', 'ready_for_review', 'edited'] },
    merge_group: null,
  },
  permissions: { contents: 'read', statuses: 'write' },
  concurrency: {
    group: 'public-repo-history-${{ github.event.pull_request.number || github.event.merge_group.head_sha }}',
    'cancel-in-progress': true,
  },
  jobs: {
    'public-repo-history': {
      name: 'public-repo-history-runner',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 10,
      env: {
        BASE_SHA: '${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha }}',
        HEAD_SHA: '${{ github.event.pull_request.head.sha || github.event.merge_group.head_sha }}',
      },
      steps: [
        {
          name: 'Mark candidate history status pending',
          uses: 'actions/github-script@v7',
          env: { HISTORY_STATE: 'pending' },
          with: { script: publishHistoryStatus },
        },
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
        {
          name: 'Publish candidate history result',
          if: 'always()',
          uses: 'actions/github-script@v7',
          env: { HISTORY_STATE: "${{ job.status == 'success' && 'success' || 'failure' }}" },
          with: { script: publishHistoryStatus },
        },
      ],
    },
  },
}, {
  'public-repo-history': 'Reads candidate Git objects with the accepted base scanner and policy; never executes candidate code or signs evidence.',
}))
