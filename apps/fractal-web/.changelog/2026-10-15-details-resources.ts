import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Agent Details and Resources beside the conversation',
  summary: 'The agent inspector opens on a Details tab that shows what the session reports, explains missing details and marks context percentages derived from token counts as estimates. Spend has Refresh, and failed reads have their own Try again action. On wider screens, Resources keeps the conversation usable, starts with the agent\'s To-dos and keeps its last reading for quick reopening.',
  kind: 'feature',
} satisfies ChangelogEntrySource
