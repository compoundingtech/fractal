import { githubWorkflow, nixDevelopStep } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'
import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import {
  buildEnv,
  commonSetupSteps,
  fractalWebRunnerStep,
  isolationVmSteps,
  linuxActionlintConfig,
  linuxRunner,
  linuxStageJob,
  linuxStageRunner,
  macosRunner,
  performanceRunner,
  perfStoresCache,
  testArchiveConsumerSetup,
  testArchiveProducer,
} from './workspace-ci.ts'
import { macosColdSetupSteps, perfLoadCachePolicy, perfLoadJobs } from './perf-load.ts'

// The long lanes, once a day on main: the ones that need a VM, real sockets across restarts or an older
// st3, the cost check, the load tests, and the whole test selection at full scale (docs/ci.md, Daily long lanes). Each runs exactly as Workspace CI (the cost check: Main upkeep; the load tests: Performance) runs it, from the same shared
// steps, on Namespace (the macOS hog run and the macOS compile check: a Namespace Mac) and at the lowest job priority. There is no pick-runner (its token is a secret), no
// evidence signing and no cache write the warm-up jobs own. Job names carry a `daily-` prefix so no check
// on main ever looks like a required check. The report job writes each lane's result, time and failed
// steps, and on red opens, or comments on, the one open issue with the title below.
// Nothing here is a required check and nothing here runs on a pull request or a merge group.
const title = 'Daily long-lane run failed'

// Job id -> check name. The ids keep Workspace CI's, so the warm caches keyed by `github.job` match.
const lanes = {
  'linux-test-build': 'daily-test-build',
  'isolation-vm': 'daily-isolation-vm',
  'mail-redelivery-canaries': 'daily-mail-redelivery-canaries',
  'linux-fleet-compat': 'daily-fleet-compat',
  'daily-tests': 'daily-tests',
  'daily-tests-shard-2': 'daily-tests-shard-2',
  'perf-cost': 'daily-perf-cost',
  'perf-load': 'daily-perf-load',
  'perf-load-hog': 'daily-perf-load-hog',
  'perf-load-hog-macos': 'daily-perf-load-hog-macos',
  'daily-macos-check': 'daily-macos-check',
} as const

const onMain = "github.ref == 'refs/heads/main'"
const consumer = { needs: ['linux-test-build'], if: `\${{ !cancelled() && ${onMain} }}` } as const

// The whole selection Workspace CI's shards run, at full scale: tests that keep a smaller size or fewer
// proptest cases on pull requests read ST_TEST_SCALE=full and run their original size here.
const fullScaleShard = (shard: 1 | 2) => ({
  ...linuxStageJob({
    name: lanes[shard === 1 ? 'daily-tests' : 'daily-tests-shard-2'],
    stage: 'tests',
    description: 'Run nextest at full scale',
    setup: testArchiveConsumerSetup,
    cacheSaveSetup: [],
    env: { CI_RUN_ID: '${{ github.run_id }}', CI_TEST_PARTITION: `hash:${shard}/2`, CI_TEST_THREADS: '8', ST_TEST_SCALE: 'full' },
    extraLogs: 'target/messaging-faults/\ntarget/boot-canaries/',
  }),
  ...consumer,
})

