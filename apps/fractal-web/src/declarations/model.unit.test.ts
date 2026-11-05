import type { ClientOptions } from '@fractal/client'
import { AgentDeclaration, MissionDeclaration, PublicationDefinition, decodeUnknownSync } from '@fractal/client/schema'
import { Effect } from 'effect'
import { describe, expect, it } from 'vitest'

import { observed, unavailable, waiting } from '../data/source.ts'
import { gatewaySubjectReads } from '../data/subjectReadPort.ts'
import { REDACTED, WITHHELD_DECLARATIONS, agentDeclarationState, declarationSubject, missionDeclarationState, redactMissionDefinition } from './model.ts'

const envelope = (value: unknown) => ({ api_version: 'st3.client.v0', snapshot: { id: 'snapshot/test', created_at: '2026-10-04T12:00:00.000Z' }, value })

const kdl = 'version 2\nagent "builder" {\n    env {\n        GH_TOKEN "<redacted>"\n    }\n}\n'
const agentWire = { id: 'agent/builder', kdl, revision: 'rev-7', revisions: ['rev-6', 'rev-7'], tree: {} }

// One secret per environment/embedded-declaration position of Fractal's MissionSpec serde shape.
const SECRET = {
  missionGate: 'sk-mission-gate', stepGate: 'sk-step-llm-gate', missionKdl: 'sk-mission-kdl', stepKdl: 'sk-step-kdl',
  metric: 'sk-loop-metric', loopRound: 'sk-loop-round-gate', loopUntil: 'sk-loop-until', completion: 'sk-completion-gate',
  dependency: 'fixture-step-dependency-gate', baseline: 'sk-baseline-gate', candidate: 'sk-candidate-gate',
} as const
const mechanical = (name: string, secret: string) => ({ predicate: 'mechanical', name, command: 'just check', host: 'host/dev', workspace: '/srv/app', environment: { API_TOKEN: secret }, time_limit_ms: 60000 })
const llm = (name: string, secret: string) => ({ predicate: 'llm', name, model: 'model/x', host: 'host/dev', workspace: '/srv/app', tools: ['read'], environment: { OPENAI_API_KEY: secret }, token_budget: 1000, time_limit_ms: 60000, prompt: 'Judge it.' })
const stepSpec = (id: string, fields: Record<string, unknown> = {}) => ({
  id, path: id, title: `Step ${id}`, goals: [`Do ${id}`], constraints: [], timeout_ms: null, retry: { attempts: 1, backoff_ms: 0 }, finally: false,
  revision_owners: [], revisions_human_only: false, dependencies: [], baselines: [], documents: [], declarations_kdl: null, products: [],
  gates: [], nested_mission: null, definition_hash: `hash-${id}`, ...fields,
})
const missionSpec = (id: string, steps: Record<string, unknown>, fields: Record<string, unknown> = {}) => ({
  id, subject: id, state: 'ready', revision: 'rev-3', inputs: { environment: { kind: 'text' } }, max_active_runs: 1, revision_owners: [], revisions_human_only: false,
  revision_cutover: 'restart-active', goals: ['Show declarations'], constraints: [], baselines: [], products: [], gates: [], steps, display_order: Object.keys(steps), ...fields,
})
// Steps named `env` and `environment` are ordinary user step ids and must survive untouched.
const envStep = stepSpec('env', { goals: ['Keep the env step goal'], work_selector: { kind: 'assigned', agent: 'agent/builder' }, dependencies: [{ dependency: 'step', step: 'build', state: 'done' }] })
const environmentStep = stepSpec('environment', { products: [{ subject: 'doc/x', fields: { environment: { MODE: 'production' } } }] })
const declaration = missionSpec('mission/kdl-view', {
  env: envStep,
  environment: environmentStep,
  build: stepSpec('build', {
    declarations_kdl: `agent "x" { env { TOKEN "${SECRET.stepKdl}" } }`,
    gates: [llm('judge', SECRET.stepGate)],
    dependencies: [{ dependency: 'predicate', gate: mechanical('ready', SECRET.dependency) }],
    baselines: [{ name: 'green', gates: [mechanical('baseline', SECRET.baseline)] }],
    loop_spec: {
      id: 'improve', path: 'build/improve', max_rounds: 3, stop: {}, on_exhausted: { outcome: 'fail' },
      metrics: [{ name: 'score', direction: 'up', min_improvement: 0.1, source: { source: 'exec', command: 'score', host: 'host/dev', workspace: '/srv/app', environment: { SCORE_TOKEN: SECRET.metric }, time_limit_ms: 1000 } }],
      until: [mechanical('done', SECRET.loopUntil)],
      candidates: { count: 2, max_parallel: 1, select: { selector: 'llm', gate: llm('pick', SECRET.candidate) } },
      round: missionSpec('mission/kdl-view/round', { env: stepSpec('env') }, { gates: [mechanical('round', SECRET.loopRound)] }),
    },
  }),
}, {
  declarations_kdl: `agent "y" { env { TOKEN "${SECRET.missionKdl}" } }`,
  gates: [mechanical('verify', SECRET.missionGate)],
  completion: { kind: 'dependencies', dependencies: [{ dependency: 'predicate', gate: mechanical('complete', SECRET.completion) }] },
})
const missionWire = { kind: 'publication-definition', subject: 'mission/kdl-view', revision: 'rev-3', token: 'token-3', declaration }
const secrets = Object.values(SECRET)
// Served exactly as Fractal rendered it, including spacing the client must not reflow.
const missionKdl = 'version 2\nmission "kdl-view" state="ready" {\n    goal "Show declarations"\n    gate "check" { exec "just check"; env { TOKEN "<redacted>"; }; }\n}\n'
const servedWire = { mission: 'mission/kdl-view', revision: 'rev-4', published_at: 1760000000000, text: missionKdl, legacy: false }

