import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep the agent sidebar in place',
  summary: 'Selecting or focusing an agent no longer shifts the sidebar left and clips its rows. Sidebar tabs keep fitting the available width, including large counts and mission alerts.',
  kind: 'fix',
} satisfies ChangelogEntrySource
