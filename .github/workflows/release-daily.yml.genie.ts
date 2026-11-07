import { fullHistoryCheckoutSteps, linuxActionlintConfig } from './workspace-ci.ts'
import { auditCaches } from './cache-audit.ts'
import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'

// Prepares an untagged daily draft from verified main archives, or builds an exact first-parent main SHA.
// Stable publication creates the tag after an explicit dispatch with the exact candidate tag, SHA and product GO.
// Tags made at publication with this workflow's token do not start a second build.
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
        summary: {
          description: 'Optional operator summary for the draft candidate',
          required: false,
          type: 'string',
        },
        sha: {
          description: 'Candidate: exact first-parent main source SHA; publish: exact candidate SHA approved for publication',
          required: false,
          type: 'string',
        },
        go_ref: {
          description: 'Product GO evidence reference approving this exact SHA',
          required: false,
          type: 'string',
        },
        allow_superseded: {
          description: 'To publish an older candidate, explicitly repeat its selected tag',
          required: false,
          type: 'string',
        },
      },
    },
  },
  permissions: { contents: 'read' },
  concurrency: {
    group: "${{ github.event_name == 'workflow_dispatch' && inputs.mode == 'candidate' && inputs.sha != '' && format('fractal-daily-release-{0}', inputs.sha) || 'fractal-daily-release' }}",
    'cancel-in-progress': false,
  },
  jobs: {
    release: {
      name: 'daily-release-candidate',
      if: "inputs.mode != 'publish'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 180,
      permissions: { contents: 'write', actions: 'write' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        GH_REPO: '${{ github.repository }}',
        TAG: '${{ inputs.tag }}',
        RUN: '${{ inputs.run }}',
        SUMMARY: '${{ inputs.summary }}',
        SOURCE_SHA: '${{ inputs.sha }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps(),
        {
          name: 'Prepare a draft candidate from verified main archives or an isolated exact-SHA build',
          run: 'scripts/release-fractal-daily ${TAG:+--tag "$TAG"} ${RUN:+--run "$RUN"} --summary "$SUMMARY" --sha "$SOURCE_SHA"',
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
        ALLOW_SUPERSEDED: '${{ inputs.allow_superseded }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps(),
        {
          name: 'Publish only the exact candidate approved by product GO',
          run: 'python3 scripts/release-fractal-publish --tag "$TAG" --sha "$SOURCE_SHA" --go-ref "$GO_REF" --allow-superseded "$ALLOW_SUPERSEDED"',
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
}, {"release": "Prepares untagged draft candidates from verified main release artifacts or dispatches and waits for an isolated exact first-parent main SHA build, with an optional operator summary.", "publish": "Creates the tag and publishes an exact draft candidate only with explicit product GO and, for an older candidate, an override naming its tag; builds nothing.", "advance": "Verifies the explicitly published archives and proposes the baseline maintenance PR; builds nothing."}))