/** A gateway whose every request is recorded, so a test can prove which reads were issued. Without `served`, the
 *  mission declaration route answers like an older daemon that lacks it: an empty, non-JSON 404. */
const recordingGateway = (body: unknown, served?: unknown) => {
  const urls: Array<string> = []
  const options: ClientOptions = {
    baseUrl: 'http://gateway.invalid',
    fetchImpl: async (input) => {
      const url = String(input)
      urls.push(url)
      if (url.endsWith('/capabilities'))
        return Response.json(envelope({ kind: 'capabilities', capabilities: [], limits: { max_page_items: 100, max_event_items: 100, max_wait_ms: 1000, max_response_bytes: 65536 }, schemas: [] }))
      if (url.includes('/v1/client/mission-declaration'))
        return served === undefined ? new Response(null, { status: 404 }) : Response.json(envelope(served))
      return Response.json(envelope(body))
    },
  }
  return { urls, reads: gatewaySubjectReads({ options }) }
}

describe('declaration subjects', () => {
  it('routes agents and missions to their canonical sources and refuses anything else', () => {
    expect(declarationSubject('agent/builder')._tag).toBe('Agent')
    expect(declarationSubject('mission/kdl-view')._tag).toBe('Mission')
    expect(declarationSubject('schedule/nightly')._tag).toBe('Unsupported')
  })
})

describe('agent declaration read', () => {
  it('asks Fractal for the canonical KDL with environment values withheld and shows it verbatim', async () => {
    const { urls, reads } = recordingGateway(agentWire)
    const value = await Effect.runPromise(reads.agentDeclaration.read('agent/builder'))
    const request = urls.find(url => url.includes('/v1/client/agent-declarations/'))
    expect(request).toBeDefined()
    expect(new URL(request!).searchParams.get('show_env_values')).toBe('false')
    expect(urls.some(url => url.includes('show_env_values=true'))).toBe(false)
    const state = agentDeclarationState(observed({ value }))
    expect(state).toEqual({ _tag: 'Ready', text: kdl, language: 'kdl', source: 'Canonical KDL from Fractal · revision rev-7 · environment values redacted' })
  })

  it('maps waiting, refusal, failure and stale reads to honest panel states', () => {
    const value = decodeUnknownSync(AgentDeclaration)(agentWire)
    expect(agentDeclarationState(waiting)).toEqual({ _tag: 'Loading' })
    expect(agentDeclarationState(unavailable({ reason: 'ungranted', detail: 'read.declarations missing' }))).toEqual({ _tag: 'Unavailable', message: 'Read access to declarations has not been granted to this connection.' })
    expect(agentDeclarationState(unavailable({ reason: 'unsupported', detail: 'Gateway is too old.' }))._tag).toBe('Unavailable')
    expect(agentDeclarationState(unavailable({ reason: 'failed', detail: 'Gateway disconnected' }))).toEqual({ _tag: 'Failed', message: 'The declaration could not be loaded: Gateway disconnected. Nothing was changed; close and try again.' })
    const stale = agentDeclarationState({ _tag: 'Observed', value, freshness: 'stale', error: { reason: 'failed', detail: 'Gateway disconnected' } })
    expect(stale._tag === 'Ready' && stale.source).toContain('last verified copy, reconnecting')
  })
})

