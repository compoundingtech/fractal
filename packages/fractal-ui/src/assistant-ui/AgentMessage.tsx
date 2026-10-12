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


export type MessageVariant = { readonly _tag: 'Incoming' } | { readonly _tag: 'Outgoing' } | { readonly _tag: 'Human' }
export interface AgentMessageProps {
  readonly variant?: MessageVariant
  readonly id?: string
  readonly from: string
  readonly to?: string
  readonly via?: AgentMessageVia
  readonly body: AgentMessageBody
  readonly subject?: string
  readonly replyTo?: string
  readonly at?: string
  readonly arrivedWhileWorking?: boolean
  readonly delivery?: { readonly _tag: 'Direct' } | { readonly _tag: 'BroadcastCopy'; readonly recipients: number }
  readonly onLoadFull?: () => void
  readonly onCopyId?: (id: string) => void
  readonly onCopy?: () => void
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
    <div {...stylex.props(styles.controls)}>
      <Button {...stylex.props(styles.button)} aria-disabled={atStart || undefined} aria-describedby={reasonId} onPress={atStart ? undefined : () => setPage(current - 1)}>Previous message page</Button>
      <Button {...stylex.props(styles.button)} aria-disabled={atEnd || undefined} aria-describedby={reasonId} onPress={atEnd ? undefined : () => setPage(current + 1)}>Next message page</Button>
      <span id={reasonId} {...stylex.props(styles.meta)}>{atStart ? 'Previous unavailable: first page.' : ''}{atStart && atEnd ? ' ' : ''}{atEnd ? 'Next unavailable: last page.' : ''}</span>
    </div>
  </div>
}

/** Shared geometry for incoming cards, quiet outgoing blocks and human intent. */
const MessageFrame = ({ variant, header, subject, children, label, outgoing = false }: {
  readonly variant: MessageVariant
  readonly header: React.ReactNode
  readonly subject?: string
  readonly children?: React.ReactNode
  readonly label: string
  readonly outgoing?: boolean
}) => <article {...stylex.props(styles.message, variant._tag === 'Incoming' ? styles.card : variant._tag === 'Outgoing' ? styles.outgoing : styles.human)} aria-label={label} data-harness-weight={variant._tag === 'Incoming' ? 'card' : 'row'} data-agent-message-direction={variant._tag === 'Incoming' ? 'incoming' : variant._tag === 'Outgoing' ? 'outgoing' : 'human'} data-message-variant={variant._tag.toLowerCase()} data-work-kind={outgoing ? 'messages-sent' : undefined}>
  <header data-message-header {...stylex.props(styles.header)}>{header}</header>
  {subject !== undefined ? <h3 data-message-subject {...stylex.props(styles.subject)}>{subject}</h3> : null}
  {children !== undefined && children !== null ? <div data-message-content {...stylex.props(styles.content)}>{children}</div> : null}
</article>

