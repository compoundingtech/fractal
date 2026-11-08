import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Each pane has its own side panel',
  summary: 'The right panel now lives inside each pane and shows that pane’s subject, opening by default only when the pane is wide enough. Split panes keep their own panels, the panel remembers its tab for each kind, and on a phone it rises as a bottom sheet. Switching workspace tabs preserves the pane’s surfaces while keeping each selected tab linked to its accessible panel.',
  kind: 'feature',
} satisfies ChangelogEntrySource
