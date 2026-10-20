import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Two-line agent rows, with subagents under their parent',
  summary: "Each agent row shows its status, last activity and usage on two lines, with a distinct glyph for each status. Subagents appear under the agent that started them, and agents outside your folders are grouped by project without changing your folders. A subagent's row menu always opens its details, even when it has no conversation to show.",
  kind: 'improvement',
} satisfies ChangelogEntrySource
