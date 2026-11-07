import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Ship only English React Aria strings',
  summary: 'React Aria controls, keyboard drag instructions and announcements use English, including in non-English browsers, and no longer download other translations. English-only Aria locale support is intentional until the app has real i18n; adding languages requires expanding the build plugin locale list and every root I18nProvider together. Native browser-formatted timestamps are unchanged.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
