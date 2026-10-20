import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Agent Details and Resources beside the conversation',
  summary: 'The agent inspector opens on a Details tab with native observations, concise not-reported summaries and token-derived context percentages explicitly marked as estimates while preserving harness-reported percentages. On wider screens, Resources opens as a side panel that keeps the conversation usable, starts with the agent\'s Todos and reopens instantly with its last reading.',
  kind: 'feature',
} satisfies ChangelogEntrySource
