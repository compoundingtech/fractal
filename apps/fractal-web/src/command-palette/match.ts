/**
 * Command palette query matching over `fixtures/ui/command-palette-matching.json`; a shared st3-views/Rust
 * implementation should replace it together with that corpus.
 *
 * Each whitespace-separated query token must match one searched field. Per token, the best tier wins:
 * - `prefix`: the token starts a word (`de` → **De**sign).
 * - `initials`: the token letters start consecutive words from the field start (`ab` → **A**tlas **B**uilder).
 * - `substring`: the token occurs inside a word (`sign` → de**sign**).
 * Only fields that reach a token's best tier are highlighted, so highlights are contiguous runs or initials,
 * never letters scattered through a word.
 */
export type MatchTier = 'prefix' | 'initials' | 'substring'

export interface TokenMatch {
  readonly tier: MatchTier
  readonly positions: readonly number[]
}

export interface FieldsMatch {
  readonly score: number
  /** Highlighted UTF-16 positions, one array per searched field, in field order. */
  readonly highlights: readonly (readonly number[])[]
  /** Earliest field at the best tier for each token; later identifiers explain only otherwise unmatched tokens. */
  readonly primaryFields: readonly number[]
}

const tierScore: Record<MatchTier, number> = { prefix: 100, initials: 60, substring: 20 }

const wordChar = /[\p{L}\p{N}]/u

export const queryTokens = (query: string) => query.toLowerCase().trim().split(/\s+/).filter(Boolean)

export function matchToken(token: string, text: string): TokenMatch | undefined {
  const lower = text.toLowerCase()
  const starts: number[] = []
  for (let index = 0; index < lower.length; index++) if (wordChar.test(lower[index]!) && (index === 0 || !wordChar.test(lower[index - 1]!))) starts.push(index)
  const prefix = starts.find(start => lower.startsWith(token, start))
  if (prefix !== undefined) return { tier: 'prefix', positions: Array.from(token, (_, offset) => prefix + offset) }
  if (token.length > 1 && token.length <= starts.length && Array.from(token).every((letter, index) => lower[starts[index]!] === letter)) {
    return { tier: 'initials', positions: starts.slice(0, token.length) }
  }
  const substring = lower.indexOf(token)
  if (substring >= 0) return { tier: 'substring', positions: Array.from(token, (_, offset) => substring + offset) }
  return undefined
}

/** Earlier fields break ties, so callers list the most identifying field first. */
export function matchFields(query: string, fields: readonly string[]): FieldsMatch | undefined {
  const highlights = fields.map(() => new Set<number>())
  let score = 0
  const primaryFields = new Set<number>()
  for (const token of queryTokens(query)) {
    const perField = fields.map(field => matchToken(token, field))
    const best = Math.max(...perField.map(found => (found === undefined ? -1 : tierScore[found.tier])))
    if (best < 0) return undefined
    const primary = perField.findIndex(found => found !== undefined && tierScore[found.tier] === best)
    primaryFields.add(primary)
    score += best - primary
    perField.forEach((found, index) => {
      if (found !== undefined && tierScore[found.tier] === best) for (const position of found.positions) highlights[index]!.add(position)
    })
  }
  return { score, highlights: highlights.map(set => [...set].sort((left, right) => left - right)), primaryFields: [...primaryFields] }
}