describe('mission declaration read', () => {
  it('shows the served canonical KDL byte for byte with its revision', async () => {
    const { urls, reads } = recordingGateway(missionWire, servedWire)
    const value = await Effect.runPromise(reads.missionDefinition.read('mission/kdl-view'))
    const request = urls.find(url => url.includes('/v1/client/mission-declaration'))
    expect(new URL(request!).searchParams.get('mission')).toBe('mission/kdl-view')
    expect(urls.some(url => url.includes('/v1/client/publication-definition'))).toBe(false)
    expect(missionDeclarationState(observed({ value }))).toEqual({ _tag: 'Ready', text: missionKdl, language: 'kdl', source: 'Canonical KDL from Fractal · revision rev-4 · environment values redacted' })
  })

  it('labels a legacy loop shape the current parser rejects', () => {
    const value = decodeUnknownSync(MissionDeclaration)({ ...servedWire, legacy: true })
    const state = missionDeclarationState(observed({ value }))
    expect(state._tag === 'Ready' && state.text).toBe(missionKdl)
    expect(state._tag === 'Ready' && state.source).toBe('Canonical KDL from Fractal · revision rev-4 · legacy loop shape: display only, the current parser rejects it · environment values redacted')
  })

  it('falls back to the redacted structured definition when the daemon lacks the declaration route', async () => {
    const { urls, reads } = recordingGateway(missionWire)
    const value = await Effect.runPromise(reads.missionDefinition.read('mission/kdl-view'))
    expect(urls.map(url => new URL(url).pathname).filter(path => path.includes('/v1/client/mission-declaration') || path.includes('/v1/client/publication-definition'))).toEqual(['/v1/client/mission-declaration', '/v1/client/publication-definition'])
    const request = urls.find(url => url.includes('/v1/client/publication-definition'))
    expect(new URL(request!).searchParams.get('subject')).toBe('mission/kdl-view')
    const held = JSON.stringify(value)
    for (const secret of secrets) expect(held).not.toContain(secret)
    for (const name of ['API_TOKEN', 'OPENAI_API_KEY', 'SCORE_TOKEN']) expect(held).toContain(name)
    const state = missionDeclarationState(observed({ value }))
    expect(state._tag).toBe('Ready')
    if (state._tag !== 'Ready') return
    expect(state.language).toBe('json')
    expect(state.source).toBe('Structured definition · revision rev-3 · this Fractal daemon predates the mission declaration route; environment values redacted')
    for (const secret of secrets) expect(state.text).not.toContain(secret)
    expect(state.text).toContain(WITHHELD_DECLARATIONS)
  })

  it('keeps steps named env or environment and arbitrary user maps verbatim', () => {
    const redacted = redactMissionDefinition(declaration)
    expect(redacted).toMatchObject({
      inputs: { environment: { kind: 'text' } },
      display_order: ['env', 'environment', 'build'],
      gates: [{ ...mechanical('verify', ''), environment: { API_TOKEN: REDACTED } }],
      steps: { env: envStep, environment: environmentStep, build: { goals: ['Do build'], loop_spec: { round: { steps: { env: stepSpec('env') } } } } },
    })
    expect(JSON.stringify(redacted)).toContain('"MODE":"production"')
  })

  it('redacts again at mapping time so a fixture-held definition cannot leak', () => {
    const raw = decodeUnknownSync(PublicationDefinition)(missionWire)
    const state = missionDeclarationState(observed({ value: raw }))
    for (const secret of secrets) expect(state._tag === 'Ready' && state.text).not.toContain(secret)
  })

  it('is idempotent', () => {
    const once = redactMissionDefinition(declaration)
    expect(redactMissionDefinition(once)).toEqual(once)
  })
})
