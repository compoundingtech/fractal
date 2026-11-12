import { markdownBlock } from './markdown-text'
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { MessagePrimitive } from '@assistant-ui/react'
import { Button } from 'react-aria-components'
import type { MessageItem } from '../embrace-data/model'
import { textVars as ink, borderVars as border, accentVars as accent, interactionVars as interaction, typeVars as t, spaceVars as s, geometryVars as g, radiusVars as r, surfaceVars as surface } from '../composition-tokens.stylex'
import { Markdown } from './Markdown'
import { TranscriptAttachments } from './TranscriptAttachments'
import { useRevealOpen } from './transcript-reveal'

/** A participant as the host names it: roster name or last ref segment, with the full ref for hover. */
export interface MailParticipant { readonly label: string; readonly ref?: string }
/** The host's naming of one message: who wrote it, to whom, and whether the person is party to it. */
export interface MailParticipants {
  readonly from: MailParticipant
  readonly to?: MailParticipant
  /** Mail to or from the person draws the stronger edge (`involving_you`); mail between others stays quiet. */
  readonly involvesPerson: boolean
}

/** Bodies longer than this collapse behind Read message, as a long prompt does behind Show full message. */
const longBody = (text: string) => text.length > 600 || text.split('\n').length > 8

function Name({ who }: { readonly who: MailParticipant }) {
  return <b title={who.ref !== undefined && who.ref !== who.label ? who.ref : undefined} {...stylex.props(styles.name)}>{who.label}</b>
}

/**
 * One message as one object: "Sender → Recipient · subject", the body below, no timestamp of its own (the
 * episode carries it). A header whose body is outside the loaded window says so instead of showing nothing.
 */
export function MailMessage({ item, participants }: { readonly item: MessageItem; readonly participants?: MailParticipants }) {
  const writer = item.author ?? item.from
  const from = participants?.from ?? { label: item.sender?.label ?? writer ?? 'Unknown sender', ...(writer === undefined ? {} : { ref: writer }) }
  const to = participants?.to ?? (item.to === undefined ? undefined : { label: item.to, ref: item.to })
  const involving = participants?.involvesPerson ?? false
  const text = item.text
  const long = text !== undefined && longBody(text)
  const [expanded, setExpanded] = React.useState(false)
  useRevealOpen(reveal => reveal.entryId === item.id, expanded, setExpanded)
  const bodyId = React.useId()
  return <MessagePrimitive.Root data-testid="mail-message" data-item-id={item.id} data-item-kind={item._tag} data-mail-look={involving ? 'involving_you' : 'between_others'} {...stylex.props(styles.mail, involving ? styles.involving : styles.between)}>
    <p data-testid="mail-header" {...stylex.props(styles.header, involving && styles.headerInvolving)}>
      <Name who={from} />{to !== undefined && <><span aria-label="to" {...stylex.props(styles.arrow)}> → </span><Name who={to} /></>}
      {item.title !== undefined && item.title !== '' && <span {...stylex.props(styles.subject)}> · <span data-search-mail-subject>{item.title}</span></span>}
    </p>
    {text === undefined
      ? <p data-testid="mail-body-missing" {...stylex.props(styles.missing)}>Message body not loaded</p>
      : <div id={bodyId} data-testid="mail-body" {...stylex.props(styles.body, involving && styles.bodyInvolving, long && !expanded && styles.clamp)}><Markdown text={markdownBlock(text)} /></div>}
    {long && <Button aria-expanded={expanded} aria-controls={bodyId} data-testid="mail-read" onPress={() => setExpanded(!expanded)} {...stylex.props(styles.read)}>{expanded ? 'Hide message' : 'Read message'}</Button>}
    {item.attachments !== undefined && item.attachments.length > 0 && <TranscriptAttachments attachments={item.attachments} messageId={item.id} />}
  </MessagePrimitive.Root>
}

const styles = stylex.create({
  mail: { display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0, paddingInlineStart: s.md, borderInlineStartStyle: 'solid', fontSize: t.bodySize, lineHeight: t.bodyLeading },
  // conversation-style.json `mail.between_others`: a quiet edge, muted sender.
  between: { borderInlineStartWidth: 2, borderInlineStartColor: border.borderStrong },
  // conversation-style.json `mail.involving_you`: the accent edge, accent bold sender, full-contrast body.
  involving: { borderInlineStartWidth: g.focusRing, borderInlineStartColor: accent.primary },
  header: { margin: 0, minWidth: 0, overflowWrap: 'anywhere', color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  headerInvolving: { color: accent.primary },
  name: { fontWeight: t.weightMedium },
  arrow: { color: ink.fgMuted },
  subject: { color: ink.fgMuted },
  body: { minWidth: 0, color: ink.fgSoft },
  bodyInvolving: { color: ink.fg },
  clamp: { maxHeight: `calc(${t.bodyLeading} * 6)`, overflow: 'hidden' },
  missing: { margin: 0, color: ink.fgMuted, fontSize: t.metaSize, fontStyle: 'italic' },
  read: { alignSelf: 'flex-start', minHeight: g.toolRow, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
})
