import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Two-line agent rows, with subagents under their parent',
  summary: 'Each agent row shows its status, last activity and usage on two lines, with a distinct glyph for each status. Subagents appear under the agent that started them, and agents outside your folders are grouped by project without changing your folders. Subagent rows show their status but do not open a conversation yet.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
