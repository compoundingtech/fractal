// RM1 reference thread parts on the kit `Pane`: one 32px pane header owns identity, the run state appears once at the
// stream tail, and the kit Transcript renders without its own title band (`showHeader={false}`).
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { Transcript, type TranscriptProps } from '../../assistant-ui/composition/Transcript'
import { Composer, type ComposerProps } from '../../assistant-ui/composition/Composer'
import { TranscriptSkeleton } from '../../assistant-ui/composition/TranscriptSkeleton'
import { EmbraceRuntimeProvider } from '../../assistant-ui/EmbraceRuntime'
import { SidebarStatus } from '../../assistant-ui/sidebar/SidebarStatus'
import type { AgentStatus } from '../../assistant-ui/sidebar/model'
import type { ThreadResource } from '../../assistant-ui/workbench/workbench-model'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'

/** A pane tab: an agent conversation carries a status glyph; a tool surface (Changes) does not. */
export interface PaneTab { readonly key: string; readonly title: string; readonly status?: AgentStatus; readonly statusLabel?: string }

/** Pane tabs use the roster's glyph (SG-1 · G3), so status reads consistently across surfaces. */
export function PaneGlyph({ tab }: { readonly tab: PaneTab }) {
  if (tab.status === undefined) return null
  return <SidebarStatus status={tab.status} statusLabel={tab.statusLabel} variant="SG-1" discRefinement="G3" iconOnly />
}

export type ThreadView =
  | { readonly _tag: 'Live' }
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Stale'; readonly lastSeen: { readonly activity: string; readonly at: number; readonly detail: string } }
  | { readonly _tag: 'Unavailable'; readonly reason: string; readonly detail: string; readonly onRetry: () => void }

export interface MilestoneThreadProps {
  readonly title: string
  readonly runtime: ThreadResource['runtime']
  readonly transcript: Omit<TranscriptProps, 'title' | 'showHeader'>
  readonly composer: Omit<ComposerProps, 'agent'>
  readonly view: ThreadView
  readonly now: number
}

/** The kit thread surface without its title band; the composer dock and lane match the kit pane exactly. */
export function MilestoneThread({ title, runtime, transcript, composer, view, now }: MilestoneThreadProps) {
  return <EmbraceRuntimeProvider options={runtime}>
    <div {...stylex.props(styles.frame)}>
      {view._tag === 'Unavailable' && <div data-thread-unavailable {...stylex.props(styles.lane, styles.notice)}><UnavailableNotice title={view.reason} detail={view.detail} onRetry={view.onRetry} /></div>}
      {view._tag === 'Loading' && <div data-thread-loading {...stylex.props(styles.lane, styles.notice)}><TranscriptSkeleton sync={transcript.sync} now={now} observedAt={transcript.observedAt} /></div>}
      {(view._tag === 'Live' || view._tag === 'Stale') && <Transcript {...transcript} title={title} showHeader={false} />}
      {view._tag === 'Stale' && <LastSeen activity={view.lastSeen.activity} at={view.lastSeen.at} detail={view.lastSeen.detail} now={now} />}
    </div>
    <div data-testid="workbench-composer-dock" {...stylex.props(styles.dock, dockTheme)}>
      <div {...stylex.props(styles.lane)}><Composer agent={title} {...composer} /></div>
    </div>
  </EmbraceRuntimeProvider>
}

/**
 * A thread that failed to load. The title carries the danger tone; the detail wraps to its own line in body ink, so the
 * reassurance ("Your draft is unchanged") is never cut off; Retry is a real bordered button.
 */
function UnavailableNotice({ title, detail, onRetry }: { readonly title: string; readonly detail: string; readonly onRetry: () => void }) {
  return <div data-system-state="error" {...stylex.props(styles.unavailable)}>
    <p role="status" {...stylex.props(styles.unavailableText)}>
      <span {...stylex.props(styles.unavailableTitle)}>{title}</span>
      <span {...stylex.props(styles.unavailableDetail)}>{detail}</span>
    </p>
    <Button aria-label={`Retry: ${title}`} onPress={onRetry} {...stylex.props(styles.actionButton)}>Retry</Button>
  </div>
}