// No daily lane enables user namespaces, so none may keep the strict Linux Nix sandbox (nix-sandbox-scope.ts).
export default githubWorkflow(scopeLinuxNixSandbox(auditCaches({
  actionlint: {
    ...linuxActionlintConfig,
    selfHostedRunnerLabels: [...linuxActionlintConfig.selfHostedRunnerLabels, ...performanceRunner, ...macosRunner],
  },
  name: 'Daily long lanes',
  on: {
    schedule: [{ cron: '7 5 * * *' }],
    workflow_dispatch: {},
  },
  // actions: read lets the lanes read this run's test archives and build snapshots.
  permissions: { contents: 'read', actions: 'read' },
  concurrency: { group: 'daily-long', 'cancel-in-progress': false },
  jobs: {
    'linux-test-build': {
      name: lanes['linux-test-build'],
      if: onMain,
      'runs-on': linuxStageRunner,
      'timeout-minutes': 120,
      ...testArchiveProducer,
    },
    'isolation-vm': {
      name: lanes['isolation-vm'],
      ...consumer,
      'runs-on': linuxRunner,
      'timeout-minutes': 60,
      defaults: { run: { shell: 'bash' } },
      env: { ...buildEnv, CI_CACHE_DEV_SHELL: 'default' },
      steps: [
        ...isolationVmSteps,
        {
          name: 'Retain the VM output',
          uses: 'actions/upload-artifact@v4',
          if: 'always()',
          with: { name: 'daily-isolation-vm-logs', path: '${{ runner.temp }}/vm-out/\n${{ runner.temp }}/sekrets-vm-out/', 'if-no-files-found': 'ignore' },
        },
      ],
    },
    'mail-redelivery-canaries': {
      ...linuxStageJob({
        name: lanes['mail-redelivery-canaries'],
        stage: 'mail-redelivery-canaries',
        description: 'Require every harness to hold old mail across boot and reconnect',
        setup: testArchiveConsumerSetup,
        cacheSaveSetup: [],
        env: { CI_RUN_ID: '${{ github.run_id }}', CI_TEST_THREADS: '8' },
        extraLogs: 'target/messaging-faults/\ntarget/boot-canaries/',
      }),
      ...consumer,
    },
    'daily-tests': fullScaleShard(1),
    'daily-tests-shard-2': fullScaleShard(2),
    'linux-fleet-compat': {
      ...linuxStageJob({
        name: lanes['linux-fleet-compat'],
        stage: 'fleet-compat',
        description: 'Run fleet compatibility against the pinned older st3',
        setup: commonSetupSteps,
        cacheSaveSetup: [],
      }),
      if: onMain,
    },
    // The cost check (scripts/ci-perf cost, with the owned link_minutes quarantine inside it), as Main
    // upkeep runs it on each push to main. It left pull requests: about 40% of a PR run's compute for an
    // optional lane. Counts, not timings, so a lightly optimized build only speeds up store generation.
    'perf-cost': {
      ...linuxStageJob({
        name: lanes['perf-cost'],
        stage: 'cost',
        description: 'Run the cost check',
        setup: commonSetupSteps,
        cacheSaveSetup: [],
        command: ['bash', 'scripts/ci-perf', 'cost'],
        env: { CARGO_PROFILE_DEV_OPT_LEVEL: '1' },
        extraLogs: '${{ runner.temp }}/perf/',
        before: [fractalWebRunnerStep, perfStoresCache('cost')],
      }),
      if: onMain,
    },
    // The load tests (perf-load.ts), as the Performance workflow runs them by dispatch. They ran on every
    // push to main, where newer pushes cancelled nearly every run, so no result reached anyone; here each
    // completes once a day and a red one reaches the daily report and its issue. Main's daily perf-load
    // publishes the build and store snapshots and the baseline reports scripts/ci-perf-cache restores.
    ...Object.fromEntries((['perf-load', 'perf-load-hog', 'perf-load-hog-macos'] as const).map((job) =>
      [job, { name: lanes[job], if: onMain, ...perfLoadJobs[job] }])),
    // The whole workspace, tests included, type-checked on a Mac: macOS CI no longer runs on each push,
    // so this is where a Linux-only item a macOS build still reaches shows up, the same day. The same
    // targets and flags as clippy on Linux and macOS (--workspace --all-targets --locked; dev-dependencies
    // turn on the test-support features). It runs no tests and builds cold, keeping no cache.
    'daily-macos-check': {
      name: lanes['daily-macos-check'],
      if: onMain,
      'runs-on': macosRunner,
      'timeout-minutes': 45,
      defaults: { run: { shell: 'bash' } },
      env: buildEnv,
      steps: [
        ...macosColdSetupSteps,
        nixDevelopStep({ name: 'Type-check the workspace and its tests on macOS', command: ['cargo', 'check', '--workspace', '--all-targets', '--locked'] }),
      ],
    },
    'daily-report': {
      name: 'daily-report',
      needs: Object.keys(lanes),
      if: `\${{ always() && ${onMain} }}`,
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read', actions: 'read', issues: 'write' },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        {
          name: 'Report lane times and failures',
          env: {
            GH_TOKEN: '${{ github.token }}',
            GH_REPO: '${{ github.repository }}',
            TITLE: title,
            LANES: JSON.stringify(lanes),
            NEEDS_JSON: '${{ toJSON(needs) }}',
          },
          run: 'python3 scripts/ci-daily-long-report',
        },
      ],
    },
  },
}, {
  ...perfLoadCachePolicy,
  'daily-macos-check': 'Type-checks cold on a macOS runner and keeps no cache.',
  'daily-report': 'Reads job results through the API and builds nothing.',
})))
