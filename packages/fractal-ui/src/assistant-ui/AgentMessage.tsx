import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel } from 'react-aria-components'
import { EmbraceMarkdownPreview } from './EmbraceToolPreview'
import { tokens, accentTokens, density } from './embrace-tokens.stylex'

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
      <strong><AgentIdHierarchy id={from} /></strong>
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
        {!isExpanded ? <div {...stylex.props(styles.messageBody)}><MessageBody text={preview} /></div> : null}
        <DisclosurePanel>{isExpanded ? <div {...stylex.props(styles.messageBody)} tabIndex={0} role="region" aria-label="Full message"><MessageBody text={body.markdown} /></div> : null}</DisclosurePanel>
      </>}
    </Disclosure> : <div {...stylex.props(styles.messageBody)}><MessageBody text={body._tag === 'Full' ? body.markdown : preview} /></div>}
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
  const firstLine = message.split('\n', 1)[0] ?? ''
  return <div {...stylex.props(styles.outgoing)} data-harness-weight="row" data-work-kind="messages-sent">
    <Disclosure defaultExpanded={defaultExpanded}>
      {({ isExpanded }) => <>
        <div {...stylex.props(styles.line)}>
          <Button slot="trigger" {...stylex.props(styles.trigger)}>
            <span aria-hidden="true">{isExpanded ? '▾' : '▸'} →</span>
            <span>{delivery._tag === 'Broadcast' ? delivery.recipientCount !== undefined ? `${delivery.recipientCount} seats` : 'Broadcast' : recipients[0] !== undefined ? <AgentIdHierarchy id={recipients[0].id} /> : 'Message sent'}</span>
            <span {...stylex.props(styles.excerpt)} title={firstLine}>{firstLine}</span>
          </Button>
          {recipients.map(recipient => <span key={recipient.id} role={recipient.receipt?._tag === 'Failed' ? 'note' : undefined} tabIndex={recipient.receipt?._tag === 'Failed' ? 0 : undefined} aria-label={recipient.receipt?._tag === 'Failed' ? `${recipient.id} · failed · ${recipient.receipt.reason}` : undefined} {...stylex.props(styles.receipt, recipient.receipt?._tag === 'Failed' && styles.danger)} title={recipient.receipt?._tag === 'Failed' ? recipient.receipt.reason : undefined}>
            <AgentIdHierarchy id={recipient.id} />{recipient.receipt !== undefined ? ` · ${recipient.receipt._tag.toLowerCase()}` : ''}
          </span>)}
          {at !== undefined ? <time {...stylex.props(styles.meta)}>{at}</time> : null}
        </div>
        <DisclosurePanel>{isExpanded ? <div {...stylex.props(styles.body)}><MessageBody text={message} />{recipients.flatMap(recipient => recipient.receipt?._tag === 'Failed' ? [<p key={recipient.id} {...stylex.props(styles.danger)}>{recipient.id}: {recipient.receipt.reason}</p>] : [])}</div> : null}</DisclosurePanel>
      </>}
    </Disclosure>
  </div>
}

const styles = stylex.create({
  card: { padding: 12, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 8, backgroundColor: tokens.panel, color: tokens.ink, minWidth: 0, overflowWrap: 'anywhere' },
  header: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8, minWidth: 0 },
  line: { display: 'flex', flexWrap: 'nowrap', alignItems: 'center', gap: 6, minWidth: 0, overflowX: 'auto', whiteSpace: 'nowrap' },
  messageBody: { maxHeight: 340, overflowY: 'auto', minWidth: 0, overflowWrap: 'anywhere', outlineColor: accentTokens.accent, outlineOffset: 2 },
  avatar: { width: 22, height: 22, display: 'grid', alignItems: 'center', justifyItems: 'center', backgroundColor: tokens.recess, borderRadius: 5, fontSize: 12 },
  chip: { paddingInline: 6, paddingBlock: 2, borderRadius: 4, backgroundColor: tokens.recess, color: tokens.muted, fontSize: 11 },
  meta: { color: tokens.muted, fontSize: 11 },
  subject: { marginBlock: 8, fontSize: 13 },
  button: { padding: 5, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 4, backgroundColor: tokens.recess, color: tokens.ink, cursor: 'pointer', outlineColor: accentTokens.accent, outlineOffset: 2 },
  outgoing: { paddingBlock: density.toolY, color: tokens.ink, minWidth: 0, fontSize: density.senderSize },
  trigger: { flex: '1 0 140px', minWidth: 0, display: 'flex', alignItems: 'center', gap: 6, borderWidth: 0, padding: 0, backgroundColor: 'transparent', color: tokens.ink, textAlign: 'left', cursor: 'pointer', outlineColor: accentTokens.accent, outlineOffset: 3 },
  excerpt: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: tokens.muted },
  receipt: { flexShrink: 0, fontSize: 10, color: tokens.muted, paddingInline: 4, borderWidth: 1, borderStyle: 'solid', borderColor: tokens.line, borderRadius: 4, outlineColor: accentTokens.accent, outlineOffset: 2 },
  body: { padding: 8, overflowWrap: 'anywhere', maxHeight: 340, overflowY: 'auto' },
  danger: { color: tokens.danger },
  plainText: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: 'monospace', fontSize: 12, lineHeight: 1.5 },
  visuallyHidden: { position: 'absolute', width: 1, height: 1, padding: 0, margin: -1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
})
