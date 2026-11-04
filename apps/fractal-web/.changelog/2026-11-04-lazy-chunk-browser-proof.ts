import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: "Cover lazy chunk recovery in three browser engines",
  summary: "A production Vite proof covers aborted chunk, static dependency and stylesheet loads in Chromium, Firefox and WebKit, including draft and selection persistence.",
  kind: 'improvement',
} satisfies ChangelogEntrySource
