import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Task requests read like instructions',
  summary: 'Recorded delegated-task details show task names, agent types and formatted instructions instead of quoted data. Launch status remains a recorded snapshot, not a live task monitor.',
  kind: 'fix',
} satisfies ChangelogEntrySource
