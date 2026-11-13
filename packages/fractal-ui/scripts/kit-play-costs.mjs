// Refresh with: node packages/fractal-ui/scripts/kit-play-costs.mjs JOB_ID [JOB_ID ...]
// Fetches immutable hosted job logs; deliberately separate from offline CI and story execution.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { writeFile } from 'node:fs/promises'

const jobs = process.argv.slice(2).sort()
if (!jobs.length || jobs.some(id => !/^\d+$/.test(id)) || new Set(jobs).size !== jobs.length) {
  throw new Error('Provide unique numeric GitHub Actions job IDs')
}
const fields = ['resetMilliseconds', 'pageCreateMilliseconds', 'guardMilliseconds',
  'navigationToDomMilliseconds', 'readinessAfterDomMilliseconds',
  'completionObservationMilliseconds', 'assertionsMilliseconds', 'closeMilliseconds']
const samples = new Map()
for (const job of jobs) {
  const log = execFileSync('gh', ['api', `repos/compoundingtech/fractal/actions/jobs/${job}/logs`, '--allow-escape-sequences'], { encoding: 'utf8', maxBuffer: 128 * 1024 * 1024 })
  const stories = new Map()
  for (const line of log.split('\n')) {
    const start = line.indexOf('{')
    if (start < 0) continue
    let row
    try { row = JSON.parse(line.slice(start)) } catch { continue }
    if (!row.timings || !row.id) continue
    const value = fields.reduce((sum, field) => sum + (row.timings[field] ?? 0), 0)
    if (!Number.isFinite(value) || value <= 0) throw new Error(`Invalid timing in job ${job}: ${row.id}`)
    const story = stories.get(row.id) ?? { cases: 0, milliseconds: 0 }
    story.cases++
    story.milliseconds += value
    stories.set(row.id, story)
  }
  for (const [id, value] of stories) {
    if (!samples.has(id)) samples.set(id, [])
    samples.get(id).push(value)
  }
}
const median = values => {
  const sorted = [...values].sort((a, b) => a - b), middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
}
const storyMilliseconds = Object.fromEntries([...samples].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([id, values]) => {
  // A cancelled job may stop partway through a story. Retain only complete observed
  // samples (including the reviewed one/two-case overrides), not its partial prefix.
  const cases = Math.max(...values.map(value => value.cases))
  return [id, Math.round(median(values.filter(value => value.cases === cases).map(value => value.milliseconds)))]
}))
if (!Object.keys(storyMilliseconds).length) throw new Error('No story timings found')
const manifest = {
  generated: 'Do not edit by hand. Regenerate with node packages/fractal-ui/scripts/kit-play-costs.mjs followed by sourceJobs.',
  sourceJobs: jobs,
  method: 'Median total observed case milliseconds per story; discard samples with fewer cases than the largest observed sample. Sum reset, pageCreate, guard, navigationToDom, readinessAfterDom, completionObservation, assertions and close; nested bootstrap/play/a11y timings are not counted twice.',
  inputSha256: createHash('sha256').update(JSON.stringify(storyMilliseconds)).digest('hex'),
  newStoryMilliseconds: Math.round(median(Object.values(storyMilliseconds))),
  storyMilliseconds,
}
await writeFile(new URL('./kit-play-costs.json', import.meta.url), JSON.stringify(manifest, null, 2) + '\n')
