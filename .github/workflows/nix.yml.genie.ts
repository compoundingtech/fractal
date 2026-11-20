import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { readyPRJobs } from './ready-pr.ts'
import { cachixPushStep, defaultActionlintConfig, githubWorkflow, installNixStep } from '../../repos/effect-utils/genie/external.ts'
import { readOnlyBinaryCaches } from './workspace-ci.ts'

const publisherIf = "github.ref == 'refs/heads/main' && (github.event_name == 'push' || github.event_name == 'workflow_dispatch')"

// Persistent-store verification stays disabled until its dedicated pool is qualified.
// The separate main-only publisher builds the consumer package on a hosted worker.
export default githubWorkflow(scopeLinuxNixSandbox({
  actionlint: {
    ...defaultActionlintConfig,
    selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), 'ci1'],
  },
  "name": "Nix",
  "on": {
    "push": { "branches": ["main"] },
    "schedule": [{ "cron": "17 1 * * *" }],
    // Optional Nix verification stays off PRs while the release owner repairs it.
    "workflow_dispatch": {}
  },
  "permissions": { "contents": "read" },
  "concurrency": {
    "group": "nix-${{ github.ref }}",
    "cancel-in-progress": "${{ github.event_name == 'pull_request' }}"
  },
  "jobs": {
    "publish-x86_64-linux": {
      "if": publisherIf,
      "runs-on": "ubuntu-latest",
      "timeout-minutes": 180,
      "steps": [
        { "uses": "actions/checkout@v4", "with": { "persist-credentials": false } },
        installNixStep({ binaryCaches: readOnlyBinaryCaches }),
        {
          "name": "Require the repository-owned binary cache",
          "env": { "CACHE_NAME": "${{ vars.FRACTAL_CACHIX_CACHE_NAME }}" },
          "run": '[[ "$CACHE_NAME" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "::error::Configure FRACTAL_CACHIX_CACHE_NAME with the repository-owned cache"; exit 1; }'
        },
        {
          "name": "Build the exact consumer package",
          "run": [
            'producer="git+file://$GITHUB_WORKSPACE?rev=$GITHUB_SHA&shallow=1#packages.x86_64-linux.fractal"',
            'nix build "$producer" --no-update-lock-file --max-jobs 1 --cores 2 --print-build-logs --out-link "$RUNNER_TEMP/fractal-native"',
            'nix build --inputs-from . nixpkgs#cachix --no-link --print-out-paths > "$RUNNER_TEMP/cachix-path"',
          ].join('\n')
        },
        cachixPushStep({
          jobIf: publisherIf,
          triggers: ['push', 'workflow_dispatch'],
          authToken: '${{ secrets.FRACTAL_CACHIX_AUTH_TOKEN }}',
          step: {
          "name": "Publish and prove the complete package closure",
          "env": { "CACHE_NAME": "${{ vars.FRACTAL_CACHIX_CACHE_NAME }}" },
          "run": [
            ': "${CACHIX_AUTH_TOKEN:?Configure the repository-scoped FRACTAL_CACHIX_AUTH_TOKEN write secret}"',
            'output=$(readlink -f "$RUNNER_TEMP/fractal-native")',
            '"$(cat "$RUNNER_TEMP/cachix-path")/bin/cachix" push "$CACHE_NAME" "$output"',
            // Authenticate only this explicit remote-store proof; never configure daemon-wide credentials.
            'umask 077',
            'cache_netrc=$(mktemp "$RUNNER_TEMP/fractal-cache-netrc.XXXXXX")',
            'trap \'rm -f -- "$cache_netrc"\' EXIT',
            'printf "machine %s.cachix.org\\nlogin token\\npassword %s\\n" "$CACHE_NAME" "$CACHIX_AUTH_TOKEN" > "$cache_netrc"',
            'nix --option netrc-file "$cache_netrc" path-info --store "https://$CACHE_NAME.cachix.org" --recursive --json "$output" > "$RUNNER_TEMP/fractal-native-closure.json"',
            'printf "source=%s\\noutput=%s\\ncache=https://%s.cachix.org\\n" "$GITHUB_SHA" "$output" "$CACHE_NAME" > "$RUNNER_TEMP/fractal-native-publication.txt"',
            'cat "$RUNNER_TEMP/fractal-native-publication.txt" >> "$GITHUB_STEP_SUMMARY"',
          ].join('\n')
          },
        }),
        {
          "uses": "actions/upload-artifact@v4",
          "with": {
            "name": "fractal-native-publication",
            "path": "${{ runner.temp }}/fractal-native-publication.txt\n${{ runner.temp }}/fractal-native-closure.json",
            "if-no-files-found": "error",
            "retention-days": 30
          }
        }
      ]
    },
    ...readyPRJobs({ jobs: {
    "nix-release-x86_64-linux": {
      // Nathan/CoS 2026-10-15: optional Nix must not occupy merge-serving workers.
      // Persistent-store verification cannot be relabelled to an ephemeral image.
      // Restore only with a reviewed, qualified dedicated persistent-store pool.
      "if": false,
      "runs-on": "ci1",
      "timeout-minutes": 180,
      "steps": [
        {
          "uses": "actions/checkout@v4",
          "with": { "persist-credentials": false }
        },
        installNixStep({ binaryCaches: readOnlyBinaryCaches }),
        {
          "name": "Test persistent-store release verification",
          "run": "python3 scripts/ci-nix-release-test"
        },
        {
          "name": "Build the package and every native check",
          "run": "scripts/ci-nix-release build"
        },
        {
          "name": "Prove the warm cache without downloads or builds",
          "run": "scripts/ci-nix-release verify-cache"
        },
        {
          "uses": "actions/upload-artifact@v4",
          "with": {
            "name": "nix-release-proof",
            "path": "${{ runner.temp }}/nix-release-build.json",
            "if-no-files-found": "error",
            "retention-days": 7
          }
        }
      ]
    }
    } }).jobs,
  }
}))