/** Transport and full-body loading belong to the host, not the message presentation. */
export const AgentMessage = ({ variant = { _tag: 'Incoming' }, id, from, to, via, body, subject, replyTo, at, arrivedWhileWorking, delivery, onLoadFull, onCopyId, onCopy, onOpenReply, fullMessageUnavailableReason }: AgentMessageProps) => {
  const [loadRequested, setLoadRequested] = React.useState(false)
  const reasonId = React.useId()
  const loaded = loadRequested && body._tag === 'Full'
  const preview = body.markdown.slice(0, 4000)
  const counterpart = variant._tag === 'Outgoing' ? to : from
  return <MessageFrame variant={variant} label={variant._tag === 'Outgoing' ? `Message to ${to ?? ''}` : `Message from ${from}`} subject={subject} header={<>
    <span aria-hidden="true" {...stylex.props(styles.avatar)}>{from.slice(0, 1).toUpperCase()}</span>
    <span aria-hidden="true" {...stylex.props(styles.direction)}>{variant._tag === 'Outgoing' ? '→' : '←'}</span>
    <span {...stylex.props(styles.counterpart)}><span {...stylex.props(styles.meta)}>{variant._tag === 'Outgoing' ? 'To' : 'From'} </span><strong {...stylex.props(styles.sender)}>{counterpart !== undefined ? <AgentIdHierarchy id={counterpart} /> : null}</strong>{variant._tag === 'Incoming' && to !== undefined ? <span {...stylex.props(styles.meta)}> → <AgentIdHierarchy id={to} /></span> : null}{via !== undefined ? <span {...stylex.props(styles.meta)}> · <span>{({ Irc: 'IRC', SmallTalk: 'Small Talk', FromParent: 'from parent', Teammate: 'teammate', InterAgent: 'inter-agent' } as const)[via._tag]}</span></span> : null}</span>
    {replyTo !== undefined ? onOpenReply !== undefined ? <Button {...stylex.props(styles.headerAction)} title={`Reply to ${replyTo}`} onPress={() => onOpenReply(replyTo)}>Reply to <AgentIdHierarchy id={replyTo} /></Button> : <span {...stylex.props(styles.meta)} title={`Reply to ${replyTo}`}>↩</span> : null}
    {delivery?._tag === 'BroadcastCopy' ? <span {...stylex.props(styles.chip)}>broadcast · {delivery.recipients} seats</span> : arrivedWhileWorking ? <span {...stylex.props(styles.chip)}>arrived while working</span> : null}
    {onCopy !== undefined ? <Button aria-label="Copy message" {...stylex.props(styles.headerAction, styles.copy)} onPress={onCopy}><span aria-hidden="true">⧉</span></Button> : id !== undefined && onCopyId !== undefined ? <Button aria-label="Copy id" {...stylex.props(styles.headerAction, styles.copy)} title={`message/${id}`} onPress={() => onCopyId(id)}><span aria-hidden="true">⧉</span></Button> : null}
    {id !== undefined && (onCopyId === undefined || onCopy !== undefined) ? <span {...stylex.props(styles.visuallyHidden)}>message/<AgentIdHierarchy id={id} /></span> : null}
    {at !== undefined ? <time {...stylex.props(styles.meta, styles.time)}>{at}</time> : null}
  </>}>
    {body._tag === 'Full' && body.markdown.length > 4000 ? <Disclosure defaultExpanded={loaded}>
      {({ isExpanded }) => <>
        <Button slot="trigger" {...stylex.props(styles.button)}>{isExpanded ? 'Hide full message' : 'Show full message'}</Button>
        {!isExpanded ? <div {...stylex.props(styles.messageBody)} tabIndex={0} role="region" aria-label="Message excerpt"><MessageBody text={preview} /></div> : null}
        <DisclosurePanel>{isExpanded ? <div {...stylex.props(styles.messageBody)} tabIndex={0} role="region" aria-label="Full message"><MessageBody text={body.markdown} /></div> : null}</DisclosurePanel>
      </>}
    </Disclosure> : <div {...stylex.props(styles.messageBody)} tabIndex={body.markdown.length > 4000 ? 0 : undefined} role="region" aria-label={body._tag === 'Full' ? 'Message body' : 'Message excerpt'}><MessageBody text={body._tag === 'Full' ? body.markdown : preview} /></div>}
    {body._tag !== 'Full' || loaded ? <div {...stylex.props(styles.controls)}>
      {!loaded ? <span {...stylex.props(styles.meta)}>Message preview</span> : null}
      <Button {...stylex.props(styles.button)} isPending={body._tag === 'Loading'} isDisabled={!loaded && onLoadFull === undefined} aria-disabled={loaded || undefined} aria-describedby={reasonId} onPress={loaded ? undefined : () => { setLoadRequested(true); onLoadFull?.() }}>{loaded ? 'Full message loaded' : body._tag === 'Loading' ? 'Loading full message…' : body._tag === 'Failed' ? 'Retry full message' : 'Show full message'}</Button>
      <span id={reasonId} tabIndex={onLoadFull === undefined ? 0 : undefined} {...stylex.props(styles.meta)}>{loaded ? 'The full message has been received.' : onLoadFull === undefined ? fullMessageUnavailableReason ?? 'This surface cannot load the full message.' : body._tag === 'Loading' ? 'Fetching the full message; the preview is kept.' : ''}</span>
      {body._tag === 'Failed' ? <span role="alert" {...stylex.props(styles.danger)}>{body.reason} · the preview is kept</span> : null}
    </div> : null}
  </MessageFrame>
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
  readonly subject?: string
  readonly onCopy?: () => void
  readonly defaultExpanded?: boolean
}

