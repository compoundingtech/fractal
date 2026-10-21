import { nixDevelopStep, plainFlakeSetupSteps } from '../../repos/effect-utils/genie/external.ts'
import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { buildEnv, fractalWebRunnerStep, fullHistoryCheckoutSteps, macosRunner, performanceRunner, readOnlyBinaryCaches } from './workspace-ci.ts'

// The load jobs, defined once. The daily long lanes run them on main (daily-perf-load, daily-perf-load-hog,
// daily-perf-load-hog-macos), and the Performance workflow (perf.yml) runs them by manual dispatch on any
// ref. Each workflow adds only its own check name and its own `needs`/`if`. The artifact names stay the same
// in both, since scripts/ci-perf-cache reads main's `perf-load-logs` reports and snapshots from either.

const snapshotAttempt = "!cancelled() && (github.event_name == 'pull_request' || github.ref == 'refs/heads/main') && (steps.load.outcome == 'success' || steps.load.outcome == 'failure')"
const snapshotPublished = "!cancelled() && steps.cache.outcome == 'success' && steps.cache.outputs.publish == 'true'"

// The checkout, snapshots, Nix and scratch space every Linux load job starts from. The first step logs the
// runner's CPU model: Namespace's 8x16 label serves two host classes, one about 2x slower, and the job
// header cannot tell them apart.
const loadSetupSteps = [
  fractalWebRunnerStep,
  ...fullHistoryCheckoutSteps({ 'persist-credentials': false }),
  {
    name: 'Name compatible snapshot recipes',
    env: {
      PERF_BUILD_SNAPSHOT: "perf-load-build-v1-${{ hashFiles('Cargo.lock', 'flake.lock', 'flake.nix', 'Cargo.toml', 'crates/fractal/Cargo.toml', 'crates/fractal-daemon/Cargo.toml', '.cargo/config.toml') }}",
      PERF_STORES_SNAPSHOT: "perf-load-stores-v1-${{ hashFiles('crates/fractal/tests/daemon_bench.rs', 'docs/st3/schema.md') }}",
    },
    run: 'printf "PERF_BUILD_SNAPSHOT=%s\\nPERF_STORES_SNAPSHOT=%s\\n" "$PERF_BUILD_SNAPSHOT" "$PERF_STORES_SNAPSHOT" >> "$GITHUB_ENV"',
  },
  {
    name: 'Restore compatible build, Nix, stores and main baseline snapshots',
    env: { GH_TOKEN: '${{ github.token }}' },
    run: 'python3 scripts/ci-perf-cache restore',
  },
  ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
  {
    name: 'Isolate test state and keep the compiler cache',
    run: `home="$RUNNER_TEMP/test-home"
mkdir -p "$home"/{.config,.cache,.local/state} "$RUNNER_TEMP/cargo-home"/{registry,git} "$RUNNER_TEMP/perf-sccache"
printf 'CARGO_HOME=%s\\nCI_CACHE_DIR=%s\\nSCCACHE_DIR=%s\\nSCCACHE_CACHE_SIZE=1G\\n' "$RUNNER_TEMP/cargo-home" "$RUNNER_TEMP/st-ci-cache" "$RUNNER_TEMP/perf-sccache" >> "$GITHUB_ENV"
printf 'HOME=%s\\nXDG_CONFIG_HOME=%s/.config\\nXDG_CACHE_HOME=%s/.cache\\nXDG_STATE_HOME=%s/.local/state\\n' "$home" "$home" "$home" "$home" >> "$GITHUB_ENV"`,
  },
  { name: 'Use the cached Nix outputs', run: 'bash scripts/ci-nix-cache use' },
  {
    name: 'Prepare fast scratch space for a new store recipe',
    // The generator already prefers RAM when enough is available. Its source stores are
    // copied to RUNNER_TEMP before measuring: measurements still use the normal disk.
    run: 'if [ ! -s "$RUNNER_TEMP/st-bench/generated-1.sqlite3" ]; then sudo mount -o remount,size=10G /dev/shm; fi',
  },
]

// The checkout and Nix setup of a cold macOS job. The daily macOS compile check (daily-long.yml) starts
// from the same steps as the macOS hog run.
export const macosColdSetupSteps = [
  ...fullHistoryCheckoutSteps({ 'persist-credentials': false }),
  ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
]

const loadEnv = {
  ...buildEnv,
  SCCACHE_IDLE_TIMEOUT: '0',
  PERF_PR_NUMBER: '${{ github.event.pull_request.number }}',
  PERF_HEAD_REPOSITORY: '${{ github.event.pull_request.head.repo.full_name }}',
}

