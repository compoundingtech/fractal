import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'See what tools are doing',
  summary: 'You can tell when an agent is waiting, sending a message, planning work, or typing into a terminal. If an interruption skips a tool, it is labelled Skipped rather than failed. Results from searches, web browsing, and other agents have their own views for reading the details.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
