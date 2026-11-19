import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Explain expired browser access',
  summary: 'When the daemon rejects browser credentials, the roster, folders, alerts, missions and other read surfaces explain that access has expired or was revoked instead of suggesting a reconnect or retry. Pair the device again to restore access.',
  kind: 'fix',
} satisfies ChangelogEntrySource
