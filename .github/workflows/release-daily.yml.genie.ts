import { fullHistoryCheckoutSteps, linuxActionlintConfig } from './workspace-ci.ts'
import { auditCaches } from './cache-audit.ts'
import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'

// Prepares a daily draft candidate only when main changed, reusing verified main archives.
// Stable publication requires an explicit dispatch with the exact candidate tag, SHA and product GO.
// Tags made with this workflow's token do not start the tag workflow, so there is no second build.
export default githubWorkflow(auditCaches({
  actionlint: linuxActionlintConfig,
  name: 'Fractal release candidates and publication',
  on: {
    schedule: [{ cron: '17 5 * * *' }],
    workflow_dispatch: {
      inputs: {
        mode: {
          description: 'Prepare a draft candidate or publish an explicitly approved candidate',
          required: true,
          type: 'choice',
          options: ['candidate', 'publish'],
          default: 'candidate',
        },
        tag: {
          description: 'Candidate tag; required for publication, otherwise defaults to the next patch',
          required: false,
          type: 'string',
        },
        run: {
          description: 'Successful main tag-release run ID; default is the newest',
          required: false,
          type: 'string',
        },
        sha: {
          description: 'Exact candidate source SHA approved for publication',
          required: false,
          type: 'string',
        },
        go_ref: {
          description: 'Product GO evidence reference approving this exact SHA',
          required: false,
          type: 'string',
        },
      },
    },
  },
  permissions: { contents: 'read' },
  concurrency: { group: 'fractal-daily-release', 'cancel-in-progress': false },
  jobs: {
    release: {
      name: 'daily-release-candidate',
      if: "inputs.mode != 'publish'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 20,
      permissions: { contents: 'write', actions: 'read' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        GH_REPO: '${{ github.repository }}',
        TAG: '${{ inputs.tag }}',
        RUN: '${{ inputs.run }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps(),
        {
          name: 'Prepare a draft candidate from the verified archives of main',
          run: 'scripts/release-fractal-daily ${TAG:+--tag "$TAG"} ${RUN:+--run "$RUN"}',
        },
      ],
    },
    publish: {
      name: 'publish-approved-release',
      if: "github.event_name == 'workflow_dispatch' && inputs.mode == 'publish' && github.ref == 'refs/heads/main'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 20,
      permissions: { contents: 'write' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        GH_REPO: '${{ github.repository }}',
        TAG: '${{ inputs.tag }}',
        SOURCE_SHA: '${{ inputs.sha }}',
        GO_REF: '${{ inputs.go_ref }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps(),
        {
          name: 'Publish only the exact candidate approved by product GO',
          run: 'python3 scripts/release-fractal-publish --tag "$TAG" --sha "$SOURCE_SHA" --go-ref "$GO_REF"',
        },
      ],
    },
    advance: {
      name: 'advance-compatibility-baselines',
      needs: 'publish',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 20,
      permissions: { contents: 'write', actions: 'write', 'pull-requests': 'write' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        GH_REPO: '${{ github.repository }}',
        TAG: '${{ inputs.tag }}',
      },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        {
          name: 'Propose baselines for the explicitly published release',
          run: 'python3 scripts/compat-baseline-release advance "$TAG"',
        },
      ],
    },
  },
}, {"release": "Prepares draft candidates from verified main release artifacts; builds nothing.", "publish": "Publishes an exact draft candidate only with explicit product GO; builds nothing.", "advance": "Verifies the explicitly published archives and proposes the baseline maintenance PR; builds nothing."}))
