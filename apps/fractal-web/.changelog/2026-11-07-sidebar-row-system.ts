import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Calmer agent rows that open their missions',
  summary: 'Sidebar rows use color only when you need to act, amber for an agent asking you something and red for a failure, and an agent waiting with no question now reads Waiting. A mission shows a small progress ring, steps done, the current step and its name, and clicking that part of the row opens the mission. Each further mission an agent works on gets its own row beneath it.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
