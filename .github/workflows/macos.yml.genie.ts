import { withCompatArchives } from './compat-archive.ts'
import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { readyPRJobs, readyPullRequestTypes } from './ready-pr.ts'
import { buildSnapshotSave } from './build-snapshot.ts'
import { auditCaches } from './cache-audit.ts'
import {
  defaultActionlintConfig,
  githubWorkflow,
  nixDevelopStep,
  plainFlakeSetupSteps,
} from '../../repos/effect-utils/genie/external.ts'
import { buildEnv, macosRunner, readOnlyBinaryCaches, saveMainDependencyCaches, workspacePreparationSteps } from './workspace-ci.ts'

const publisherOnly = "github.event_name == 'push' && github.ref == 'refs/heads/main' && github.repository_owner == 'compoundingtech' && github.event.repository.fork == false"

// Optional on PRs and never required. A separate workflow lets label changes start macOS without
// restarting the required Linux gate.
const workflow = scopeLinuxNixSandbox(readyPRJobs(withCompatArchives(auditCaches({
  name: 'macOS CI',
  on: {
    pull_request: { types: [...readyPullRequestTypes, 'labeled'] },
    push: { branches: ['main'] },
  },
  permissions: { contents: 'read', actions: 'read' },
  concurrency: {
    // Main pushes run independently, including while earlier runs are still pending.
    group: 'macos-${{ github.event.pull_request.number || github.run_id }}',
    'cancel-in-progress': '${{ github.event_name == \'pull_request\' }}',
  },
  actionlint: defaultActionlintConfig,
  jobs: {
    'ios-rust-toolchain-cache': {
      name: 'Publish iOS Rust toolchain cache',
      if: publisherOnly,
      'runs-on': macosRunner,
      'timeout-minutes': 60,
      permissions: { contents: 'read' },
      defaults: { run: { shell: 'bash' } },
      steps: [
        {
          name: 'Check cache publishing credentials',
          id: 'credentials',
          env: {
            FRACTAL_CACHIX_CACHE: '${{ vars.FRACTAL_CACHIX_CACHE }}',
            CACHIX_AUTH_TOKEN: '${{ secrets.CACHIX_AUTH_TOKEN }}',
          },
          run: `if [[ -n "$FRACTAL_CACHIX_CACHE" && -n "$CACHIX_AUTH_TOKEN" ]]; then
  echo 'enabled=true' >> "$GITHUB_OUTPUT"
else
  echo 'enabled=false' >> "$GITHUB_OUTPUT"
  printf '## iOS toolchain cache publishing skipped\\n\\nConfigure the FRACTAL_CACHIX_CACHE repository variable and CACHIX_AUTH_TOKEN repository secret to enable publishing.\\n' >> "$GITHUB_STEP_SUMMARY"
fi`,
        },
        {
          uses: 'actions/checkout@v4',
          if: "steps.credentials.outputs.enabled == 'true'",
          with: { 'persist-credentials': false },
        },
        ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }).map((step) => ({
          ...step,
          if: "steps.credentials.outputs.enabled == 'true'",
        })),
        {
          name: 'Configure producer-owned Cachix cache',
          if: "steps.credentials.outputs.enabled == 'true'",
          uses: 'cachix/cachix-action@38b082610b782e7e93e209c35fd730d399dee866', // v17
          with: {
            name: '${{ vars.FRACTAL_CACHIX_CACHE }}',
            authToken: '${{ secrets.CACHIX_AUTH_TOKEN }}',
            // Publish only the explicit closure below, including substituted paths.
            skipPush: true,
          },
        },
        {
          name: 'Build and publish the full iOS Rust toolchain closure',
          if: "steps.credentials.outputs.enabled == 'true'",
          env: {
            FRACTAL_CACHIX_CACHE: '${{ vars.FRACTAL_CACHIX_CACHE }}',
            CACHIX_AUTH_TOKEN: '${{ secrets.CACHIX_AUTH_TOKEN }}',
          },
          run: `toolchain=$(nix build --no-link --print-out-paths .#ios-rust-toolchain)
nix path-info --recursive "$toolchain" | cachix push "$FRACTAL_CACHIX_CACHE"`,
        },
      ],
    },
    'macos-ci': {
      name: 'macos-ci',
      if: "github.event_name == 'push' || contains(github.event.pull_request.labels.*.name, 'macos-ci')",
      'runs-on': macosRunner,
      'timeout-minutes': 120,
      defaults: { run: { shell: 'bash' } },
      env: { ...buildEnv, CI_CACHE_DEV_SHELL: 'default' },
      steps: [
        ...workspacePreparationSteps,
        {
          ...nixDevelopStep({ name: 'Run macOS workspace tests (25 minute limit)', command: ['bash', 'scripts/ci-nextest', 'run'] }),
          'timeout-minutes': 25,
        },
        nixDevelopStep({ name: 'Cargo clippy', command: ['cargo', 'clippy', '--workspace', '--all-targets', '--locked'] }),
        { name: 'Save Nix outputs', if: "success() && env.CI_LOCAL_CACHES != '1'", run: 'bash scripts/ci-nix-cache save' },
        ...saveMainDependencyCaches(workspacePreparationSteps),
        ...buildSnapshotSave,
      ],
    },
  },
}, {
  'ios-rust-toolchain-cache': 'This producer publishes the explicit iOS Rust toolchain closure to Cachix, including substituted paths; no Actions build cache is configured.',
}))))

// Keep the publisher's positive main-push conjuncts visible to cache-policy validation.
// PR draft guards are unnecessary for a job that never admits PR events.
export default githubWorkflow({
  ...workflow,
  jobs: {
    ...workflow.jobs,
    'ios-rust-toolchain-cache': {
      ...workflow.jobs['ios-rust-toolchain-cache'],
      if: publisherOnly,
    },
  },
})
