// Optional surfaces: imported only through DeferredRowActions.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Dialog, Popover, Text } from 'react-aria-components'
import { Menu, MenuItem, Popover as MenuPopover } from '../composition/Controls'
import { Icon } from '../composition/Icons'
import { borderVars as border, elevationVars as elevation, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { SidebarAgentRow as Row } from './model'
import type { AgentRowActionsProps } from './DeferredRowActions.tsx'
import { sidebarRowTitle, sidebarSubagentObservationText, usageText, tokenCount, spendCount, openRow } from './rowPresentation.tsx'
import { SidebarStatus } from './SidebarStatus'
import { SidebarTime, type SidebarClock } from './SidebarTime'
import { SidebarDuration } from './SidebarDuration'
import { useTouchLayout } from './useTouchLayout'

/** Complete reported facts stay accessible; unreported fields are omitted, never shown as placeholders. */
export function AgentRowDetails({ row, reportedStatusOnly = false, ...clock }: { readonly row: Row; readonly reportedStatusOnly?: boolean } & SidebarClock) {
  return <dl {...stylex.props(styles.details)}>
    <dt {...stylex.props(styles.detailLabel)}>Name</dt><dd {...stylex.props(styles.detailValue)}>{sidebarRowTitle(row)}</dd>
    <dt {...stylex.props(styles.detailLabel)}>ID</dt><dd {...stylex.props(styles.detailValue)}>{row.ref}</dd>
    {row.description !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Description</dt><dd {...stylex.props(styles.detailValue)}>{row.description}</dd></>}
    {(!reportedStatusOnly || row.status !== 'unobserved') && <><dt {...stylex.props(styles.detailLabel)}>Status</dt><dd {...stylex.props(styles.detailValue)}>{row.statusLabel}; {row.freshness} observation</dd></>}
    {row.statusSince !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Elapsed in status</dt><dd {...stylex.props(styles.detailValue)}><SidebarTime at={row.statusSince} {...clock} />; since {new Date(row.statusSince).toISOString()}</dd></>}
    {row.lastActivityAt !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Last activity</dt><dd {...stylex.props(styles.detailValue)}><SidebarTime at={row.lastActivityAt} {...clock} kind="activity" />; {new Date(row.lastActivityAt).toISOString()}</dd></>}
    <dt {...stylex.props(styles.detailLabel)}>Needs you</dt><dd {...stylex.props(styles.detailValue)}>{row.needsMe === undefined ? 'Unknown' : row.needsMe ? 'Yes' : 'No'}</dd><dt {...stylex.props(styles.detailLabel)}>Unread</dt><dd {...stylex.props(styles.detailValue)}>{row.unread ?? 'Unknown'}</dd>
    <dt {...stylex.props(styles.detailLabel)}>Host</dt><dd {...stylex.props(styles.detailValue)}>{row.host}</dd>
    {row.harness !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Harness</dt><dd {...stylex.props(styles.detailValue)}>{row.harness}</dd></>}
    {row.model !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Model</dt><dd {...stylex.props(styles.detailValue)}>{row.model}</dd></>}
    {row.usage._tag === 'Known' && <><dt {...stylex.props(styles.detailLabel)}>Usage</dt><dd {...stylex.props(styles.detailValue)}>{usageText(row.usage)}</dd></>}
    <ContextDetails row={row} />
    {row.duration._tag === 'Known' && <><dt {...stylex.props(styles.detailLabel)}>Duration</dt><dd {...stylex.props(styles.detailValue)}><SidebarDuration duration={row.duration} /></dd></>}
    {row.lastTurn._tag === 'Known' && <><dt {...stylex.props(styles.detailLabel)}>Last turn or activity</dt><dd {...stylex.props(styles.detailValue)}><SidebarTime at={row.lastTurn.at} {...clock} kind={row.lastTurn.kind === 'turn-completed' ? 'turn' : 'activity'} showKind /></dd></>}
    {row.branch !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Branch</dt><dd {...stylex.props(styles.detailValue)}>{row.branch}</dd></>}
    {row.worktree !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Worktree</dt><dd {...stylex.props(styles.detailValue)}>{row.worktree}</dd></>}
    {row.pullRequest !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Pull request</dt><dd {...stylex.props(styles.detailValue)}>#{row.pullRequest.number}: {row.pullRequest.title}; {row.pullRequest.state}; {row.pullRequest.ref}</dd></>}
    {row.terminal !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Terminal</dt><dd {...stylex.props(styles.detailValue)}>{row.terminal}</dd></>}
    {row.mission !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Mission</dt><dd {...stylex.props(styles.detailValue)}>{row.mission}</dd></>}
    {row.missionContext?._tag === 'Mission' && <>
      {row.missionContext.progress._tag === 'Known' && <><dt {...stylex.props(styles.detailLabel)}>Steps</dt><dd {...stylex.props(styles.detailValue)}>{row.missionContext.progress.done} of {row.missionContext.progress.total}</dd></>}
      {row.missionContext.step._tag === 'Step' && <><dt {...stylex.props(styles.detailLabel)}>Step</dt><dd {...stylex.props(styles.detailValue)}>{row.missionContext.step.name}</dd></>}
      {row.missionContext.step._tag === 'Waiting' && <><dt {...stylex.props(styles.detailLabel)}>Waiting for you</dt><dd {...stylex.props(styles.detailValue)}>{row.missionContext.step.handle === undefined ? '' : `${row.missionContext.step.handle} `}{row.missionContext.step.question}</dd></>}
      {row.missionContext.queued.length > 0 && <><dt {...stylex.props(styles.detailLabel)}>Queued missions</dt><dd {...stylex.props(styles.detailValue)}><ul>{row.missionContext.queued.map(mission => <li key={mission.id}>{mission.slug}</li>)}</ul></dd></>}
    </>}
    {row.subagent !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Subagent ID</dt><dd {...stylex.props(styles.detailValue)}>{row.subagent.id}</dd>
      {row.subagent.subagent_type !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Kind</dt><dd {...stylex.props(styles.detailValue)}>{row.subagent.subagent_type}</dd></>}
      {row.subagent.work_id !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Work</dt><dd {...stylex.props(styles.detailValue)}>{row.subagent.work_id}</dd></>}
      {row.subagent.session_id !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Session</dt><dd {...stylex.props(styles.detailValue)}>{row.subagent.session_id}</dd></>}
      {row.subagent.started_at !== undefined && <><dt {...stylex.props(styles.detailLabel)}>Started</dt><dd {...stylex.props(styles.detailValue)}><time dateTime={row.subagent.started_at}>{row.subagent.started_at}</time></dd></>}
      <dt {...stylex.props(styles.detailLabel)}>Lease expires</dt><dd {...stylex.props(styles.detailValue)}><time dateTime={row.subagent.lease_expires_at}>{row.subagent.lease_expires_at}</time></dd></>}
    <dt {...stylex.props(styles.detailLabel)}>Subagents</dt><dd {...stylex.props(styles.detailValue)}>{row.subagentObservation?._tag === 'Reported' ? `${row.children.length} collapsed children` : sidebarSubagentObservationText(row.subagentObservation)}{row.subagentObservation?._tag === 'Reported' && row.children.length > 0 && <ul>{row.children.map(child => <li key={child.ref}>{[sidebarRowTitle(child), child.ref, ...(child.subagent !== undefined ? [child.subagent.subagent_type, child.subagent.work_id, `lease ${child.subagent.lease_expires_at}`] : [])].filter((part): part is string => part !== undefined).join('; ')}</li>)}</ul>}</dd>
  </dl>
}
/** Only reported facts render; unknown or unavailable fields are omitted rather than shown as placeholders. */
export function AgentHoverCard({ row, reportedStatusOnly = false, ...clock }: { readonly row: Row; readonly reportedStatusOnly?: boolean } & SidebarClock) {
  const showStatus = !reportedStatusOnly || row.status !== 'unobserved'
  return <div data-testid="agent-hover-card" {...stylex.props(styles.card)}>
    <strong {...stylex.props(styles.cardTitle)}>{row.title}</strong>
    <div {...stylex.props(styles.cardSummary)}>{showStatus && <><SidebarStatus status={row.status} statusLabel={row.statusLabel} variant="SG-1" freshness={row.freshness} iconOnly /><span>{row.statusLabel}</span><span>·</span></>}<Icon name="gear" size={12} /><span>{row.host}</span></div>
    {row.description !== undefined && <p {...stylex.props(styles.cardWork)}><Icon name="message" size={12} />{row.description}</p>}
    <dl {...stylex.props(styles.details)}>
      {row.usage._tag === 'Known' && <><dt {...stylex.props(styles.cardLabel)}><Icon name="dot" size={12} />{row.usage.usd === undefined ? 'Tokens' : 'Spend'}</dt><dd {...stylex.props(styles.detailValue)}>{row.usage.usd === undefined ? '' : `${spendCount.format(row.usage.usd)} · `}{tokenCount.format(row.usage.tokens)} tokens<div {...stylex.props(styles.cardScope)}>{row.usage.scope === '24h-root-and-subagents' ? '24h · includes subagents' : row.usage.scope === 'lifetime-incl-subagents' ? 'Lifetime · includes subagents' : 'Reported by agent'}</div></dd></>}
      <ContextDetails row={row} />
      {row.duration._tag === 'Known' && <><dt {...stylex.props(styles.cardLabel)}><Icon name="clock" size={12} />Duration</dt><dd {...stylex.props(styles.detailValue)}><SidebarDuration duration={row.duration} /></dd></>}
      {row.lastTurn._tag === 'Known' && <><dt {...stylex.props(styles.cardLabel)}><Icon name="clock" size={12} />{row.lastTurn.kind === 'turn-completed' ? 'Last turn' : 'Last activity'}</dt><dd {...stylex.props(styles.detailValue)}><SidebarTime at={row.lastTurn.at} {...clock} kind={row.lastTurn.kind === 'turn-completed' ? 'turn' : 'activity'} /></dd></>}
      {row.model !== undefined && <><dt {...stylex.props(styles.cardLabel)}><Icon name="gear" size={12} />Model</dt><dd {...stylex.props(styles.detailValue)}>{row.model}</dd></>}
      {row.pullRequest !== undefined && <><dt {...stylex.props(styles.cardLabel)}><Icon name="swap" size={12} />PR</dt><dd {...stylex.props(styles.detailValue)}>#{row.pullRequest.number} · {row.pullRequest.title}</dd></>}
      {row.branch !== undefined && <><dt {...stylex.props(styles.cardLabel)}><Icon name="swap" size={12} />Branch</dt><dd {...stylex.props(styles.detailValue)}>{row.branch}</dd></>}
      {row.children.length > 0 && <><dt {...stylex.props(styles.cardLabel)}><Icon name="message" size={12} />Subagents</dt><dd {...stylex.props(styles.detailValue)}>{row.children.length}<ul {...stylex.props(styles.cardChildren)}>{row.children.slice(0, 3).map(child => <li key={child.ref}>{sidebarRowTitle(child)}</li>)}</ul></dd></>}
    </dl>
  </div>
}
/** Context is disclosed only in popovers, with its native scope and observation provenance. */
const ContextDetails = ({ row }: { readonly row: Row }) => row.context?._tag !== 'Known' ? null : <>
  <dt {...stylex.props(styles.detailLabel)}>Context</dt>
  <dd data-context-observation {...stylex.props(styles.detailValue)}>
    {row.freshness === 'live' ? 'Reported' : 'Last observed'} main-session context: {Math.round(row.context.usedPercent)}%
    {row.context.usedTokens === undefined ? '' : `; ${row.context.usedTokens.toLocaleString('en')} used tokens`}
    {row.context.windowTokens === undefined ? '' : `; ${row.context.windowTokens.toLocaleString('en')} token window`}
    {`; as of ${new Date(row.context.observedAt).toISOString()}`}
  </dd>
</>
// A 320px Agents sheet, the 340px details surface and its two 8px viewport gutters.
const compactDetailsQuery = '(max-width: 676px), (pointer: coarse)'
const subscribeDetailsLayout = (onChange: () => void) => {
  const media = window.matchMedia(compactDetailsQuery)
  media.addEventListener('change', onChange)
  return () => media.removeEventListener('change', onChange)
}
const compactDetailsLayout = () => window.matchMedia(compactDetailsQuery).matches
/** The reported-details surface mounts only after its eager DialogTrigger opens. */
export function AgentReportedDetails({ row, now, reportedStatusOnly = false, anchorRef, menuTriggerRef }: AgentRowActionsProps) {
  const touch = useTouchLayout()
  const compactDetails = React.useSyncExternalStore(subscribeDetailsLayout, compactDetailsLayout, () => false) || touch
  return <Popover triggerRef={compactDetails ? menuTriggerRef : anchorRef} placement={compactDetails ? 'bottom end' : 'right'} shouldFlip containerPadding={8}
    {...stylex.props(styles.hovercard, compactDetails && styles.detailsCompact)}
  ><Dialog aria-label={`Reported details for ${row.title}`}><AgentRowDetails row={row} reportedStatusOnly={reportedStatusOnly} now={now} /></Dialog></Popover>
}
/** One shared action menu for the live and generic rows, imported only when opened. */
export function AgentRowMenu({ row, actions, onOpen, onMarkRead, onDetailsPress }: AgentRowActionsProps) {
  const touch = useTouchLayout()
  const menuItemStyle = touch ? styles.menuItemFocusNarrow : styles.menuItemFocus
  const title = sidebarRowTitle(row)
  const ownerId = row.parentRef?.replace(/^agent\//, '') ?? row.id
  const canOpen = actions?.select !== undefined || onOpen !== undefined
  return <MenuPopover placement="bottom end"><Menu aria-label={`Actions for ${title}`}>
      <MenuItem id="open" textValue="Open conversation" isDisabled={!canOpen} onAction={() => openRow(row, actions, onOpen)} style={menuItemStyle}>Open conversation{!canOpen && <Text slot="description" {...stylex.props(styles.menuReason)}>{row.subagent !== undefined && row.subagent.session_id === undefined ? 'No session reported for this subagent' : 'Conversation opening is unavailable'}</Text>}</MenuItem>
      {actions?.split !== undefined && <MenuItem id="split-right" onAction={() => actions.split?.(row.ref, 'right')} style={menuItemStyle}>Open in split right</MenuItem>}
      {actions?.split !== undefined && <MenuItem id="split-below" onAction={() => actions.split?.(row.ref, 'below')} style={menuItemStyle}>Open in split below</MenuItem>}
      <MenuItem id="reported-details" onAction={onDetailsPress} style={menuItemStyle}>Reported agent details</MenuItem>
      {actions?.details !== undefined && <MenuItem id="details" onAction={() => actions.details?.(row.ref)} style={menuItemStyle}>Agent details</MenuItem>}
      {actions?.declaration !== undefined && row.subagent === undefined && <MenuItem id="declaration" onAction={() => actions.declaration?.(row.ref)} style={menuItemStyle}>View KDL</MenuItem>}
      {actions?.declaration !== undefined && row.mission !== undefined && <MenuItem id="mission-declaration" onAction={() => actions.declaration?.(row.mission!)} style={menuItemStyle}>View mission definition</MenuItem>}
      {(actions?.terminal !== undefined || actions?.resource !== undefined) && row.terminal !== undefined && <MenuItem id="terminal" onAction={() => actions?.terminal !== undefined ? actions.terminal(row.terminal!, ownerId) : actions?.resource?.(row.terminal!, ownerId)} style={menuItemStyle}>Open terminal</MenuItem>}
      {actions?.resource !== undefined && row.mission !== undefined && <MenuItem id="mission" onAction={() => actions.resource?.(row.mission!, ownerId)} style={menuItemStyle}>Open mission</MenuItem>}
      {actions?.resource !== undefined && row.worktree !== undefined && <MenuItem id="worktree" onAction={() => actions.resource?.(row.worktree!, ownerId)} style={menuItemStyle}>Open worktree</MenuItem>}
      {actions?.resource !== undefined && row.pullRequest !== undefined && <MenuItem id="pr" onAction={() => actions.resource?.(row.pullRequest!.ref, ownerId)} style={menuItemStyle}>Open pull request</MenuItem>}
      {(row.unread ?? 0) > 0 && onMarkRead !== undefined && <MenuItem id="read" onAction={() => onMarkRead(row.ref)} style={menuItemStyle}>Mark read</MenuItem>}
    </Menu></MenuPopover>
}
const styles = stylex.create({
  hovercard: { backgroundColor: surface.raised, color: ink.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, padding: s.lg, width: g.specimenAside, boxSizing: 'border-box', fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, maxWidth: 'calc(100vw - 32px)', maxHeight: g.commandMaxHeight, overflowY: 'auto', boxShadow: elevation.popover, zIndex: 30 },
  detailsCompact: { maxWidth: 'calc(100vw - 16px)' },
  details: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 2fr)', columnGap: s.md, rowGap: s.sm, margin: s.zero, overflowWrap: 'anywhere' },
  card: { display: 'flex', flexDirection: 'column', gap: s.md, width: g.specimenAside, maxWidth: '100%', boxSizing: 'border-box', overflowWrap: 'anywhere' },
  cardTitle: { fontSize: t.uiSize, lineHeight: t.uiLeading },
  cardSummary: { display: 'flex', alignItems: 'center', gap: s.sm, color: ink.fgSoft },
  cardWork: { display: 'flex', alignItems: 'start', gap: s.sm, margin: s.zero, paddingBlock: s.sm, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  cardScope: { color: ink.fgSoft, fontSize: t.denseSize },
  cardLabel: { display: 'flex', alignItems: 'center', gap: s.sm, margin: s.zero, color: ink.fgSoft },
  cardChildren: { margin: s.zero, padding: s.zero, listStyleType: 'none', color: ink.fgSoft },
  detailLabel: { margin: s.zero, color: ink.fgSoft },
  detailValue: { margin: s.zero, minWidth: 0, color: ink.fg },
  menuItemFocus: { outlineColor: { ':is([data-focus-visible])': ink.fgMuted } },
  menuReason: { display: 'block', whiteSpace: 'normal', fontSize: t.metaSize, lineHeight: t.metaLeading, paddingBlockEnd: s.xs },
  menuItemFocusNarrow: { outlineColor: { ':is([data-focus-visible])': ink.fgMuted }, height: 'auto', minHeight: g.touchTarget },
})
