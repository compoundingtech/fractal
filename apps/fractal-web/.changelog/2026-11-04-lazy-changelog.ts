import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep Recent changes out of workspace startup',
  summary: 'Recent changes loads separately from the agent roster and threads. Its content warms after the roster is ready and when the Changes button is hovered or focused; opening remains immediate, with a stable panel size and contained load failures.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
