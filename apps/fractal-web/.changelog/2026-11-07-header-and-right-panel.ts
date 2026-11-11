import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'A calmer conversation header and a right panel with three tabs',
  summary: 'The header keeps the title, the terminal toggle, one overflow menu and the panel toggle. Agent actions and View KDL are in the overflow menu, Show all system events is in Settings, and the right panel has Details, Changes and Resources tabs with counts and whole-row resources. Changes counts update independently of unrelated conversation frames, without rerendering the inspector.',
  kind: 'feature',
} satisfies ChangelogEntrySource
