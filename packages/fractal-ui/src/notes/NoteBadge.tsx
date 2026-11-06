import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Focusable } from 'react-aria-components'
import { VisuallyHidden } from 'react-aria'
import type { SubjectNote } from '@fractal/client/schema'
import { Tooltip } from '../assistant-ui/composition/Controls'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, textVars as text, typeVars as t } from '../assistant-ui/composition-tokens.stylex'
import { newestNote } from './order'

export interface NoteBadgeProps {
  /** Current notes on one target; order does not matter. Renders nothing when empty. */
  readonly notes: readonly SubjectNote[]
  /**
   * `true` (default): its own focus stop, naming count and newest text, with the full text in a tooltip.
   * `false`: presentational content for a control that is already interactive (a disclosure summary):
   * no focus stop, tooltip or role, and the count and newest text become that control's label.
   */
  readonly interactive?: boolean
}

export const noteCountLabel = (count: number): string => count === 1 ? '1 note' : `${count} notes`

/**
 * Compact note signal for a target: count and a one-line preview of the newest note. Every current
 * note is agent context, so the badge carries no audience signal. The full newest text is the
 * tooltip and part of the accessible name.
 */
export function NoteBadge({ notes, interactive = true }: NoteBadgeProps) {
  const newest = React.useMemo(() => newestNote(notes), [notes])
  if (newest === undefined) return null
  const description = `${noteCountLabel(notes.length)}. Newest: ${newest.text}`
  const content = <>
    <span {...stylex.props(styles.count)}>{notes.length}</span>
    <span aria-hidden="true" {...stylex.props(styles.divider)} />
    <span data-testid="note-badge-preview" {...stylex.props(styles.preview)}>{newest.text}</span>
  </>
  if (!interactive) return <span data-testid="note-badge" data-note-count={notes.length} data-note-badge-static="" {...stylex.props(styles.badge, styles.static)}>
    <span aria-hidden="true" {...stylex.props(styles.visual)}>{content}</span>
    <VisuallyHidden>{description}</VisuallyHidden>
  </span>
  return <Tooltip label={description} placement="top">
    <Focusable>
      <span
        tabIndex={0}
        role="img"
        aria-label={description}
        data-testid="note-badge"
        data-note-count={notes.length}
        {...stylex.props(styles.badge, styles.focusable)}
      >
        {content}
      </span>
    </Focusable>
  </Tooltip>
}

const styles = stylex.create({
  badge: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, maxWidth: '100%', height: g.controlSm, boxSizing: 'border-box',
    paddingInline: s.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.control,
    color: text.fgMuted, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'default',
  },
  focusable: {
    outlineStyle: 'none',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  // The host control owns the pointer affordance and the focus ring.
  static: { cursor: 'inherit' },
  visual: { display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, alignSelf: 'stretch' },
  count: { fontVariantNumeric: 'tabular-nums', fontWeight: t.weightSemibold, color: text.fg },
  divider: { alignSelf: 'stretch', width: g.hairline, marginBlock: s.xs, flexShrink: 0, backgroundColor: border.border },
  preview: { minWidth: 0, maxWidth: g.crumbMax, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: text.fgSoft },
})