/** q23: last-known activity is labelled as last seen, with no spinner and no running clock. */
function LastSeen({ activity, at, detail, now }: { readonly activity: string; readonly at: number; readonly detail: string; readonly now: number }) {
  return <div {...stylex.props(styles.lane)}>
    <p role="status" data-last-seen {...stylex.props(styles.lastSeen)}>
      {/* The sentence says "Last seen"; the glyph is decorative so the live region reads only the sentence. */}
      <SidebarStatus status="stale" statusLabel="Last seen" variant="SG-1" discRefinement="G3" iconOnly decorative />
      <span>{`Last seen ${activity} · ${Math.round((now - at) / 1000)} s ago`}</span>
      <span aria-hidden="true" {...stylex.props(styles.sep)}>·</span>
      <code {...stylex.props(styles.code)}>{detail}</code>
    </p>
  </div>
}

/** RC-1 inside prose: phrasing content only (span + button), so the card is valid wherever the paragraph puts it. */
export function AgentResultCard({ tab, summary, onOpen }: { readonly tab: PaneTab; readonly summary: string; readonly onOpen: () => void }) {
  return <span data-resource-card="RC-1" {...stylex.props(styles.card)}>
    <Button onPress={onOpen} {...stylex.props(styles.cardButton)}>
      <PaneGlyph tab={tab} />
      <span {...stylex.props(styles.cardTitle)}>{tab.title}</span>
      <span {...stylex.props(styles.cardSummary)}>{summary}</span>
      <span {...stylex.props(styles.cardMeta)}>{tab.statusLabel}</span>
      <span {...stylex.props(styles.cardMeta)}>Open</span>
    </Button>
  </span>
}

const dockTheme = stylex.createTheme(g, { blur: '0px' })
const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  glyph: { display: 'inline-flex', flexShrink: 0 },
  unavailable: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0 },
  unavailableText: { display: 'flex', flexDirection: 'column', gap: s.xs2, flex: '1 1 0', minWidth: 0, margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  unavailableTitle: { color: tone.dangerFg, fontWeight: t.weightMedium },
  unavailableDetail: { color: ink.fgSoft, overflowWrap: 'anywhere' },
  actionButton: { flexShrink: 0, height: g.controlSm, paddingInline: s.md, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  frame: { flex: '1 1 0', minHeight: 0, minWidth: 0, display: 'flex', flexDirection: 'column' },
  notice: { paddingBlock: s.lg },
  dock: { flexShrink: 0, paddingBlock: s.sm, backgroundColor: surface.canvas },
  lane: { width: '100%', maxWidth: g.lane, boxSizing: 'border-box', paddingInline: s.lg, marginInline: 'auto' },
  lastSeen: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, height: g.toolRow, margin: 0, color: ink.fgMuted, fontSize: t.metaSize, whiteSpace: 'nowrap', overflow: 'hidden' },
  sep: { color: ink.fgMuted },
  code: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', fontFamily: t.fontMono, fontSize: t.denseSize },
  card: { display: 'flex', width: '100%', minWidth: 0, borderRadius: r.md, overflow: 'hidden', backgroundColor: surface.washSubtle },
  cardButton: { display: 'flex', alignItems: 'center', gap: s.md, width: '100%', minWidth: 0, height: g.resourceCard, paddingInline: s.lg, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  cardTitle: { flexShrink: 0, maxWidth: '45%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fg, fontWeight: t.weightMedium },
  cardSummary: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgMuted },
  cardMeta: { flexShrink: 0, color: ink.fgMuted, fontSize: t.denseSize, whiteSpace: 'nowrap' },
})
