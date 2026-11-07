import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Explain expired browser access',
  summary: 'When the daemon refuses browser access, the agent roster now explains that access has expired or was revoked instead of loading indefinitely. Pair the device again to restore access.',
  kind: 'fix',
} satisfies ChangelogEntrySource
