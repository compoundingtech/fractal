import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Understand why a subagent report is missing',
  summary: 'Subagent details now say why a subagent list is missing. They separate a harness that does not report subagents, including OpenCode, Pi, unknown harnesses and child sessions, from a running Claude, Codex or OMP session on an older daemon. Starting Claude, Codex and OMP sessions show “Not reported yet” until their first report arrives. Empty reports from these harnesses still show zero subagents.',
  kind: 'fix',
} satisfies ChangelogEntrySource
