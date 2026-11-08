import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'See what is new since you last looked',
  summary: 'Fractal remembers where you last read each conversation. Coming back, a line marks the first new message, and tabs and sidebar rows count new messages from the agent, or show a dot when only newer messages are known. Tool calls and your own messages never count. The position stays in this browser, per person.',
  kind: 'feature',
} satisfies ChangelogEntrySource
