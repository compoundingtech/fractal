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
    "workflow_dispatch": {
      "inputs": {
        "sha": {
          "description": "Full commit SHA on main to publish",
          "required": true,
          "type": "string"
        }
      }
    }
  },
  "permissions": { "contents": "read" },
  "jobs": {
    "publish-x86_64-linux": {
      "if": publisherIf,
      "concurrency": {
        "group": "nix-publish-${{ inputs.sha || github.sha }}",
        "cancel-in-progress": false
      },
      "runs-on": "ubuntu-latest",
      "timeout-minutes": 180,
      "steps": [
        {
          "uses": "actions/checkout@v4",
          "with": {
            "ref": "${{ inputs.sha || github.sha }}",
            "fetch-depth": 1,
            "persist-credentials": false
          }
        },
        {
          "name": "Require an exact commit on main",
          "env": { "PUBLISH_SHA": "${{ inputs.sha || github.sha }}" },
          "run": [
            '[[ "$PUBLISH_SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "::error::Provide a full commit SHA"; exit 1; }',
            '[[ "$(git rev-parse HEAD)" == "$PUBLISH_SHA" ]] || { echo "::error::Checkout does not match the requested commit"; exit 1; }',
            'git fetch --no-tags --unshallow origin +refs/heads/main:refs/remotes/origin/main',
            'git merge-base --is-ancestor "$PUBLISH_SHA" origin/main || { echo "::error::Publication source must be an ancestor of origin/main"; exit 1; }',
            'printf "PUBLISH_SHA=%s\\n" "$PUBLISH_SHA" >> "$GITHUB_ENV"',
          ].join('\n')
        },
        installNixStep({ binaryCaches: readOnlyBinaryCaches }),
        {
          "name": "Require the repository-owned binary cache",
          "env": { "CACHE_NAME": "${{ vars.FRACTAL_CACHIX_CACHE_NAME }}" },
          "run": '[[ "$CACHE_NAME" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "::error::Configure FRACTAL_CACHIX_CACHE_NAME with the repository-owned cache"; exit 1; }'
        },
        {
          "name": "Build the exact consumer package",
          "run": [
            'producer="git+file://$GITHUB_WORKSPACE?rev=$PUBLISH_SHA&shallow=1#packages.x86_64-linux.fractal"',
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
            // Authenticate only explicit private-cache queries; never configure daemon-wide credentials.
            'umask 077',
            'cache_netrc=$(mktemp "$RUNNER_TEMP/fractal-cache-netrc.XXXXXX")',
            'trap \'rm -f -- "$cache_netrc"\' EXIT',
            'printf "machine %s.cachix.org\\nlogin token\\npassword %s\\n" "$CACHE_NAME" "$CACHIX_AUTH_TOKEN" > "$cache_netrc"',
            // The producer output must be in the owned cache, not merely an upstream cache.
            'nix --option netrc-file "$cache_netrc" path-info --store "https://$CACHE_NAME.cachix.org" "$output" > /dev/null',
            // Cachix omits upstream paths. Prove every local closure member remotely, without local-store fallback.
            'nix path-info --recursive "$output" > "$RUNNER_TEMP/fractal-native-closure.paths"',
            'mapfile -t closure < "$RUNNER_TEMP/fractal-native-closure.paths"',
            'nix path-info --json "${closure[@]}" > "$RUNNER_TEMP/fractal-native-closure.json"',
            // --print-invalid partitions a whole batch; path-info aborts before printing partial results.
            'nix-store --store https://cache.nixos.org --check-validity --print-invalid "${closure[@]}" > "$RUNNER_TEMP/fractal-native-upstream-missing.paths"',
            'mapfile -t remainder < "$RUNNER_TEMP/fractal-native-upstream-missing.paths"',
            ': > "$RUNNER_TEMP/fractal-native-missing.paths"',
            'if (( ${#remainder[@]} )); then',
            '  nix-store --option netrc-file "$cache_netrc" --store "https://$CACHE_NAME.cachix.org" --check-validity --print-invalid "${remainder[@]}" > "$RUNNER_TEMP/fractal-native-missing.paths"',
            'fi',
            '[[ ! -s "$RUNNER_TEMP/fractal-native-missing.paths" ]] || { echo "::error::Closure paths missing from both caches"; cat "$RUNNER_TEMP/fractal-native-missing.paths" >&2; exit 1; }',
            'printf "source=%s\\noutput=%s\\ncache=https://%s.cachix.org\\n" "$PUBLISH_SHA" "$output" "$CACHE_NAME" > "$RUNNER_TEMP/fractal-native-publication.txt"',
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
