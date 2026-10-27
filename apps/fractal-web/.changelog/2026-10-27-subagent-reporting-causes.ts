import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Understand why a subagent report is missing',
  summary: 'Subagent details distinguish a harness that does not report subagents from an older daemon that omits the report. Claude, Codex and OMP empty reports remain a reported zero; pending count tooltips say “Not reported yet”.',
  kind: 'fix',
} satisfies ChangelogEntrySource
