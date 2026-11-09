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
          description: 'Successful main tag-release run ID. Provide the exact source SHA to reuse a dispatched candidate.',
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
        recover_draft: {
          description: 'Opt in to one minimal draft POST and exact-match recovery; requires explicit tag, sha and run',
          required: false,
          type: 'boolean',
          default: false,
        },
      },
    },
  },
  permissions: { contents: 'read' },
  jobs: {
    build: {
      name: 'build-exact-release-candidate',
      if: "github.event_name == 'workflow_dispatch' && inputs.mode == 'candidate' && inputs.sha != '' && inputs.run == '' && github.ref == 'refs/heads/main' && inputs.recover_draft != true",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 180,
      concurrency: {
        group: 'fractal-daily-release-build-${{ inputs.sha }}',
        'cancel-in-progress': false,
      },
      permissions: { contents: 'read', actions: 'write' },
      outputs: { build_run: '${{ steps.build.outputs.build_run }}' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        GH_REPO: '${{ github.repository }}',
        SOURCE_SHA: '${{ inputs.sha }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps(),
        {
          name: 'Build the exact first-parent main SHA without release mutations',
          id: 'build',
          run: 'scripts/release-fractal-daily --build-only --sha "$SOURCE_SHA"',
        },
      ],
    },
    release: {
      name: 'daily-release-candidate',
      needs: 'build',
      if: "${{ always() && !cancelled() && (needs.build.result == 'success' || needs.build.result == 'skipped') && inputs.mode != 'publish' && github.ref == 'refs/heads/main' }}",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 20,
      concurrency: {
        group: 'fractal-release-mutation',
        'cancel-in-progress': false,
      },
      permissions: { contents: 'write', actions: 'read' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        GH_REPO: '${{ github.repository }}',
        TAG: '${{ inputs.tag }}',
        RUN: '${{ needs.build.outputs.build_run || inputs.run }}',
        SUMMARY: '${{ inputs.summary }}',
        SOURCE_SHA: '${{ inputs.sha }}',
        RECOVER_DRAFT: '${{ inputs.recover_draft }}',
      },
      steps: [
        ...fullHistoryCheckoutSteps(),
        {
          name: 'Prepare a draft candidate from verified main archives',
          run: 'if [ "$RECOVER_DRAFT" = true ]; then set -- --recover-draft; else set --; fi\nscripts/release-fractal-daily "$@" ${TAG:+--tag "$TAG"} ${RUN:+--run "$RUN"} --summary "$SUMMARY" --sha "$SOURCE_SHA"',
        },
      ],
    },
    publish: {
      name: 'publish-approved-release',
      if: "github.event_name == 'workflow_dispatch' && inputs.mode == 'publish' && github.ref == 'refs/heads/main'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 20,
      concurrency: {
        group: 'fractal-release-mutation',
        'cancel-in-progress': false,
      },
      permissions: { contents: 'write', actions: 'read' },
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
    observe: {
      name: 'report-cancelled-release-jobs',
      needs: ['build', 'release', 'publish'],
      if: "always() && contains(needs.*.result, 'cancelled')",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read' },
      steps: [
        {
          name: 'Fail visibly when a pending release job is cancelled',
          run: [
            'echo "::error::A release job was cancelled. GitHub concurrency can cancel an older pending job even with cancel-in-progress disabled. Re-dispatch this workflow with the full original inputs (mode, tag, run, summary, sha, go_ref, allow_superseded); do not assume publication completed."',
            'exit 1',
          ].join('\n'),
        },
      ],
    },
    advance: {
      name: 'advance-compatibility-baselines',
      needs: 'publish',
      if: "github.ref == 'refs/heads/main'",
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
}, {"build": "Builds an explicitly dispatched exact first-parent main SHA in SHA-isolated concurrency without release mutations, exposing the successful build run ID.", "release": "Prepares untagged draft candidates from verified main release artifacts or the completed exact-SHA build, serialized with publication.", "publish": "Creates the tag and publishes an exact draft candidate only with explicit product GO and, for an older candidate, an override naming its tag; serialized with candidate mutations and builds nothing.", "observe": "Fails visibly if a build or mutation job is cancelled, including pending concurrency cancellation, and directs operators to re-dispatch every original input; performs no mutations.", "advance": "Verifies the explicitly published archives and proposes the baseline maintenance PR; builds nothing."}))
