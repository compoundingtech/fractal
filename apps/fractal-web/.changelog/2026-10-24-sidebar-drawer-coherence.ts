import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'One sidebar on both sides of the breakpoint',
  summary: 'The agents drawer uses the docked sidebar\'s header, with the wordmark and alerts, and its collapse button in the same place. Controls get larger touch targets only on touch screens, not because the window is narrow. In dark mode the drawer sits above the dimmed thread with a visible edge. Closing the drawer moves focus back to the button that opened it.',
  kind: 'fix',
} satisfies ChangelogEntrySource
