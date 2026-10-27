# Product embed

Build from the repository root:

```sh
pnpm --dir apps/fractal-web build:embed
```

Serve `apps/fractal-web/dist-embed/` as static files. The entry is `embed.html`;
all assets are relative, so a pinned Fractal revision can be deployed under a
website subdirectory. Build output is not committed.

Iframe URLs support `embed.html?theme=light#web`, `#web-narrow`, `#claude-code`,
and `#codex` (also `theme=dark`). Web views reuse the real LiveAgentWorkspace,
DataSourceProvider and AtomRegistry over the fixed-clock fixture source.
Terminal views decode bundled TerminalScreen JSON and reuse DomTerminal.
No gateway or telemetry runtime is imported. No frames are fetched at runtime.

The embed build replaces browser persistence with an iframe-local memory adapter:
it neither reads nor writes `wf.ui@1`, and storage events cannot synchronize sibling
frames. Agent and pane navigation stays in memory, preserving the embed URL and the
host page's Back history. Reload resets the fixture selection. The workspace uses
its measured container dimensions; `#web-narrow` selects the phone layout even in a
wide iframe. Document-only CSS belongs to the entry, not the Storybook surface.

After building, run the focused production iframe proof in the web shell:

```sh
nix develop .#web --command pnpm --dir apps/fractal-web test:embed
```

The proof covers opposite-theme sibling frames, real-app storage isolation,
container responsiveness, private navigation and reload reset, accessible pending
states, and all five browser transport APIs. Set `EMBED_SCREENSHOT_DIR` to retain
the eight fragment/theme screenshots.

Claude Code and Codex currently show **Capture pending**, not imitations.
`captures/provenance.json` records version pins and the lack of captures.
To refresh, install `pyte==0.8.2` in a task-local virtual environment, initialize
a scratch Git repository, and use the sanctioned PATH wrappers:

```sh
python apps/fractal-web/scripts/capture-embed.py claude-code \
  --repo "$task_tmp/repo" --session fractal.embed.claude \
  --output "$task_tmp/claude-code.json"
python apps/fractal-web/scripts/capture-embed.py codex \
  --repo "$task_tmp/repo" --session fractal.embed.codex \
  --output "$task_tmp/codex.json"
```

The script uses a named ephemeral PTY, fixed size, one read-only fixture prompt,
the operator's HOME and normal fleet account routing; it never copies credentials
or logs screen bytes. It refuses version drift and login screens and tears down
its own session. A launcher refusal must be reported, not bypassed. Never log in
as part of capture. Record the scratch repo and session in TEMPORARY_FILES.md.
Review frames for account email, organization, credentials, internal paths and
usage before copying reviewed frames to `captures/{claude-code,codex}.json`.
Keep provenance alongside the frame. Terminal data must come from the CLI, never
from manually authored terminal runs. Each capture must finish within 15 minutes.
