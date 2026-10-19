import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Changes beside your thread',
  summary: 'Captured changes dock beside the thread with compact pane headers and a resizable, keyboard-accessible split that remembers its width for each workspace. The dock normally ranges from 320 to 720 px and narrows when needed to keep the thread and composer usable without replacing your saved width. Changes loads only when opened and closes without leaving the conversation.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
