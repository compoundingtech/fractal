import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Tabs, TabList, Tab, TabPanel } from 'react-aria-components'
import { Composer } from '../assistant-ui/composition/Composer'
import { Transcript, type TranscriptTurn } from '../assistant-ui/composition/Transcript'
import { SidebarAgentRow } from '../assistant-ui/sidebar/SidebarAgentRow'
import type { SidebarAgentRow as Row } from '../assistant-ui/sidebar/model'
import { Icon } from '../assistant-ui/composition/Icons'
import { EmbraceRuntimeProvider } from '../assistant-ui/EmbraceRuntime'
import type { TextItem } from '../assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from '../assistant-ui/taste/work-log'
import { baselineTheme } from '../assistant-ui/neutral-theme'
import { lightTheme } from '../assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, statusVars as tone, borderVars as border, spaceVars as s, typeVars as t, radiusVars as r, geometryVars as g, accentVars as accent } from '../assistant-ui/composition-tokens.stylex'
import { switchingGeometry as geometry } from './switching-tokens.stylex'

export const variants = ['A', 'B', 'C'] as const
export const scenarios = ['running', 'finished', 'failed', 'many', 'nested', 'attention'] as const
export interface SwitchingArgs { variant: typeof variants[number]; state: typeof scenarios[number]; compare: boolean; allStates: boolean; allLanes: boolean; scheme: 'dark' | 'light'; width: 1440 | 900 }
type Agent = { id: string; name: string; parent?: string; status: 'Running' | 'Finished' | 'Failed' | 'Needs you'; unread: number; task: string }
const now = Date.parse('2026-01-15T12:05:00Z')
const at = '2026-01-15T12:00:00Z'
const names = ['Layout', 'Keyboard', 'Coverage', 'Copy', 'Routing', 'Storage', 'History', 'Focus', 'Overflow', 'Messages', 'Resources', 'Review', 'Delivery', 'Recovery']
export function agentsFor(state: SwitchingArgs['state']): Agent[] {
  const root: Agent = { id: 'root', name: 'Workspace', status: 'Running', unread: 0, task: 'Coordinate the workspace navigation changes.' }
  const children: Agent[] = names.slice(0, state === 'many' ? 14 : 3).map((name, index) => ({ id: `child-${index + 1}`, name, parent: 'root', status: state === 'finished' ? 'Finished' : state === 'failed' && index === 0 ? 'Failed' : state === 'attention' && index === 0 ? 'Needs you' : index === 1 ? 'Finished' : 'Running', unread: state === 'attention' ? index === 0 ? 1 : 3 : 0, task: `Check ${name.toLowerCase()} behaviour in the workspace navigation.` }))
  if (state === 'nested') children.push({ id: 'grandchild', name: 'Key map', parent: 'child-1', status: 'Running', unread: 2, task: 'Check the key map delegated by Layout.' }, { id: 'great-grandchild', name: 'Shortcut audit', parent: 'grandchild', status: 'Finished', unread: 1, task: 'Audit the shortcuts delegated by Key map.' })
  return [root, ...children]
}
function asRow(agent: Agent, agents: Agent[]): Row {
  return { ref: `agent/${agent.id}`, id: agent.id, parentRef: agent.parent ? `agent/${agent.parent}` : undefined, title: agent.name, description: agent.task, status: agent.status === 'Running' ? 'working' : agent.status === 'Needs you' ? 'waiting' : 'ended', statusLabel: agent.status, host: 'Workspace', freshness: 'live', unread: agent.unread, needsMe: agent.status === 'Needs you', usage: { _tag: 'Known', scope: 'lifetime-incl-subagents', tokens: 1800, usd: 0.03 }, duration: { _tag: 'Known', scope: 'lifetime', ms: 45000 }, lastTurn: { _tag: 'Known', kind: 'activity', at: now - 60000 }, children: agents.filter(child => child.parent === agent.id).map(child => asRow(child, agents)) }
}
function AgentCue({ agent }: { agent: Agent }) {
  return <><span aria-hidden="true" {...stylex.props(styles.glyph, agent.status === 'Running' && styles.running, agent.status === 'Failed' && styles.failed, agent.status === 'Needs you' && styles.attention)}><Icon name={agent.status === 'Failed' ? 'x' : agent.status === 'Finished' ? 'check' : agent.status === 'Running' ? 'spinner' : 'alert'} /></span><span {...stylex.props(styles.name)}>{agent.name}</span><span {...stylex.props(styles.meta)}>{agent.status}{agent.unread > 0 ? ` · ${agent.unread} unread` : ''}</span></>
}
function threadFor(agent: Agent, sent: readonly string[]): TranscriptTurn[] {
  const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: `${agent.id}/prompt`, role: 'user', text: agent.task, attachments: [], streaming: false, at, sender: { kind: 'human', label: 'Operator' } }
  const text = agent.status === 'Failed' ? 'The navigation check stopped because the fixture could not load. The partial findings are preserved; the root agent can review the missing input.' : agent.status === 'Needs you' ? 'Should the switcher include finished agents by default? I have compared both paths and am waiting for your preference.' : agent.status === 'Finished' ? 'The navigation review is complete. Selection stays attached to the active thread, and returning to the root preserves the conversation.' : 'I am checking the navigation path. The root conversation stays available while this delegated thread continues independently.'
  const answer: TextItem = { _tag: 'Text', id: `${agent.id}/answer`, role: 'assistant', text, attachments: [], streaming: false, at: '2026-01-15T12:01:00Z', sender: { kind: 'agent', label: agent.name } }
  const items: TextItem[] = [answer, ...sent.map((message, index): TextItem => ({ ...prompt, id: `${agent.id}/sent-${index}`, text: message }))]
  return [{ id: agent.id, prompt, items, work: workLogTurnFromItems(items, { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true }), senderCaptions: { [answer.id]: agent.name } }]
}
export function SwitchingLane({ variant, state, scheme, width }: Pick<SwitchingArgs, 'variant' | 'state' | 'scheme' | 'width'>) {
  const agents = React.useMemo(() => agentsFor(state), [state])
  const [selected, setSelected] = React.useState(state === 'nested' ? 'great-grandchild' : state === 'finished' || state === 'failed' || state === 'attention' ? 'child-1' : 'root')
  const [drawer, setDrawer] = React.useState(false)
  const [messages, setMessages] = React.useState<Record<string, readonly string[]>>({})
  const [read, setRead] = React.useState<ReadonlySet<string>>(new Set())
  const active = agents.find(agent => agent.id === selected) ?? agents[0]!
  const turns = React.useMemo(() => threadFor(active, messages[active.id] ?? []), [active, messages])
  const runtime = React.useMemo(() => ({ messages: turns.flatMap(turn => [turn.prompt!, ...turn.items]), isRunning: false }), [turns])
  const choose = (id: string) => { setSelected(id); setRead(current => new Set([...current, id])) }
  const cycle = (direction: number) => choose(agents[(agents.findIndex(agent => agent.id === active.id) + direction + agents.length) % agents.length]!.id)
  const displayed = agents.slice(0, width === 900 ? 3 : 5)
  if (!displayed.some(agent => agent.id === active.id)) displayed.push(active)
  const path: Agent[] = [active]
  while (path[0]!.parent) path.unshift(agents.find(agent => agent.id === path[0]!.parent)!)
  const cue = (agent: Agent) => <AgentCue agent={read.has(agent.id) ? { ...agent, unread: 0 } : agent} />
  const overflow = <Button aria-label="All agent threads" aria-expanded={drawer} onPress={() => setDrawer(value => !value)} {...stylex.props(styles.control)}><Icon name="drawer" />{agents.length} threads</Button>
  const strip = <nav aria-label="Bottom agent switcher" {...stylex.props(styles.strip)}>{displayed.map(agent => <Button key={agent.id} aria-label={`Switch to ${agent.name}`} aria-pressed={active.id === agent.id} onPress={() => choose(agent.id)} {...stylex.props(styles.control, active.id === agent.id && styles.selected)}>{cue(agent)}</Button>)}{overflow}</nav>
  return <section data-switching-lane data-variant={variant} data-state={state} data-scheme={scheme} data-width={width} data-active={active.id} aria-label={`${variant} ${state} ${scheme} ${width}`} onKeyDown={event => { if (event.ctrlKey && (event.key === 'ArrowRight' || event.key === 'ArrowLeft')) { event.preventDefault(); cycle(event.key === 'ArrowRight' ? 1 : -1) } }} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.lane, width === 1440 ? styles.wide : styles.narrow)}>
    <header {...stylex.props(styles.heading)}><strong>{variant} · {variant === 'A' ? 'Composer rail' : variant === 'B' ? 'Thread tabs' : 'Breadcrumb + thread drawer'}</strong><span>{state} / {scheme} / {width}</span>{cue(active)}<span>Ctrl + Left / Right cycles threads</span></header>
    <div {...stylex.props(styles.shell)}><aside aria-label="Workspace sidebar" {...stylex.props(styles.sidebar)}><SidebarAgentRow item={asRow(agents[0]!, agents)} now={now} active={active.id === 'root'} onOpen={() => choose('root')} /><p {...stylex.props(styles.hint)}>The root stays here. Child threads open in the conversation lane.</p></aside>
      <div {...stylex.props(styles.workspace)}>
        {variant === 'B' ? <Tabs selectedKey={active.id} onSelectionChange={key => choose(String(key))}><div {...stylex.props(styles.strip)}><TabList aria-label="Agent thread tabs" {...stylex.props(styles.tabs)}>{agents.map(agent => <Tab id={agent.id} key={agent.id} {...stylex.props(styles.control, active.id === agent.id && styles.selected)}>{cue(agent)}</Tab>)}</TabList>{overflow}</div>{agents.map(agent => <TabPanel id={agent.id} key={agent.id}><span {...stylex.props(styles.meta)}>Selected thread: {agent.name}</span></TabPanel>)}</Tabs> : null}
        {variant === 'C' ? <nav aria-label="Agent ancestry" {...stylex.props(styles.strip)}>{path.map((agent, index) => <React.Fragment key={agent.id}>{index > 0 ? <Icon name="chevron-right" /> : null}<Button onPress={() => choose(agent.id)} aria-label={`Switch to ${agent.name}`} {...stylex.props(styles.control)}>{agent.name}</Button></React.Fragment>)}{overflow}</nav> : null}
        <div {...stylex.props(styles.body)}>
          <EmbraceRuntimeProvider options={runtime}>
            <div {...stylex.props(styles.conversation)}>
              <div {...stylex.props(styles.transcript)}><Transcript key={active.id} title={active.name} turns={turns} sync={{ _tag: 'Live', since: now - 5000 }} now={now} observedAt={now} /></div>
              <div {...stylex.props(styles.composer)}>
                {active.status === 'Needs you' ? <div role="status" {...stylex.props(styles.needs)}>{active.name} is waiting on your preference. Reply in this thread.</div> : null}
                <Composer agent={active.name} draftKey={`switch-explore/${variant}/${state}/${scheme}/${width}/${active.id}`} onSend={message => setMessages(current => ({ ...current, [active.id]: [...(current[active.id] ?? []), message] }))} />
              </div>
            </div>
          </EmbraceRuntimeProvider>
          {drawer ? <aside aria-label="All agent threads" data-thread-drawer {...stylex.props(styles.drawer)}><header {...stylex.props(styles.heading)}><strong>Agent threads · {agents.length}</strong><Button aria-label="Close agent threads" onPress={() => setDrawer(false)} {...stylex.props(styles.control)}><Icon name="x" /></Button></header>{agents.map(agent => { let depth = 0; let parent = agent.parent; while (parent) { depth++; parent = agents.find(candidate => candidate.id === parent)?.parent } return <div key={agent.id} data-depth={depth} {...stylex.props(depth > 0 && styles.indent, depth > 1 && styles.indentTwo, depth > 2 && styles.indentThree)}><SidebarAgentRow item={asRow(read.has(agent.id) ? { ...agent, unread: 0 } : agent, agents)} now={now} active={agent.id === active.id} variant="SR-2" onOpen={() => choose(agent.id)} /></div> })}</aside> : null}
        </div>
        {variant === 'A' ? strip : null}
      </div>
    </div>
  </section>
}
const styles = stylex.create({
  lane: { backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, flexShrink: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border }, wide: { width: geometry.wide }, narrow: { width: geometry.narrow },
  heading: { display: 'flex', alignItems: 'center', gap: s.lg, padding: s.lg, color: ink.fgMuted, fontSize: t.metaSize }, shell: { display: 'grid', gridTemplateColumns: geometry.columns, height: geometry.height }, sidebar: { padding: s.md, borderRightWidth: g.hairline, borderRightStyle: 'solid', borderRightColor: border.border }, hint: { padding: s.lg, color: ink.fgMuted, lineHeight: t.uiLeading },
  workspace: { display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0 }, body: { display: 'flex', flex: 1, minHeight: 0 }, conversation: { flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }, transcript: { flex: 1, minHeight: 0, overflow: 'auto' }, composer: { padding: s.lg }, needs: { color: tone.attention, fontSize: t.metaSize, padding: s.md },
  strip: { display: 'flex', alignItems: 'center', gap: s.xs, padding: s.md, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, overflowX: 'auto', flexShrink: 0 }, tabs: { display: 'flex', gap: s.xs, overflowX: 'auto', minWidth: 0, flex: 1 },
  control: { display: 'inline-flex', alignItems: 'center', gap: s.sm, minHeight: g.controlLg, paddingInline: s.md, paddingBlock: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', flexShrink: 0, ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, selected: { backgroundColor: surface.rowActive, color: ink.fg, borderColor: accent.primary }, name: { whiteSpace: 'nowrap' }, meta: { color: ink.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap' }, glyph: { width: g.icon, height: g.icon, display: 'inline-flex' }, running: { color: tone.runningFg }, failed: { color: tone.dangerFg }, attention: { color: tone.attention },
  drawer: { width: geometry.drawer, flexShrink: 0, overflowY: 'auto', borderLeftWidth: g.hairline, borderLeftStyle: 'solid', borderLeftColor: border.border, backgroundColor: surface.raised }, indent: { marginLeft: geometry.indent }, indentTwo: { marginLeft: `calc(${geometry.indent} * 2)` }, indentThree: { marginLeft: `calc(${geometry.indent} * 3)` },
})
