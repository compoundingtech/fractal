// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MissionDeclaration, PublicationDefinition, SourceBlob, SubjectSourceMap, decodeUnknownSync } from '@fractal/client/schema'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { within } from 'storybook/test'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, unavailable } from '../data/source.ts'
import { blobRef, pinnedRef } from '../data/subjectReadPort.ts'
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
// A mapped mission whose change between two revisions came from a helper file, not its own.
const mappedKdl = (goal: string) => `version 2\nmission "land/code" {\n    step "review" {\n        goal "${goal}"\n    }\n}\n`
const ownHash = 'a'.repeat(64)
const helperHash = (n: number) => String(n).repeat(64)
const ownText = "import { review } from '../lib/review.ts'\nexport default mission({\n  id: 'land/code',\n  steps: [review()],\n})\n"
const helperText = (goal: string) => `export const review = () => ({\n  id: 'review',\n  goal: '${goal}',\n})\n`
const position = (line: number) => ({ line, column: 1 })
const sourceMapOf = (revision: string, hash: string, sequence: number) => ({
  subject: 'mission/land/code', revision, repository: 'schickling/factory', ref: 'main', sha: String(sequence).repeat(40), sequence, set_revision: `set-${sequence}`, declaration: 0, inputs: [0, 1],
  files: [{ repository: 'schickling/factory', path: 'graph/land/code/mission.ts', blob: 'b0', blob_hash: ownHash }, { repository: 'schickling/factory', path: 'lib/review.ts', blob: 'b1', blob_hash: hash }],
  nodes: [
    { kdl: 'mission[land/code]', kdlLines: [2, 6], span: { file: 0, start: position(2), end: position(5) }, frames: [] },
    { kdl: 'mission[land/code]/step[review]', kdlLines: [3, 5], span: { file: 1, start: position(1), end: position(4) }, frames: [{ file: 0, start: position(4), end: position(4), role: 'declaration', precision: 'exact' }, { file: 1, start: position(1), end: position(4), role: 'helper', precision: 'exact' }] },
    { kdl: 'mission[land/code]/step[review]/goal[0]', kdlLines: [4, 4], span: { file: 1, start: position(3), end: position(3) }, frames: [] },
  ],
})
const revisions = [6, 5].map(sequence => ({ revision: `map-${sequence}`, repository: 'schickling/factory', ref: 'main', sha: String(sequence).repeat(40), sequence, set_revision: `set-${sequence}`, recorded_at_unix_ms: 1760000000000 + sequence }))
const mapped = (revision: string, hash: string, sequence: number, goal: string) => observed({ value: { _tag: 'Mapped' as const, value: decodeUnknownSync(SubjectSourceMap)({ subject: 'mission/land/code', revision, current: 'map-6', map: sourceMapOf(revision, hash, sequence), kdl: mappedKdl(goal), revisions }) } })
const atRevision = (revision: string, goal: string) => observed({ value: decodeUnknownSync(MissionDeclaration)({ mission: 'mission/land/code', revision, published_at: 1760000000000, text: mappedKdl(goal), legacy: false }) })
const source = fixtureSource({ world: { now: Date.parse('2026-10-06T12:00:00.000Z'), agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: { _tag: 'undeclared' }, subjectReads: {
  missionDefinition: { 'mission/kdl-view': observed({ value: mission }), 'mission/served': observed({ value: served }), 'mission/land/code': atRevision('map-6', 'Review the diff') },
  agentDeclaration: { 'agent/denied': unavailable({ reason: 'ungranted', detail: 'read.declarations missing' }) },
  // The sheet reads the current map by bare subject, older ones pinned.
  sourceMap: { 'mission/land/code': mapped('map-6', helperHash(6), 6, 'Review the diff'), [pinnedRef('mission/land/code', 'map-5')]: mapped('map-5', helperHash(5), 5, 'Review') },
  sourceBlob: {
    [blobRef(pinnedRef('mission/land/code', 'map-6'), ownHash)]: observed({ value: decodeUnknownSync(SourceBlob)({ hash: ownHash, text: ownText }) }),
    [blobRef(pinnedRef('mission/land/code', 'map-6'), helperHash(6))]: observed({ value: decodeUnknownSync(SourceBlob)({ hash: helperHash(6), text: helperText('Review the diff') }) }),
    [blobRef(pinnedRef('mission/land/code', 'map-5'), helperHash(5))]: observed({ value: decodeUnknownSync(SourceBlob)({ hash: helperHash(5), text: helperText('Review') }) }),
  },
} } })

let registry: AtomRegistry.AtomRegistry
let root: Root
let host: HTMLDivElement
const onOpenChange = vi.fn()
const render = (subject: string, isOpen: boolean, node?: string) => act(async () => {
  root.render(<DataSourceProvider source={source} registry={registry}><DeclarationDialog subject={subject} {...(node === undefined ? {} : { node })} isOpen={isOpen} onOpenChange={onOpenChange} /></DataSourceProvider>)
  if (isOpen) await Promise.all([import('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx'), import('../../../../packages/fractal-ui/src/assistant-ui/declaration/SubjectInspector.tsx')])
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

  it('opens a mapped mission on its Source tab and explains a change that came from a helper', async () => {
    await render('mission/land/code', false)
    await render('mission/land/code', true, 'mission[land/code]/step[review]')
    const node = dialog()!
    const view = within(node)
    expect(node.querySelector('[data-testid="source-provenance"]')?.textContent).toContain('schickling/factory@6666666')
    expect(node.querySelector('[data-testid="frame-strip"]')?.textContent).toContain('declared mission.ts:4')
    expect(node.querySelector('[data-testid="frame-strip"]')?.textContent).toContain('via review.ts:1–4')
    expect(node.querySelector('[data-testid="source-code"]')?.textContent).toContain("goal: 'Review the diff'")
    await act(async () => view.getByRole('tab', { name: 'Changes' }).click())
    expect(node.querySelector('[data-testid="changes-why"]')?.textContent).toBe('The mission file mission.ts did not change. The change comes from review.ts, which this mission uses.')
    expect([...node.querySelectorAll('[data-testid="file-diff"]')].map(file => file.getAttribute('data-path'))).toEqual(['lib/review.ts'])
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
      // Restore transport availability without replacing the mounted dialog or its loader.
      vi.resetModules()
      vi.doUnmock('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx')
      await act(async () => root.render(<><button type="button">Workspace remains available</button><DeferredDeclarationDialog subject="schedule/nightly" isOpen onOpenChange={onOpenChange} /></>))
      expect(await within(host).findByText('Only agents and missions have a declaration to show.')).toBeTruthy()
      expect(within(host).queryByRole('alert')).toBeNull()
      expect(host.contains(workspace)).toBe(true)
    } finally {
      vi.doUnmock('../../../../packages/fractal-ui/src/assistant-ui/declaration/DeclarationPanel.tsx')
    }
  })
})
