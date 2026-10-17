import { createHash } from 'node:crypto'

// FRACTAL_WEB_SHARD is optional and one-based. Reject ambiguous spellings rather
// than silently running the whole suite when a CI matrix value is malformed.
export const parseWebShard = value => {
  if (value === undefined) return null
  const match = typeof value === 'string' && /^([1-9]\d*)\/([1-9]\d*)$/.exec(value)
  const index = match ? Number(match[1]) : NaN
  const total = match ? Number(match[2]) : NaN
  if (match?.[0] !== value || !Number.isSafeInteger(index) || !Number.isSafeInteger(total) || index > total) {
    throw new Error(`Invalid FRACTAL_WEB_SHARD ${JSON.stringify(value)}: expected i/N with safe positive integers and 1 <= i <= N`)
  }
  return { index, total }
}

export const selectKitCases = (ids, shard) => ids.flatMap(id =>
  ['dark', 'light'].flatMap(scheme => [440, 1440].flatMap(width => {
    const candidate = { id, scheme, width }
    if (shard === null) return [candidate]
    // SHA-256 over an unambiguous tuple makes ownership independent of story
    // ordering, worker scheduling, and additions/removals elsewhere in the index.
    const hash = createHash('sha256').update(JSON.stringify([id, scheme, width])).digest().readBigUInt64BE(0)
    return hash % BigInt(shard.total) === BigInt(shard.index - 1) ? [candidate] : []
  })),
)
