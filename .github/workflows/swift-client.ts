// Optional execution-only lane: no required-check or reusable-evidence registration.
const swiftImage = 'swift:6.0.3-jammy@sha256:95f6ef898992dc840b35517e780f6f6db9661432775ab0b33cc0a531ea749bdb'

export const swiftClientJobs = {
  'swift-client-changes': {
    name: 'swift-client-changes',
    'runs-on': 'ubuntu-latest',
    'timeout-minutes': 5,
    permissions: { contents: 'read' },
    outputs: { relevant: '${{ steps.detect.outputs.relevant }}' },
    steps: [
      { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
      {
        name: 'Detect Swift client inputs',
        id: 'detect',
        env: {
          BASE_SHA: '${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha || github.event.before }}',
          EVENT_NAME: '${{ github.event_name }}',
        },
        run: `python3 - <<'PY'
import fnmatch
import os
import subprocess

patterns = (
    'clients/swift/**',
    'crates/fractal-client-codegen/templates/*.swift.in',
    'crates/fractal-client-codegen/src/**',
    'docs/st3/client-v0/fixtures/**',
    'docs/st3/client-v0/schemas/**',
    'fixtures/clients/**',
    'fixtures/scenarios/**',
    '.github/workflows/swift-client.ts',
    '.github/workflows/fleet.yml*',
    'scripts/ci-swift-client',
)
base = os.environ.get('BASE_SHA', '')
relevant = True
if os.environ['EVENT_NAME'] != 'workflow_dispatch' and base and set(base) != {'0'}:
    subprocess.run(['git', 'fetch', '--no-tags', '--depth=1', 'origin', base], check=True)
    names = subprocess.check_output(['git', 'diff', '--name-only', '--no-renames', '-z', base, 'HEAD']).decode().split('\\0')
    relevant = any(fnmatch.fnmatchcase(name, pattern) for name in names for pattern in patterns)
with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
    output.write(f'relevant={str(relevant).lower()}\\n')
PY`,
      },
    ],
  },
  'swift-client': {
    name: 'swift-client',
    needs: ['swift-client-changes'],
    if: "${{ !cancelled() && needs.swift-client-changes.result == 'success' && needs.swift-client-changes.outputs.relevant == 'true' }}",
    'runs-on': 'ubuntu-latest',
    'timeout-minutes': 15,
    permissions: { contents: 'read' },
    steps: [
      { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
      {
        name: 'Compile and test the Swift package on Linux',
        env: { SWIFT_IMAGE: swiftImage },
        run: 'bash scripts/ci-swift-client',
      },
    ],
  },
} as const
