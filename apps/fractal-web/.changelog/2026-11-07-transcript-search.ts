import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Search inside a conversation',
  summary: 'Cmd or Ctrl+F opens a search box over the conversation that finds text in messages and tool output, opens the rows it lands on, and steps with Enter and Shift+Enter; press it again for the browser\'s Find. Search full history pages older messages in on demand. The line above the transcript is gone: earlier messages now load as you scroll up.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
