import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Say what the connection and the seat actually report',
  summary: 'A revoked browser credential now shows one "Access revoked" band with how to pair again and no Reconnect, and every surface uses the same sentence. While disconnected or before this device\'s grants are read, alert answers say updates are paused instead of calling the device read-only. A seat that reports no turn in progress shows its own state instead of a running spinner from its last open run, a thread tab without a title says whether it is loading or unavailable, and a run status no longer repeats itself.',
  kind: 'fix',
} satisfies ChangelogEntrySource
