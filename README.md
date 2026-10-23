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
- [Talking to agents](docs/site/guides/talking-to-agents.md): terminal, phone, CLI and attachments.
- [Agent lifecycle](docs/site/guides/agent-lifecycle.md): create, restart and suspend agents.
- [Upgrading](docs/site/guides/upgrading.md): choose a build and check recovery.
- [GitHub integration](docs/site/guides/github-integration.md): repository intake and review.
- [Troubleshooting](docs/site/guides/troubleshooting.md): diagnose startup and health problems.
- [iOS app](docs/site/guides/ios-app.md): build and pair the phone client.

## Reference

- [Installation](docs/site/getting-started/install.md)
- [Concepts](docs/site/concepts/index.md)
- [Engineering documentation](docs/st3/README.md)

[Contributing](docs/development.md#contributing) covers source builds, checks, and repository layout.
This repository also carries the previous generation; see [its guide](README.st2.md).
