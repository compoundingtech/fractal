/**
 * The palette's pure core: query parsing with prefix scopes, a fuzzy ranker, sections and open targets.
 * Rendering, data and what an item does are the host's.
 */

export interface PaletteSection {
  readonly id: string
  readonly label: string
  /** A leading character that scopes the list to this section, such as `>` or `@`. */
  readonly prefix?: string | undefined
}

export interface PaletteItem<A = unknown> {
  readonly id: string
  readonly section: string
  readonly label: string
  readonly detail?: string | undefined
  /** What the query matches against; the label when absent. An empty string matches only an empty query. */
  readonly search?: string | undefined
  /** Higher is more recent. */
  readonly recency?: number | undefined
  readonly action: A
}

export type OpenTarget = 'here' | 'tab' | 'right' | 'below'

export type PaletteMode =
  | { readonly _tag: 'Search' }
  | { readonly _tag: 'Naming'; readonly intent: 'create' | 'rename' | 'duplicate'; readonly initial: string; readonly subject?: string | undefined }

export interface ParsedQuery {
  readonly section: string | undefined
  readonly text: string
}

/** A leading prefix scopes the list to its section; the rest is the query. */
export const parseQuery = (input: string, sections: readonly PaletteSection[]): ParsedQuery => {
  const trimmed = input.trimStart()
  for (const section of sections) {
    if (section.prefix !== undefined && section.prefix !== '' && trimmed.startsWith(section.prefix)) {
      return { section: section.id, text: trimmed.slice(section.prefix.length).trim() }
    }
  }
  return { section: undefined, text: input.trim() }
}

const isAlphanumeric = (char: string | undefined) => char !== undefined && /[\p{L}\p{N}]/u.test(char)

/**
 * How well `query` matches `text`: higher is better, `undefined` is no match. A query matches as a
 * substring, or letter by letter where each letter either follows the last one or starts a word ("ab"
 * finds "Atlas Builder"); letters scattered inside words do not match.
 */
export const fuzzy = (query: string, text: string): number | undefined => {
  const wanted = [...query.toLowerCase()].filter(char => !/\s/u.test(char))
  if (wanted.length === 0) return 0
  const lower = text.toLowerCase()
  const chars = [...lower]
  const startsWord = (index: number) => index === 0 || !isAlphanumeric(chars[index - 1])
  const at = lower.indexOf(wanted.join(''))
  if (at !== -1) {
    const index = [...lower.slice(0, at)].length
    return 1000 + (startsWord(index) ? 50 : 0) - index
  }
  let score = 0
  let from = 0
  let last: number | undefined
  for (const char of wanted) {
    let found: number | undefined
    for (let index = from; index < chars.length; index += 1) {
      if (chars[index] === char && ((last !== undefined && last + 1 === index) || startsWord(index))) {
        found = index
        break
      }
    }
    if (found === undefined) return undefined
    score += last !== undefined && last + 1 === found ? 5 : 3
    last = found
    from = found + 1
  }
  return score * 10 - chars.length
}

/** `query` against `text`, or, when it holds several words, every word against it. */
export const fuzzyWords = (query: string, text: string): number | undefined => {
  const whole = fuzzy(query, text)
  if (whole !== undefined) return whole
  const words = query.split(/\s+/u).filter(word => word !== '')
  if (words.length < 2) return undefined
  let total = 0
  for (const word of words) {
    const score = fuzzy(word, text)
    if (score === undefined) return undefined
    total += score
  }
  return total
}

export type Ranker = (query: string, text: string) => number | undefined

export interface RankedItem<A> {
  readonly item: PaletteItem<A>
  readonly score: number
}

export interface RankedGroup<A> {
  readonly section: PaletteSection
  readonly items: readonly RankedItem<A>[]
}

/** A row that jumps its section to the top while `query` is typed, such as a command typed as it begins. */
export type Leads<A> = (item: PaletteItem<A>, query: string) => boolean

