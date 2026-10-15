import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Service startup shows readiness',
  summary: 'Service rows show the recorded service name and process state, with a readiness chip instead of a generic OK. A readiness timeout is shown in a warning colour; a service whose ready check passed says Ready.',
  kind: 'fix',
} satisfies ChangelogEntrySource
