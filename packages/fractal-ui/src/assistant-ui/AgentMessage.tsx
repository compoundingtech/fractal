import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel } from 'react-aria-components'
import { EmbraceMarkdownPreview } from './EmbraceToolPreview'
import { surfaceVars, textVars, borderVars, accentVars, statusVars, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from './composition-tokens.stylex'

export type AgentMessageVia =
  | { readonly _tag: 'Irc' }
  | { readonly _tag: 'SmallTalk' }
  | { readonly _tag: 'FromParent' }
  | { readonly _tag: 'Teammate' }
  | { readonly _tag: 'InterAgent' }

export type AgentMessageBody =
  | { readonly _tag: 'Full'; readonly markdown: string }
  | { readonly _tag: 'Preview'; readonly markdown: string }
  | { readonly _tag: 'Loading'; readonly markdown: string }
  | { readonly _tag: 'Failed'; readonly markdown: string; readonly reason: string }

export interface AgentMessageProps {
  readonly id?: string
  readonly from: string
  readonly to?: string
  readonly via: AgentMessageVia
  readonly body: AgentMessageBody
  readonly subject?: string
  readonly replyTo?: string
  readonly at?: string
  readonly arrivedWhileWorking?: boolean
  readonly delivery?: { readonly _tag: 'Direct' } | { readonly _tag: 'BroadcastCopy'; readonly recipients: number }
  readonly onLoadFull?: () => void
  readonly onCopyId?: (id: string) => void
  readonly onOpenReply?: (id: string) => void
  readonly fullMessageUnavailableReason?: string
}

/** Dotted harness identifiers preserve the parent/child relation without inventing labels. */
export const AgentIdHierarchy = ({ id }: { readonly id: string }) => <span data-agent-id={id}><span aria-hidden="true">{id.split('.').map((part, index) => <React.Fragment key={index}>{index > 0 ? ' › ' : ''}{part}</React.Fragment>)}</span><span {...stylex.props(styles.visuallyHidden)}>{id}</span></span>

/** Large untrusted bodies bypass Markdown parsing and mount only one bounded window. */
const MessageBody = ({ text }: { readonly text: string }) => {
  const [page, setPage] = React.useState(0)
  const reasonId = React.useId()
  if (text.length <= 16000) return <div data-message-renderer="markdown"><EmbraceMarkdownPreview markdown={text} /></div>
  const pages = Math.ceil(text.length / 16000)
  const current = Math.min(page, pages - 1)
  const atStart = current === 0
  const atEnd = current === pages - 1
  return <div data-message-renderer="plain">
    <p {...stylex.props(styles.meta)}>Large message · plain text · page {current + 1} of {pages}</p>
    <pre data-message-window={current} {...stylex.props(styles.plainText)}>{text.slice(current * 16000, (current + 1) * 16000)}</pre>
    <div {...stylex.props(styles.header)}>
      <Button {...stylex.props(styles.button)} aria-disabled={atStart || undefined} aria-describedby={reasonId} onPress={atStart ? undefined : () => setPage(current - 1)}>Previous message page</Button>
      <Button {...stylex.props(styles.button)} aria-disabled={atEnd || undefined} aria-describedby={reasonId} onPress={atEnd ? undefined : () => setPage(current + 1)}>Next message page</Button>
      <span id={reasonId} {...stylex.props(styles.meta)}>{atStart ? 'Previous unavailable: first page.' : ''}{atStart && atEnd ? ' ' : ''}{atEnd ? 'Next unavailable: last page.' : ''}</span>
    </div>
  </div>
}

/** A promoted incoming card. Transport and full-body loading belong to its owner. */
export const AgentMessage = ({ id, from, to, via, body, subject, replyTo, at, arrivedWhileWorking, delivery, onLoadFull, onCopyId, onOpenReply, fullMessageUnavailableReason }: AgentMessageProps) => {
  const [loadRequested, setLoadRequested] = React.useState(false)
  const reasonId = React.useId()
  const loaded = loadRequested && body._tag === 'Full'
  const preview = body.markdown.slice(0, 4000)
  return <article {...stylex.props(styles.card)} aria-label={`Message from ${from}`} data-harness-weight="card" data-agent-message-direction="incoming">
    <header {...stylex.props(styles.header)}>
      <span aria-hidden="true" {...stylex.props(styles.avatar)}>{from.slice(0, 1).toUpperCase()}</span>
      <strong {...stylex.props(styles.sender)}><AgentIdHierarchy id={from} /></strong>
      {to !== undefined ? <><span aria-hidden="true">→</span><AgentIdHierarchy id={to} /></> : null}
      <span {...stylex.props(styles.chip)}>{({ Irc: 'IRC', SmallTalk: 'Small Talk', FromParent: 'from parent', Teammate: 'teammate', InterAgent: 'inter-agent' } as const)[via._tag]}</span>
      {at !== undefined ? <time {...stylex.props(styles.meta)}>{at}</time> : null}
      {arrivedWhileWorking ? <span {...stylex.props(styles.meta)}>arrived while working</span> : null}
      {delivery?._tag === 'BroadcastCopy' ? <span {...stylex.props(styles.chip)}>broadcast · {delivery.recipients} seats</span> : null}
    </header>
    {subject !== undefined ? <h3 {...stylex.props(styles.subject)}>{subject}</h3> : null}
    {replyTo !== undefined ? <div {...stylex.props(styles.meta)}>Reply to {onOpenReply !== undefined ? <Button {...stylex.props(styles.button)} onPress={() => onOpenReply(replyTo)}><AgentIdHierarchy id={replyTo} /></Button> : <AgentIdHierarchy id={replyTo} />}</div> : null}
    {body._tag === 'Full' && body.markdown.length > 4000 ? <Disclosure defaultExpanded={loaded}>
      {({ isExpanded }) => <>
        <Button slot="trigger" {...stylex.props(styles.button)}>{isExpanded ? 'Hide full message' : 'Show full message'}</Button>
        {!isExpanded ? <div {...stylex.props(styles.messageBody)} tabIndex={0} role="region" aria-label="Message excerpt"><MessageBody text={preview} /></div> : null}
        <DisclosurePanel>{isExpanded ? <div {...stylex.props(styles.messageBody)} tabIndex={0} role="region" aria-label="Full message"><MessageBody text={body.markdown} /></div> : null}</DisclosurePanel>
      </>}
    </Disclosure> : <div {...stylex.props(styles.messageBody)} tabIndex={0} role="region" aria-label={body._tag === 'Full' ? 'Message body' : 'Message excerpt'}><MessageBody text={body._tag === 'Full' ? body.markdown : preview} /></div>}
    {body._tag !== 'Full' || loaded ? <div {...stylex.props(styles.header)}>
      {!loaded ? <span {...stylex.props(styles.meta)}>Message preview</span> : null}
      <Button {...stylex.props(styles.button)} isPending={body._tag === 'Loading'} isDisabled={!loaded && onLoadFull === undefined} aria-disabled={loaded || undefined} aria-describedby={reasonId} onPress={loaded ? undefined : () => { setLoadRequested(true); onLoadFull?.() }}>{loaded ? 'Full message loaded' : body._tag === 'Loading' ? 'Loading full message…' : body._tag === 'Failed' ? 'Retry full message' : 'Show full message'}</Button>
      <span id={reasonId} tabIndex={onLoadFull === undefined ? 0 : undefined} {...stylex.props(styles.meta)}>{loaded ? 'The full message has been received.' : onLoadFull === undefined ? fullMessageUnavailableReason ?? 'This surface cannot load the full message.' : body._tag === 'Loading' ? 'Fetching the full message; the preview is kept.' : ''}</span>
      {body._tag === 'Failed' ? <span role="alert" {...stylex.props(styles.danger)}>{body.reason} · the preview is kept</span> : null}
    </div> : null}
    {id !== undefined ? <footer {...stylex.props(styles.header, styles.meta)}><span>message/<AgentIdHierarchy id={id} /></span>{onCopyId !== undefined ? <Button {...stylex.props(styles.button)} onPress={() => onCopyId(id)}>Copy id</Button> : null}</footer> : null}
  </article>
}

export type AgentMessageReceipt =
  | { readonly _tag: 'Injected' }
  | { readonly _tag: 'Queued' }
  | { readonly _tag: 'Revived' }
  | { readonly _tag: 'Failed'; readonly reason: string }

export interface OutgoingMessageRowProps {
  readonly message: string
  readonly recipients: readonly { readonly id: string; readonly receipt?: AgentMessageReceipt }[]
  readonly delivery: { readonly _tag: 'Single' } | { readonly _tag: 'Broadcast'; readonly recipientCount?: number }
  readonly at?: string
  readonly defaultExpanded?: boolean
}

/** One send is one work-log item, including broadcasts; receipts are per recipient. */
export const OutgoingMessageRow = ({ message, recipients, delivery, at, defaultExpanded = false }: OutgoingMessageRowProps) => {
  const firstLine = message.slice(0, 200).split('\n', 1)[0] ?? ''
  const recipientCount = delivery._tag === 'Broadcast' ? delivery.recipientCount ?? (recipients.length > 0 ? recipients.length : undefined) : undefined
  return <div {...stylex.props(styles.card)} data-harness-weight="card" data-work-kind="messages-sent">
    <Disclosure defaultExpanded={defaultExpanded}>
      {({ isExpanded }) => <>
        <div {...stylex.props(styles.line)}>
          <Button slot="trigger" {...stylex.props(styles.trigger)}>
            <span aria-hidden="true">{isExpanded ? '▾' : '▸'} →</span>
            <span>{delivery._tag === 'Broadcast' ? recipientCount !== undefined ? `${recipientCount} seats` : 'Broadcast' : recipients[0] !== undefined ? <AgentIdHierarchy id={recipients[0].id} /> : 'Message sent'}</span>
            <span {...stylex.props(styles.excerpt)} title={firstLine}>{firstLine}</span>
          </Button>
          {recipients.map(recipient => <span key={recipient.id} role={recipient.receipt?._tag === 'Failed' ? 'note' : undefined} tabIndex={recipient.receipt?._tag === 'Failed' ? 0 : undefined} aria-label={recipient.receipt?._tag === 'Failed' ? `${recipient.id} · failed · ${recipient.receipt.reason}` : undefined} {...stylex.props(styles.receipt, recipient.receipt?._tag === 'Failed' && styles.danger)} title={recipient.receipt?._tag === 'Failed' ? recipient.receipt.reason : undefined}>
            <AgentIdHierarchy id={recipient.id} />{recipient.receipt !== undefined ? ` · ${recipient.receipt._tag.toLowerCase()}` : ''}
          </span>)}
          {at !== undefined ? <time {...stylex.props(styles.meta)}>{at}</time> : null}
        </div>
        <DisclosurePanel>{isExpanded ? <div {...stylex.props(styles.body)} tabIndex={0} role="region" aria-label="Outgoing message body"><MessageBody text={message} />{recipients.flatMap(recipient => recipient.receipt?._tag === 'Failed' ? [<p key={recipient.id} {...stylex.props(styles.danger)}>{recipient.id}: {recipient.receipt.reason}</p>] : [])}</div> : null}</DisclosurePanel>
      </>}
    </Disclosure>
  </div>
}

const styles = stylex.create({
  card: { padding: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.md, backgroundColor: surfaceVars.raised, color: textVars.fg, minWidth: 0, overflowWrap: 'anywhere', fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, minWidth: 0 },
  sender: { fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  line: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm, minWidth: 0 },
  messageBody: { maxHeight: g.workExpandedMax, overflowY: 'auto', minWidth: 0, overflowWrap: 'anywhere', outlineColor: accentVars.primary, outlineOffset: g.focusOffset },
  avatar: { width: g.controlSm, height: g.controlSm, display: 'grid', alignItems: 'center', justifyItems: 'center', backgroundColor: surfaceVars.rowActive, borderRadius: r.sm, fontSize: t.metaSize, lineHeight: t.metaLeading },
  chip: { paddingInline: s.sm, paddingBlock: s.xs2, borderRadius: r.sm, backgroundColor: surfaceVars.rowActive, color: textVars.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading },
  meta: { color: textVars.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  subject: { marginBlock: s.md, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  button: { padding: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, backgroundColor: surfaceVars.rowActive, color: textVars.fg, cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  trigger: { flex: '1 1 100%', minWidth: 0, display: 'flex', alignItems: 'center', gap: s.sm, borderWidth: 0, padding: 0, backgroundColor: 'transparent', color: textVars.fg, textAlign: 'left', cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  excerpt: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: textVars.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  receipt: { maxWidth: '100%', overflowWrap: 'anywhere', fontSize: t.denseSize, lineHeight: t.metaLeading, color: textVars.fgMuted, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, outlineColor: accentVars.primary, outlineOffset: g.focusOffset },
  body: { padding: s.md, overflowWrap: 'anywhere', maxHeight: g.workExpandedMax, overflowY: 'auto' },
  danger: { color: statusVars.dangerFg },
  plainText: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading },
  visuallyHidden: { position: 'absolute', width: g.hairline, height: g.hairline, padding: 0, margin: 0, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
})
