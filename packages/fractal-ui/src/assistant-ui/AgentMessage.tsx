import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel, Focusable, VisuallyHidden } from 'react-aria-components'
import { Tooltip } from './composition/Controls'
import { EmbraceMarkdownPreview } from './EmbraceToolPreview'
import { Icon, type IconName } from './composition/Icons'
import { surfaceVars, textVars, borderVars, accentVars, statusVars, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from './composition-tokens.stylex'

const emptyStyles: ReadonlyArray<stylex.StyleXStyles> = []

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


export type MessageVariant = { readonly _tag: 'Incoming' } | { readonly _tag: 'Outgoing' } | { readonly _tag: 'Human'; readonly frame?: { readonly _tag: 'Rail' } | { readonly _tag: 'Card' } }
export interface AgentMessageProps {
  readonly variant?: MessageVariant
  readonly headerMeta?: React.ReactNode
  readonly headerGlyph?: { readonly _tag: 'Hidden' } | { readonly _tag: 'Icon'; readonly name: IconName }
  readonly frameStyles?: ReadonlyArray<stylex.StyleXStyles>
  readonly bodyStyles?: ReadonlyArray<stylex.StyleXStyles>
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

/** Dotted harness identifiers preserve the parent/child relation without inventing labels.
 * The path is one non-wrapping run that containers truncate. With `tooltip`, the run is one focus stop
 * named by the full identity and the kit tooltip shows that identity on hover and keyboard focus. */
export const AgentIdHierarchy = ({ id, tooltip = false }: { readonly id: string; readonly tooltip?: boolean }) => {
  const nameId = React.useId()
  if (!id.includes('.')) return <span data-agent-id={id}>{id}</span>
  const path = <span aria-hidden="true" {...stylex.props(styles.agentPath)}>{id.split('.').map((part, index) => <React.Fragment key={index}>{index > 0 ? <Icon name="chevron-right" scale="disclosure" /> : null}{part}</React.Fragment>)}</span>
  if (!tooltip) return <span data-agent-id={id}>{path}<VisuallyHidden>{id}</VisuallyHidden></span>
  return <span data-agent-id={id}><Tooltip label={id}><Focusable><span tabIndex={0} role="img" aria-labelledby={nameId} data-agent-id-trigger {...stylex.props(styles.agentTrigger)}>{path}</span></Focusable></Tooltip><VisuallyHidden id={nameId}>{id}</VisuallyHidden></span>
}

/** Large untrusted bodies bypass Markdown parsing and mount only one bounded window. */
const MessageBody = ({ text, foreground = false }: { readonly text: string; readonly foreground?: boolean }) => {
  const [page, setPage] = React.useState(0)
  const reasonId = React.useId()
  if (text.length <= 16000) return <div data-message-renderer="markdown"><EmbraceMarkdownPreview markdown={text} tone={foreground ? 'foreground' : 'default'} /></div>
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
const MessageFrame = ({ variant, header, subject, children, label, outgoing = false, frameStyles }: {
  readonly variant: MessageVariant
  readonly header: React.ReactNode
  readonly subject?: string
  readonly children?: React.ReactNode
  readonly label: string
  readonly outgoing?: boolean
  readonly frameStyles?: ReadonlyArray<stylex.StyleXStyles>
}) => <article {...stylex.props(styles.message, variant._tag === 'Incoming' ? styles.card : variant._tag === 'Outgoing' ? styles.outgoing : variant.frame?._tag === 'Card' ? styles.card : styles.human, ...(frameStyles ?? []))} aria-label={label} data-fractal-card={variant._tag === 'Incoming' || (variant._tag === 'Human' && variant.frame?._tag === 'Card') ? '' : undefined} data-harness-weight={variant._tag === 'Incoming' || (variant._tag === 'Human' && variant.frame?._tag === 'Card') ? 'card' : 'row'} data-agent-message-direction={variant._tag === 'Incoming' ? 'incoming' : variant._tag === 'Outgoing' ? 'outgoing' : 'human'} data-message-variant={variant._tag.toLowerCase()} data-work-kind={outgoing ? 'messages-sent' : undefined}>
  <header data-message-header {...stylex.props(styles.header)}>{header}</header>
  {subject !== undefined ? <h2 data-message-subject {...stylex.props(styles.subject)}>{subject}</h2> : null}
  {children !== undefined && children !== null ? <div data-message-content {...stylex.props(styles.content, subject === undefined && styles.contentWithoutSubject)}>{children}</div> : null}
</article>

/** Transport and full-body loading belong to the host, not the message presentation. */
export const AgentMessage = ({ variant = { _tag: 'Incoming' }, headerMeta, headerGlyph, frameStyles, bodyStyles = emptyStyles, id, from, to, via, body, subject, replyTo, at, arrivedWhileWorking, delivery, onLoadFull, onCopyId, onCopy, onOpenReply, fullMessageUnavailableReason }: AgentMessageProps) => {
  const [loadRequested, setLoadRequested] = React.useState(false)
  const reasonId = React.useId()
  const loaded = loadRequested && body._tag === 'Full'
  const preview = body.markdown.slice(0, 4000)
  const counterpart = variant._tag === 'Outgoing' ? to : from
  return <MessageFrame variant={variant} frameStyles={frameStyles} label={variant._tag === 'Outgoing' ? `Message to ${to ?? ''}` : `Message from ${from}`} subject={subject} header={<>
    {headerGlyph?._tag !== 'Hidden' ? <span aria-hidden="true" data-message-glyph {...stylex.props(styles.direction)}><Icon name={headerGlyph?._tag === 'Icon' ? headerGlyph.name : variant._tag === 'Human' ? 'person' : 'message'} scale="status" /></span> : null}
    <span {...stylex.props(styles.counterpart)}><span {...stylex.props(styles.meta)}>{variant._tag === 'Outgoing' ? 'To' : 'From'} </span><strong {...stylex.props(styles.sender)}>{counterpart !== undefined ? <AgentIdHierarchy id={counterpart} tooltip /> : null}</strong>{variant._tag === 'Incoming' && to !== undefined ? <span {...stylex.props(styles.meta)}> → <AgentIdHierarchy id={to} tooltip /></span> : null}{via !== undefined ? <span {...stylex.props(styles.meta)}> · <span>{({ Irc: 'IRC', SmallTalk: 'Small Talk', FromParent: 'from parent', Teammate: 'teammate', InterAgent: 'inter-agent' } as const)[via._tag]}</span></span> : null}</span>
    {replyTo !== undefined ? onOpenReply !== undefined ? <Button {...stylex.props(styles.headerAction)} onPress={() => onOpenReply(replyTo)}>Reply to <AgentIdHierarchy id={replyTo} /></Button> : <Tooltip label={`Reply to ${replyTo}`}><Focusable><span tabIndex={0} role="img" aria-label={`Reply to ${replyTo}`} {...stylex.props(styles.meta, styles.agentTrigger)}><Icon name="arrow-left" scale="status" /></span></Focusable></Tooltip> : null}
    {delivery?._tag === 'BroadcastCopy' ? <span {...stylex.props(styles.chip)}>broadcast · {delivery.recipients} seats</span> : arrivedWhileWorking ? <span {...stylex.props(styles.chip)}>arrived while working</span> : null}
    {headerMeta !== undefined ? headerMeta : onCopy !== undefined ? <Button aria-label="Copy message" {...stylex.props(styles.headerAction, styles.copy)} onPress={onCopy}><Icon name="copy" scale="status" /></Button> : id !== undefined && onCopyId !== undefined ? <Button aria-label={`Copy id`} {...stylex.props(styles.headerAction, styles.copy)} onPress={() => onCopyId(id)}><Icon name="copy" scale="status" /></Button> : null}
    {id !== undefined && (onCopyId === undefined || onCopy !== undefined) ? <span {...stylex.props(styles.visuallyHidden)}>message/<AgentIdHierarchy id={id} /></span> : null}
    {headerMeta === undefined && at !== undefined ? <time {...stylex.props(styles.meta, styles.time)}>{at}</time> : null}
  </>}>
    {body._tag === 'Full' && body.markdown.length > 4000 ? <Disclosure defaultExpanded={loaded}>
      {({ isExpanded }) => <>
        <Button slot="trigger" {...stylex.props(styles.button)}>{isExpanded ? 'Hide full message' : 'Show full message'}</Button>
        {!isExpanded ? <div {...stylex.props(styles.messageBody, ...bodyStyles)} tabIndex={0} role="group" aria-label="Message excerpt"><MessageBody text={preview} foreground={variant._tag !== 'Outgoing'} /></div> : null}
        <DisclosurePanel>{isExpanded ? <div {...stylex.props(styles.messageBody, ...bodyStyles)} tabIndex={0} role="region" aria-label="Full message"><MessageBody text={body.markdown} foreground={variant._tag !== 'Outgoing'} /></div> : null}</DisclosurePanel>
      </>}
    </Disclosure> : <div {...stylex.props(styles.messageBody, ...bodyStyles)} tabIndex={body.markdown.length > 4000 ? 0 : undefined} role={body.markdown.length > 4000 ? 'region' : 'group'} aria-label={body._tag === 'Full' ? 'Message body' : 'Message excerpt'}><MessageBody text={body._tag === 'Full' ? body.markdown : preview} foreground={variant._tag !== 'Outgoing'} /></div>}
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
  const receiptCount = recipients.filter(recipient => recipient.receipt !== undefined).length
  const normalizedSubject = subject?.replace(/^(?:re:\s*)+/i, 'Re: ')
  return <Disclosure defaultExpanded={defaultExpanded}>
    {({ isExpanded }) => <article {...stylex.props(styles.message)} aria-label={`Message to ${target ?? ''}`} data-message-variant="outgoing" data-harness-weight="row" data-work-kind="messages-sent">
      <header data-message-header {...stylex.props(styles.workHeader)} title={at}>
        <span aria-hidden="true" data-message-glyph {...stylex.props(styles.direction)}><Icon name="message" scale="status" /></span>
        <Button slot="trigger" aria-label={`To ${target ?? ''} · ${firstLine}`} {...stylex.props(styles.trigger)}>
          <span {...stylex.props(styles.rowTitle)}>
          <span {...stylex.props(styles.meta)}>To </span>
          {target !== undefined ? <strong {...stylex.props(styles.sender)}>{delivery._tag === 'Broadcast' ? target : <AgentIdHierarchy id={target} />}</strong> : null}
          {' · '}
          <span {...stylex.props(styles.excerpt)} title={firstLine}>{firstLine}</span>
          </span>
          <span data-message-disclosure aria-hidden="true" {...stylex.props(styles.disclosureGlyph)}><Icon name={isExpanded ? 'chevron-down' : 'chevron-right'} scale="disclosure" /></span>
        </Button>
        <span {...stylex.props(styles.rowMetadata)}>
          {receiptCount > 0 ? <span {...stylex.props(styles.meta)}>{receiptCount} {receiptCount === 1 ? 'receipt' : 'receipts'}</span> : null}
          {onCopy !== undefined ? <Button aria-label="Copy message" {...stylex.props(styles.headerAction, styles.copy)} onPress={onCopy}><Icon name="copy" scale="status" /></Button> : null}
        </span>
      </header>
      {isExpanded ? <DisclosurePanel>
        <div data-opened-body {...stylex.props(styles.openedBody)}>
          {normalizedSubject !== undefined ? <h2 data-message-subject {...stylex.props(styles.openedSubject)}>{normalizedSubject}</h2> : null}
          <div data-message-content {...stylex.props(styles.messageBody)} tabIndex={message.length > 4000 ? 0 : undefined} role="region" aria-label="Outgoing message body"><MessageBody text={message} /></div>
          <div {...stylex.props(styles.controls)}>{recipients.map(recipient => <span key={recipient.id} role={recipient.receipt?._tag === 'Failed' ? 'note' : undefined} tabIndex={recipient.receipt?._tag === 'Failed' ? 0 : undefined} aria-label={recipient.receipt?._tag === 'Failed' ? `${recipient.id} · failed · ${recipient.receipt.reason}` : undefined} {...stylex.props(styles.receipt, recipient.receipt?._tag === 'Failed' && styles.danger)} title={recipient.receipt?._tag === 'Failed' ? recipient.receipt.reason : recipient.id}>
            <AgentIdHierarchy id={recipient.id} />{recipient.receipt !== undefined ? ` · ${recipient.receipt._tag.toLowerCase()}` : ''}
          </span>)}</div>
          {recipients.flatMap(recipient => recipient.receipt?._tag === 'Failed' ? [<p key={recipient.id} {...stylex.props(styles.danger)}>{recipient.id}: {recipient.receipt.reason}</p>] : [])}
        </div>
      </DisclosurePanel> : null}
    </article>}
  </Disclosure>
}

const styles = stylex.create({
  message: { color: textVars.fg, minWidth: 0, boxSizing: 'border-box', overflowWrap: 'anywhere', fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading },
  card: { paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.border, borderRadius: r.md, backgroundColor: surfaceVars.raised },
  outgoing: { padding: 0, backgroundColor: surfaceVars.transparent },
  human: { paddingBlock: s.xs, paddingInline: s.lg, borderLeftWidth: g.focusRing, borderLeftStyle: 'solid', borderLeftColor: accentVars.primary, backgroundColor: surfaceVars.transparent },
  header: { display: 'flex', alignItems: 'center', gap: s.sm, height: g.icon, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  workHeader: { position: 'relative', display: 'grid', gridTemplateColumns: g.workRowColumns, alignItems: 'center', gap: s.sm, height: g.toolRow, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  rowMetadata: { gridColumn: 3, gridRow: 1, pointerEvents: 'none', display: 'inline-flex', alignItems: 'center', gap: s.sm, whiteSpace: 'nowrap', minWidth: 0 },
  rowTitle: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap' },
  disclosureGlyph: { position: 'absolute', insetInlineEnd: `calc(${g.durationColumn} + ${s.sm})`, top: s.xs, width: g.icon, height: g.icon, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', color: textVars.fgMuted },
  openedBody: { marginTop: s.xs2, marginLeft: g.textColumn, paddingBlock: s.md, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.border, borderRadius: r.sm, display: 'flex', flexDirection: 'column', gap: s.md },
  openedSubject: { margin: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  controls: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.md, minWidth: 0 },
  counterpart: { flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  agentPath: { display: 'inline-flex', alignItems: 'center', flexWrap: 'nowrap', whiteSpace: 'nowrap', verticalAlign: 'top', maxWidth: '100%' },
  agentTrigger: { display: 'inline-flex', alignItems: 'center', maxWidth: '100%', verticalAlign: 'top', borderRadius: r.sm, outlineColor: accentVars.primary, outlineOffset: g.focusOffset, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid' } },
  direction: { flex: '0 0 auto', width: g.icon, height: g.icon, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', color: textVars.fgMuted },
  sender: { color: textVars.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium },
  time: { marginLeft: 'auto', flex: '0 0 auto', whiteSpace: 'nowrap', textAlign: 'right' },
  content: { marginTop: s.xs, minWidth: 0 },
  contentWithoutSubject: { marginTop: s.xs2 },
  messageBody: { maxHeight: g.workExpandedMax, overflowY: 'auto', minWidth: 0, overflowWrap: 'anywhere', outlineColor: accentVars.primary, outlineOffset: g.focusOffset },
  chip: { paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, color: textVars.fgSoft, fontSize: t.denseSize, lineHeight: t.metaLeading, whiteSpace: 'nowrap', flex: '0 1 auto' },
  meta: { color: textVars.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  subject: { margin: 0, marginTop: s.xs, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  button: { padding: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, backgroundColor: surfaceVars.rowActive, color: textVars.fg, cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  headerAction: { flex: '0 1 auto', minWidth: 0, height: g.icon, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', borderWidth: 0, padding: 0, backgroundColor: surfaceVars.transparent, color: textVars.fgSoft, cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  copy: { pointerEvents: 'auto', opacity: { default: 0, ':hover': 1, ':focus-visible': 1 }, flex: '0 0 auto', width: g.icon },
  trigger: { gridColumn: '2 / 5', gridRow: 1, height: g.toolRow, minWidth: 0, display: 'grid', gridTemplateColumns: 'subgrid', alignItems: 'center', gap: s.sm, borderWidth: 0, padding: 0, backgroundColor: surfaceVars.transparent, color: textVars.fg, textAlign: 'left', cursor: 'pointer', outlineColor: accentVars.primary, outlineOffset: g.focusOffset, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  excerpt: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: textVars.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  receipt: { flex: '0 1 auto', minWidth: 0, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.denseSize, lineHeight: t.metaLeading, color: textVars.fgMuted, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: borderVars.borderStrong, borderRadius: r.sm, outlineColor: accentVars.primary, outlineOffset: g.focusOffset },
  danger: { color: statusVars.dangerFg },
  plainText: { margin: 0, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading },
  visuallyHidden: { position: 'absolute', width: g.hairline, height: g.hairline, padding: 0, margin: 0, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
})
