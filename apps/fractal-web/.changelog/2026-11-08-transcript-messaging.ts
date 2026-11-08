import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Messages in the transcript read as one system',
  summary: 'Mail, interjections and IRC are cards with an avatar, a status dot and one head per run of messages from the same sender; subagent and background-job results are single rows, with a red edge and red status when they fail. A head names its recipient only when it is someone other than this agent, and shows its time only when the minute changes. Your GitHub avatar appears on your messages, light-theme cards are white, and more than four attachments collapse to three and "+N more".',
  kind: 'improvement',
} satisfies ChangelogEntrySource
