import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Threads stay with their Inbox item',
  summary: 'Start a thread inside an Inbox item when it has a responsible agent. Its draft stays with that item, and replies can continue after the thread is closed.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
