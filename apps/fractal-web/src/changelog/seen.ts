import type { ChangelogEntry } from './contract.ts'

/** Origin-local storage is device-local; deployment is a stable channel, never a build version. */
export interface ChangelogSeen {
  readonly visited: boolean
  readonly entryIds: readonly string[]
}
export const initialSeen: ChangelogSeen = { visited: false, entryIds: [] }
export const seenKey = (deployment: string): string => `changelog.fractal-web.${deployment}`
export const unseenEntries = (entries: readonly ChangelogEntry[], seen: ChangelogSeen): readonly ChangelogEntry[] => {
  const ids = new Set(seen.entryIds)
  return entries.filter((entry) => !ids.has(entry.id))
}
/** Union preserves acknowledgements across rollbacks and non-linear build membership. */
export const acknowledgeEntryIds = (entryIds: readonly string[], seen: ChangelogSeen): ChangelogSeen => ({
  visited: true,
  entryIds: [...new Set([...seen.entryIds, ...entryIds])],
})
