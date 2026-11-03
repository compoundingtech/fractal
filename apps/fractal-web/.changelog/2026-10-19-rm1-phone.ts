import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Navigate phone workspaces with a drawer and tabs',
  summary: 'On phones, open agents in a focus-trapped drawer and switch between Thread, Changes and open resources with accessible tabs. The drawer closes after agent selection and returns focus to its trigger, while the composer stays within the visible viewport when the keyboard opens. Drafts and reading positions remain intact when resizing between phone and desktop layouts.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
