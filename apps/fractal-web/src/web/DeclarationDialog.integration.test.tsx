// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { PublicationDefinition, decodeUnknownSync } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, unavailable } from '../data/source.ts'
import { DeclarationDialog } from './DeclarationDialog.tsx'

vi.mock('@stylexjs/stylex', () => ({ create: (styles: unknown) => styles, defineVars: (variables: unknown) => variables, createTheme: () => ({}), keyframes: () => 'animation', props: () => ({}) }))

const SECRET = 'fixture'
// A fixture-held definition skips the gateway decode, so this proves the mapping redacts on its own.
const mission = decodeUnknownSync(PublicationDefinition)({
  kind: 'publication-definition', subject: 'mission/kdl-view', revision: 'rev-3', token: 'token-3',
  declaration: { id: 'mission/kdl-view', goals: ['Show declarations'], steps: { gate: { id: 'gate', goals: ['Check'], gates: [{ predicate: 'mechanical', name: 'check', command: 'just check', host: 'host/dev', workspace: '/srv/app', environment: { OPENAI_API_KEY: SECRET }, time_limit_ms: 60000 }] } } },
})
const source = fixtureSource({ world: { now: Date.parse('2026-10-06T12:00:00.000Z'), agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' }, subjectReads: {
  missionDefinition: { 'mission/kdl-view': observed({ value: mission }) },
  agentDeclaration: { 'agent/denied': unavailable({ reason: 'ungranted', detail: 'read.declarations missing' }) },
} } })

let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
const onOpenChange = vi.fn()
const render = (subject: string, isOpen: boolean) => act(async () => {
  root.render(<DataSourceProvider source={source} registry={registry}><DeclarationDialog subject={subject} isOpen={isOpen} onOpenChange={onOpenChange} /></DataSourceProvider>)
  if (isOpen) await import('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx')
})
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
  it('shows a mission as a labelled, redacted structured fallback and closes on Escape', async () => {
    await render('mission/kdl-view', false)
    expect(dialog()).toBeNull()
    await render('mission/kdl-view', true)
    const node = dialog()!
    expect(node.querySelector('h2')?.textContent).toBe('mission/kdl-view')
    expect(node.getAttribute('aria-labelledby')).toBe(node.querySelector('h2')?.id)
    expect(node.querySelector('[data-testid="declaration-language"]')?.textContent).toBe('JSON')
    expect(node.querySelector('[data-testid="declaration-source"]')?.textContent).toContain('Fractal serves no mission KDL')
    expect(node.textContent).toContain('OPENAI_API_KEY')
    expect(node.textContent).not.toContain(SECRET)
    expect(host.contains(node)).toBe(true)
    await act(async () => node.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })))
    expect(onOpenChange).toHaveBeenCalledWith(false)
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

  it('contains a rejected declaration chunk inside a dismissable local dialog', async () => {
    vi.resetModules()
    vi.doMock('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx', () => { throw new Error('Chunk unavailable') })
    try {
      // A fresh lazy identity is required to exercise rejection rather than the already-loaded panel.
      const { DeclarationDialog: DeferredDeclarationDialog } = await import('./DeclarationDialog.tsx')
      await act(async () => root.render(<><button type="button">Workspace remains available</button><DeferredDeclarationDialog subject="schedule/nightly" isOpen={false} onOpenChange={onOpenChange} /></>))
      const workspace = within(host).getByRole('button', { name: 'Workspace remains available' })
      await act(async () => root.render(<><button type="button">Workspace remains available</button><DeferredDeclarationDialog subject="schedule/nightly" isOpen onOpenChange={onOpenChange} /></>))
      expect((await within(host).findByRole('alert')).textContent).toBe('This declaration view could not load.')
      expect(host.contains(workspace)).toBe(true)
      const close = within(host).getByRole('button', { name: 'Close declaration' })
      await act(async () => close.click())
      expect(onOpenChange).toHaveBeenCalledWith(false)
      await act(async () => root.render(<><button type="button">Workspace remains available</button><DeferredDeclarationDialog subject="schedule/nightly" isOpen={false} onOpenChange={onOpenChange} /></>))
      expect(within(host).getByRole('button', { name: 'Workspace remains available' })).toBe(workspace)
      workspace.focus()
      expect(document.activeElement).toBe(workspace)
    } finally {
      vi.doUnmock('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx')
    }
  })
})
