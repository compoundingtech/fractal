import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Browse agents in folders and a responsive sidebar',
  summary:
    'Agents are grouped in folders with consistent indentation, and on narrow screens the sidebar opens as a drawer that assistive technology announces. Reported context is available in the agent row popover, without a ring around its status icon. Reopening a terminal that has already loaded keeps its output visible.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
