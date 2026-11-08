import { describe, expect, it } from 'vitest'
import { flatten, fuzzy, fuzzyWords, parseQuery, pushRecent, rank, sectionJump, targetFromEvent, type PaletteItem, type PaletteSection } from './core.ts'

const sections: readonly PaletteSection[] = [
  { id: 'people', label: 'People', prefix: '@' },
  { id: 'projects', label: 'Projects', prefix: '#' },
  { id: 'commands', label: 'Commands', prefix: '>' },
]
const item = (section: string, label: string, recency = 0, search?: string): PaletteItem<string> =>
  ({ id: `${section}/${label}`, section, label, recency, action: label, ...(search === undefined ? {} : { search }) })

describe('fuzzy matching', () => {
  it('matches substrings and word starts, never letters scattered inside words', () => {
    expect(fuzzy('atl', 'Atlas Builder')).toBeDefined()
    expect(fuzzy('ab', 'Atlas Builder')).toBeDefined()
    expect(fuzzy('rel', 'Rekey Worker')).toBeUndefined()
    expect(fuzzy('zz', 'Atlas Builder')).toBeUndefined()
    expect(fuzzy('atlas', 'Atlas Builder')!).toBeGreaterThan(fuzzy('ab', 'Atlas Builder')!)
    expect(fuzzy('lease', 'Release Captain')).toBeDefined()
    expect(fuzzy('bu', 'Atlas Builder')!).toBeGreaterThan(fuzzy('bu', 'Rebuild atlas')!)
  })

  it('matches several words each against the text', () => {
    expect(fuzzy('builder atlas', 'example/atlas/builder')).toBeUndefined()
    expect(fuzzyWords('builder atlas', 'example/atlas/builder')).toBeDefined()
    expect(fuzzyWords('atlas builder', 'example/atlas/builder')).toBeDefined()
    expect(fuzzyWords('atlas zz', 'example/atlas/builder')).toBeUndefined()
  })
})

describe('ranking', () => {
  const items = [
    item('projects', 'Atlas', 1),
    item('people', 'Ada', 5),
    item('people', 'Atlas Builder', 9),
    item('commands', 'Toggle theme', 0),
    item('people', 'Hidden', 0, ''),
  ]

  it('lists each section by recency for an empty query, in section order', () => {
    const groups = rank(items, { query: '', sections })
    expect(groups.map(group => group.section.id)).toEqual(['people', 'projects', 'commands'])
    expect(groups[0]!.items.map(row => row.item.label)).toEqual(['Atlas Builder', 'Ada', 'Hidden'])
  })

  it('orders matches by section, then match, then recency', () => {
    expect(flatten(rank(items, { query: 'atlas', sections })).map(row => row.label)).toEqual(['Atlas Builder', 'Atlas'])
    expect(flatten(rank([item('people', 'Atlas'), item('people', 'Atlas', 3)], { query: 'atlas', sections })).map(row => row.recency)).toEqual([3, 0])
  })

  it('ranks a typed query in its own section order, as the TUI does, while the empty list keeps the given one', () => {
    const typed = ['commands', 'projects', 'people']
    expect(rank(items, { query: 't', sections, typedOrder: typed }).map(group => group.section.id)).toEqual(['commands', 'projects', 'people'])
    expect(rank(items, { query: '', sections, typedOrder: typed }).map(group => group.section.id)).toEqual(['people', 'projects', 'commands'])
  })

  it('puts a section first when one of its rows leads, such as a command typed as it begins', () => {
    const leads = (row: PaletteItem<string>, query: string) => row.section === 'commands' && row.label.toLowerCase().startsWith(query.toLowerCase())
    expect(rank(items, { query: 't', sections }).map(group => group.section.id)).toEqual(['people', 'projects', 'commands'])
    expect(rank(items, { query: 'to', sections, leads }).map(group => group.section.id)).toEqual(['commands'])
    expect(rank(items, { query: 't', sections, leads }).map(group => group.section.id)).toEqual(['commands', 'people', 'projects'])
  })

  it('scopes by prefix', () => {
    const parsed = parseQuery('#atl', sections)
    expect(parsed).toEqual({ section: 'projects', text: 'atl' })
    expect(flatten(rank(items, { query: parsed.text, sections, section: parsed.section })).map(row => row.label)).toEqual(['Atlas'])
    expect(parseQuery('atl', sections)).toEqual({ section: undefined, text: 'atl' })
  })
})

describe('keys', () => {
  const enter = { key: 'Enter', altKey: false, shiftKey: false, metaKey: false, ctrlKey: false }
  it('maps modifiers to open targets, falling back to here', () => {
    expect(targetFromEvent(enter, undefined, true)).toBe('here')
    expect(targetFromEvent({ ...enter, metaKey: true }, undefined, true)).toBe('tab')
    expect(targetFromEvent({ ...enter, ctrlKey: true }, undefined, false)).toBe('tab')
    expect(targetFromEvent({ ...enter, altKey: true }, undefined, true)).toBe('right')
    expect(targetFromEvent({ ...enter, altKey: true, shiftKey: true }, undefined, true)).toBe('below')
    expect(targetFromEvent({ ...enter, altKey: true }, ['here', 'tab'], true)).toBe('here')
    expect(targetFromEvent({ ...enter, key: 'a' }, undefined, true)).toBeUndefined()
  })

  it('jumps sections with Alt and a digit key, leaving plain digits to the text', () => {
    expect(sectionJump({ ...enter, key: '¡', code: 'Digit1', altKey: true })).toBe(0)
    expect(sectionJump({ ...enter, key: '1', code: 'Digit1' })).toBeUndefined()
  })

  it('keeps a bounded recent list, newest first', () => {
    expect(pushRecent(['a', 'b', 'c'], 'b', 3)).toEqual(['b', 'a', 'c'])
    expect(pushRecent(['a', 'b', 'c'], 'd', 3)).toEqual(['d', 'a', 'b'])
  })
})
