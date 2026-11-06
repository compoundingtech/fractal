import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import * as React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { createServer } from 'vite'
import stylex from '@stylexjs/unplugin'

const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, plugins: [stylex.vite({ useCSSLayers: false })], server: { middlewareMode: true } })
const { SidebarRelatedRuns, relatedRunText } = await server.ssrLoadModule('/src/assistant-ui/sidebar/SidebarRelatedRuns.tsx')
const { AgentRelatedRuns, decodeUnknownSync } = await server.ssrLoadModule(new URL('../../../../../clients/typescript/fractal-client/ReadSchema.generated.ts', import.meta.url).pathname)
after(() => server.close())

const run = (slug, rest = {}) => ({ id: `mission-run/${slug}`, mission_id: `mission/${slug}`, status: 'running', status_chip: 'running', progress: { done: 1, total: 4 }, eta: null, waiting_on_you: false, late: false, deadline_at: null, completed_at: null, current_step: null, waits_on: [], urgency_rank: 3, folded: false, ...rest })
const eta = { remaining_low_ms: 600_000, remaining_high_ms: 900_000, basis: { method: 'per-step', samples: 3 }, as_of: 0 }
const render = state => renderToStaticMarkup(React.createElement(SidebarRelatedRuns, { state, now: 60_000 }))
// Wire JSON decoded once through the Native codec, as the host does: nullable fields reach the component as Option.
const loaded = (rows, total = rows.length) => ({ _tag: 'Loaded', value: decodeUnknownSync(AgentRelatedRuns, 'strict')({ kind: 'agent-related-runs', agent_id: 'agent/a', as_of: 0, work_cut: 1, missions_cut: 1, total_rows: total, truncated: total > rows.length, rows }) })

test('rows keep served order, active before the finished fold, without client reordering', () => {
  // Served order deliberately disagrees with urgency_rank and slug order: the client must not re-sort.
  const html = render(loaded([run('zeta', { urgency_rank: 3 }), run('alpha', { urgency_rank: 0 }), run('done', { folded: true, status_chip: 'completed', urgency_rank: 4 })]))
  assert.deepEqual([...html.matchAll(/data-run="([^"]+)"/g)].map(match => match[1]), ['mission-run/zeta', 'mission-run/alpha', 'mission-run/done'])
  assert.doesNotMatch(html, /<details[^>]*\sopen/, 'finished runs start folded')
  assert.ok(html.indexOf('mission-run/alpha') < html.indexOf('data-fold="finished"'), 'active rows precede the fold')
  assert.match(html, /Finished · <!-- -->1|Finished · 1/)
})

test('waiting on you replaces the ETA; a late ETA is marked; truncation is explicit', () => {
  const html = render(loaded([run('hold', { waiting_on_you: true, status_chip: 'waiting-on-you', eta }), run('late', { late: true, eta }), run('late-no-eta', { late: true })], 140))
  assert.equal(html.match(/data-waiting-on-you/g)?.length, 1)
  assert.equal(html.match(/data-late/g)?.length, 2)
  assert.equal(html.match(/data-mission-eta="/g)?.length, 1, 'the held run draws no ETA')
  assert.match(html, /waiting on you/)
  assert.match(html, /Showing <!-- -->3<!-- --> of <!-- -->140<!-- --> runs|Showing 3 of 140 runs/)
})

test('loading, failure and empty states and the row text', () => {
  assert.match(render({ _tag: 'Loading' }), /role="status"[^>]*>Loading related missions/)
  assert.match(render({ _tag: 'Failed', message: 'offline' }), /Related missions unavailable: <!-- -->offline|unavailable: offline/)
  assert.match(render(loaded([])), /No related missions/)
  const [row] = loaded([run('x', { late: true, current_step: { step: 's', path: 'p', state: 'open', kind: 'step', title: 'Ship' }, waits_on: [{ step: 's', path: 'p', state: 'open', kind: 'ask', who: 'person/reviewer' }] })]).value.rows
  assert.equal(relatedRunText(row), 'Running, x, 1 of 4 steps, late, Ship, waits on reviewer')
})
