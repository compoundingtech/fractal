import { auditCaches } from './cache-audit.ts'
import { buildSnapshotPrepare, buildSnapshotRestore } from './build-snapshot.ts'
import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { buildEnv, cargoCacheStep, commonSetupSteps, fullHistoryCheckoutSteps, megarepoApplyStep, nixCacheStep } from './workspace-ci.ts'

/**
 * Burst: one allowlisted suite on one exact commit, dispatched on demand (docs/ci.md, "Burst runs").
 * The suite commands come from this workflow on main; the scripts they call come from the requested commit.
 * The dispatch input, the checkout and the `burst/<suite>` commit status name the same SHA.
 */
const suites = ['cargo-test', 'nextest', 'st-test-all'] as const

// Optional work: the same 8x16 shape and lowest queue class as Performance, below merge groups and PR checks.
// `vars.BURST_RUNS_ON` (a JSON label array) replaces it without a workflow change.
const defaultRunsOn = JSON.stringify([
  'nscloud-ubuntu-24.04-amd64-8x16-with-features;job.priority=3',
  'namespace-features:github.run-id={0}',
])
const runsOn = `\${{ fromJSON(vars.BURST_RUNS_ON || format('${defaultRunsOn}', github.run_id)) }}`

// Main upkeep's perf-cost job keeps the default dev shell's Nix outputs and a Cargo home (registry, proc macros,
// build scripts) on main; restoring them keeps a burst run from starting fully cold. Restore only: burst saves nothing.
const fromPerfCost = <T extends { with: Record<string, string> }>(step: T): T => ({
  ...step,
  with: Object.fromEntries(Object.entries(step.with).map(([key, value]) => [key, value.replaceAll('${{ github.job }}', 'perf-cost')])),
})
const setupSteps = commonSetupSteps
  .filter((step) => step !== buildSnapshotRestore && step !== buildSnapshotPrepare
    && !('uses' in step && String(step.uses).startsWith('actions/checkout@'))
    && !('name' in step && step.name === 'Fetch the whole history without evidence tags'))
  .map((step) => step === cargoCacheStep ? fromPerfCost(cargoCacheStep) : step === nixCacheStep ? fromPerfCost(nixCacheStep) : step)

const validate = `[ "$GITHUB_REF" = refs/heads/main ] || { echo "::error::burst runs only from main's workflow (got $GITHUB_REF)"; exit 1; }
sha_pattern='^[0-9a-f]{40}$' request_pattern='^[A-Za-z0-9._-]{1,64}$' args_pattern='^[A-Za-z0-9_:./=@ ,+-]*$'
[[ "$SHA" =~ $sha_pattern ]] || { echo "::error::sha must match $sha_pattern"; exit 1; }
[[ "$REQUEST" =~ $request_pattern ]] || { echo "::error::request must match $request_pattern"; exit 1; }
[[ "$ARGS" =~ $args_pattern ]] || { echo "::error::args must match $args_pattern"; exit 1; }
case "$SUITE" in
  ${suites.join('|')}) ;;
  *) echo "::error::suite must be one of: ${suites.join(' ')}"; exit 1 ;;
esac
if [ "$SUITE" = st-test-all ] && [ -n "$ARGS" ]; then echo "::error::st-test-all takes no args"; exit 1; fi`

const postStatus = (state: string, description: string) => `curl -fsS --retry 3 -X POST \\
  -H "Authorization: Bearer $GH_TOKEN" -H 'Accept: application/vnd.github+json' \\
  "$GITHUB_API_URL/repos/$GITHUB_REPOSITORY/statuses/$SHA" \\
  -d "{\\"state\\":\\"${state}\\",\\"context\\":\\"burst/$SUITE\\",\\"description\\":\\"${description}\\",\\"target_url\\":\\"$RUN_URL\\"}" > /dev/null`

// st-test-all: every whole-run check in scripts/st-test's LOCAL table at this commit, unsigned and in its order.
// Report readers (linux-clippy, linux-fleet-compat, rust-tests) read linux-gate's run; fractal-web is path-filtered
// and stays with Workspace CI. Every check runs; the first failure's exit is the suite's.
const stTestAll = `python3 - <<'BURST_ST_TEST_ALL'
import importlib.machinery, importlib.util, shlex, subprocess, sys
loader = importlib.machinery.SourceFileLoader('st_test', 'scripts/st-test')
module = importlib.util.module_from_spec(importlib.util.spec_from_loader(loader.name, loader))
loader.exec_module(module)
skip = set(module.KEYED) | {module.RUST_TESTS, module.WEB}
status = 0
for name, argv in module.LOCAL.items():
    if name in skip:
        continue
    print(f'burst: {name}: {shlex.join(argv)}', flush=True)
    code = subprocess.run(argv, stdin=subprocess.DEVNULL).returncode
    print(f'burst: {name} exit={code}', flush=True)
    status = status or code
sys.exit(status)
BURST_ST_TEST_ALL`

