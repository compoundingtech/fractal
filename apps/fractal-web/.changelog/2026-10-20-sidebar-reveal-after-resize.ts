import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Selected agents stay reachable after resizing the sidebar',
  summary: 'After narrowing the sidebar, opening an agent inside an offscreen folder now scrolls its row into view instead of leaving it hidden.',
  kind: 'fix',
} satisfies ChangelogEntrySource
