import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Understand why a subagent report is missing',
  summary: 'Subagent details distinguish a harness without subagent reporting from a running Claude, Codex or OMP session on an older daemon. Starting observing sessions show “Not reported yet” until their first report arrives. Explicit empty reports still show zero subagents.',
  kind: 'fix',
} satisfies ChangelogEntrySource
