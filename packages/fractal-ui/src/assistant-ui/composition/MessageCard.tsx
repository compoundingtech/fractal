import { markdownBlock, markdownInline } from './markdown-text'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { MessagePrimitive } from '@assistant-ui/react'
import { Button, Disclosure, DisclosurePanel } from 'react-aria-components'
import { backgroundJobGroupLead, backgroundJobStateLabel, harnessOutcomeTone, type Attachment, type ConversationItem, type EventItem, type HarnessNotice, type MessageItem } from '../embrace-data/model'
import { textVars as ink, borderVars as border, accentVars as accent, interactionVars as interaction, statusVars as status, typeVars as t, spaceVars as s, geometryVars as g, radiusVars as r, surfaceVars as surface } from '../composition-tokens.stylex'
import { Icon, type IconName } from './Icons'
import { InlineMarkdown, Markdown } from './Markdown'
import { HarnessNoticeDetail } from './HarnessNotice'
import { TranscriptAttachments } from './TranscriptAttachments'
import { useRevealOpen } from './transcript-reveal'
import type { MessageRunLayout } from './message-runs'

/** Who a sender is: a person writes cards, an agent or subagent has a coloured monogram, a machine an icon. */
export type MessageSenderKind = 'person' | 'agent' | 'subagent' | 'machine'
/** The sender's live state, drawn as the 8px dot on the avatar's corner. */
export type MessageSenderStatus = 'working' | 'idle' | 'needs' | 'done' | 'failed'
/** A participant as the host names it: roster name or last ref segment, with the full ref for hover. */
export interface MessageParticipant {
  readonly label: string
  readonly ref?: string
  readonly kind?: MessageSenderKind
  /** A person's real avatar, resolved by the host (e.g. from a GitHub login); the monogram stands in until it loads or if it fails. */
  readonly avatarUrl?: string
  readonly status?: MessageSenderStatus
}
/** The host's naming of one message: who wrote it, to whom, and whether the person is party to it. */
export interface MessageParticipants {
  readonly from: MessageParticipant
  readonly to?: MessageParticipant
  /** Mail to or from the person draws the 'to you' edge; mail between others stays quiet. */
  readonly involvesPerson: boolean
}

/** How a message-like item is drawn: a person's or agent's card, a machine result's row, or neither (regular output). */
export type MessageShape =
  | { readonly _tag: 'Card'; readonly participants: MessageParticipants; readonly kindWord?: string; readonly subject?: string; readonly text?: string; readonly attachments?: readonly Attachment[] }
  | { readonly _tag: 'Row'; readonly participants: MessageParticipants; readonly kindWord: string; readonly statusWord?: string; readonly failed: boolean; readonly excerpt: string; readonly icon?: IconName; /** What Read names: a result, or a machine's message. */ readonly noun: 'result' | 'message'; readonly notice: HarnessNotice }

const fallbackFrom = (item: MessageItem): MessageParticipant => {
  const writer = item.author ?? item.from
  return { label: item.sender?.label ?? writer ?? 'Unknown sender', ...(writer === undefined ? {} : { ref: writer }), kind: writer?.startsWith('person/') ? 'person' : writer?.startsWith('daemon/') ? 'machine' : 'agent' }
}

/**
 * The one place that decides what is a message. Mail is a card; a person's interjection, IRC and deliveries are cards;
 * subagent and background-job results are rows. Everything else (summaries, skills, system records) stays output.
 */
