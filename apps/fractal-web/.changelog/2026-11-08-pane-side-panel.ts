import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Each pane has its own side panel',
  summary: 'The right panel now lives inside each pane and shows that pane’s subject, opening by default only when the pane is wide enough. Split panes keep their own panels, the panel remembers its tab for each kind, and on a phone it rises as a bottom sheet. Switching workspace tabs preserves the pane’s surfaces while keeping each selected tab linked to its accessible panel. Tab actions now live on the tab: right-click, Shift+F10 or the context-menu key opens Split right, Split down, Close and Close others, and a close button appears on hover or keyboard focus. The “+” menu lists surfaces to open and the open tabs to go to, so a pane’s strip ends with just the agent’s More actions, the terminal toggle and the side panel toggle. A phone keeps both lists inside More actions.',
  kind: 'feature',
} satisfies ChangelogEntrySource