/** Job id -> job, without a check name. Ids are the same in both workflows. */
export const perfLoadJobs = scopeLinuxNixSandbox({
  jobs: {
    // The release load test against main's last five reports; main's runs also publish the build and
    // store snapshots every later run restores. They publish only after a complete workload report, so a
    // run on a new recipe must finish cold to seed it: run 39488050274 compiled for 10m38s and generated
    // stores for 966s, and its 30-minute limit cancelled the measurement before anything was saved.
    // 60 minutes leaves that cold run room in both workflows.
    'perf-load': {
      'runs-on': performanceRunner,
      'timeout-minutes': 60,
      defaults: { run: { shell: 'bash' } },
      env: loadEnv,
      steps: [
        ...loadSetupSteps,
        { ...nixDevelopStep({ name: 'Run the release load test', flake: '.#perf', command: ['bash', 'scripts/ci-perf', 'load'] }), id: 'load' },
        {
          id: 'cache',
          name: 'Save build and Nix snapshots',
          if: snapshotAttempt,
          run: `if ! python3 scripts/ci-perf-cache check-report; then exit 0; fi
nix print-dev-env .#perf --profile "$RUNNER_TEMP/perf-shell" > /dev/null
nix develop .#perf -c env TMPDIR="$RUNNER_TEMP" sccache --show-stats || echo "::warning::sccache statistics unavailable; retaining the successful build"
CI_PERF_SHELL_ROOT="$RUNNER_TEMP/perf-shell" bash scripts/ci-nix-cache save
python3 scripts/ci-perf-cache pack`,
        },
        {
          name: 'Retain the build and Nix cache',
          if: snapshotPublished,
          uses: 'actions/upload-artifact@v4',
          with: { name: '${{ env.PERF_BUILD_SNAPSHOT }}', path: '${{ runner.temp }}/perf-snapshots/build.tar.zst', 'compression-level': 0, 'retention-days': 7, 'if-no-files-found': 'error', overwrite: true },
        },
        {
          name: 'Retain the generated stores',
          if: snapshotPublished,
          uses: 'actions/upload-artifact@v4',
          with: { name: '${{ env.PERF_STORES_SNAPSHOT }}', path: '${{ runner.temp }}/perf-snapshots/stores.tar.zst', 'compression-level': 0, 'retention-days': 7, 'if-no-files-found': 'error', overwrite: true },
        },
        {
          name: 'Retain the load report, log and timing',
          uses: 'actions/upload-artifact@v4',
          if: 'always()',
          with: { name: 'perf-load-logs', path: '${{ runner.temp }}/perf/', 'retention-days': 30, 'if-no-files-found': 'ignore', overwrite: true },
        },
      ],
    },
    // The same daemon beside a CPU and disk hog at seat priority, reported against slo/targets.toml. A missed
    // target only reports (until ST_LOAD_HOG_STRICT=1); a failed request or a hog that never loaded the
    // machine fails the job, and a red daily lane alerts. It takes a second performance runner for up to 45
    // minutes, so it runs once a day, not on pull requests.
    'perf-load-hog': {
      'runs-on': performanceRunner,
      'timeout-minutes': 45,
      defaults: { run: { shell: 'bash' } },
      env: loadEnv,
      steps: [
        ...loadSetupSteps,
        { ...nixDevelopStep({ name: 'Run the release load test beside a seat-priority hog', flake: '.#perf', command: ['bash', 'scripts/ci-perf', 'load-hog'] }), id: 'load' },
        {
          name: 'Retain the hog load report, log and timing',
          uses: 'actions/upload-artifact@v4',
          if: 'always()',
          with: { name: 'perf-load-hog-logs', path: '${{ runner.temp }}/perf/', 'retention-days': 30, 'if-no-files-found': 'ignore', overwrite: true },
        },
      ],
    },
    // The same hog run on a macOS runner, under `taskpolicy -b`: the hog's priority on a Mac. It also
    // keeps the compiled test binary, which is what runs on a fleet Mac (builds happen on hetz2 and
    // CI only). It builds cold and keeps no cache.
    'perf-load-hog-macos': {
      'runs-on': macosRunner,
      'timeout-minutes': 60,
      defaults: { run: { shell: 'bash' } },
      env: { ...buildEnv, SCCACHE_IDLE_TIMEOUT: '0' },
      steps: [
        ...macosColdSetupSteps,
        {
          name: 'Isolate test state',
          run: `home="$RUNNER_TEMP/test-home"
mkdir -p "$home"/{.config,.cache,.local/state}
printf 'HOME=%s\\nXDG_CONFIG_HOME=%s/.config\\nXDG_CACHE_HOME=%s/.cache\\nXDG_STATE_HOME=%s/.local/state\\n' "$home" "$home" "$home" "$home" >> "$GITHUB_ENV"`,
        },
        nixDevelopStep({ name: 'Run the release load test beside a seat-priority hog', flake: '.#perf', command: ['bash', 'scripts/ci-perf', 'load-hog'] }),
        {
          name: 'Keep the compiled test binary',
          if: 'always()',
          run: `mkdir -p "$RUNNER_TEMP/perf"
binary=$(find target/release/deps -maxdepth 1 -type f -name 'perf_load-*' ! -name '*.d' -perm -u+x | head -n 1)
[ -n "$binary" ] && cp "$binary" "$RUNNER_TEMP/perf/perf_load-macos-arm64" || echo "no perf_load binary was built"`,
        },
        {
          name: 'Retain the macOS hog report, log and test binary',
          uses: 'actions/upload-artifact@v4',
          if: 'always()',
          with: { name: 'perf-load-hog-macos', path: '${{ runner.temp }}/perf/', 'retention-days': 30, 'if-no-files-found': 'ignore', overwrite: true },
        },
      ],
    },
  },
}).jobs

/** Why the daily lanes' cache audit finds no Actions cache step in these jobs. */
export const perfLoadCachePolicy = {
  'perf-load': 'Restores and publishes its own build and store snapshots as Performance artifacts (scripts/ci-perf-cache), outside the dependency-cache pool.',
  'perf-load-hog': 'Restores the build and store snapshots perf-load publishes (scripts/ci-perf-cache); it saves none.',
  'perf-load-hog-macos': 'Builds cold on a macOS runner and keeps no cache.',
} as const
