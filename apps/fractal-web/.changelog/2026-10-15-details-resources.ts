import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Agent Details and Resources beside the conversation',
  summary: 'The agent inspector opens on a Details tab that shows what the session reports, with short summaries for anything it doesn\'t, and marks context percentages derived from token counts as estimates. On wider screens, Resources opens as a side panel that keeps the conversation usable, starts with the agent\'s Todos and reopens instantly with its last reading.',
  kind: 'feature',
} satisfies ChangelogEntrySource
