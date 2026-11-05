import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep workspace panels out of startup',
  summary: 'Workspace menus and details load separately from the first roster and thread. Their content warms after the roster paints and when controls are hovered or focused, with stable-size panels and local recovery if a chunk cannot load.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
