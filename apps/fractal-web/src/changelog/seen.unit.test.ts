import { describe, expect, it } from 'vitest'
import type { ChangelogEntry } from './contract.ts'
import { acknowledgeEntryIds, initialSeen, seenKey, unseenEntries } from './seen.ts'

const entries: readonly ChangelogEntry[] = [
  { id: '2026-10-11-context', date: '2026-10-11', visibility: 'public', title: 'Context details', summary: 'Conversation context is easier to inspect.' },
  { id: '2026-10-10-tree', date: '2026-10-10', visibility: 'public', title: 'Tree alignment', summary: 'Nested rows align at each level.' },
]
describe('changelog seen membership', () => {
  it('first visit is unacknowledged and opening acknowledges every included id', () => {
    expect(initialSeen.visited).toBe(false)
    expect(unseenEntries(entries, initialSeen)).toEqual(entries)
    const opened = acknowledgeEntryIds(entries.map((entry) => entry.id), initialSeen)
    expect(opened.visited).toBe(true)
    expect(unseenEntries(entries, opened)).toEqual([])
  })
  it('does not use build versions or timestamps to infer unseen membership', () => {
    const seen = { visited: true, entryIds: [entries[0]!.id] }
    expect(unseenEntries(entries, seen)).toEqual([entries[1]])
    expect(unseenEntries(entries, acknowledgeEntryIds([entries[1]!.id], seen))).toEqual([])
  })
  it('retains acknowledgements through rollbacks and skips', () => {
    const opened = acknowledgeEntryIds(entries.map((entry) => entry.id), initialSeen)
    const rollback = acknowledgeEntryIds([entries[1]!.id], opened)
    expect(rollback.entryIds).toEqual(opened.entryIds)
    expect(unseenEntries(entries, rollback)).toEqual([])
    expect(acknowledgeEntryIds([entries[0]!.id], rollback).entryIds).toEqual(opened.entryIds)
  })
  it('separates live, main and stable deployment cursors', () => {
    expect(new Set(['live', 'main', 'stable'].map(seenKey)).size).toBe(3)
    expect(seenKey('live')).not.toContain('0.1.0')
  })
})
