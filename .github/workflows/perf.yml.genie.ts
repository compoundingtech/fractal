import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'
import { performanceRunner } from './workspace-ci.ts'

// Measurement-only branch: explicit dispatch, never a production/merge vehicle.
const workflow = {
  "name": "Performance",
  "on": {
    "workflow_dispatch": {}
  },
  "permissions": {
    "contents": "read",
    "actions": "read",
    "pull-requests": "read"
  },
  "concurrency": {
    "group": "trim-write-diagnostic-${{ github.run_id }}",
    "cancel-in-progress": false
  },
  "jobs": {
    "trim-write-diagnostic": {
      "name": "trim-write-diagnostic",
      "runs-on": [
        "nscloud-ubuntu-24.04-amd64-8x16-with-features;job.priority=3",
        "namespace-features:github.run-id=${{ github.run_id }}"
      ],
      "timeout-minutes": 90,
      "defaults": {
        "run": {
          "shell": "bash"
        }
      },
      "env": {
        "CARGO_PROFILE_DEV_DEBUG": "0",
        "CARGO_PROFILE_TEST_DEBUG": "0",
        "CARGO_INCREMENTAL": "0",
        "SCCACHE_IDLE_TIMEOUT": "0",
        "PERF_PR_NUMBER": "${{ github.event.pull_request.number }}",
        "PERF_HEAD_REPOSITORY": "${{ github.event.pull_request.head.repo.full_name }}"
      },
      "steps": [
        {
          "uses": "actions/checkout@v4",
          "with": {
            "fetch-depth": 0,
            "persist-credentials": false
          }
        },
        {
          "name": "Freeze reviewed diagnostic driver before runtime checkout",
          "run": "python3 - <<'PY_DRIVER'\nimport pathlib,json,hashlib,os,subprocess,datetime\nr=pathlib.Path(os.environ['RUNNER_TEMP'])/'trim-write-hosted';r.mkdir(exist_ok=False)\np=pathlib.Path('scripts/trim-write-hosted-diagnostic')\nassert hashlib.sha256(p.read_bytes()).hexdigest() == 'ff5a584c8d91bec317025a8e96407d6a99f2f86fe6f503deca1a73015bfc69d4'\nassert subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip() == os.environ['GITHUB_SHA']\n(r.parent/'trim-write-hosted-controller.py').write_bytes(p.read_bytes())\n(r/'driver.json').write_text(json.dumps({'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'head':os.environ['GITHUB_SHA'],'tree':subprocess.check_output(['git','rev-parse','HEAD^{tree}'],text=True).strip(),'controller_sha256':hashlib.sha256(p.read_bytes()).hexdigest()},indent=2)+'\\n')\nassert datetime.datetime.now(datetime.timezone.utc)<datetime.datetime(2026,10,14,22,25,tzinfo=datetime.timezone.utc), 'window closed before heavy setup'\nPY_DRIVER"
        },
        {
          "uses": "actions/checkout@v4",
          "with": {
            "ref": "e9956673e61ef018cc3928818fe17c91fb36798f",
            "fetch-depth": 0,
            "persist-credentials": false
          }
        },
        {
          "name": "Verify frozen runtime and unchanged original test contract",
          "run": "test \"$(git rev-parse HEAD)\" = e9956673e61ef018cc3928818fe17c91fb36798f\ntest \"$(git rev-parse 'HEAD^{tree}')\" = d296f28ece1ebdf9d159c994966c44c3f7c5509f\ntest -z \"$(git status --porcelain --untracked-files=no)\"\ngit diff f51971b5d62c21e235c5349ddd752641a9fe3a40 HEAD -- crates/st3/src/store/tombstones_tests.rs > \"$RUNNER_TEMP/trim-write-hosted/runtime-hook.patch\"\nprintf '%s\\n' \"$(git rev-parse HEAD)\" \"$(git rev-parse 'HEAD^{tree}')\" > \"$RUNNER_TEMP/trim-write-hosted/runtime-source.txt\""
        },
        {
          "name": "Name compatible snapshot recipes",
          "env": {
            "PERF_BUILD_SNAPSHOT": "perf-load-build-v1-${{ hashFiles('Cargo.lock', 'flake.lock', 'flake.nix', 'Cargo.toml', 'crates/st3/Cargo.toml', '.cargo/config.toml') }}",
            "PERF_STORES_SNAPSHOT": "perf-load-stores-v1-${{ hashFiles('crates/st3/tests/daemon_bench.rs', 'docs/st3/schema.md') }}"
          },
          "run": "printf \"PERF_BUILD_SNAPSHOT=%s\\nPERF_STORES_SNAPSHOT=%s\\n\" \"$PERF_BUILD_SNAPSHOT\" \"$PERF_STORES_SNAPSHOT\" >> \"$GITHUB_ENV\""
        },
        {
          "name": "Restore compatible build, Nix, stores and main baseline snapshots",
          "env": {
            "GH_TOKEN": "${{ github.token }}"
          },
          "run": "python3 scripts/ci-perf-cache restore"
        },
        {
          "name": "Install Nix",
          "uses": "DeterminateSystems/determinate-nix-action@v3",
          "env": {
            "GITHUB_TOKEN": "${{ github.token }}"
          },
          "with": {
            "extra-conf": "experimental-features = nix-command flakes\naccept-flake-config = true\nextra-substituters = https://overeng-effect-utils.cachix.org\nextra-trusted-public-keys = overeng-effect-utils.cachix.org-1:KFmqYNF6Q7ZzVYPl2znpJYZGEolage9YNCA9res6vKc=\naccess-tokens = github.com=${{ github.token }}\n",
            "summarize": true
          }
        },
        {
          "name": "Isolate test state and keep the compiler cache",
          "run": "home=\"$RUNNER_TEMP/test-home\"\nmkdir -p \"$home\"/{.config,.cache,.local/state} \"$RUNNER_TEMP/cargo-home\"/{registry,git} \"$RUNNER_TEMP/perf-sccache\"\nprintf 'CARGO_HOME=%s\\nCI_CACHE_DIR=%s\\nSCCACHE_DIR=%s\\nSCCACHE_CACHE_SIZE=1G\\n' \"$RUNNER_TEMP/cargo-home\" \"$RUNNER_TEMP/st-ci-cache\" \"$RUNNER_TEMP/perf-sccache\" >> \"$GITHUB_ENV\"\nprintf 'HOME=%s\\nXDG_CONFIG_HOME=%s/.config\\nXDG_CACHE_HOME=%s/.cache\\nXDG_STATE_HOME=%s/.local/state\\n' \"$home\" \"$home\" \"$home\" \"$home\" >> \"$GITHUB_ENV\"\n"
        },
        {
          "name": "Use the cached Nix outputs",
          "run": "bash scripts/ci-nix-cache use"
        },
        {
          "name": "Run one frozen bounded diagnostic cohort",
          "id": "load",
          "timeout-minutes": 80,
          "run": "nix develop . -c python3 \"$RUNNER_TEMP/trim-write-hosted-controller.py\""
        },
        {
          "id": "cache",
          "name": "Save build and Nix snapshots",
          "if": "!cancelled() && (github.event_name == 'pull_request' || github.ref == 'refs/heads/main') && (steps.load.outcome == 'success' || steps.load.outcome == 'failure')",
          "run": "if ! python3 scripts/ci-perf-cache check-report; then exit 0; fi\nnix print-dev-env .#perf --profile \"$RUNNER_TEMP/perf-shell\" > /dev/null\nnix develop .#perf -c env TMPDIR=\"$RUNNER_TEMP\" sccache --show-stats || echo \"::warning::sccache statistics unavailable; retaining the successful build\"\nCI_PERF_SHELL_ROOT=\"$RUNNER_TEMP/perf-shell\" bash scripts/ci-nix-cache save\npython3 scripts/ci-perf-cache pack\n"
        },
        {
          "name": "Retain the build and Nix cache",
          "if": "!cancelled() && steps.cache.outcome == 'success' && steps.cache.outputs.publish == 'true'",
          "uses": "actions/upload-artifact@v4",
          "with": {
            "name": "${{ env.PERF_BUILD_SNAPSHOT }}",
            "path": "${{ runner.temp }}/perf-snapshots/build.tar.zst",
            "compression-level": 0,
            "retention-days": 7,
            "if-no-files-found": "error",
            "overwrite": true
          }
        },
        {
          "name": "Retain the generated stores",
          "if": "!cancelled() && steps.cache.outcome == 'success' && steps.cache.outputs.publish == 'true'",
          "uses": "actions/upload-artifact@v4",
          "with": {
            "name": "${{ env.PERF_STORES_SNAPSHOT }}",
            "path": "${{ runner.temp }}/perf-snapshots/stores.tar.zst",
            "compression-level": 0,
            "retention-days": 7,
            "if-no-files-found": "error",
            "overwrite": true
          }
        },
        {
          "name": "Retain every diagnostic partial, adverse and manifest",
          "uses": "actions/upload-artifact@v4",
          "if": "always()",
          "with": {
            "name": "trim-write-diagnostic-${{ github.run_id }}-${{ github.run_attempt }}",
            "path": "${{ runner.temp }}/trim-write-hosted/",
            "retention-days": 14,
            "if-no-files-found": "warn",
            "overwrite": false
          }
        }
      ]
    }
  }
}
export default githubWorkflow({ ...workflow, actionlint: { ...defaultActionlintConfig, selfHostedRunnerLabels: [...(defaultActionlintConfig.selfHostedRunnerLabels ?? []), ...performanceRunner] } })
