import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { readingColumnStyles } from '../../../../packages/fractal-ui/src/assistant-ui/reading-column.stylex.ts'
import { ErrorOverlayHost } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ErrorOverlay.tsx'
import { SyncLine } from '../../../../packages/fractal-ui/src/assistant-ui/st3-views/SyncLine.tsx'
import { surfaceVars as surface, textVars as ink, borderVars as border, accentVars as accent, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'

const waiting = { _tag: 'Requested', since: 0 } as const

/** The import boundary has no conversation demand or runtime of its own. Keep the same
 * Waiting frame as the kit so loading code cannot move the header or history lane. */
export const ConversationPaneFallback = ({ agentName, visible = true }: {
  readonly agentName: string
  readonly visible?: boolean
}): React.ReactElement => <div style={{ display: 'contents' }} aria-hidden={!visible}>
  <div aria-label="Transcript" {...stylex.props(styles.frame)}>
    <header data-testid="transcript-header" {...stylex.props(styles.header)}><strong {...stylex.props(styles.title)}>{agentName}</strong></header>
    <ErrorOverlayHost lane>
      <div {...stylex.props(styles.viewportFrame)}>
      <div data-testid="transcript-scroll" aria-label="Conversation history" aria-busy={false} tabIndex={0} {...stylex.props(styles.lane)}>
      <div data-embrace-entry-id="__fractal_history__" {...stylex.props(readingColumnStyles.column, styles.content)}>
        <div data-testid="history-boundary" data-history="pending" {...stylex.props(styles.historyBoundary)}><span {...stylex.props(styles.historyNote)}>Earlier messages not loaded</span></div>
        <div data-testid="transcript-placeholder" aria-label="Loading conversation" {...stylex.props(styles.placeholder)}>
          <p role="status">Loading conversation…</p>
          <SyncLine status={waiting} label="conversation" now={0} observedAt={0} />
          <div aria-hidden="true" {...stylex.props(styles.turn)}>
            <div {...stylex.props(styles.skeletonPrompt)} />
            <div {...stylex.props(styles.skeletonWork)} />
            <div {...stylex.props(styles.skeletonAnswer)} />
          </div>
        </div>
      </div></div></div>
    </ErrorOverlayHost>
  </div>
</div>

const styles = stylex.create({
  frame: { minWidth: 0, minHeight: g.threadViewportMin, height: '100%', display: 'flex', flexDirection: 'column', overflow: 'hidden', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  viewportFrame: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minHeight: 0, minWidth: 0 },
  header: { height: g.band, minHeight: g.band, position: 'relative', display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.lg, flexShrink: 0, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  title: { flex: '1 1 0', minWidth: 0, fontWeight: t.weightMedium, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  lane: { flex: '1 1 0', minHeight: 0, minWidth: 0, overflowY: 'auto', overflowX: 'hidden', overflowAnchor: 'none' },
  content: { paddingBlock: s.lg },
  historyBoundary: { display: 'flex', alignItems: 'center', justifyContent: 'center', gap: s.md, minHeight: g.controlMd, flexShrink: 0 },
  historyNote: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  placeholder: { display: 'flex', flexDirection: 'column', gap: s.lg },
  turn: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0 },
  skeletonPrompt: { width: '100%', height: `calc(${t.bodyLeading} + ${s.md})`, borderLeftWidth: g.focusRing, borderLeftStyle: 'solid', borderLeftColor: accent.primary, backgroundColor: surface.rowActive },
  skeletonWork: { height: g.toolRow, width: '30%', borderRadius: r.sm, backgroundColor: surface.rowActive },
  skeletonAnswer: { height: g.resourceCard, width: '80%', borderRadius: r.sm, backgroundColor: surface.rowHover },
})
