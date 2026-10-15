import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Close tool details with Escape',
  summary: 'Press Escape in tool details to close the panel and return keyboard focus to the control that opened it, just like the Close button. Other keys leave the panel open, and controls that already handle Escape keep their own behavior. The panel stays beside the conversation without an extra container.',
  kind: 'fix',
} satisfies ChangelogEntrySource
