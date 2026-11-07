import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Preserve development gateway rejection details',
  summary: 'The development proxy now forwards the gateway status and diagnostic body when a collections WebSocket upgrade is rejected. Expired-credential responses retain their 403 status and credential-expired explanation instead of losing the diagnostic.',
  kind: 'fix',
} satisfies ChangelogEntrySource
