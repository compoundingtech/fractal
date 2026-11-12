/**
 * The layout grammar every entity card shares, taken from the sidebar agent card:
 *   header   title, then a summary line: status glyph · status word · where (host, folder, repository)
 *   text     an optional lead line (the agent's current work, a mission step, an excerpt), icon-led
 *   sections labelled groups (Missions, Agents, Files)
 *   meta     icon-led label / value lines (spend, duration, last turn, model, PR, branch, times)
 *   note     muted provenance (stale, failed read, scope)
 * Only reported facts render: an unknown field is omitted, never a placeholder. The card has no actions of its own;
 * it is passive inside `HoverCardSurface`, and actions stay on the row menu, the tab and the details dialog.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Icon, type IconName } from '../composition/Icons'
import { borderVars as border, geometryVars as g, spaceVars as s, statusVars as tone, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import type { EntityRef, EntityStatus } from './entity.ts'

export interface EntityCardProps {
  readonly entity: EntityRef
  readonly children: React.ReactNode
  /** Kept for existing proofs, e.g. `agent-hover-card`. */
  readonly testId?: string
}
export function EntityCard({ entity, children, testId }: EntityCardProps) {
  return <div data-entity-card={entity.kind} data-entity-id={entity.id} data-testid={testId} {...stylex.props(styles.card)}>{children}</div>
}

export interface EntityCardHeaderProps {
  readonly title: React.ReactNode
  /** The kind word ("Mission", "Pull request", "Folder"); agents omit it, as the sidebar card does. */
  readonly kind?: string
  /** The status glyph; any node (a status mark, an initials disc), never an image load. */
  readonly glyph?: React.ReactNode
  /** Status word and location, joined by middots in the order given. */
  readonly summary?: readonly React.ReactNode[]
}
export function EntityCardHeader({ title, kind, glyph, summary = [] }: EntityCardHeaderProps) {
  const parts = summary.filter(part => part !== undefined && part !== null && part !== false && part !== '')
  return <>
    <div {...stylex.props(styles.titleRow)}>
      <strong data-entity-card-title {...stylex.props(styles.title)}>{title}</strong>
      {kind !== undefined && <span data-entity-card-kind {...stylex.props(styles.kind)}>{kind}</span>}
    </div>
    {(glyph !== undefined || parts.length > 0) && <div data-entity-card-summary {...stylex.props(styles.summary)}>
      {glyph}
      {parts.map((part, index) => <React.Fragment key={index}>{index > 0 && <span aria-hidden="true">·</span>}<span {...stylex.props(styles.summaryPart)}>{part}</span></React.Fragment>)}
    </div>}
  </>
}

/** A lead line under the header, e.g. the agent's current work. */
export function EntityCardText({ icon = 'message', children, ...data }: { readonly icon?: IconName; readonly children: React.ReactNode } & Record<`data-${string}`, string | boolean | undefined>) {
  return <p data-entity-card-text {...data} {...stylex.props(styles.text)}><Icon name={icon} size={12} />{children}</p>
}

/** A labelled group, e.g. Missions. */
export function EntityCardSection({ label, icon, testId, children }: { readonly label: string; readonly icon: IconName; readonly testId?: string; readonly children: React.ReactNode }) {
  // A group, not a region landmark: several cards (or one card twice) on a page would repeat the landmark name.
  return <section role="group" aria-label={label} data-testid={testId} {...stylex.props(styles.section)}>
    <div {...stylex.props(styles.label)}><Icon name={icon} size={12} />{label}</div>
    {children}
  </section>
}

export function EntityCardMeta({ children }: { readonly children: React.ReactNode }) {
  return <dl data-entity-card-meta {...stylex.props(styles.meta)}>{children}</dl>
}
export function EntityCardMetaLine({ icon, label, children }: { readonly icon: IconName; readonly label: React.ReactNode; readonly children: React.ReactNode }) {
  return <><dt {...stylex.props(styles.label)}><Icon name={icon} size={12} />{label}</dt><dd {...stylex.props(styles.value)}>{children}</dd></>
}

export function EntityCardNote({ children, ...data }: { readonly children: React.ReactNode } & Record<`data-${string}`, string | undefined>) {
  return <p {...data} {...stylex.props(styles.note)}>{children}</p>
}

/** A compact list inside a section: a title with an optional trailing count and a muted second line. */
export function EntityCardList({ children }: { readonly children: React.ReactNode }) {
  return <ul {...stylex.props(styles.list)}>{children}</ul>
}
export function EntityCardListItem({ title, trailing, detail }: { readonly title: React.ReactNode; readonly trailing?: React.ReactNode; readonly detail?: React.ReactNode }) {
  return <li {...stylex.props(styles.item)}>
    <span {...stylex.props(styles.itemHead)}><span {...stylex.props(styles.itemTitle)}>{title}</span>{trailing !== undefined && <span {...stylex.props(styles.itemCount)}>{trailing}</span>}</span>
    {detail !== undefined && <span {...stylex.props(styles.itemDetail)}>{detail}</span>}
  </li>
}

/** A generic status glyph for entities without their own (missions, resources): color only for needs-you and failed. */
export function EntityStatusMark({ status }: { readonly status: EntityStatus }) {
  return <span role="img" aria-label={status.label} data-entity-status={status.tone} {...stylex.props(styles.mark, status.tone === 'needs-you' && styles.markNeeds, status.tone === 'failed' && styles.markFailed, status.tone === 'working' && styles.markWorking, status.tone === 'done' && styles.markDone)} />
}

const styles = stylex.create({
  card: { display: 'flex', flexDirection: 'column', gap: s.md, width: g.specimenAside, maxWidth: '100%', boxSizing: 'border-box', overflowWrap: 'anywhere' },
  titleRow: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: s.md, minWidth: 0 },
  title: { minWidth: 0, fontSize: t.uiSize, lineHeight: t.uiLeading },
  kind: { flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize },
  summary: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: s.sm, color: ink.fgSoft },
  summaryPart: { display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0 },
  text: { display: 'flex', alignItems: 'start', gap: s.sm, margin: s.zero, paddingBlock: s.sm, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  section: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingBlockStart: s.sm, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border },
  label: { display: 'flex', alignItems: 'center', gap: s.sm, margin: s.zero, color: ink.fgSoft },
  meta: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) minmax(0, 2fr)', columnGap: s.md, rowGap: s.sm, margin: s.zero, overflowWrap: 'anywhere' },
  value: { margin: s.zero, minWidth: 0, color: ink.fg },
  note: { margin: s.zero, color: ink.fgSoft, fontSize: t.denseSize },
  list: { display: 'flex', flexDirection: 'column', gap: s.xs, margin: s.zero, padding: s.zero, listStyleType: 'none' },
  item: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  itemHead: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: s.sm, minWidth: 0 },
  itemTitle: { minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', color: ink.fg },
  itemCount: { flexShrink: 0, color: ink.fgSoft, fontVariantNumeric: 'tabular-nums' },
  itemDetail: { minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', color: ink.fgSoft, fontSize: t.denseSize },
  mark: { display: 'inline-block', flexShrink: 0, width: 8, height: 8, borderRadius: '50%', borderWidth: g.hairline, borderStyle: 'solid', borderColor: ink.fgMuted, boxSizing: 'border-box' },
  markWorking: { borderColor: ink.fgSoft },
  markDone: { backgroundColor: ink.fgMuted },
  markNeeds: { backgroundColor: tone.attention, borderColor: tone.attention },
  markFailed: { backgroundColor: tone.danger, borderColor: tone.danger },
})
