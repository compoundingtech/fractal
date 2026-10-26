import * as React from 'react'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { Schema } from 'effect'
import { persistedAtom } from '../state/persistence.ts'
import { TerminalScreen, decodeUnknownSync } from '@fractal/client/schema'
import { DataSourceProvider } from '../data/react.tsx'
import { fixtureSource } from '../data/fixtureSource.ts'
import { observed, waiting } from '../data/source.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveAgentWorkspace } from '../web/LiveAgentWorkspace.tsx'
import { DomTerminal } from '../terminal/DomTerminal.tsx'
import { defaultTerminalPalette } from '../terminal/palette.ts'

export const fragments = ['web', 'web-narrow', 'claude-code', 'codex'] as const
export type EmbedFragment = typeof fragments[number]
export const embedNow = fixtureProjections.now
export const embedAgent = 'agent/build-host-a/workbench-shell'
const resources = { byAgent: () => Atom.make(observed({ value: { items: [], nextCursor: null } })), byId: () => Atom.make(waiting), subjects: Atom.make([]), loadMore: () => undefined, refresh: () => undefined, resolveFile: () => undefined }
const source = { ...fixtureSource({ world: fixtureProjections }), resources }
// Eager JSON modules are bundled, never fetched at runtime. Missing real captures remain explicit.
const captures = import.meta.glob('./captures/*.json', { eager: true, import: 'default' })
export const terminalCapture = (fragment: 'claude-code' | 'codex'): TerminalScreen | undefined => {
  const frame = captures[`./captures/${fragment}.json`]
  return frame === undefined ? undefined : decodeUnknownSync(TerminalScreen)(frame)
}
export const Embed = ({ fragment, scheme = 'dark' }: { readonly fragment: EmbedFragment; readonly scheme?: 'light' | 'dark' }) => {
  const [registry] = React.useState(() => {
    const value = AtomRegistry.make()
    value.set(persistedAtom({ key: 'round2.scheme', schema: Schema.Literals(['dark', 'light']), defaultValue: 'dark' }), scheme)
    value.set(persistedAtom({ key: 'round2.agent', schema: Schema.String, defaultValue: '' }), embedAgent)
    value.set(persistedAtom({ key: 'round2.sidebarClosed', schema: Schema.Boolean, defaultValue: false }), false)
    value.set(persistedAtom({ key: `round2.panes.${embedAgent}`, schema: Schema.Array(Schema.Struct({ ref: Schema.String, presentation: Schema.optional(Schema.Literals(['detail', 'overview', 'resources'])) })), defaultValue: [] }), [])
    return value
  })
  React.useEffect(() => () => registry.dispose(), [registry])
  if (fragment === 'claude-code' || fragment === 'codex') {
    const screen = terminalCapture(fragment)
    return screen === undefined
      ? <main className="capture-pending" role="status"><h1>{fragment === 'codex' ? 'Codex' : 'Claude Code'}</h1><p>Capture pending — a real authenticated CLI frame has not been recorded.</p></main>
      : <DomTerminal screen={screen} palette={defaultTerminalPalette} />
  }
  return <div className={fragment === 'web-narrow' ? 'embed-narrow' : 'embed-wide'}><DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace /></DataSourceProvider></div>
}
