import { defaultActionlintConfig, githubWorkflow, plainFlakeSetupSteps } from '../../repos/effect-utils/genie/external.ts'
import { auditCaches } from './cache-audit.ts'
import { megarepoApplyStep, pnpmStoreEnv, readOnlyBinaryCaches } from './workspace-ci.ts'
import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'

export default githubWorkflow(scopeLinuxNixSandbox(auditCaches({
  name: 'iOS landing screenshots',
  on: { workflow_dispatch: {} },
  permissions: { contents: 'read' },
  actionlint: defaultActionlintConfig,
  concurrency: { group: 'ios-landing-screenshots', 'cancel-in-progress': false },
  jobs: {
    capture: {
      name: 'Capture the real iOS client',
      'runs-on': 'macos-15',
      'timeout-minutes': 90,
      defaults: { run: { shell: 'bash' } },
      env: { MAESTRO_CLI_NO_ANALYTICS: 'true' },
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false } },
        ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
        megarepoApplyStep,
        { name: 'Install locked workspace dependencies', env: pnpmStoreEnv, run: 'nix develop .#web -c pnpm install --frozen-lockfile' },
        { uses: 'actions/setup-java@v4', with: { distribution: 'temurin', 'java-version': '17' } },
        {
          name: 'Install pinned Maestro CLI without an account',
          run: `mkdir -p "$RUNNER_TEMP/maestro"
cd "$RUNNER_TEMP/maestro"
curl --fail --location --silent --show-error https://github.com/mobile-dev-inc/Maestro/releases/download/cli-2.11.0/maestro.zip --output maestro.zip
printf '%s  maestro.zip\\n' 5384593cb4e7a106489e75a821d157dd43f4e438df6bc308b72e82c685e1283a | shasum -a 256 -c -
unzip -q maestro.zip
printf '%s\\n' "$RUNNER_TEMP/maestro/maestro/bin" >> "$GITHUB_PATH"`,
        },
        {
          name: 'Build and capture one disposable simulator',
          env: pnpmStoreEnv,
          run: `gateway_host=$(/usr/sbin/ipconfig getifaddr en0)
nix develop .#web -c nix shell nixpkgs#cocoapods -c apps/ios/scripts/landing-screenshots \\
  --gateway-host "$gateway_host" --output "$RUNNER_TEMP/ios-landing-screenshots"`,
        },
        {
          name: 'Upload light and dark PNGs',
          uses: 'actions/upload-artifact@v4',
          with: {
            name: 'ios-landing-screenshots-${{ github.sha }}',
            path: '${{ runner.temp }}/ios-landing-screenshots/*.png',
            'if-no-files-found': 'error',
            'retention-days': 14,
          },
        },
        {
          name: 'Retain command and service logs on failure',
          if: 'always()',
          uses: 'actions/upload-artifact@v4',
          with: {
            name: 'ios-landing-screenshots-logs-${{ github.run_id }}',
            path: '${{ runner.temp }}/ios-landing-screenshots/*.log',
            'if-no-files-found': 'ignore',
            'retention-days': 7,
          },
        },
      ],
    },
  },
}, { capture: 'One-off screenshot capture uses read-only upstream Nix caches only; no build cache is uploaded.' })))
