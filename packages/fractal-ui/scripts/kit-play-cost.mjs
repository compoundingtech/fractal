import { appendFile, readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { costTable, notice } from './kit-play-timings.mjs'

const [directory] = process.argv.slice(2)
if (!directory) throw new Error('Usage: node kit-play-cost.mjs <artifact-directory>')
const shards = new Map()
for (const name of await readdir(directory)) {
  const match = /^kit-play-(\d+)-([a-f0-9]{40})-shard-(\d+)-attempt-(\d+)$/.exec(name)
  if (!match || match[1] !== process.env.GITHUB_RUN_ID || match[2] !== process.env.GITHUB_SHA || Number(match[4]) > Number(process.env.GITHUB_RUN_ATTEMPT)) throw new Error('Cost artifact provenance mismatch')
  const shardIndex = Number(match[3]), attempt = Number(match[4])
  if (shards.has(shardIndex) && shards.get(shardIndex).attempt >= attempt) continue
  shards.set(shardIndex, { name, shardIndex, attempt })
}
// Select attempts before opening any files: an obsolete receipt may be truncated.
const costs = []
for (const { name, shardIndex, attempt } of shards.values()) {
  const optional = async file => {
    try { return await readFile(join(directory, name, file), 'utf8') }
    catch (error) { if (error.code !== 'ENOENT') throw error }
  }
  const receipt = await optional('receipt.json')
  const lane = await optional('lane-seconds')
  costs.push({ shardIndex, attempt, laneSeconds: lane === undefined ? undefined : Number(lane), receipt: receipt === undefined ? undefined : JSON.parse(receipt) })
}
const table = costTable(costs.sort((a, b) => a.shardIndex - b.shardIndex))
if (process.env.GITHUB_STEP_SUMMARY) await appendFile(process.env.GITHUB_STEP_SUMMARY, table)
// Workflow-command notices become check-run annotations, readable without authenticated logs.
console.log(notice(table))