const runSuite = `read -r -a argv <<< "$ARGS"
case "$SUITE" in
  cargo-test) command=(nix develop -c cargo test --locked "\${argv[@]}") ;;
  nextest) command=(nix develop -c cargo nextest run --locked "\${argv[@]}") ;;
  st-test-all) command=(bash -c "$ST_TEST_ALL") ;;
esac
log="$RUNNER_TEMP/burst.log"
start=$(date +%s)
code=0
set -o pipefail
"\${command[@]}" 2>&1 | tee "$log" || code=$?
line="burst suite=$SUITE sha=$SHA request=$REQUEST exit=$code seconds=$(( $(date +%s) - start ))"
printf 'exit=%s\\nline=%s\\n' "$code" "$line" >> "$GITHUB_OUTPUT"
printf '%s\\n' "$line" >> "$GITHUB_STEP_SUMMARY"
printf '%s\\n' "$line" | tee -a "$log"`

const requestEnv = {
  SHA: '${{ inputs.sha }}',
  SUITE: '${{ inputs.suite }}',
  ARGS: '${{ inputs.args }}',
  REQUEST: '${{ inputs.request }}',
  RUN_URL: '${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}',
}

// Only the two status jobs hold `statuses: write`, and neither checks out the requested commit:
// the suite job runs that commit's code with a read-only token.
export default githubWorkflow(scopeLinuxNixSandbox(auditCaches({
  name: 'burst',
  'run-name': 'burst ${{ inputs.suite }} ${{ inputs.sha }} ${{ inputs.request }}',
  on: {
    workflow_dispatch: {
      inputs: {
        sha: { description: 'Full 40-character commit SHA to test', required: true, type: 'string' },
        suite: { description: `One of: ${suites.join(', ')}`, required: true, type: 'string' },
        args: { description: 'Extra arguments for cargo-test or nextest, split on spaces', required: false, type: 'string', default: '' },
        request: { description: 'Unique request id per dispatch ([A-Za-z0-9._-], at most 64)', required: true, type: 'string' },
      },
    },
  },
  permissions: { contents: 'read' },
  concurrency: { group: 'burst-${{ inputs.request }}', 'cancel-in-progress': false },
  actionlint: defaultActionlintConfig,
  jobs: {
    pending: {
      name: 'pending',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { statuses: 'write' },
      defaults: { run: { shell: 'bash' } },
      env: requestEnv,
      steps: [
        { name: 'Validate the request', run: validate },
        { name: 'Report the run as pending', env: { GH_TOKEN: '${{ github.token }}' }, run: postStatus('pending', 'request=$REQUEST') },
      ],
    },
    burst: {
      name: 'burst',
      needs: ['pending'],
      'runs-on': runsOn,
      'timeout-minutes': 180,
      defaults: { run: { shell: 'bash' } },
      env: { ...buildEnv, ...requestEnv },
      outputs: { exit: '${{ steps.suite.outputs.exit }}' },
      steps: [
        // st-test-all's checks read the history (genie freshness, the CI script self-tests); the other suites need one commit.
        ...fullHistoryCheckoutSteps({ ref: '${{ inputs.sha }}', 'persist-credentials': false }, {}, "inputs.suite == 'st-test-all'"),
        {
          name: 'Require the requested commit',
          run: 'head=$(git rev-parse HEAD); [ "$head" = "$SHA" ] || { echo "::error::checked out $head, not $SHA"; exit 1; }',
        },
        ...setupSteps,
        { ...megarepoApplyStep, if: "inputs.suite == 'st-test-all'" },
        { name: 'Run the suite', id: 'suite', env: { ST_TEST_ALL: stTestAll, COREPACK_ENABLE_DOWNLOAD_PROMPT: '0' }, run: runSuite },
        {
          name: 'Retain the suite log',
          if: "always() && steps.suite.outcome != 'skipped'",
          uses: 'actions/upload-artifact@v4',
          with: { name: 'burst-log-${{ inputs.request }}', path: '${{ runner.temp }}/burst.log', 'if-no-files-found': 'warn', 'retention-days': 14 },
        },
        {
          name: 'Finish with the suite exit',
          if: 'always()',
          env: { EXIT: '${{ steps.suite.outputs.exit }}', LINE: '${{ steps.suite.outputs.line }}' },
          run: `[ -n "$EXIT" ] || { echo "::error::the suite did not run"; exit 1; }
printf '%s\\n' "$LINE"
exit "$EXIT"`,
        },
      ],
    },
    // A suite job that failed before the suite, was cancelled or timed out has no exit: the status is error.
    report: {
      name: 'report',
      needs: ['pending', 'burst'],
      if: "always() && needs.pending.result == 'success'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { statuses: 'write' },
      defaults: { run: { shell: 'bash' } },
      env: requestEnv,
      steps: [{
        name: 'Report the result on the commit',
        env: { GH_TOKEN: '${{ github.token }}', EXIT: '${{ needs.burst.outputs.exit }}' },
        run: `if [ -z "$EXIT" ]; then state=error; elif [ "$EXIT" = 0 ]; then state=success; else state=failure; fi
${postStatus('$state', 'exit=${EXIT:-none} request=$REQUEST')}`,
      }],
    },
  },
}, {
  pending: 'Validates the request and posts a commit status without a checkout.',
  report: 'Posts the final commit status without a checkout.',
})))
