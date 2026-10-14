import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Focusable } from 'react-aria-components'
import type { SubjectNote } from '@smalltalk/st3-client/schema'
import { Tooltip } from '../assistant-ui/composition/Controls'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { AgentMarker } from './AgentMarker'
import { audiencePresentation } from './audience'
import { newestNote } from './order'

export interface NoteBadgeProps {
  /** Current notes on one target; order does not matter. Renders nothing when empty. */
  readonly notes: readonly SubjectNote[]
}

export const noteCountLabel = (count: number): string => count === 1 ? '1 note' : `${count} notes`

/**
 * Compact note signal for a target: count plus a one-line preview of the newest note.
 * The full newest text is the tooltip and part of the accessible name.
 */
export function NoteBadge({ notes }: NoteBadgeProps) {
  const newest = React.useMemo(() => newestNote(notes), [notes])
  if (newest === undefined) return null
  const audience = audiencePresentation(newest.audience)
  return <Tooltip label={newest.text} placement="top">
    <Focusable>
      <span
        tabIndex={0}
        role="img"
        aria-label={`${noteCountLabel(notes.length)}. Newest, ${audience.label}: ${newest.text}`}
        data-testid="note-badge"
        data-note-count={notes.length}
        data-note-audience={audience.kind}
        {...stylex.props(styles.badge)}
      >
        <span {...stylex.props(styles.count)}>{notes.length}</span>
        {audience.kind === 'agent' && <AgentMarker />}
        <span data-testid="note-badge-preview" {...stylex.props(styles.preview)}>{newest.text}</span>
      </span>
    </Focusable>
  </Tooltip>
}

const styles = stylex.create({
  badge: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, maxWidth: '100%', height: g.controlSm, boxSizing: 'border-box',
    paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.control,
    color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'default',
    outlineStyle: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.focusOffset },
  },
  count: { fontVariantNumeric: 'tabular-nums', fontWeight: t.weightSemibold, color: text.fg },
  preview: { minWidth: 0, maxWidth: g.crumbMax, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: text.fgSoft },
})
