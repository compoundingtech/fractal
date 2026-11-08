import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See what is new since you last looked',
  summary: 'Fractal remembers where you last read each conversation, in this browser and per person, and marks the first new message with a line when you come back. Tabs and sidebar rows count new messages from the agent, or show a dot when only newer messages are known. Tool calls and your own messages never count.',
  kind: 'feature',
} satisfies ChangelogEntrySource
