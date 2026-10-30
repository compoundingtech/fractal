import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'

// SHADOW ONLY: this job reports whether a pull request would be rejected for lacking a trusted signed note; it never fails and is not
// a required check. `pull_request_target` always runs the workflow file of the BASE branch, so a pull request cannot edit or delete this
// job, and the checkout below is the base branch tip. The pull request's code is never checked out or executed: scripts/ci-signed-tree
// fetches its head commit as git objects and reads them as data, with the verifier, keys, policy and lane file of the base checkout.
// No secret is used and the token is read-only.
export default githubWorkflow(auditCaches({
  name: 'Signed tree (shadow)',
  on: {
    pull_request_target: { types: ['opened', 'synchronize', 'reopened', 'ready_for_review'] },
  },
  permissions: { contents: 'read' },
  concurrency: { group: 'signed-tree-${{ github.event.pull_request.number }}', 'cancel-in-progress': true },
  actionlint: defaultActionlintConfig,
  jobs: {
    'signed-tree': {
      name: 'signed-tree',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      'continue-on-error': true,
      defaults: { run: { shell: 'bash' } },
      steps: [
        // No ref: for pull_request_target this is the base branch tip, never the pull request.
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        {
          name: 'Would this pull request be rejected for lacking a signed note?',
          env: {
            REPOSITORY: '${{ github.repository }}',
            PR_NUMBER: '${{ github.event.pull_request.number }}',
            PR_HEAD_SHA: '${{ github.event.pull_request.head.sha }}',
            PR_HEAD_REPOSITORY: '${{ github.event.pull_request.head.repo.full_name }}',
            PR_DRAFT: '${{ github.event.pull_request.draft }}',
          },
          run: 'python3 scripts/ci-signed-tree',
        },
      ],
    },
  },
}, { 'signed-tree': 'Reads git objects and notes only; builds nothing and restores no cache.' }))
