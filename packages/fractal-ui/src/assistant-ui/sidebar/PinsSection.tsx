// Server-ordered Pins above the roster. The host passes only current memberships; it never invents retired placeholders.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, IconButton } from '../composition/Controls'
import { SystemNotice } from '../composition/SystemState'
import { spaceVars as s, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

export interface PinsSectionProps {
  readonly title?: string
  /** Rendered in the given order. */
  readonly items: readonly { readonly id: string; readonly label: string }[]
  /** False keeps Open available and disables Unpin. */
  readonly editable: boolean
  readonly onOpen: (id: string) => void
  readonly onUnpin: (id: string) => void
  /** One plain sentence, e.g. a failed edit; never a raw error code. */
  readonly detail?: string
  readonly onRetry?: () => void
}

/** Nothing renders without Pins. */
export function PinsSection({ items, editable, onOpen, onUnpin, detail, onRetry, title = 'Pinned' }: PinsSectionProps) {
  const headingId = React.useId()
  if (items.length === 0) return null
  return <section aria-labelledby={headingId} data-pins-section="" {...stylex.props(styles.section)}>
    <h2 id={headingId} {...stylex.props(styles.heading)}>{title}</h2>
    <ul {...stylex.props(styles.list)}>{items.map(item => <li key={item.id} data-pin={item.id} {...stylex.props(styles.item)}>
      <Button onPress={() => onOpen(item.id)}><span {...stylex.props(styles.label)}>{item.label}</span></Button>
      <IconButton icon="x" label={`Unpin ${item.label}`} isDisabled={!editable} onPress={() => onUnpin(item.id)} />
    </li>)}</ul>
    {detail !== undefined && <SystemNotice kind={onRetry === undefined ? 'stale' : 'error'} title={detail} scope="rail" onRetry={onRetry} />}
  </section>
}

const styles = stylex.create({
  section: { display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0, minHeight: 0, maxHeight: '40%', flexShrink: 0, boxSizing: 'border-box', overflow: 'hidden', paddingBlockEnd: s.sm, fontFamily: t.fontSans },
  heading: { margin: s.zero, paddingInline: s.sm, flexShrink: 0, color: ink.fgSoft, fontSize: t.denseSize, fontWeight: t.weightMedium },
  list: { display: 'flex', flexDirection: 'column', flex: '1 1 auto', minHeight: 0, overflowY: 'auto', overflowX: 'hidden', overscrollBehaviorY: 'contain', margin: s.zero, padding: s.zero, listStyleType: 'none' },
  item: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) max-content', alignItems: 'center', minWidth: 0 },
  label: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', textAlign: 'start', color: ink.fg },
})
