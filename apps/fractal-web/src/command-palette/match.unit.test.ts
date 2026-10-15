import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { matchFields, matchToken, queryTokens, type MatchTier } from './match.ts'

type Field = 'agent' | 'session' | 'folder' | 'host'
const corpus = JSON.parse(readFileSync(new URL('../../../../fixtures/ui/command-palette-matching.json', import.meta.url), 'utf8')) as {
  readonly fields: readonly Field[]
  readonly cases: readonly { readonly query: string; readonly fields: Record<Field, string>; readonly matches: boolean; readonly tier?: MatchTier; readonly highlights?: Partial<Record<Field, string>> }[]
  readonly ranking: readonly { readonly query: string; readonly order: readonly string[]; readonly rows: readonly Record<Field, string>[] }[]
}
const tierRank: Record<MatchTier, number> = { prefix: 3, initials: 2, substring: 1 }

/** `[UI] [de]sign`: bracketed runs make highlight expectations readable in the corpus. */
const bracket = (text: string, positions: readonly number[]) => {
  const marked = new Set(positions)
  return Array.from(text, (letter, index) => `${marked.has(index) && !marked.has(index - 1) ? '[' : ''}${letter}${marked.has(index) && !marked.has(index + 1) ? ']' : ''}`).join('')
}

describe('shared palette matching corpus', () => {
  it.each(corpus.cases.map((entry, index) => [index + 1, entry] as const))('case %i', (_, entry) => {
    const fields = corpus.fields.map(field => entry.fields[field])
    const result = matchFields(entry.query, fields)
    expect(result !== undefined).toBe(entry.matches)
    if (result === undefined) return
    if (entry.tier !== undefined) {
      const best = queryTokens(entry.query).flatMap(token => fields.flatMap(field => matchToken(token, field)?.tier ?? [])).toSorted((left, right) => tierRank[right] - tierRank[left])[0]
      expect(best).toBe(entry.tier)
    }
    corpus.fields.forEach((field, index) => expect(bracket(entry.fields[field], result.highlights[index]!)).toBe(entry.highlights?.[field] ?? entry.fields[field]))
  })
  it.each(corpus.ranking.map(entry => [entry.query, entry] as const))('ranks %j', (_, entry) => {
    const ranked = entry.rows.map(row => ({ row, score: matchFields(entry.query, corpus.fields.map(field => row[field]))!.score })).toSorted((left, right) => right.score - left.score)
    expect(ranked.map(({ row }) => row.agent)).toEqual(entry.order)
  })
})
