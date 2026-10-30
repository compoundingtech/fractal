import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Service startup shows readiness',
  summary: 'Recorded service details show the service name, process state and readiness evidence, including successful ready checks and readiness timeouts, within the existing inspector.',
  kind: 'fix',
} satisfies ChangelogEntrySource
