import * as stylex from '@stylexjs/stylex'
import { Button, VisuallyHidden } from 'react-aria-components'
import { SidebarStatus } from '../assistant-ui/sidebar/SidebarStatus'
import { Icon } from '../assistant-ui/composition/Icons'
import { activeCountName, activeSubagents, failedSubagents, sidebarFixtures, waitingSubagents, type Scenario, type WorkspaceFixture } from './switching-fixtures'
import { surfaceVars as surface, textVars as ink, statusVars as tone, borderVars as border, spaceVars as s, typeVars as t, radiusVars as r, geometryVars as g, accentVars as accent } from '../assistant-ui/composition-tokens.stylex'

/** P1 puts the count right after the name, where row metadata reads; P2 puts it at the trailing edge beside the status disc. */
export type CountPlacement = 'chip' | 'badge'
export function SwitchingSidebar({ placement, selected, context, onOpen }: { placement: CountPlacement; selected: Scenario; context?: string; onOpen: (state: Scenario, focusSwitcher: boolean) => void }) {
  return <nav aria-label={context === undefined ? 'Agents' : `Agents, ${context}`}>
    <ul {...stylex.props(styles.list)}>{sidebarFixtures.map(fixture => <li key={fixture.state}><CountRow fixture={fixture} placement={placement} selected={fixture.state === selected} onOpen={onOpen} /></li>)}</ul>
  </nav>
}
function CountRow({ fixture, placement, selected, onOpen }: { fixture: WorkspaceFixture; placement: CountPlacement; selected: boolean; onOpen: (state: Scenario, focusSwitcher: boolean) => void }) {
  const active = activeSubagents(fixture)
  const failed = failedSubagents(fixture) > 0
  const waiting = waitingSubagents(fixture) > 0
  const visible = placement === 'chip' ? `${active} active` : String(active)
  // The visible text is the start of the spoken name ("3 active" + " subagents"), so label-in-name holds without an aria-label override.
  const count = active === 0 ? null : <Button data-active-count={active} onPress={() => onOpen(fixture.state, true)} {...stylex.props(styles.countHit)}>
    <span {...stylex.props(styles.tag, placement === 'badge' && styles.badge, failed && styles.failed, waiting && styles.waiting)}>{failed || waiting ? <Icon name={failed ? 'x' : 'alert'} size={12} /> : null}<span data-count-text>{visible}</span></span>
    <VisuallyHidden elementType="span">{activeCountName(fixture).slice(visible.length)}</VisuallyHidden>
  </Button>
  // The empty stretch and the disc extend the name's pointer target; keyboard users reach the same action on the name button.
  const openRow = () => onOpen(fixture.state, false)
  return <div data-count-row={fixture.state} data-placement={placement} {...stylex.props(styles.row, selected && styles.selected)}>
    <Button aria-current={selected ? 'page' : undefined} onPress={openRow} {...stylex.props(styles.name)}><span data-row-name {...stylex.props(styles.nameText)}>{fixture.label}</span></Button>
    {placement === 'chip' ? count : null}
    <span aria-hidden="true" onClick={openRow} {...stylex.props(styles.fill)} />
    <span onClick={openRow} {...stylex.props(styles.disc)}><SidebarStatus status={waiting ? 'waiting' : 'working'} statusLabel={waiting ? 'Needs you' : 'Running'} freshness="live" variant="SG-1" iconOnly /></span>
    {placement === 'badge' ? count : null}
  </div>
}
const styles = stylex.create({
  list: { display: 'flex', flexDirection: 'column', gap: s.md, margin: s.zero, padding: s.zero, listStyle: 'none' },
  row: { display: 'flex', alignItems: 'center', gap: s.md, height: g.controlLg, minWidth: 0, boxSizing: 'border-box', paddingInline: s.md, borderRadius: r.sm, color: ink.fg, ':hover': { backgroundColor: surface.rowHover } },
  selected: { backgroundColor: surface.rowActive, ':hover': { backgroundColor: surface.rowActive } },
  name: { flex: '0 1 auto', minWidth: 0, display: 'flex', alignItems: 'center', height: g.controlLg, padding: s.zero, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  nameText: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  fill: { flex: '1 1 0', minWidth: 0, alignSelf: 'stretch', cursor: 'pointer' },
  disc: { width: g.icon, height: g.icon, flexShrink: 0, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' },
  // The hit area stays 24px tall; the visible tag inside it is the 11/16 tag: 1px border, r6, no fill, tone on text and border only.
  countHit: { flexShrink: 0, display: 'inline-flex', alignItems: 'center', minHeight: g.controlSm, padding: s.zero, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset } },
  tag: { display: 'inline-flex', alignItems: 'center', gap: s.xs, paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' },
  badge: { justifyContent: 'center', minWidth: g.icon, color: ink.fgSoft },
  failed: { color: tone.dangerFg, borderColor: tone.dangerMuted },
  waiting: { color: tone.attention, borderColor: tone.attention },
})
