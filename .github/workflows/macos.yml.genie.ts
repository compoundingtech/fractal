import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { readyPullRequestTypes } from './ready-pr.ts'
import { buildSnapshotSave } from './build-snapshot.ts'
import { auditCaches } from './cache-audit.ts'
import {
  defaultActionlintConfig,
  githubWorkflow,
  nixDevelopStep,
} from '../../repos/effect-utils/genie/external.ts'
import { buildEnv, macosRunner, saveMainDependencyCaches, workspacePreparationSteps } from './workspace-ci.ts'

// The focused hosted lane is path-filtered; the full workspace lane remains opt-in on PRs.
// Neither lane changes required-check or reusable-evidence policy.
export default githubWorkflow(scopeLinuxNixSandbox(auditCaches({
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
    'macos-bound-peer-changes': {
      name: 'macos-bound-peer-changes',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      outputs: { relevant: '${{ steps.detect.outputs.relevant }}' },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        {
          name: 'Detect bound-peer inputs',
          id: 'detect',
          env: { BASE_SHA: '${{ github.event.pull_request.base.sha || github.event.before }}' },
          run: `python3 - <<'PY'
import fnmatch
import os
import subprocess

patterns = (
    'crates/fractal-api-core/**',
    'crates/fractal-api-client-v0/**',
    'crates/fractal-runtime/**',
    'crates/fractal-drivers/**',
    'crates/fractal-daemon/**',
    'Cargo.toml',
    'Cargo.lock',
    '.cargo/**',
    '.github/workflows/macos.yml*',
)
base = os.environ.get('BASE_SHA', '')
relevant = True
if base and set(base) != {'0'}:
    subprocess.run(['git', 'fetch', '--no-tags', '--depth=1', 'origin', base], check=True)
    names = subprocess.check_output(['git', 'diff', '--name-only', '--no-renames', '-z', base, 'HEAD']).decode().split('\\0')
    relevant = any(fnmatch.fnmatchcase(name, pattern) for name in names for pattern in patterns)
with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
    output.write(f'relevant={str(relevant).lower()}\\n')
PY`,
        },
      ],
    },
    'macos-bound-peer': {
      name: 'macos-bound-peer',
      needs: ['macos-bound-peer-changes'],
      if: "${{ !cancelled() && needs.macos-bound-peer-changes.result == 'success' && needs.macos-bound-peer-changes.outputs.relevant == 'true' }}",
      'runs-on': 'macos-14',
      'timeout-minutes': 25,
      env: buildEnv,
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        { uses: 'dtolnay/rust-toolchain@stable' },
        { uses: 'Swatinem/rust-cache@v2', with: { 'shared-key': 'macos-bound-peer' } },
        {
          name: 'Test process identity',
          run: 'cargo test --locked -p fractal-runtime --lib peer_process::',
        },
        {
          name: 'Test bound-peer authority',
          run: `for test in transport_publication a_unix_peer a_harness_descendant masked_child unverifiable; do
  cargo test --locked -p fractal-api-core --lib "$test" || exit
done
cargo test --locked -p fractal-api-client-v0 --lib bound_actor_refusals`,
        },
        {
          name: 'Test worker ingress',
          run: 'cargo test --locked -p fractal-daemon --lib worker_ingress::',
        },
      ],
    },
    'macos-ci': {
      name: 'macos-ci',
      if: "(github.event_name != 'pull_request' || github.event.pull_request.draft == false) && (github.event_name == 'push' || contains(github.event.pull_request.labels.*.name, 'macos-ci'))",
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
}, { 'macos-bound-peer-changes': 'Only classifies changed paths; no build inputs.' })))
