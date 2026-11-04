// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MissionDeclaration, PublicationDefinition, decodeUnknownSync } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, unavailable } from '../data/source.ts'
import { DeclarationDialog } from './DeclarationDialog.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

const SECRET = 'fixture'
// An older daemon's structured definition held by a fixture skips the gateway decode, so this proves the mapping redacts on its own.
const mission = decodeUnknownSync(PublicationDefinition)({
  kind: 'publication-definition', subject: 'mission/kdl-view', revision: 'rev-3', token: 'token-3',
  declaration: { id: 'mission/kdl-view', goals: ['Show declarations'], steps: { gate: { id: 'gate', goals: ['Check'], gates: [{ predicate: 'mechanical', name: 'check', command: 'just check', host: 'host/dev', workspace: '/srv/app', environment: { OPENAI_API_KEY: SECRET }, time_limit_ms: 60000 }] } } },
})
const servedKdl = 'version 2\nmission "served" state="ready" {\n    goal "Show declarations"\n}\n'
const served = decodeUnknownSync(MissionDeclaration)({ mission: 'mission/served', revision: 'rev-5', published_at: 1760000000000, text: servedKdl, legacy: true })
const source = fixtureSource({ world: { now: Date.parse('2026-10-06T12:00:00.000Z'), agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' }, subjectReads: {
  missionDefinition: { 'mission/kdl-view': observed({ value: mission }), 'mission/served': observed({ value: served }) },
  agentDeclaration: { 'agent/denied': unavailable({ reason: 'ungranted', detail: 'read.declarations missing' }) },
} } })

let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
const onOpenChange = vi.fn()
const render = (subject: string, isOpen: boolean) => act(async () => root.render(<DataSourceProvider source={source} registry={registry}><DeclarationDialog subject={subject} isOpen={isOpen} onOpenChange={onOpenChange} /></DataSourceProvider>))
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"][data-testid="declaration-dialog"]')

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  registry = AtomRegistry.make()
  onOpenChange.mockReset()
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => { await act(async () => root.unmount()); registry.dispose(); host.remove(); vi.unstubAllGlobals() })

describe('declaration dialog', () => {
  it('shows a mission as a labelled, redacted structured fallback from an older daemon and closes on Escape', async () => {
    await render('mission/kdl-view', false)
    expect(dialog()).toBeNull()
    await render('mission/kdl-view', true)
    const node = dialog()!
    expect(node.querySelector('h2')?.textContent).toBe('mission/kdl-view')
    expect(node.getAttribute('aria-labelledby')).toBe(node.querySelector('h2')?.id)
    expect(node.querySelector('[data-testid="declaration-language"]')?.textContent).toBe('JSON')
    expect(node.querySelector('[data-testid="declaration-source"]')?.textContent).toContain('this Fractal daemon predates the mission declaration route')
    expect(node.textContent).toContain('OPENAI_API_KEY')
    expect(node.textContent).not.toContain(SECRET)
    expect(host.contains(node)).toBe(true)
    await act(async () => node.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })))
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it('shows served mission KDL with its legacy loop shape note', async () => {
    await render('mission/served', false)
    await render('mission/served', true)
    const node = dialog()!
    expect(node.querySelector('[data-testid="declaration-language"]')?.textContent).toBe('KDL')
    expect(node.querySelector('[data-testid="declaration-source"]')?.textContent).toBe('Canonical KDL from Fractal · revision rev-5 · legacy loop shape: display only, the current parser rejects it · environment values redacted')
    expect(node.textContent).toContain('goal "Show declarations"')
  })

  it('explains a refused read without showing the raw denial', async () => {
    await render('agent/denied', false)
    await render('agent/denied', true)
    expect(dialog()?.querySelector('[role="status"]')?.textContent).toBe('Read access to declarations has not been granted to this connection.')
    expect(dialog()?.textContent).not.toContain('read.declarations missing')
  })

  it('refuses subjects that have no declaration', async () => {
    await render('schedule/nightly', false)
    await render('schedule/nightly', true)
    expect(dialog()?.textContent).toContain('Only agents and missions have a declaration to show.')
  })
})
