import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep keyboard focus when closing the agent inspector',
  summary: 'Closing the agent inspector beside a conversation now moves focus to the Toggle right panel button, so you can press Enter to reopen it. This works with the close button, with the keyboard, and with Enter on the resize handle. Switching to another agent still leaves focus where it was.',
  kind: 'fix',
} satisfies ChangelogEntrySource
