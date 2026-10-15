import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Switch agents with the command palette',
  summary: 'Press Cmd+K (Ctrl+K elsewhere) or the header Switch button to search agents by name, folder or host and open one with Enter; agents that need you stay pinned first, followed by recent visits and missions that open through their working agent. Commands switches to shell actions such as terminal, details, sidebar, theme and link copy; switching modes resets keyboard focus, focused details, shortcut chips and Current/Recent marks remain readable in both themes, and recents keep only stable target refs under a non-identifying connection scope. Escape closes the palette and returns focus to where you were.',
  kind: 'feature',
} satisfies ChangelogEntrySource
