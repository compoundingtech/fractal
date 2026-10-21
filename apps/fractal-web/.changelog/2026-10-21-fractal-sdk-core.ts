import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Shared client SDK beyond the browser',
  summary: 'Fractal uses an app-independent Effect SDK that other clients can share. Browser startup and visible-view priorities stay in the web app, while follows retain the same shared connection and freshness handling. Saved conversations and wire formats are unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
