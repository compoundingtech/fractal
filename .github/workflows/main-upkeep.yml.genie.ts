import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { buildSnapshotPrepare, buildSnapshotRestore, buildSnapshotSave } from './build-snapshot.ts'
import { auditCaches } from './cache-audit.ts'
import { buildFleetWorkflow } from './fleet.yml.genie.ts'
import { defaultActionlintConfig, githubWorkflow, nixDevelopStep, plainFlakeSetupSteps } from '../../repos/effect-utils/genie/external.ts'
import {
  buildEnv,
  cargoCacheStep,
  commonSetupSteps,
  megarepoApplyStep,
  fractalWebShellCache,
  fractalWebShellEnv,
  fractalWebShellKeyStep,
  fractalWebRunnerStep,
  fractalWebStoreCache,
  genieNixCacheStep,
  genieSetupSteps,
  linuxRunner,
  linuxStageJob,
  linuxStageRunner,
  nixCacheStep,
  perfStoresCache,
  pnpmStoreEnv,
  readOnlyBinaryCaches,
  workspacePreparationSteps,
} from './workspace-ci.ts'

const proofId = (id: string) => `proof-${id}`
const proofSelected = "github.event_name == 'push' && github.ref == 'refs/heads/main' && needs.main-checks.outputs.evidence-skipped == 'true'"
const fleetWorkflow = buildFleetWorkflow({ topology: 'merge-group' })
// Queue-ref cancellation and PR-only advice cannot apply to a landed main commit.
const proofOmissions: Record<string, string> = {
  'namespace-capacity': 'The ephemeral queue ref no longer controls landed proof.',
  'evidence-presign-notice': 'Pre-signing advice applies only to an open PR.',
}
// Preserve fleet's dependency identities inside aggregate evidence JSON, while namespacing actual jobs.
export const proofJobs = Object.fromEntries(Object.entries(fleetWorkflow.jobs).filter(([id]) => !Object.hasOwn(proofOmissions, id)).map(([id, job]) => {
  const dependencies: string[] = job.needs == null ? [] : typeof job.needs === 'string' ? [job.needs] : job.needs
  const jsonFormat = `{{${dependencies.map((name, i) => `"${name}":{${i}}`).join(',')}}}`
  const needsJson = dependencies.length
    ? `format('${jsonFormat}', ${dependencies.map(name => `toJSON(needs.${proofId(name)})`).join(', ')})`
    : "'{}'"
  const expression = (text: string) => text.replaceAll('toJSON(needs)', needsJson)
    .replace(/\bneeds(?:\.([\w-]+)|\[['"]([\w-]+)['"]\])/g, (whole, dot, bracket) => {
      const name = dot ?? bracket
      return Object.hasOwn(fleetWorkflow.jobs, name) ? `needs.${proofId(name)}` : whole
    }).replaceAll('github.job', `'${id}'`)
  const rewrite = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(rewrite)
    if (value !== null && typeof value === 'object')
      return Object.fromEntries(Object.entries(value).map(([key, entry]) =>
        [key, key === 'if' && typeof entry === 'string' ? expression(entry) : rewrite(entry)]))
    if (typeof value !== 'string') return value
    return value.replace(/\$\{\{([\s\S]*?)\}\}/g, (_, text: string) => '${{' + expression(text) + '}}')
  }
  const projected = rewrite(job) as Record<string, unknown>
  const condition = String(projected.if ?? 'success()').replace(/\$\{\{|\}\}/g, '')
  return [proofId(id), { ...projected, name: `post-merge ${job.name ?? id}`,
    needs: ['main-checks', ...dependencies.map(proofId)], if: `\${{ ${proofSelected} && (${condition}) }}` }]
}))

const missing = "steps.cargo-probe.outputs.cache-hit != 'true' || steps.nix-probe.outputs.cache-hit != 'true'"
const whenMissing = (steps: readonly unknown[], conditionMissing = missing) => steps.map((value) => {
  const step = value as Record<string, unknown>
  const condition = String(step.if ?? 'success()').replace(/\$\{\{|\}\}/g, '')
  return { ...step, if: `success() && (${conditionMissing}) && (${condition})` }
})

// Cache scope includes the ref: merge-group saves cannot replace main's cache saves. Probe main's
// exact entries before provisioning Nix or restoring gigabytes; fill only missing entries.
const warmJob = (stage: string, setup: readonly unknown[], nixOnly = false) => ({
  name: `warm-${stage}`,
  'runs-on': stage === 'genie' ? 'ubuntu-latest' : linuxStageRunner,
  'timeout-minutes': 120,
  defaults: { run: { shell: 'bash' } },
  env: { ...buildEnv, CI_CACHE_DEV_SHELL: stage === 'genie' ? 'genie' : 'default' },
  steps: [
    { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
    ...(nixOnly ? [genieNixCacheStep] : [cargoCacheStep, nixCacheStep]).map((step) => ({
      ...step,
      if: 'success()',
      id: step.id === 'cargo-cache' ? 'cargo-probe' : 'nix-probe',
      uses: 'actions/cache/restore@v4',
      with: { ...step.with, 'restore-keys': '', 'lookup-only': true },
    })),
    {
      name: 'Report cache coverage',
      env: { CARGO_HIT: nixOnly ? 'not used' : '${{ steps.cargo-probe.outputs.cache-hit }}', NIX_HIT: '${{ steps.nix-probe.outputs.cache-hit }}' },
      run: `printf 'Main cache coverage: Cargo=%s, Nix=%s\\n' "$CARGO_HIT" "$NIX_HIT" >> "$GITHUB_STEP_SUMMARY"
if ${nixOnly ? '[ \"$NIX_HIT\" != true ]' : '[ \"$CARGO_HIT\" != true ] || [ \"$NIX_HIT\" != true ]'}; then
  echo "::warning::P0: missing main-scope cache; filling it for Namespace overflow and pull requests"
fi`,
    },
    ...whenMissing([
      ...(stage === 'genie' ? setup.filter((step) => step !== buildSnapshotRestore && step !== buildSnapshotPrepare) : setup),
      ...(stage === 'genie' ? [megarepoApplyStep] : []),
      nixDevelopStep({ name: 'Build missing cache contents', flake: stage === 'genie' ? '.#genie' : '.', command: ['bash', 'scripts/ci-cache-warm', stage] }),
      { name: 'Save Nix outputs', run: 'bash scripts/ci-nix-cache save' },
      // PR/merge builds restore these entries and retain exact-source artifacts. Only
      // protected main fills the shared dependency quota, including after eviction.
      ...(nixOnly ? [genieNixCacheStep] : [cargoCacheStep, nixCacheStep]).map((step) => ({
        name: `Save the missing main ${step.id}`,
        if: `github.ref == 'refs/heads/main' && steps.${step.id === 'cargo-cache' ? 'cargo-probe' : 'nix-probe'}.outputs.cache-hit != 'true'`,
        uses: 'actions/cache/save@v4',
        with: { path: step.with.path, key: step.with.key },
      })),
      ...(stage === 'genie' ? [] : buildSnapshotSave),
    ], nixOnly ? "steps.nix-probe.outputs.cache-hit != 'true'" : missing),
  ],
})

const upkeepWorkflow = scopeLinuxNixSandbox(auditCaches({
  name: 'Main upkeep',
  on: { push: { branches: ['main'] }, workflow_dispatch: {} },
  permissions: { contents: 'read', actions: 'read' },
  // Every landed tree that skipped queue work needs hosted proof; never coalesce pending main pushes.
  concurrency: {
    group: 'main-upkeep-${{ github.run_id }}',
    'cancel-in-progress': false,
  },
  actionlint: {
    ...defaultActionlintConfig,
    selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), ...linuxRunner, ...linuxStageRunner],
  },
  jobs: {
    'main-checks': {
      name: 'main-checks',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read', actions: 'read', checks: 'read' },
      outputs: { 'evidence-skipped': '${{ steps.queue.outputs.evidence-skipped }}' },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        { name: 'Confirm main retains the queue checks', id: 'queue', env: { GH_TOKEN: '${{ github.token }}' }, run: 'python3 scripts/check-main-ci' },
      ],
    },
    'hosted-proof-failure': {
      name: 'report red main',
      needs: Object.keys(proofJobs),
      if: "always() && (contains(needs.*.result, 'failure') || contains(needs.*.result, 'cancelled'))",
      'runs-on': 'ubuntu-latest',
      permissions: {},
      steps: [{
        name: 'Signal revert needed',
        run: `echo "::error::Post-merge hosted proof failed for $GITHUB_SHA: main is red; revert the merged change."
echo "## Revert needed: hosted proof failed for $GITHUB_SHA" >> "$GITHUB_STEP_SUMMARY"
exit 1`,
      }],
    },
    'cache-maintenance': {
      name: 'cache-maintenance',
      needs: ['main-checks'],
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read', actions: 'write', 'pull-requests': 'read' },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        { name: 'Prune closed-PR and redundant Zig caches', env: { GH_TOKEN: '${{ github.token }}' }, run: 'python3 scripts/ci-cache-prune' },
      ],
    },
    // Keep these IDs: commonSetupSteps keys the existing caches with github.job.
    'linux-tests': warmJob('tests', workspacePreparationSteps),
    'linux-clippy': warmJob('clippy', commonSetupSteps),
    'linux-fleet-compat': warmJob('fleet-compat', commonSetupSteps),
    'genie-freshness': warmJob('genie', genieSetupSteps, true),
    'isolation-vm': warmJob('isolation-vm', commonSetupSteps),
    'typescript-cache': {
      name: 'warm-typescript',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 10,
      env: { COREPACK_ENABLE_DOWNLOAD_PROMPT: '0' },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        // As Workspace CI's typescript-client: pnpm reads workspace packages from the megarepo members.
        ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
        megarepoApplyStep,
        { uses: 'actions/setup-node@v4', with: { 'node-version': '24.18.0' } },
        {
          name: 'Fingerprint locked dependencies', id: 'lockfiles',
          run: `lockfiles_hash=$(sha256sum pnpm-lock.yaml | cut -d ' ' -f1)
printf 'hash=%s\\n' "$lockfiles_hash" >> "$GITHUB_OUTPUT"`,
        },
        {
          name: 'Cache locked TypeScript dependencies', id: 'typescript-cache', uses: 'actions/cache@v5',
          with: {
            path: pnpmStoreEnv.pnpm_config_store_dir,
            key: 'typescript-client-${{ runner.os }}-node24.18.0-pnpm12.15.0-${{ steps.lockfiles.outputs.hash }}',
          },
        },
        {
          ...nixDevelopStep({ name: 'Fill missing dependencies', flake: '.#web', command: ['pnpm', 'fetch'] }),
          if: "steps.typescript-cache.outputs.cache-hit != 'true'", env: pnpmStoreEnv,
        },
      ],
    },
    // Fractal-web runs on GitHub-hosted capacity, whose cache is separate from Namespace's.
    'fractal-web-cache': {
      name: 'warm-fractal-web',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 15,
      defaults: { run: { shell: 'bash' } },
      steps: [
        fractalWebRunnerStep,
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
        megarepoApplyStep,
        fractalWebShellKeyStep,
        { name: 'Probe the pnpm store', id: 'pnpm-store', uses: 'actions/cache/restore@v4', with: { ...fractalWebStoreCache, 'lookup-only': true } },
        { name: 'Restore the web shell closure', id: 'web-shell', uses: 'actions/cache/restore@v4', with: fractalWebShellCache },
        {
          name: 'Verify web key completeness against the shell derivation',
          id: 'web-shell-verify',
          shell: 'bash',
          run: `if bash scripts/ci-web-shell verify; then
  exit 0
else
  status=$?
  if [ "$status" -ne 2 ]; then exit "$status"; fi
fi`,
          env: { ...fractalWebShellEnv, CI_WEB_SHELL_CACHE_HIT: '${{ steps.web-shell.outputs.cache-hit }}' },
        },
        ...[
          { ...nixDevelopStep({ name: 'Fetch the locked packages', flake: '.#web', command: ['pnpm', 'fetch'] }), env: pnpmStoreEnv },
          { name: 'Save the pnpm store', uses: 'actions/cache/save@v4', with: fractalWebStoreCache },
        ].map((step) => ({ ...step, if: "steps.pnpm-store.outputs.cache-hit != 'true'" })),
        ...[
          {
            name: 'Fill the web shell closure',
            shell: 'bash',
            run: 'bash scripts/ci-web-shell fill',
            env: { ...fractalWebShellEnv, CI_WEB_SHELL_IDENTITY: '${{ steps.web-shell-verify.outputs.identity }}' },
          },
          { name: 'Save the web shell closure', uses: 'actions/cache/save@v4', with: fractalWebShellCache },
        ].map((step) => ({ ...step, if: "steps.web-shell-verify.outputs.cache-complete != 'true'" })),
      ],
    },
    // This check never ran in merge_group; keep its main measurements and generated-store cache.
    'perf-cost': linuxStageJob({
      name: 'perf-cost', stage: 'cost', setup: commonSetupSteps,
      description: 'Run the cost check', command: ['bash', 'scripts/ci-perf', 'cost'],
      env: { CARGO_PROFILE_DEV_OPT_LEVEL: '1' }, extraLogs: '${{ runner.temp }}/perf/',
      before: [perfStoresCache('cost')],
    }),
  },
}, {"main-checks": "Verifies exact queue checks through the API and builds nothing.", "hosted-proof-failure": "Reports a failed hosted proof and builds nothing.", "cache-maintenance": "Cache maintenance uses live API data and builds nothing."}))

export default githubWorkflow({ ...upkeepWorkflow, jobs: { ...upkeepWorkflow.jobs, ...proofJobs } })
