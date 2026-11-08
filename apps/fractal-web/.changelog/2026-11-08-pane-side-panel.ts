import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Each pane has its own side panel',
  summary: 'The right panel now lives inside each pane and shows that pane’s subject, opening by default only when the pane is wide enough; split panes keep their own panels, each remembers its tab per kind, and on a phone it rises as a bottom sheet. Switching workspace tabs preserves the pane’s surfaces, and each tab carries its own actions: right-click, Shift+F10 or the context-menu key offers Split right, Split down, Close and Close others, and a close button appears on hover or keyboard focus. The “+” menu lists surfaces to open and open tabs to go to, so a pane’s strip ends with the agent’s More actions, the terminal toggle and the side panel toggle, while a phone keeps both lists in More actions.',
  kind: 'feature',
} satisfies ChangelogEntrySource
