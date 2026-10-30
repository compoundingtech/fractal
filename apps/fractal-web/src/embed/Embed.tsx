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
import { createEmbedNavigation } from './navigation.ts'

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
  const headingId = React.useId()
  const [registry] = React.useState(() => {
    const value = AtomRegistry.make()
    value.set(persistedAtom({ key: 'round2.scheme', schema: Schema.Literals(['dark', 'light']), defaultValue: 'dark' }), scheme)
    value.set(persistedAtom({ key: 'round2.agent', schema: Schema.String, defaultValue: '' }), embedAgent)
    value.set(persistedAtom({ key: 'round2.sidebarClosed', schema: Schema.Boolean, defaultValue: false }), false)
    value.set(persistedAtom({ key: `round2.panes.${embedAgent}`, schema: Schema.Array(Schema.Struct({ ref: Schema.String, presentation: Schema.optional(Schema.Literals(['detail', 'overview', 'resources'])) })), defaultValue: [] }), [])
    return value
  })
  React.useEffect(() => () => registry.dispose(), [registry])
  const [navigation] = React.useState(() => createEmbedNavigation(`/w/${embedAgent}?open=thread`))
  const container = React.useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = React.useState(() => ({
    width: fragment === 'web-narrow' ? Math.min(window.innerWidth, 440) : window.innerWidth,
    height: window.innerHeight,
  }))
  React.useLayoutEffect(() => {
    const node = container.current
    if (node === null) return
    const measure = () => {
      const { width, height } = node.getBoundingClientRect()
      if (width > 0 && height > 0) setDimensions(previous => previous.width === width && previous.height === height ? previous : { width, height })
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(node)
    return () => observer.disconnect()
  }, [fragment])
  if (fragment === 'claude-code' || fragment === 'codex') {
    const screen = terminalCapture(fragment)
    return screen === undefined
      ? <section className="capture-pending" aria-labelledby={headingId}><h2 id={headingId}>{fragment === 'codex' ? 'Codex' : 'Claude Code'}</h2><p role="status">Capture pending — a real authenticated CLI frame has not been recorded.</p></section>
      : <DomTerminal screen={screen} palette={defaultTerminalPalette} />
  }
  return <div ref={container} className={fragment === 'web-narrow' ? 'embed-narrow' : 'embed-wide'}><DataSourceProvider source={source} registry={registry}><LiveAgentWorkspace dimensions={dimensions} navigation={navigation} persistGeometry={false} workspaceLabel={fragment === 'web-narrow' ? 'Fractal narrow client workspace' : 'Fractal web client workspace'} /></DataSourceProvider></div>
}
