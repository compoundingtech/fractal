# Build your own software factory

Fractal is an open framework for building a software factory: a system that produces
software to meet a set of goals. Give your agents a mission with goals, constraints
and checks; durable agents carry it through and bring you the decisions that need you.

Use the `fractal` CLI to talk to your coding agents and follow their work.

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/compoundingtech/fractal/main/install.sh | sh
```

The installer selects your platform, verifies the downloaded archive and opens `fractal`.
On the first run, choose your name and this machine's name, accept the background
service, and choose an installed coding harness. Before it starts, setup says that the
built-in Fractal Assistant runs an agent on your own harness account (using its usage and quota) and
asks; it then guides you through your first mission and stays available afterward. Nothing
is started on an installation that already has a configuration.

After quitting the interface with **Ctrl+Q**, open it again:

```sh
export PATH="$HOME/.local/bin:$PATH"
fractal
```

Supported platforms are Linux x86_64 with glibc 2.35+ (Ubuntu 22.04+) and macOS Apple
Silicon with macOS 15+. The installer needs `curl`, `tar` and a SHA256 checker; macOS
also needs Python 3. Harness CLIs and their accounts are separate. If none is
installed, `fractal` still opens; install a supported harness and run `fractal setup` later.
For Claude, first complete its native welcome, account or API-key confirmation and
permission-bypass consent as the same user and profile. Successful auth-status or
print checks alone do not prove that the Assistant can authenticate.
Neither GitHub nor `gh` is required. Clean-machine macOS app, launchd and permission
setup still need a manual rehearsal; native builds do not establish that evidence.

`fractal` or `fractal tui` opens the terminal interface. Use commands such as
`fractal work ls` to inspect mission work from your shell.
`fractal web` opens an already-running web interface at `http://127.0.0.1:5173`
(override with the `FRACTAL_WEB_URL` environment variable); it does not start a
server. If no browser opener is available, it prints the URL instead.

Read the **[Fractal documentation](https://fractal.how/docs)** or browse its
[Introduction](docs/site/index.md),
[installation guide](docs/site/getting-started/install.md), and
[Understand Fractal](docs/site/concepts/index.md) pages in this repository.
Each person currently runs their own fleet; start with one machine and add others
when you need them.

## Guides

- [Two machines](docs/site/guides/two-machines.md): connect machines and check replication.
- [Missions in practice](docs/site/guides/missions-in-practice.md): revise work and ask for decisions.
- [Talking to agents](docs/site/guides/talking-to-agents.md): web app, phone, CLI and attachments.
- [Agent lifecycle](docs/site/guides/agent-lifecycle.md): create, restart and suspend agents.
- [Upgrading](docs/site/guides/upgrading.md): choose a build and check recovery.
- [GitHub integration](docs/site/guides/github-integration.md): repository intake and review.
- [Troubleshooting](docs/site/guides/troubleshooting.md): diagnose startup and health problems.
- [iOS app](docs/site/guides/ios-app.md): build and pair the phone client.

## Pre-signing your checks

Run the per-PR checks on your own machine and sign them before you push, so CI can skip them and your pull request goes green at
once. **Pre-signing is not required today:** a push without a signature just runs CI as before. It **will be required** for every
contributor's pull requests later, so set it up now.

**You need** a Linux x86_64 machine with Nix. On our machines a run peaks at about 16 cores and 18 GB of RAM. The kept build
folder takes 60 to 110 GB of disk. A full warm run takes about 12 minutes (our target is 12 minutes or less).

**Once, enrol a key.** Run `scripts/evidence-keygen --level agent --name ALIAS`. Use an invented alias, never a machine or account
name. It prints one line; open a pull request that adds that line to `.st/evidence-keys`. Keys are read from `main`, so the key counts
once that pull request merges. Deleting the line revokes everything the key signed. The accounts and commit emails that open and
author your pull requests must be listed in `.st/evidence-authors.json`.

**Before each push**, from a clean checkout of the commit:

```sh
scripts/st-test --all --push origin
```

- It runs `linux-gate` (tests, clippy, fleet compatibility), `typescript-client`, `genie-freshness`, `fractal-web` (only when a web
  input changed) and the per-crate `rust-tests`, in a clean worktree of `HEAD`. The `rust-tests` notes are signed from the same test
  run; CI starts skipping on them once that check is turned on.
- It signs each result with your key and pushes the signed notes.
- It ends with one row per check: `passed` and `covered by ...` is signed. `not signed` names the reason: a branch that changes a
  check's own definition (for example its script) cannot be pre-signed for that check, so CI runs it. `--status` runs nothing and says
  whether the tree is already covered.

**Keep it within 12 minutes.** A full warm run of `scripts/st-test` (build, test and sign) has a budget of 12 minutes on a dev box
today. Keep your changes, and any tests you add, inside it. The table st-test prints at the end gives each check's time in seconds.

The pre-push hook, which `nix develop` installs (`scripts/install-git-hooks`), checks this for you. For an agent whose key is listed,
it refuses a push whose tree is not covered and prints the command to run.

**What CI does with it.** On a pull request, a check skips when a trusted note covers the head's tree. One run in five still runs for
real as a canary. The branch must be at most 24 commits behind `main` and must not change a protected file such as the workflow,
the gate or the keys. The merge queue and `main` always run every check.

**What runs once a day instead.** `isolation-vm`, the mail redelivery canaries and the long performance lanes do not run per pull
request. They run once a day on `main`, in the `daily-long` workflow. A red day is reported as a comment on the open issue titled
"Daily long-lane run failed" (#3501 today) and is looked at the next day.

**To see that it worked**, open the job `evidence-gate-<check>` in the pull request's run, for example `evidence-gate-linux-gate`.
Its decision line reads `evidence-decision v1 check=... decision=skip outcome=satisfied ... signer=agent/ALIAS`, and the step
`evidence skip taken` ran. When the check ran anyway, `reason=` on that line says why.

Details: [the evidence gate and the skip policy](docs/st3/gate-evidence.md#evidence-gate-and-the-skip-policy),
[agent keys](docs/st3/gate-evidence.md#agent-keys) and
[running the checks locally](docs/st3/gate-evidence.md#running-the-checks-locally-scriptsst-test). Questions or problems: open an
issue in this repository.

## Reference

- [Installation](docs/site/getting-started/install.md)
- [Concepts](docs/site/concepts/index.md)
- [Engineering documentation](docs/st3/README.md)
- [UI component Storybook](https://storybook.compounding.tech): Fractal UI stories; [deployment setup](docs/development.md#storybook-deployment).

[Contributing](docs/development.md#contributing) covers source builds, checks, and repository layout.
This repository also carries the previous generation; see [its guide](README.st2.md).