export function messageShape(item: ConversationItem, hostParticipants: MessageParticipants | undefined): MessageShape | undefined {
  if (item._tag === 'Message') {
    const participants = hostParticipants ?? { from: fallbackFrom(item), ...(item.to === undefined ? {} : { to: { label: item.to, ref: item.to } }), involvesPerson: false }
    return { _tag: 'Card', participants, ...(item.title === undefined || item.title === '' ? {} : { subject: item.title }), ...(item.text === undefined ? {} : { text: item.text }), ...(item.attachments === undefined ? {} : { attachments: item.attachments }) }
  }
  if (item._tag !== 'Event' || item.notice === undefined) return undefined
  const notice = item.notice
  const to = (label: string | undefined) => label === undefined ? {} : { to: { label } }
  switch (notice._tag) {
    case 'Interjection': return { _tag: 'Card', participants: { from: { label: notice.from, kind: 'person' }, ...to(notice.to), involvesPerson: true }, kindWord: 'Interjection', text: notice.payload }
    case 'Irc': return { _tag: 'Card', participants: { from: { label: notice.from, kind: 'agent' }, ...to(notice.to), involvesPerson: false }, kindWord: 'IRC', text: notice.payload }
    // Fractal's own mail (a wake, a restart handoff) is a machine's message: a row.
    case 'Delivery': if (item.sender?.kind === 'system') return { _tag: 'Row', participants: { from: { label: notice.from, kind: 'machine' }, ...to(notice.to), involvesPerson: false }, kindWord: 'Wake', failed: false, excerpt: item.title, noun: 'message', notice }
      return { _tag: 'Card', participants: { from: { label: notice.from, kind: 'agent' }, ...to(notice.to), involvesPerson: false }, ...(notice.subject === undefined ? {} : { subject: notice.subject }), text: notice.payload }
    case 'SubagentResult': return { _tag: 'Row', participants: { from: { label: notice.taskId, kind: 'subagent' }, involvesPerson: false }, kindWord: 'Subagent', statusWord: notice.status, failed: harnessOutcomeTone(notice) === 'failed', excerpt: item.text, noun: 'result', notice }
    case 'BackgroundJob': return { _tag: 'Row', participants: { from: { label: notice.label ?? item.title, kind: 'machine' }, involvesPerson: false }, kindWord: 'Background job', statusWord: backgroundJobStateLabel(notice.state), failed: harnessOutcomeTone(notice) === 'failed', excerpt: item.text, icon: 'clock', noun: 'result', notice }
    case 'BackgroundJobGroup': return { _tag: 'Row', participants: { from: { label: item.title, kind: 'machine' }, involvesPerson: false }, kindWord: 'Background jobs', statusWord: backgroundJobStateLabel(backgroundJobGroupLead(notice.outcomes)), failed: harnessOutcomeTone(notice) === 'failed', excerpt: item.text, icon: 'clock', noun: 'result', notice }
    default: return undefined
  }
}

/** The kind word is dropped when the name already says it: "Background job 42" needs no "Background job". */
const kindWordShown = (label: string, word: string | undefined): word is string =>
  word !== undefined && !label.toLowerCase().includes(word.toLowerCase().split(' ').at(-1)!.replace(/s$/, ''))

const hueOf = (seed: string) => {
  let hash = 2166136261
  for (const char of seed) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619)
  return (hash >>> 0) % 360
}
const monogram = (label: string) => {
  const parts = label.replace(/^@/, '').split(/[-_.\s/]+|(?=[A-Z][a-z])/).filter(part => part !== '')
  return (parts.length >= 2 ? `${parts[0]![0]}${parts[1]![0]}` : label.slice(0, 2)).toUpperCase()
}
const machineIcon: IconName = 'gear'

/** One avatar for every sender: a person's photo or monogram, an agent's monogram in the shared palette, a machine's icon; the status dot sits on its corner. */
export function MessageAvatar({ who, icon }: { readonly who: MessageParticipant; readonly icon?: IconName }) {
  const [failed, setFailed] = React.useState(false)
  const machine = who.kind === 'machine'
  return <span data-testid="message-avatar" data-sender-kind={who.kind ?? 'agent'} aria-hidden="true" {...stylex.props(styles.avatar, machine ? styles.avatarMachine : styles.avatarHue(hueOf(who.ref ?? who.label)), who.kind === 'person' && styles.avatarPerson)}>
    {machine ? <Icon name={icon ?? machineIcon} size={12} />
      : who.avatarUrl !== undefined && !failed ? <img src={who.avatarUrl} alt="" referrerPolicy="no-referrer" onError={() => setFailed(true)} {...stylex.props(styles.avatarImage, who.kind === 'person' && styles.avatarPerson)} />
        : monogram(who.label)}
    {who.status !== undefined && <span data-testid="message-status-dot" data-status={who.status} {...stylex.props(styles.dot, dotStyles[who.status])} />}
  </span>
}

/** Pointer over or focus within a message reveals its same-minute time. */
function useHover() {
  const [hovered, setHovered] = React.useState(false)
  const [focused, setFocused] = React.useState(false)
  return [hovered || focused, { onMouseEnter: () => setHovered(true), onMouseLeave: () => setHovered(false), onFocus: () => setFocused(true), onBlur: (event: React.FocusEvent) => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false) } }] as const
}

const timeFormatter = new Intl.DateTimeFormat([], { hour: 'numeric', minute: '2-digit' })
const noLayout: MessageRunLayout = { stacked: false, lastInStack: true, showRecipient: true, showTime: true }

