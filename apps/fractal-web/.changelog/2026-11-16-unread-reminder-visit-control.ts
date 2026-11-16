import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Read conversations only in the foreground',
  summary: 'Visible replies in a focused conversation now advance natural read history, never from background tabs, list rows or behind a dialog. Private unread reminder controls and sticky-visit behavior are implemented behind a separate port; they remain unavailable on gateways until the authoritative reminder and return-anchor APIs are present.',
  kind: 'feature',
} satisfies ChangelogEntrySource
