import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Search inside a conversation',
  summary: 'Cmd or Ctrl+F opens a search box over the conversation that finds text in messages and tool output, opens the rows it lands on, steps with Enter and Shift+Enter, and pages older messages in on demand to search full history; press it again for the browser\'s Find. The line above the transcript is gone: earlier messages now load as you scroll up.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