/** One send is one quiet work-log block, including broadcasts; receipts stay per recipient. */
export const OutgoingMessageRow = ({ message, recipients, delivery, at, subject, onCopy, defaultExpanded = false }: OutgoingMessageRowProps) => {
  const firstLine = message.slice(0, 200).split('\n', 1)[0] ?? ''
  const recipientCount = delivery._tag === 'Broadcast' ? delivery.recipientCount ?? (recipients.length > 0 ? recipients.length : undefined) : undefined
  const target = delivery._tag === 'Broadcast' ? recipientCount !== undefined ? `${recipientCount} seats` : 'Broadcast' : recipients[0]?.id
  return <Disclosure defaultExpanded={defaultExpanded}>
    {({ isExpanded }) => <MessageFrame variant={{ _tag: 'Outgoing' }} outgoing label={`Message to ${target ?? ''}`} subject={isExpanded ? subject : undefined} header={<>
      <Button slot="trigger" aria-label={`To ${target ?? ''} · ${firstLine}`} {...stylex.props(styles.trigger)}>
        <span aria-hidden="true" {...stylex.props(styles.direction)}>{isExpanded ? '▾' : '▸'}</span>
        <span {...stylex.props(styles.meta)}>To</span>
        {target !== undefined ? <strong {...stylex.props(styles.sender)}>{delivery._tag === 'Broadcast' ? target : <AgentIdHierarchy id={target} />}</strong> : null}
        {!isExpanded ? <span {...stylex.props(styles.excerpt)} title={firstLine}>{firstLine}</span> : null}
      </Button>
      {recipients.map(recipient => <span key={recipient.id} role={recipient.receipt?._tag === 'Failed' ? 'note' : undefined} tabIndex={recipient.receipt?._tag === 'Failed' ? 0 : undefined} aria-label={recipient.receipt?._tag === 'Failed' ? `${recipient.id} · failed · ${recipient.receipt.reason}` : undefined} {...stylex.props(styles.receipt, recipient.receipt?._tag === 'Failed' && styles.danger)} title={recipient.receipt?._tag === 'Failed' ? recipient.receipt.reason : recipient.id}>
        <AgentIdHierarchy id={recipient.id} />{recipient.receipt !== undefined ? ` · ${recipient.receipt._tag.toLowerCase()}` : ''}
      </span>)}
      {onCopy !== undefined ? <Button aria-label="Copy message" {...stylex.props(styles.headerAction, styles.copy)} onPress={onCopy}><span aria-hidden="true">⧉</span></Button> : null}
      {at !== undefined ? <time {...stylex.props(styles.meta, styles.time)}>{at}</time> : null}
    </>}>
      {isExpanded ? <DisclosurePanel><div {...stylex.props(styles.messageBody)} tabIndex={message.length > 4000 ? 0 : undefined} role="region" aria-label="Outgoing message body"><MessageBody text={message} />{recipients.flatMap(recipient => recipient.receipt?._tag === 'Failed' ? [<p key={recipient.id} {...stylex.props(styles.danger)}>{recipient.id}: {recipient.receipt.reason}</p>] : [])}</div></DisclosurePanel> : null}
    </MessageFrame>}
  </Disclosure>
}

const styles = stylex.create({
  message: { color: textVars.fg, minWidth: 0, boxSizing: 'border-box', overflowWrap: 'anywhere', fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  card: { paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.border, borderRadius: r.md, backgroundColor: surfaceVars.raised },
  outgoing: { paddingBlock: s.xs2, paddingLeft: s.lg, paddingRight: 0, borderLeftWidth: g.focusRing, borderLeftStyle: 'solid', borderLeftColor: borderVars.railStrong, backgroundColor: surfaceVars.transparent },
  human: { paddingBlock: s.xs, paddingInline: s.lg, borderLeftWidth: g.focusRing, borderLeftStyle: 'solid', borderLeftColor: accentVars.primary, backgroundColor: surfaceVars.transparent },
  header: { display: 'flex', alignItems: 'center', gap: s.sm, height: g.icon, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  controls: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, minWidth: 0 },
  counterpart: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  direction: { flex: '0 0 auto', width: g.icon, textAlign: 'center', color: textVars.fgMuted },
  sender: { color: textVars.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  time: { marginLeft: 'auto', flex: '0 0 auto', whiteSpace: 'nowrap', textAlign: 'right' },
  content: { marginTop: s.xs, minWidth: 0 },
  messageBody: { maxHeight: g.workExpandedMax, overflowY: 'auto', minWidth: 0, overflowWrap: 'anywhere', outlineColor: accentVars.primary, outlineOffset: g.focusOffset },
  avatar: { flex: '0 0 auto', width: g.avatarSm, height: g.avatarSm, display: 'grid', alignItems: 'center', justifyItems: 'center', backgroundColor: surfaceVars.rowActive, borderRadius: r.sm, fontSize: t.metaSize, lineHeight: t.metaLeading },
  chip: { paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, color: textVars.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', flex: '0 1 auto' },
  meta: { color: textVars.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  subject: { margin: 0, marginTop: s.xs, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  button: { padding: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, backgroundColor: surfaceVars.rowActive, color: textVars.fg, cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  headerAction: { flex: '0 1 auto', minWidth: 0, height: g.icon, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', borderWidth: 0, padding: 0, backgroundColor: surfaceVars.transparent, color: textVars.fgSoft, cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  copy: { opacity: { default: 0, ':hover': 1, ':focus-visible': 1 }, flex: '0 0 auto', width: g.icon },
  trigger: { flex: '1 1 auto', minWidth: 0, display: 'flex', alignItems: 'center', gap: s.sm, borderWidth: 0, padding: 0, backgroundColor: surfaceVars.transparent, color: textVars.fg, textAlign: 'left', cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  excerpt: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: textVars.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  receipt: { flex: '0 1 auto', minWidth: 0, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.denseSize, lineHeight: t.metaLeading, color: textVars.fgMuted, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, outlineColor: accentVars.primary, outlineOffset: g.focusOffset },
  danger: { color: statusVars.dangerFg },
  plainText: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading },
  visuallyHidden: { position: 'absolute', width: g.hairline, height: g.hairline, padding: 0, margin: 0, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
})
