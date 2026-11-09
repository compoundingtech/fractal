import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { perfLoadJobs } from './perf-load.ts'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { performanceRunner } from './workspace-ci.ts'

const load = perfLoadJobs['perf-load']
const setup = load.steps.slice(0, load.steps.findIndex(step => step.id === 'load'))

// Throwaway #6089 component measurements, not production SLO evidence.
// Base predates the harness: transplant only its test module and dependency manifests.
export default githubWorkflow(scopeLinuxNixSandbox({
  name: 'Performance',
  on: { workflow_dispatch: {} },
  permissions: { contents: 'read', actions: 'read', 'pull-requests': 'read' },
  concurrency: { group: 'perf-${{ github.run_id }}', 'cancel-in-progress': false },
  actionlint: {
    ...defaultActionlintConfig,
    selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), ...performanceRunner],
  },
  jobs: {
    'component-bench': {
      name: 'CI Manager hosted component bench (not SLO proof)',
      'runs-on': performanceRunner,
      'timeout-minutes': 120,
      defaults: { run: { shell: 'bash' } },
      env: load.env,
      steps: [
        ...setup,
        {
          name: 'Prepare baseline worktree with the identical bench harness',
          run: `set -euo pipefail
base="$RUNNER_TEMP/component-base"
git worktree add --detach "$base" d8532489
for path in Cargo.lock crates/fractal-missions/Cargo.toml crates/fractal-missions/src/store/tests/huge_part_rebuild_bench.rs; do
  mkdir -p "$base/$(dirname "$path")"
  git show "092555e74d35bf761ea5aa78e7fa281cf2af4ce1:$path" > "$base/$path"
done
printf '\\nmod huge_part_rebuild_bench;\\n' >> "$base/crates/fractal-missions/src/store/tests.rs"`,
        },
        {
          name: 'Compile release once per revision and run base then head serially',
          run: `set -euo pipefail
mkdir -p "$RUNNER_TEMP/component-bench"
nix develop '.#perf' -c bash -euo pipefail <<'BENCH' 2>&1 | tee "$RUNNER_TEMP/component-bench/results.log"
export RUSTC_WRAPPER= CI=1 FRACTAL_HUGE_PART_CHILDREN=64 FRACTAL_HUGE_PART_STEPS=32 FRACTAL_HUGE_PART_GENERATIONS=1 FRACTAL_HUGE_PART_HISTORY=8 FRACTAL_HUGE_PART_SWEEP=0
export CARGO_TARGET_DIR="$GITHUB_WORKSPACE/target"
T=store::tests::huge_part_rebuild_bench
run() {
  local label=$1; shift
  echo "== $label START $(date -u +%TZ)"
  env "$@" cargo test --release --locked -p fractal-missions --lib "$T::$TEST" -- --exact --ignored --nocapture --test-threads=1
  echo "== $label END $(date -u +%TZ)"
}
for revision in base head; do
  if [ "$revision" = base ]; then cd "$RUNNER_TEMP/component-base"; else cd "$GITHUB_WORKSPACE"; fi
  echo "REVISION=$revision commit=$(git rev-parse HEAD) host=$(hostname) arch=$(uname -m) nproc=$(nproc)"
  cargo test --release --locked -p fractal-missions --lib --no-run
  TEST=measure_single_huge_part_rebuild
  run "$revision-1024-p512" FRACTAL_HUGE_PART_BACKLOG=1024 FRACTAL_HUGE_PART_PAGE=512
  run "$revision-1024-p4096" FRACTAL_HUGE_PART_BACKLOG=1024 FRACTAL_HUGE_PART_PAGE=4096
  TEST=measure_huge_part_contended_mutations
  run "$revision-contended-4096" FRACTAL_HUGE_PART_BACKLOG=4096 FRACTAL_HUGE_PART_PAGE=4096
done
BENCH`,
        },
        {
          name: 'Retain component and contended measurements',
          uses: 'actions/upload-artifact@v4',
          if: 'always()',
          with: { name: '6089-component-bench', path: '${{ runner.temp }}/component-bench/', 'retention-days': 7, 'if-no-files-found': 'ignore' },
        },
      ],
    },
  },
}))
