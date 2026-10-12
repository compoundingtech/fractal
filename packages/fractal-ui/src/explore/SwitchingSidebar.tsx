import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { SidebarStatus } from '../assistant-ui/sidebar/SidebarStatus'
import { Icon } from '../assistant-ui/composition/Icons'
import { activeSubagents, sidebarFixtures, type Scenario, type WorkspaceFixture } from './switching-fixtures'
import { surfaceVars as surface, textVars as ink, statusVars as tone, borderVars as border, spaceVars as s, typeVars as t, radiusVars as r, geometryVars as g, accentVars as accent } from '../assistant-ui/composition-tokens.stylex'

export type CountPlacement = 'chip' | 'badge'
export function SwitchingSidebar({ placement, selected, onOpen }: { placement: CountPlacement; selected: Scenario; onOpen: (state: Scenario) => void }) {
  return <nav aria-label="Workspace agents" {...stylex.props(styles.list)}>{sidebarFixtures.map(fixture => <CountRow key={fixture.state} fixture={fixture} placement={placement} selected={fixture.state === selected} onOpen={() => onOpen(fixture.state)} />)}</nav>
}
function CountRow({ fixture, placement, selected, onOpen }: { fixture: WorkspaceFixture; placement: CountPlacement; selected: boolean; onOpen: () => void }) {
  const count = activeSubagents(fixture)
  const failed = fixture.agents.filter(agent => agent.status === 'Failed').length
  const needs = fixture.agents.filter(agent => agent.status === 'Needs you').length
  const visible = placement === 'chip' ? `${count} active` : String(count)
  const label = `${count} active subagents${failed ? `, ${failed} failed subagent` : ''}${needs ? `, ${needs} subagent needs you` : ''}`
  const counter = <Button data-active-count={count} data-count-visible={visible} aria-label={label} onPress={onOpen} {...stylex.props(styles.count, placement === 'chip' && styles.chip, failed > 0 && styles.failed, needs > 0 && styles.attention)}>{failed > 0 || needs > 0 ? <Icon name={failed > 0 ? 'x' : 'alert'} scale="status" /> : null}<span data-count-text>{visible}</span></Button>
  return <div data-count-row={fixture.state} data-placement={placement} {...stylex.props(styles.row, selected && styles.selected)}><Button onPress={onOpen} aria-label={`Open ${fixture.label}`} {...stylex.props(styles.nameButton)}><span data-row-name {...stylex.props(styles.name)}>{fixture.label}</span></Button>{placement === 'chip' ? <span data-meta-line {...stylex.props(styles.meta)}>{counter}</span> : null}<span data-status-disc {...stylex.props(styles.disc)}><SidebarStatus status={needs > 0 ? 'waiting' : 'working'} statusLabel={needs > 0 ? 'Needs you' : 'Running'} freshness="live" variant="SG-1" iconOnly /></span>{placement === 'badge' ? counter : null}</div>
}
const styles = stylex.create({
  list: { display: 'flex', flexDirection: 'column', gap: s.md },
  row: { display: 'flex', alignItems: 'center', gap: s.md, height: g.controlLg, minWidth: 0, width: '100%', boxSizing: 'border-box', paddingInline: s.sm, borderRadius: r.sm, color: ink.fg, ':hover': { backgroundColor: surface.rowHover } }, selected: { backgroundColor: surface.rowActive },
  nameButton: { flex: '1 1 0', minWidth: 0, display: 'flex', alignItems: 'center', borderWidth: 0, backgroundColor: surface.transparent, color: ink.fg, padding: s.zero, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, name: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  meta: { display: 'inline-flex', flexShrink: 0, alignItems: 'center' },
  count: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', gap: s.xs, minHeight: g.controlSm, paddingInline: s.sm, paddingBlock: s.zero, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgSoft, backgroundColor: surface.transparent, cursor: 'pointer', whiteSpace: 'nowrap', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } }, chip: { color: ink.fgMuted }, disc: { width: g.icon, height: g.icon, flexShrink: 0, display: 'inline-flex', alignItems: 'center' }, failed: { color: tone.dangerFg }, attention: { color: tone.attention },
})
