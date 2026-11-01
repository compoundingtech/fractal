import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep pending questions reachable',
  summary: 'Conversation alerts render reliably and Open terminal reaches the selected seat. Pending OMP questions also remain visible when native optional fields contain null placeholders.',
  kind: 'fix',
} satisfies ChangelogEntrySource