/** Avatar, name, '→ recipient', kind word, subject, then the time pinned to the end; names truncate before the time moves. */
function MessageHead({ shape, layout, at, hovered, trailing }: { readonly shape: MessageShape; readonly layout: MessageRunLayout; readonly at?: string; /** Pointer or focus inside the message: a same-minute time shows. */ readonly hovered: boolean; readonly trailing?: React.ReactNode }) {
  const { from, to } = shape.participants
  const time = at === undefined ? NaN : Date.parse(at)
  const subject = shape._tag === 'Card' ? shape.subject : undefined
  return <div data-testid="message-head" {...stylex.props(styles.head)}>
    <MessageAvatar who={from} {...(shape._tag === 'Row' && shape.icon !== undefined ? { icon: shape.icon } : {})} />
    <span {...stylex.props(styles.names)}>
      <b title={from.ref !== undefined && from.ref !== from.label ? from.ref : from.label} {...stylex.props(styles.name)}>{from.label}</b>
      {layout.showRecipient && to !== undefined && <span data-testid="message-recipient" title={to.ref ?? to.label} {...stylex.props(styles.recipient)}><span aria-label="to"> → </span>{to.label}</span>}
      {kindWordShown(from.label, shape.kindWord) && <span data-testid="message-kind" {...stylex.props(styles.meta)}> · {shape.kindWord}</span>}
      {subject !== undefined && <span {...stylex.props(styles.subject)}> · <InlineMarkdown text={markdownInline(subject)} /></span>}
    </span>
    {trailing}
    {Number.isFinite(time) && <time data-testid="message-time" data-time-visible={layout.showTime} dateTime={new Date(time).toISOString()} {...stylex.props(styles.time, !layout.showTime && !hovered && styles.timeHidden)}>{timeFormatter.format(time)}</time>}
  </div>
}

/** Bodies longer than this collapse behind Read message, as a long prompt does behind Show full message. */
const longBody = (text: string) => text.length > 600 || text.split('\n').length > 8

/**
 * A person's or agent's message as one tinted card. Consecutive messages from one sender stack under the first head 4px
 * apart; mail to or from the person carries the 'to you' edge. A header whose body is outside the loaded window says so.
 */
export function MessageCard({ item, shape, layout = noLayout }: { readonly item: ConversationItem; readonly shape: Extract<MessageShape, { _tag: 'Card' }>; readonly layout?: MessageRunLayout }) {
  const involving = shape.participants.involvesPerson
  const text = shape.text
  const long = text !== undefined && longBody(text)
  const [expanded, setExpanded] = React.useState(false)
  useRevealOpen(reveal => reveal.entryId === item.id, expanded, setExpanded)
  const bodyId = React.useId()
  const [hovered, hover] = useHover()
  return <MessagePrimitive.Root {...hover} data-testid="message-card" data-item-id={item.id} data-item-kind={item._tag} data-message-look={involving ? 'to_you' : 'between_others'} data-stacked={layout.stacked || undefined} data-sender-kind={shape.participants.from.kind ?? 'agent'} {...stylex.props(styles.card, involving && styles.toYou, layout.stacked ? styles.stacked : styles.headed)}>
    {!layout.stacked && <MessageHead shape={shape} layout={layout} at={item.at} hovered={hovered} />}
    {text === undefined
      ? <p data-testid="message-body-missing" {...stylex.props(styles.missing)}>Message body not loaded</p>
      : <div id={bodyId} data-testid="message-body" {...stylex.props(styles.body, long && !expanded && styles.clamp)}><Markdown text={markdownBlock(text)} /></div>}
    {long && <Button aria-expanded={expanded} aria-controls={bodyId} data-testid="message-read" onPress={() => setExpanded(!expanded)} {...stylex.props(styles.read)}>{expanded ? 'Hide message' : 'Read message'}</Button>}
    {shape.attachments !== undefined && shape.attachments.length > 0 && <TranscriptAttachments attachments={shape.attachments} messageId={item.id} />}
  </MessagePrimitive.Root>
}

/** A machine's message (a subagent or background-job result) as one row; a failure draws a red edge and a red status word, never a red fill. */
export function MessageRow({ item, shape, layout = noLayout }: { readonly item: EventItem; readonly shape: Extract<MessageShape, { _tag: 'Row' }>; readonly layout?: MessageRunLayout }) {
  const [expanded, setExpanded] = React.useState(false)
  useRevealOpen(reveal => reveal.entryId === item.id, expanded, setExpanded)
  const [hovered, hover] = useHover()
  return <MessagePrimitive.Root {...hover} data-testid="message-row" data-item-id={item.id} data-item-kind={item._tag} data-failed={shape.failed || undefined} {...stylex.props(styles.row, shape.failed && styles.rowFailed)}>
    <Disclosure isExpanded={expanded} onExpandedChange={setExpanded}>
      <MessageHead shape={shape} layout={layout} at={item.at} hovered={hovered} trailing={<>
        {shape.statusWord !== undefined && <span data-testid="message-status" {...stylex.props(styles.statusWord, shape.failed && styles.statusFailed)}>{shape.failed && <Icon name="x" size={12} />}{shape.statusWord}</span>}
        <span data-testid="message-excerpt" {...stylex.props(styles.excerpt)}>{shape.excerpt}</span>
        <Button slot="trigger" aria-label={`${expanded ? 'Hide' : 'Read'} ${shape.noun}`} data-testid="message-row-trigger" {...stylex.props(styles.rowTrigger)}><Icon name={expanded ? 'chevron-down' : 'chevron-right'} size={12} /></Button>
      </>} />
      <DisclosurePanel><div {...stylex.props(styles.rowPanel)}><HarnessNoticeDetail notice={shape.notice} /></div></DisclosurePanel>
    </Disclosure>
  </MessagePrimitive.Root>
}

