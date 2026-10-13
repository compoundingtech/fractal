import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Focusable, Tooltip, TooltipTrigger, VisuallyHidden } from 'react-aria-components'
import type { IncomingDelivery } from '../embrace-data/model'
import { surfaceVars as surface, textVars as ink, borderVars as border, elevationVars as elevation, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g, accentVars as accent } from '../composition-tokens.stylex'
import { Icon } from './Icons'
import { Markdown } from './Markdown'

/** Verification is a recorded mailbox verdict, never a claim inferred from the body. */
export function IncomingMessage({ delivery, text }: { readonly delivery: IncomingDelivery; readonly text: string }) {
  const detailId = React.useId()
  const verified = delivery.provenance?.verdict === 'verified'
  const detail = delivery.provenance?.device === undefined ? 'Signature verified' : `Signature verified · ${delivery.provenance.device}`
  return <article data-testid="incoming-message-card" data-message-id={delivery.messageId} {...stylex.props(styles.card)}>
    <header {...stylex.props(styles.header)}>
      <span data-testid="incoming-message-sender">From {delivery.from}</span>
      <span>Small Talk</span>
      {verified
        ? <TooltipTrigger delay={150} closeDelay={0}><Focusable><span data-testid="message-verified" tabIndex={0} role="note" aria-describedby={detailId} {...stylex.props(styles.verdict, styles.focusable)}><Icon name="check" size={12} />Signed · verified<VisuallyHidden id={detailId}>{detail}</VisuallyHidden></span></Focusable><Tooltip {...stylex.props(styles.tooltip)}>{detail}</Tooltip></TooltipTrigger>
        : <span data-testid="message-unverified" {...stylex.props(styles.verdict)}>Unverified</span>}
    </header>
    {delivery.title !== undefined && <h3 data-testid="incoming-message-subject" {...stylex.props(styles.subject)}>{delivery.title}</h3>}
    <Markdown text={text} />
  </article>
}
const styles = stylex.create({
  card: { minWidth: 0, backgroundColor: surface.raised, color: ink.fg, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.md, padding: s.lg, overflowWrap: 'anywhere' },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, color: ink.fgMuted, fontSize: t.metaSize, marginBottom: s.sm },
  verdict: { display: 'inline-flex', alignItems: 'center', gap: s.xs, marginLeft: 'auto', borderRadius: r.control },
  focusable: { ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: g.hairline } },
  tooltip: { backgroundColor: surface.raised, color: ink.fg, borderWidth: 1, borderStyle: 'solid', borderColor: border.border, borderRadius: r.control, boxShadow: elevation.popover, paddingBlock: s.xs, paddingInline: s.sm, fontSize: t.metaSize },
  subject: { marginBlock: s.sm, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: 500 },
})
