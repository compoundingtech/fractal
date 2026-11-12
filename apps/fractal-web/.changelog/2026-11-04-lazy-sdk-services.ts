import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Load terminal and editing services when needed',
  summary: 'Terminal and mutation SDK services load on first use instead of during startup, leaving read-only views unaffected. The first terminal or editing action may briefly wait for its service; if loading fails, retrying the action can load it again without reloading the app.',
  kind: 'fix',
} satisfies ChangelogEntrySource
