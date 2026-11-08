import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'One top bar for tabs and the conversation',
  summary: 'The separate header line is gone: the tab strip is the top bar, with the conversation\'s menu, terminal and panel buttons at its end. The selected tab is marked by a line along its top, a tab whose agent needs you shows an amber status ring instead of an outline, and resting on a tab shows the same card as the sidebar.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
