import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { perfLoadJobs } from './perf-load.ts'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { performanceRunner } from './workspace-ci.ts'

const load = perfLoadJobs['perf-load']
const setup = load.steps.slice(0, load.steps.findIndex(step => step.id === 'load'))

// Throwaway #6089 component measurements, not production SLO evidence.
// Head runs independently; the unsupported base harness is explicitly not measured.
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
          name: 'Compile head release once and run component and contended benches serially',
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
cd "$GITHUB_WORKSPACE"
echo "REVISION=head commit=$(git rev-parse HEAD) host=$(hostname) arch=$(uname -m) nproc=$(nproc)"
cargo test --release --locked -p fractal-missions --lib --no-run
TEST=measure_single_huge_part_rebuild
run "head-1024-p512" FRACTAL_HUGE_PART_BACKLOG=1024 FRACTAL_HUGE_PART_PAGE=512
run "head-1024-p4096" FRACTAL_HUGE_PART_BACKLOG=1024 FRACTAL_HUGE_PART_PAGE=4096
TEST=measure_huge_part_contended_mutations
run "head-contended-4096" FRACTAL_HUGE_PART_BACKLOG=4096 FRACTAL_HUGE_PART_PAGE=4096
BENCH`,
        },
        {
          name: 'Report unsupported baseline without blocking head measurements',
          if: 'always()',
          'continue-on-error': true,
          run: `echo 'BASE NOT RUN: harness needs head-only APIs' | tee -a "$GITHUB_STEP_SUMMARY"`,
        },
        {
          name: 'Summarize raw component and contended measurements',
          if: 'always()',
          run: `python3 - <<'PY'
import os
from pathlib import Path
log = Path(os.environ['RUNNER_TEMP']) / 'component-bench/results.log'
lines = [line for line in log.read_text().splitlines() if line.startswith(('huge_receive ', 'huge_receive_settle ', 'contended_mutations '))] if log.exists() else []
with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as summary:
    summary.write('## Hosted component bench (not production SLO proof)\\n\\n')
    summary.write('\\n'.join(lines) + '\\n' if lines else 'HEAD measurements NOT RUN or no measurement lines emitted; see raw log.\\n')
PY`,
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
