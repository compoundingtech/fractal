// All lifecycle marks use the document's performance clock, never Node's clock.
export function readinessTimings(marks, a11yEnabled) {
  if (!marks) return {}
  const { dom, playing, played, completing, finished, observed, a11yStart, a11yEnd } = marks
  if (![dom, completing, finished, observed].every(Number.isFinite)) return {}
  const start = Math.max(dom, playing ?? completing)
  const end = Math.max(start, played ?? completing)
  if (finished < end || observed < finished) return {}
  if (a11yEnabled && ![a11yStart, a11yEnd].every(Number.isFinite)) return {}
  if (a11yEnabled && (a11yStart < end || a11yEnd < a11yStart || a11yEnd > finished)) return {}
  const bootstrapMilliseconds = start - dom
  const playMilliseconds = end - start
  const a11yMilliseconds = a11yEnabled ? a11yEnd - Math.max(dom, a11yStart) : 0
  // Includes animation settling and other afterEach hooks, not hidden in the axe budget.
  const lifecycleMilliseconds = finished - dom - bootstrapMilliseconds - playMilliseconds - a11yMilliseconds
  if (lifecycleMilliseconds < 0) return {}
  return {
    readinessAfterDomMilliseconds: observed - dom,
    completionObservationMilliseconds: observed - finished,
    bootstrapMilliseconds, playMilliseconds, a11yMilliseconds, lifecycleMilliseconds,
  }
}

export function summarizeTimings(rows) {
  const phases = new Map(), families = new Map()
  for (const row of rows) {
    for (const [name, value] of Object.entries(row.timings ?? {})) {
      if (!Number.isFinite(value) || value < 0) continue
      if (!phases.has(name)) phases.set(name, [])
      phases.get(name).push(value)
    }
    const family = row.id.split('--')[0]
    const total = families.get(family) ?? { family, cases: 0, elapsedSeconds: 0 }
    total.cases++
    total.elapsedSeconds += row.elapsedSeconds
    families.set(family, total)
  }
  const phaseMilliseconds = Object.fromEntries([...phases].map(([name, values]) => {
    values.sort((a, b) => a - b)
    return [name, { samples: values.length, mean: values.reduce((sum, value) => sum + value, 0) / values.length, p95: values[Math.ceil(values.length * 0.95) - 1] }]
  }))
  const topCases = [...rows].sort((a, b) => b.elapsedSeconds - a.elapsedSeconds || a.id.localeCompare(b.id) || a.scheme.localeCompare(b.scheme) || a.width - b.width)
    .slice(0, 20).map(({ id, scheme, width, elapsedSeconds, pass }) => ({ id, scheme, width, elapsedSeconds, pass }))
  return { phaseMilliseconds, topCases, storyFamilies: [...families.values()].sort((a, b) => b.elapsedSeconds - a.elapsedSeconds || a.family.localeCompare(b.family)) }
}

export function costTable(shards) {
  const number = value => Number.isFinite(value) ? value.toFixed(2) : 'unavailable'
  const cell = value => String(value).replaceAll('|', '\\|').replaceAll('\n', ' ').replaceAll('\r', ' ').replaceAll('`', '')
  const lines = [
    '## Fractal web cost', '',
    'Lane seconds cover ci-fractal-web (including install), not job provisioning. Case totals are worker-seconds, not runner-seconds.', '',
    `Runner CPU classes: ${[...new Set(shards.map(shard => shard.runnerCpu?.cpuClass ?? 'unknown'))].sort().join(', ') || 'unavailable'}.`, '',
    '| Shard | Lane s | Plays s | Storybook s | Cases | CPU class | CPU model |', '| --- | ---: | ---: | ---: | ---: | --- | --- |',
    ...shards.map(({ shardIndex, laneSeconds, receipt, runnerCpu }) => `| ${shardIndex} | ${number(laneSeconds)} | ${number(receipt?.elapsedSeconds)} | ${number(receipt?.storybookBuildSeconds)} | ${receipt?.cases ?? 'not run'} | ${cell(runnerCpu?.cpuClass ?? 'unknown')} | ${cell(runnerCpu?.modelName || 'unavailable')} |`),
    '', '| Phase | Mean ms | p95 ms | Samples |', '| --- | ---: | ---: | ---: |',
  ]
  const rows = shards.flatMap(shard => shard.receipt?.rows ?? [])
  const summary = summarizeTimings(rows)
  for (const [name, phase] of Object.entries(summary.phaseMilliseconds)) {
    lines.push(`| ${name.replace('Milliseconds', '')} | ${number(phase.mean)} | ${number(phase.p95)} | ${phase.samples} |`)
  }
  lines.push('', 'A11y is the unchanged addon step (axe plus its import/configuration/reporting). Missing lifecycle samples are not reported as zero.', '',
    '| Slowest case | Scheme | Width | Seconds |', '| --- | --- | ---: | ---: |',
    ...summary.topCases.slice(0, 10).map(row => `| ${cell(row.id)} | ${cell(row.scheme)} | ${row.width} | ${number(row.elapsedSeconds)} |`))
  const probes = rows.flatMap(row => row.filterBenchmark ?? [])
  const margins = probes.filter(probe => Number.isFinite(probe.pooledRatio) && probe.pooledRatio > 0 && Number.isFinite(probe.threshold) && probe.threshold > 0)
    .map(probe => probe.threshold / probe.pooledRatio)
  lines.push('', margins.length ? `Filter benchmark minimum margin: ${Math.min(...margins).toFixed(3)}× (threshold / pooled ratio; ${margins.length} probes).` : 'Filter benchmark minimum margin: unavailable (no measured probes).')
  return lines.join('\n') + '\n'
}

export const notice = table => `::notice title=Fractal web cost::${table.replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A')}`