const avatarSize = 20
const dotStyles = stylex.create({
  working: { backgroundColor: status.running },
  idle: { backgroundColor: ink.fgMuted },
  needs: { backgroundColor: status.attention },
  done: { backgroundColor: status.done },
  failed: { backgroundColor: status.danger },
})
const styles = stylex.create({
  // 8px between head and body, 16px between unrelated messages (the turn's 8px gap plus 8px), 4px inside a stack.
  card: { display: 'flex', flexDirection: 'column', gap: s.md, minWidth: 0, paddingBlock: s.lg, paddingInline: s.lg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, backgroundColor: surface.messageCard, fontSize: t.bodySize, lineHeight: t.bodyLeading },
  headed: { marginBlockStart: s.md },
  stacked: { marginBlockStart: `calc(${s.xs} - ${s.md})` },
  // The 'to you' edge: the accent at about a third, so it marks the card without shouting.
  toYou: { borderInlineStartWidth: 2, borderInlineStartColor: `color-mix(in srgb, ${accent.primary} 34%, transparent)` },
  head: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  names: { display: 'flex', alignItems: 'baseline', minWidth: 0, flex: '1 1 auto', overflow: 'hidden', whiteSpace: 'nowrap' },
  name: { minWidth: 0, flexShrink: 1, overflow: 'hidden', textOverflow: 'ellipsis', color: ink.fg, fontWeight: t.weightMedium },
  // 'pre' keeps the leading space a flex item would otherwise drop.
  recipient: { whiteSpace: 'pre', minWidth: 0, flexShrink: 1, overflow: 'hidden', textOverflow: 'ellipsis' },
  meta: { whiteSpace: 'pre', flexShrink: 0 },
  subject: { whiteSpace: 'pre', minWidth: 0, flexShrink: 2, overflow: 'hidden', textOverflow: 'ellipsis' },
  time: { flexShrink: 0, marginInlineStart: 'auto', fontVariantNumeric: 'tabular-nums' },
  // Same-minute times wait for hover or focus on their message.
  timeHidden: { opacity: 0 },
  body: { minWidth: 0, color: ink.fg },
  clamp: { maxHeight: `calc(${t.bodyLeading} * 6)`, overflow: 'hidden' },
  missing: { margin: 0, color: ink.fgMuted, fontSize: t.metaSize, fontStyle: 'italic' },
  read: { alignSelf: 'flex-start', minHeight: g.toolRow, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  row: { minWidth: 0, marginBlockStart: s.md, paddingBlock: s.xs, paddingInline: s.md, borderInlineStartWidth: 2, borderInlineStartStyle: 'solid', borderInlineStartColor: border.borderStrong, fontSize: t.metaSize, lineHeight: t.metaLeading },
  rowFailed: { borderInlineStartColor: status.danger },
  statusWord: { display: 'inline-flex', alignItems: 'center', gap: s.xs, flexShrink: 0, whiteSpace: 'nowrap', color: ink.fgSoft },
  statusFailed: { color: status.dangerFg },
  excerpt: { minWidth: 0, flex: '1 1 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  rowTrigger: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.controlSm, height: g.controlSm, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  rowPanel: { marginBlockStart: s.md, paddingBlock: s.sm, paddingInline: s.md, maxHeight: 'min(32rem, 60vh)', overflowY: 'auto', overflowWrap: 'anywhere', borderRadius: r.sm, backgroundColor: surface.washSubtle },
  avatar: { position: 'relative', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: avatarSize, height: avatarSize, borderRadius: r.sm, color: '#ffffff', fontSize: 9, fontWeight: 600, letterSpacing: '0.02em', lineHeight: 1 },
  avatarPerson: { borderRadius: r.full },
  avatarMachine: { backgroundColor: surface.washSubtle, color: ink.fgMuted, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong },
  // One palette: the same lightness and chroma for every sender, only the hue varies.
  // 54% lightness keeps the white 9px monogram at 4.5:1 or better on every hue (worst, teal near 191°: 4.7:1); 58% fell to 4.05:1.
  avatarHue: (hue: number) => ({ backgroundColor: `oklch(54% 0.1 ${hue})` }),
  avatarImage: { width: '100%', height: '100%', objectFit: 'cover', borderRadius: r.sm },
  dot: { position: 'absolute', insetInlineEnd: -2, insetBlockEnd: -2, width: 8, height: 8, borderRadius: r.full, boxShadow: `0 0 0 2px ${surface.messageCard}` },
})
