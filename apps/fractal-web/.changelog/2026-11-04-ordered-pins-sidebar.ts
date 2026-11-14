import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Show ordered Pins in the sidebar',
  summary: 'Existing ordered Pins appear above the agent roster, with actions to open or unpin each item. Agent row menus can pin or unpin agents and their missions while Pins are editable; empty sections and retired subjects stay hidden, and creating a Pins arrangement remains disabled by default.',
  kind: 'feature',
} satisfies ChangelogEntrySource