export interface RankOptions<A> {
  readonly query: string
  readonly sections: readonly PaletteSection[]
  readonly section?: string | undefined
  readonly ranker?: Ranker | undefined
  /** Section ids in the order a typed query ranks them; the given order when absent. Missing ids follow. */
  readonly typedOrder?: readonly string[] | undefined
  /** Section ids in the order the empty query lists them, likewise. `sections` stays the order of Alt+digit. */
  readonly emptyOrder?: readonly string[] | undefined
  readonly leads?: Leads<A> | undefined
}

const orderedSections = <A>(items: readonly PaletteItem<A>[], options: RankOptions<A>, query: string): readonly PaletteSection[] => {
  const order = query === '' ? options.emptyOrder : options.typedOrder
  const position = (id: string) => {
    const at = order?.indexOf(id) ?? -1
    return at === -1 ? Number.MAX_SAFE_INTEGER : at
  }
  const leading = new Set(options.leads === undefined || query === '' ? [] : items.filter(item => options.leads!(item, query)).map(item => item.section))
  return options.sections
    .map((section, index) => ({ section, index }))
    .toSorted((left, right) =>
      Number(leading.has(right.section.id)) - Number(leading.has(left.section.id))
      || position(left.section.id) - position(right.section.id)
      || left.index - right.index)
    .map(row => row.section)
}

/**
 * An empty query lists each section by recency, in the given order (or `emptyOrder`). A query keeps only matches, by
 * section (`typedOrder`, with a section whose row `leads` first), then match quality, then recency.
 * `section` scopes to one section.
 */
export const rank = <A>(items: readonly PaletteItem<A>[], options: RankOptions<A>): readonly RankedGroup<A>[] => {
  const ranker = options.ranker ?? fuzzyWords
  const query = options.query.trim()
  const sections = orderedSections(items, options, query)
  return sections.flatMap(section => {
    if (options.section !== undefined && options.section !== section.id) return []
    const ranked = items.flatMap((item): RankedItem<A>[] => {
      if (item.section !== section.id) return []
      const search = item.search ?? item.label
      if (query === '') return [{ item, score: 0 }]
      if (search === '') return []
      const score = ranker(query, search)
      return score === undefined ? [] : [{ item, score }]
    }).toSorted((left, right) => right.score - left.score || (right.item.recency ?? 0) - (left.item.recency ?? 0))
    return ranked.length === 0 ? [] : [{ section, items: ranked }]
  })
}

/** The groups as one list, for selection by index. */
export const flatten = <A>(groups: readonly RankedGroup<A>[]): readonly PaletteItem<A>[] =>
  groups.flatMap(group => group.items.map(row => row.item))

export interface ModifierKeys {
  readonly key: string
  readonly altKey: boolean
  readonly shiftKey: boolean
  readonly metaKey: boolean
  readonly ctrlKey: boolean
}

export const isMac = (): boolean => typeof navigator !== 'undefined' && /Mac|iPhone|iPad/u.test(navigator.platform || navigator.userAgent)

/**
 * Enter opens here; Mod+Enter in a new tab; Alt+Enter in a split to the right; Alt+Shift+Enter in one
 * below. A target the host does not allow (such as a split on a phone) falls back to here.
 */
export const targetFromEvent = (event: ModifierKeys, allowed: readonly OpenTarget[] = ['here', 'tab', 'right', 'below'], mac = isMac()): OpenTarget | undefined => {
  if (event.key !== 'Enter') return undefined
  const mod = mac ? event.metaKey : event.ctrlKey
  const wanted: OpenTarget = event.altKey ? (event.shiftKey ? 'below' : 'right') : mod ? 'tab' : 'here'
  return allowed.includes(wanted) ? wanted : 'here'
}

/** Alt+1 to Alt+9 (by physical key, so layouts that type symbols with Alt still work): a section index. */
export const sectionJump = (event: ModifierKeys & { readonly code: string }): number | undefined => {
  if (!event.altKey || event.metaKey || event.ctrlKey) return undefined
  const digit = event.code.match(/^Digit([1-9])$/u)?.[1]
  return digit === undefined ? undefined : Number(digit) - 1
}

/** Keeps the most recent `limit` ids, newest first. */
export const pushRecent = (recent: readonly string[], id: string, limit = 20): readonly string[] =>
  [id, ...recent.filter(other => other !== id)].slice(0, limit)
