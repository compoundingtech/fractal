import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'One sidebar on both sides of the breakpoint',
  summary: 'The agents drawer shares the dock header, wordmark and alerts, and it has an elevated surface with a visible edge. Controls use larger targets only for coarse pointers, not narrow windows. When the drawer closes, the application returns focus to the button that opened it.',
  kind: 'fix',
} satisfies ChangelogEntrySource
