import { createHash } from 'node:crypto'
import { readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

export function shardNumber(id, count) {
  if (!Number.isSafeInteger(count) || count < 1) throw new Error('Shard count must be a positive integer')
  return Number(BigInt(`0x${createHash('sha256').update(id).digest('hex')}`) % BigInt(count))
}

export function shardStories(ids, index, count) {
  shardNumber('', count)
  if (!Number.isSafeInteger(index) || index < 0 || index >= count) throw new Error('Invalid shard index')
  if (!ids.length || new Set(ids).size !== ids.length) throw new Error('Invalid full story index')
  const selected = ids.filter(id => shardNumber(id, count) === index)
  if (!selected.length && count <= ids.length) throw new Error(`Unexpected empty shard ${index}/${count}`)
  return selected
}

export function aggregateShards(receipts, count) {
  if (receipts.length !== count) throw new Error(`Expected ${count} shard receipts; got ${receipts.length}`)
  const full = receipts[0].fullIds
  if (!Array.isArray(full)) throw new Error('Missing full index')
  const seen = new Set()
  const allCases = new Set()
  for (const receipt of receipts) {
    const { shardIndex, shardCount, rows } = receipt
    if (!receipt.pass || shardCount !== count || seen.has(shardIndex)) throw new Error('Failed, duplicate or mismatched shard')
    seen.add(shardIndex)
    if (JSON.stringify(receipt.fullIds) !== JSON.stringify(full)) throw new Error('Shards validated different full indexes')
    const expected = shardStories(full, shardIndex, count)
    if (!Array.isArray(rows) || rows.length !== expected.length * 4) throw new Error('Incomplete shard cases')
    const assigned = new Set(expected)
    for (const row of rows) {
      const key = JSON.stringify([row.id, row.scheme, row.width])
      if (!row.pass || !assigned.has(row.id) || !['dark', 'light'].includes(row.scheme) || ![440, 1440].includes(row.width) || allCases.has(key)) throw new Error('Missing, duplicate or invalid play case')
      allCases.add(key)
    }
  }
  if (allCases.size !== full.length * 4) throw new Error('Shard union does not cover the full index')
  return { stories: full.length, cases: allCases.size, shards: count, pass: true }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [directory, count] = process.argv.slice(2)
  const names = await readdir(directory)
  const receipts = await Promise.all(names.map(async name => JSON.parse(await readFile(join(directory, name, 'receipt.json'), 'utf8'))))
  console.log(JSON.stringify(aggregateShards(receipts, Number(count))))
}
