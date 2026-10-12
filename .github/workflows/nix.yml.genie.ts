import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'

// The dedicated runner retains its Nix store and per-runner GC roots. No Actions cache or FlakeHub service
// participates in this build. Fork pull requests remain covered by the public Linux gate.
export default githubWorkflow({
  actionlint: {
    ...defaultActionlintConfig,
    selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), 'smalltalk-nix-persistent'],
  },
  "name": "Nix",
  "on": {
    "push": {
      "tags": [
        "**"
      ]
    },
    "schedule": [{ "cron": "17 1 * * *" }],
    "workflow_dispatch": {
      "inputs": {
        "tag": {
          "description": "Existing release tag to verify (empty uses the workflow revision)",
          "type": "string",
          "required": false,
          "default": ""
        }
      }
    },
    "pull_request": {
      "paths": [".github/workflows/nix.yml*", ".github/messaging-compat-baseline.json", "scripts/ci-nix-release*", "flake.nix", "flake.lock", "nix/**"]
    }
  },
  "permissions": { "contents": "read" },
  "concurrency": {
    "group": "nix-${{ github.ref }}",
    "cancel-in-progress": "${{ github.event_name == 'pull_request' }}"
  },
  "jobs": {
    "nix-release-x86_64-linux": {
      "if": "github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository",
      "runs-on": ["self-hosted", "Linux", "X64", "smalltalk-nix-persistent"],
      "timeout-minutes": 180,
      "steps": [
        {
          "uses": "actions/checkout@v4",
          "with": {
            "persist-credentials": false,
            "ref": "${{ inputs.tag && format('refs/tags/{0}', inputs.tag) || github.sha }}"
          }
        },
        {
          "name": "Verify the persistent Nix runner",
          "run": "nix --version\ntest -d \"$CI_LOCAL_CARGO_HOME\""
        },
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
  }
})
