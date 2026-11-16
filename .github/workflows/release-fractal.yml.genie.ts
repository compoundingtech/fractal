import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'
import { readyPRJobs, readyPullRequestTypes } from './ready-pr.ts'
import { fullHistoryCheckoutSteps, linuxActionlintConfig, readOnlyBinaryCaches, priorityRunnerJob, megarepoApplyStep } from './workspace-ci.ts'
import { buildSnapshotPrepare, buildSnapshotRestore, buildSnapshotSave } from './build-snapshot.ts'
import { auditCaches } from './cache-audit.ts'
import { githubWorkflow, plainFlakeSetupSteps } from '../../repos/effect-utils/genie/external.ts'
import { readFileSync } from 'node:fs'

const releaseRustVersion = readFileSync(new URL('../../scripts/release-rust-version', import.meta.url), 'utf8').trim()

// Preserve release triggers and source verification; tag builds cannot draft or publish releases.
// Every commit on main also builds and verifies both archives and keeps short-lived artifacts,
// so release breakage fails on main instead of at tag time. Daily candidates dispatch isolated exact-source builds.
// After a green main run, one rolling `edge` pre-release is moved to that commit. It is excluded from
// the tag trigger below. Only the daily workflow prepares untagged stable candidates and publishes with explicit product GO.
export default githubWorkflow(scopeLinuxNixSandbox(readyPRJobs(auditCaches({
  actionlint: linuxActionlintConfig,
  "name": "Fractal tag release",
  "run-name": "${{ github.event_name == 'workflow_dispatch' && inputs.sha != '' && format('Fractal candidate {0} {1}', inputs.sha, inputs.request_id) || format('Fractal tag release {0}', github.ref_name) }}",
  "on": {
    "push": {
      "branches": [
        "main"
      ],
      "tags": [
        "**",
        "!edge",
        "!embed-*",
        "!fleet-*",
        // Evidence records may live under refs/tags/evidence/ (one immutable tag per signed note); pushing one must never release.
        "!evidence/**"
      ]
    },
    "workflow_dispatch": {
      inputs: {
        sha: { description: 'Exact first-parent main source SHA for an isolated candidate build', required: false, type: 'string' },
        request_id: { description: 'Caller correlation ID for this candidate build', required: false, type: 'string' },
        purpose: { description: 'Candidate requires main first-parent history; compatibility builds any deployed source without publication', required: false, type: 'choice', default: 'candidate', options: ['candidate', 'compatibility'] },
      },
    },
    "pull_request": {
      types: readyPullRequestTypes,
      "paths": [
        ".github/workflows/release-fractal.yml",
        ".github/workflows/release-fractal.yml.genie.ts",
        ".gitignore",
        "crates/fractal-build-stamp/**",
        "crates/fractal-drivers/src/version.rs",
        "install.sh",
        "scripts/download-install-test",
        "scripts/linux-release-cc",
        "scripts/check-linux-release.py",
        "scripts/release-build-stamp.py",
        "scripts/release-rust-version",
        "scripts/fixtures/install-release-v0.3.17",
        "scripts/release-zig-preseed*",
        "vendor/libghostty-vt-sys/**",
        "scripts/release-fractal*",
        "scripts/release_artifact.py",
        "scripts/release_draft_recovery.py",
        "scripts/release_gh.py",
        "scripts/release_tag.py",
        "scripts/release-publish-embed*",
        "scripts/release-fractal-embed.mjs",
        "scripts/release_gh.py",
        "apps/fractal-web/**",
        "scripts/release_notes*",
        "scripts/compat-baseline-release*",
        ".github/*-compat-baseline.json",
        "scripts/ci-build-snapshot*",
        "scripts/ci-cache-audit*",
        "scripts/ci-perf-cache",
        ".github/workflows/build-snapshot.ts",
        "scripts/install-release*",
        "scripts/install-macos*",
        "docs/st3/binary-releases.md",
        ".github/workflows/workspace-ci.ts",
        "scripts/ci-pick-runner",
        "flake.lock"
      ]
    }
  },
  "permissions": {
    "contents": "read",
    "actions": "read"
  },
  "concurrency": {
    "group": "fractal-release-${{ github.event_name == 'pull_request' && github.ref || (github.event_name == 'workflow_dispatch' && inputs.sha != '' && format('candidate-{0}', inputs.sha)) || (github.event_name == 'push' && github.ref == 'refs/heads/main' && 'main') || github.run_id }}",
    "cancel-in-progress": "${{ github.event_name == 'pull_request' }}"
  },
  "jobs": {
    "validate-source": {
      name: 'Validate the requested release source',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      steps: [
        ...fullHistoryCheckoutSteps(
          { ref: '${{ github.sha }}', 'persist-credentials': false },
          { name: 'Check out the trusted dispatch ref, never the unvalidated candidate', if: "github.event_name == 'workflow_dispatch'" },
          "github.event_name == 'workflow_dispatch'",
        ),
        {
          name: 'Validate exact source and preserve the stable candidate main gate',
          if: "github.event_name == 'workflow_dispatch'",
          shell: 'bash',
          env: { REQUESTED_SHA: '${{ inputs.sha }}', DISPATCH_REF: '${{ github.ref }}', PURPOSE: '${{ inputs.purpose }}' },
          run: `set -euo pipefail
if [[ "$PURPOSE" != compatibility && "$DISPATCH_REF" != refs/heads/main ]]; then
  echo "::error::Release builds must be dispatched from main."
  exit 1
fi
if [[ "$PURPOSE" == compatibility && -z "$REQUESTED_SHA" ]]; then
  echo "::error::Compatibility archives require an exact source SHA."
  exit 1
fi
if [[ -n "$REQUESTED_SHA" ]]; then
  if [[ ! "$REQUESTED_SHA" =~ ^[0-9a-f]{40}$ ]]; then
    echo "::error::The candidate source must be a full lowercase 40-character SHA."
    exit 1
  fi
  if [[ "$PURPOSE" == compatibility ]]; then
    # Full history is fetched from origin before inspecting this exact commit.
    git cat-file -e "$REQUESTED_SHA^{commit}"
    refs=$(git for-each-ref --contains "$REQUESTED_SHA" --format='%(refname)' refs/remotes/origin refs/tags)
    if [[ -z "$refs" ]]; then
      echo "::error::Compatibility source must be reachable from an origin ref."
      exit 1
    fi
    printf 'Compatibility archive source=%s reachable-from:\\n%s\\n' "$REQUESTED_SHA" "$refs"
  else
    # Read the whole stream: pipefail must not mistake an early reader exit for invalid history.
    if ! git rev-list --first-parent refs/remotes/origin/main | grep -Fx -- "$REQUESTED_SHA"; then
      echo "::error::The candidate source must belong to origin/main first-parent history."
      exit 1
    fi
  fi
fi`,
        },
      ],
    },
    "pick-runner": priorityRunnerJob,
    "installer": {
      "name": "installer (${{ matrix.runner }})",
      "needs": "validate-source",
      "strategy": {
        "fail-fast": false,
        "matrix": {
          "runner": [
            'ubuntu-latest',
            'macos-15'
          ]
        }
      },
      "runs-on": "${{ matrix.runner }}",
      "timeout-minutes": "${{ matrix.runner == 'macos-15' && 10 || 5 }}",
      "steps": [
        {
          // Exercise the workflow's current publication automation, including tests absent from historical candidates.
          "uses": "actions/checkout@v4",
          "with": {
            "persist-credentials": false,
            "ref": "${{ github.sha }}"
          }
        },
        {
          "name": "Test release names, install entry points and isolated app transactions",
          "run": "scripts/install-test\nscripts/install-release-test\npython3 scripts/install-macos-test\npython3 scripts/release_notes_test.py\npython3 scripts/release-fractal-edge-test\npython3 scripts/release-publish-embed-test\npython3 scripts/release-fractal-manifest-test\npython3 scripts/release-fractal-publish-test\npython3 scripts/compat-baseline-release-test\n"
        },
        {
          "name": "Test the daily release failure path",
          "run": "python3 scripts/release-fractal-daily-test"
        }
      ]
    },
    embed: {
      name: 'release-web-embed',
      needs: 'validate-source',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 20,
      steps: [
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false, ref: '${{ inputs.sha || github.sha }}' } },
        { uses: 'actions/cache@v4', with: { path: '${{ runner.temp }}/pnpm-store', key: "web-embed-pnpm-${{ runner.os }}-${{ hashFiles('pnpm-lock.yaml', 'pnpm-source-inputs.json') }}" } },
        ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
        megarepoApplyStep,
        { name: 'Build the offline embed and capture four posters', env: { pnpm_config_store_dir: '${{ runner.temp }}/pnpm-store' }, run: 'nix develop .#web --command node scripts/release-fractal-embed.mjs dist' },
        { uses: 'actions/upload-artifact@v4', with: { name: 'release-web-embed', path: 'dist/*', 'if-no-files-found': 'error', 'retention-days': 7 } },
      ],
    },
    publishEmbed: {
      name: 'release-publish-web-embed',
      if: "github.event_name == 'push' && github.ref == 'refs/heads/main'",
      needs: 'embed',
      'runs-on': 'ubuntu-latest',
      concurrency: {
        group: 'fractal-release-mutation',
        'cancel-in-progress': false,
      },
      permissions: { contents: 'write' },
      env: { GH_TOKEN: '${{ github.token }}', GH_REPO: '${{ github.repository }}', SOURCE_SHA: '${{ github.sha }}' },
      steps: [
        // The publish script, at the commit this run releases.
        { uses: 'actions/checkout@v4', with: { 'persist-credentials': false, ref: '${{ inputs.sha || github.sha }}' } },
        { uses: 'actions/download-artifact@v4', with: { name: 'release-web-embed', path: 'dist' } },
        {
          name: 'Publish once under a source-addressed release, never the rolling edge tag',
          // No --clobber or edit of a published release: even reruns cannot replace a pin. A draft a
          // failed attempt left is continued, and GitHub's 5xx and timeouts are retried (#4606).
          run: 'scripts/release-publish-embed --source "$SOURCE_SHA" --dist dist',
        },
      ],
    },
    "build": {
      "needs": ["pick-runner", "validate-source"],
      "if": "${{ !cancelled() && needs['validate-source'].result == 'success' && (github.event_name == 'workflow_dispatch' || (github.event_name == 'push' && github.event.deleted == false) || github.event_name == 'pull_request') }}",
      "name": "release-build (${{ matrix.target }})",
      "strategy": {
        "fail-fast": false,
        "matrix": {
          "include": [
            {
              "runner": 'namespace-profile-linux-x86-64;job.priority=2;github.run-id=${{ github.run_id }}',
              "target": "x86_64-unknown-linux-gnu"
            },
            {
              "runner": "macos-15",
              "target": "aarch64-apple-darwin"
            }
          ]
        }
      },
      "runs-on": "${{ matrix.target == 'x86_64-unknown-linux-gnu' && github.event_name != 'pull_request' && (needs['pick-runner'].outputs.ci1 && fromJSON(needs['pick-runner'].outputs.ci1)[0] || 'ci1-priority') || matrix.runner }}",
      "timeout-minutes": 90,
      "env": {
        "MACOSX_DEPLOYMENT_TARGET": "15.0",
        CI_NATIVE_SNAPSHOT: '1',
        "RELEASE_TAG": "${{ github.ref_type == 'tag' && github.ref_name || '' }}"
      },
      "steps": [
        {
          "uses": "actions/checkout@v4",
          "with": {
            "persist-credentials": false,
            "ref": "${{ inputs.sha || github.sha }}"
          }
        },
        {
          "uses": "dtolnay/rust-toolchain@7e38f4b43b4db5c8dd498af069a4f6196df1d067",
          "with": { "toolchain": releaseRustVersion }
        },
        { name: 'Name the pinned compiler cache', run: `printf 'CI_ZIG_CACHE_DIR=%s/zig/0.15.2\\n' \"$RUNNER_TOOL_CACHE\" >> \"$GITHUB_ENV\"` },
        buildSnapshotRestore,
        {
          if: "env.CI_BUILD_SNAPSHOT_HIT != '1'",
          "uses": "Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6",
          "with": {
            "key": "release-${{ matrix.target }}",
            "cache-on-failure": true
          }
        },
        {
          name: 'Restore the pinned Zig compiler',
          if: "env.CI_BUILD_SNAPSHOT_ZIG_HIT != '1'",
          uses: 'actions/cache@v4',
          with: {
            path: '${{ env.CI_ZIG_CACHE_DIR }}',
            key: 'zig-tool-v1-${{ runner.os }}-${{ runner.arch }}-0.15.2',
          },
        },
        {
          "uses": "mlugg/setup-zig@d1434d08867e3ee9daa34448df10607b98908d29",
          "with": {
            "version": "0.15.2",
            "use-cache": false
          }
        },
        // Snapshot preparation and release-build-stamp.py both derive their source revision from checked-out HEAD.
        buildSnapshotPrepare,
        ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }),
        {
          name: 'Provide the pinned Linux host C compiler',
          if: "runner.os == 'Linux'",
          run: `set -euo pipefail
compiler=$(nix build .#release-cc^out --no-link --print-out-paths)
"$compiler/bin/cc" --version
printf '%s/bin\n' "$compiler" >> "$GITHUB_PATH"`,
        },
        {
          "name": "Test installer",
          "run": "scripts/install-release-test\npython3 scripts/install-macos-test\npython3 scripts/release-fractal-test\npython3 scripts/release_notes_test.py\npython3 scripts/release-zig-preseed-test.py\n"
        },
        {
          name: 'Preseed hash-verified Zig GitHub archives',
          run: 'python3 scripts/release-zig-preseed.py --github-env "$GITHUB_ENV"',
        },
        {
          "name": "Build, package, and test extracted tools",
          "run": "scripts/release-fractal '${{ matrix.target }}' dist"
        },
        ...buildSnapshotSave,
        {
          "uses": "actions/upload-artifact@v4",
          "with": {
            "name": "release-${{ matrix.target }}",
            "path": "dist/*",
            "if-no-files-found": "error",
            "retention-days": 7
          }
        }
      ]
    },
    "assemble": {
      "name": "release-assemble",
      "needs": [
        "build",
        "embed",
        "installer"
      ],
      "runs-on": "ubuntu-latest",
      "outputs": { "artifact-id": "${{ steps.release.outputs.artifact-id }}" },
      "steps": [
        {
          "uses": "actions/checkout@v4",
          "with": {
            "persist-credentials": false,
            "ref": "${{ inputs.sha || github.sha }}"
          }
        },
        {
          "uses": "actions/download-artifact@v4",
          "with": {
            "pattern": "release-*",
            "merge-multiple": true,
            "path": "dist"
          }
        },
        {
          "name": "Verify both targets and record exact sources",
          "env": {
            "SOURCE_SHA": "${{ inputs.sha || github.sha }}"
          },
          "run": "set -euo pipefail\ncd dist\ncat ./*.sha256 > SHA256SUMS\nsha256sum --check SHA256SUMS\npython3 ../scripts/release-fractal-manifest.py \"$SOURCE_SHA\"\ncp ../docs/st3/binary-releases.md RELEASE-NOTES.md\n# Markdown backticks are literal, not command substitution.\n# shellcheck disable=SC2016\nprintf '\\nExact source: `%s`. See RELEASE.json for the PTY revision and targets.\\n' \"$SOURCE_SHA\" >> RELEASE-NOTES.md\n"
        },
        {
          "id": "release",
          "uses": "actions/upload-artifact@v4",
          "with": {
            "name": "fractal-release-attempt-${{ github.run_attempt }}",
            "path": "dist/*",
            "if-no-files-found": "error",
            "retention-days": 7
          }
        }
      ]
    },
    "publishEdge": {
      "name": "release-publish-edge",
      "if": "github.event_name == 'push' && github.ref == 'refs/heads/main'",
      "needs": ["assemble", "publishEmbed"],
      "runs-on": "ubuntu-latest",
      "timeout-minutes": 15,
      // Wait for embed publication before entering the shared mutation lock; never queue two publishers from this run.
      "concurrency": {
        "group": "fractal-release-mutation",
        "cancel-in-progress": false
      },
      "permissions": {
        "contents": "write"
      },
      "env": {
        "GH_TOKEN": "${{ github.token }}",
        "GH_REPO": "${{ github.repository }}",
        "SOURCE_SHA": "${{ github.sha }}",
        "RUN_URL": "${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}"
      },
      "steps": [
        ...fullHistoryCheckoutSteps({ 'persist-credentials': false, ref: '${{ inputs.sha || github.sha }}' }),
        {
          "uses": "actions/download-artifact@v4",
          "with": {
            "artifact-ids": "${{ needs.assemble.outputs.artifact-id }}",
            "merge-multiple": true,
            "path": "dist"
          }
        },
        {
          "name": "Move the rolling edge pre-release to this verified commit",
          "run": "scripts/release-fractal-edge --source \"$SOURCE_SHA\" --dist dist --run-url \"$RUN_URL\""
        }
      ]
    },
    observePublishers: {
      name: 'report-cancelled-release-publishers',
      needs: ['publishEmbed', 'publishEdge'],
      if: "always() && contains(needs.*.result, 'cancelled')",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read', actions: 'read' },
      env: {
        GH_TOKEN: '${{ github.token }}',
        CANCELLED_RUN: '${{ github.run_id }}',
        REPOSITORY: '${{ github.repository }}',
      },
      steps: [{
        name: 'Fail visibly with cancelled publisher job-ID recovery commands',
        run: `set -euo pipefail
echo "::error::A release publisher was cancelled. Check edge/embed state, then run only the cancelled publisher job IDs printed below; do not rebuild successful embed artifacts."
gh api --paginate --slurp "repos/$REPOSITORY/actions/runs/$CANCELLED_RUN/jobs?filter=latest&per_page=100" |
  jq -er --arg repo "$REPOSITORY" '
    [.[].jobs[]
      | select(.conclusion == "cancelled")
      | select(.name == "release-publish-web-embed" or .name == "release-publish-edge")
      | "gh run rerun --job " + (.id | tostring) + " --repo " + $repo]
    | if length == 0 then error("No cancelled publisher job IDs found; inspect the original run before retrying.") else .[] end'
exit 1`,
      }],
    }
  }
}, {"validate-source": "Validates dispatch inputs using trusted inline shell and main history; never executes candidate source.", "publishEmbed": "Publishes the verified immutable web archive; builds nothing.", "pick-runner": "Only reads organization runner metadata; checks out and builds nothing.", "installer": "Runs isolated installer and publication-gate fixtures without downloads or compilation.", "assemble": "Downloads verified build artifacts; tag runs stop at verified artifacts without drafting or publishing releases; builds nothing.", "publishEdge": "Moves the rolling edge pre-release after embed publication in the shared mutation lock; builds nothing.", "observePublishers": "Fails visibly on cancelled publishers and explains re-dispatch of the original push event; performs no release mutations."}))))
