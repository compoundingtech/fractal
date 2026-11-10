import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Open missions from a sidebar tab',
  summary: 'Missions now has its own sidebar tab next to Agents and Alerts, with a count, and tabs carry icons. Selecting a mission focuses it in the main area through its own link. Ad hoc work stays folded until someone must act on it. Expired device access keeps its pairing notice rather than appearing as a generic load failure.',
  kind: 'feature',
} satisfies ChangelogEntrySource
