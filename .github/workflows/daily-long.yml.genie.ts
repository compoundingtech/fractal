import { githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'
import {
  buildEnv,
  commonSetupSteps,
  isolationVmSteps,
  linuxActionlintConfig,
  linuxRunner,
  linuxStageJob,
  linuxStageRunner,
  testArchiveConsumerSetup,
  testArchiveProducer,
} from './workspace-ci.ts'

// The long lanes, once a day on main: the ones that need a VM, real sockets across restarts or an older
// st3 (docs/ci.md, Daily long lanes). Each runs exactly as Workspace CI runs it, from the same shared
// steps, on Namespace and at the lowest job priority. There is no pick-runner (its token is a secret), no
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
} as const

const onMain = "github.ref == 'refs/heads/main'"
const consumer = { needs: ['linux-test-build'], if: `\${{ !cancelled() && ${onMain} }}` } as const

export default githubWorkflow(auditCaches({
  actionlint: linuxActionlintConfig,
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
}, { 'daily-report': 'Reads job results through the API and builds nothing.' }))
