import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { AttachmentPrimitive, ComposerPrimitive, useAui, useAuiEvent, useAuiState } from '@assistant-ui/react'
import { Button } from 'react-aria-components'
import { interactionVars as interaction, borderVars as border, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'

/**
 * The composer's pending attachments, each removable before send, in one horizontally scrolling row of equal tiles,
 * so several images never push the field out of view above a phone keyboard. Renders nothing while the draft has
 * no attachment. Why a draft cannot be sent is the host's `ComposerSendRefusal`, not the tray's.
 */
export function ComposerAttachmentTray() {
  const count = useAuiState(state => state.composer.attachments.length)
  if (count === 0) return null
  return <div data-testid="composer-attachments" {...stylex.props(styles.tray)}>
    <ComposerPrimitive.Attachments>{() => <AttachmentPreview />}</ComposerPrimitive.Attachments>
  </div>
}

/**
 * Why Send is withheld, as a polite live region that stays mounted (visually hidden while empty) so the first
 * refusal is announced. Send references `id` as its description. Each blocked press bumps `attempt`, which
 * re-renders the line as a new node: assistive tech announces it again and the line briefly highlights.
 */
export function ComposerSendRefusal({ id, message, attempt, action }: {
  readonly id: string
  readonly message: string | undefined
  readonly attempt: number
  /** One-step recovery shown beside the message, outside the live region so it is not read as part of it. */
  readonly action?: React.ReactNode
}) {
  return <div data-testid="composer-send-refusal" {...stylex.props(styles.refusal, message === undefined && styles.noticeIdle)}>
    <div id={id} role="status" {...stylex.props(styles.refusalText)}>
      {message !== undefined && <p key={attempt} data-refusal-attempt={attempt} {...stylex.props(styles.problem, styles.line, attempt > 0 && styles.flash)}>{message}</p>}
    </div>
    {message !== undefined && action}
  </div>
}

export interface AttachmentRefusal { readonly id: number; readonly message: string }
interface AttachmentAddError { readonly reason: string; readonly message: string; readonly contentType?: string }

const typeLabels: Readonly<Record<string, string>> = { 'image/png': 'PNG', 'image/jpeg': 'JPEG', 'image/gif': 'GIF', 'image/webp': 'WebP', 'image/svg+xml': 'SVG', 'image/heic': 'HEIC' }
const spokenList = (labels: readonly string[]) => labels.length < 2 ? labels.join('') : `${labels.slice(0, -1).join(', ')} or ${labels.at(-1)}`

/**
 * User copy for one refused add. The host adapter's own refusal (size, upload unavailable) is already user copy and
 * is kept verbatim; the runtime's type refusal is technical, so it is restated with the formats the adapter accepts.
 */
export const attachmentAddErrorNotice = (error: AttachmentAddError, accept: string): string => {
  if (error.reason === 'adapter-error' && error.message !== '') return error.message
  if (error.reason !== 'not-accepted') return 'This image could not be attached.'
  const kind = error.contentType === undefined || error.contentType === '' ? undefined : typeLabels[error.contentType]
  const subject = kind === undefined ? 'This file' : `${kind} files`
  const accepted = accept.split(',').map(type => type.trim()).filter(type => type !== '')
  const labels = accepted.flatMap(type => typeLabels[type] ?? [])
  return labels.length === 0 || labels.length !== accepted.length ? `${subject} can't be attached here.` : `${subject} can't be attached. Use ${spokenList(labels)}.`
}

/**
 * Refused picker and paste adds for the thread composer, from the public `composer.attachmentAddError` event, plus
 * `refuse` for an add the composer itself withholds (a blocked Attach). Refusals of one pick or paste are listed
 * together; the next accepted add, a removal or a send clears them.
 */
export function useAttachmentRefusals(enabled: boolean): { readonly refusals: readonly AttachmentRefusal[]; readonly refuse: (message: string) => void } {
  const aui = useAui()
  const [refusals, setRefusals] = React.useState<readonly AttachmentRefusal[]>([])
  const sequence = React.useRef(0)
  const batch = React.useRef<{ open: boolean; refused: boolean }>({ open: false, refused: false })
  // One pick or paste adds its files in the same task; a later task starts a new batch.
  const enterBatch = () => {
    if (batch.current.open) return batch.current
    const next = { open: true, refused: false }
    batch.current = next
    window.setTimeout(() => { next.open = false }, 0)
    return next
  }
  useAuiEvent('composer.attachmentAddError', event => {
    if (!enabled || event.messageId !== undefined) return
    const current = enterBatch()
    const refusal = { id: ++sequence.current, message: attachmentAddErrorNotice(event, aui.composer.getState().attachmentAccept) }
    const append = current.refused
    current.refused = true
    setRefusals(previous => append ? [...previous, refusal] : [refusal])
  })
  useAuiEvent('composer.attachmentAdd', event => {
    if (!enabled || event.messageId !== undefined) return
    if (!enterBatch().refused) setRefusals([])
  })
  useAuiEvent('composer.send', event => { if (event.messageId === undefined) setRefusals([]) })
  const staged = useAuiState(state => state.composer.attachments.length)
  const [previous, setPrevious] = React.useState(staged)
  if (staged !== previous) {
    setPrevious(staged)
    if (staged < previous) setRefusals([])
  }
  // A new line each time, so a repeated refusal is announced again.
  const refuse = React.useCallback((message: string) => setRefusals([{ id: ++sequence.current, message }]), [])
  return { refusals: enabled ? refusals : [], refuse }
}

/**
 * Announces refused adds even while nothing is staged. The live region stays mounted (visually hidden while
 * empty) so the first refusal is announced; each refusal is a new line, so a repeated refusal is announced again.
 */
export function ComposerAttachmentNotice({ refusals }: { readonly refusals: readonly AttachmentRefusal[] }) {
  return <div aria-live="polite" data-testid="composer-attachment-notice" {...stylex.props(styles.notice, refusals.length === 0 && styles.noticeIdle)}>
    {refusals.map(refusal => <p key={refusal.id} {...stylex.props(styles.problem, styles.line)}>{refusal.message}</p>)}
  </div>
}

function AttachmentPreview() {
  const attachment = useAuiState(state => state.attachment)
  const file = 'file' in attachment ? attachment.file : undefined
  const [url, setUrl] = React.useState<string>()
  React.useEffect(() => {
    if (file === undefined) return
    const next = URL.createObjectURL(file)
    setUrl(next)
    return () => URL.revokeObjectURL(next)
  }, [file])
  // An upload that failed returns the attachment to the draft with the reason; the draft keeps it until removed.
  const problem = attachment.status.type === 'incomplete' ? attachment.status.message ?? 'This image could not be uploaded.' : undefined
  const uploading = attachment.status.type === 'running'
  return <AttachmentPrimitive.Root data-attachment-status={attachment.status.type} {...stylex.props(styles.attachment, problem !== undefined && styles.failed)}>
    {url !== undefined && <img src={url} alt={attachment.name} {...stylex.props(styles.image)} />}
    <span {...stylex.props(styles.text)}>
      <span title={attachment.name} {...stylex.props(styles.name)}><AttachmentPrimitive.Name /></span>
      {problem !== undefined ? <span role="alert" {...stylex.props(styles.problem, styles.line)}>{problem}</span> : uploading ? <span role="status" {...stylex.props(styles.detail)}>Uploading…</span> : null}
    </span>
    <AttachmentPrimitive.Remove asChild><Button aria-label={`Remove ${attachment.name}`} {...stylex.props(composerControl.ghost, composerControl.icon, composerControl.touchHit)}><Icon name="x" /></Button></AttachmentPrimitive.Remove>
  </AttachmentPrimitive.Root>
}

const flash = stylex.keyframes({ from: { backgroundColor: tone.dangerMuted }, to: { backgroundColor: surface.transparent } })

/**
 * The composer's secondary controls (Attach, Remove, inline recovery): one ghost treatment in both themes, with a
 * hover/pressed wash. `touchHit` extends the hit area to the 44px touch target around the visible 28px control.
 */
export const composerControl = stylex.create({
  ghost: {
    position: 'relative', display: 'inline-flex', justifyContent: 'center', alignItems: 'center', gap: s.xs, flexShrink: 0, boxSizing: 'border-box',
    minHeight: g.controlMd, paddingInline: s.md, borderWidth: 0, borderRadius: r.control,
    backgroundColor: { default: surface.transparent, ':hover': interaction.wash, ':active': interaction.wash },
    color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, cursor: 'pointer',
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  icon: { width: g.controlMd, height: g.controlMd, paddingInline: 0 },
  touchHit: { '::before': { content: '""', position: 'absolute', inset: `min(0px, calc((100% - ${g.touchTarget}) / 2))` } },
  // Keeps its place and focus but reads as unavailable; activation is guarded by the caller.
  blocked: { color: ink.fgMuted, cursor: 'not-allowed', backgroundColor: { default: surface.transparent, ':hover': surface.transparent } },
})

const styles = stylex.create({
  // Tiles align with the field's text inset; block padding keeps focus rings clear of the scroll clip.
  tray: {
    display: 'flex', gap: s.md, minWidth: 0, overflowX: 'auto', overflowY: 'hidden', paddingInline: s.xs, paddingBlock: s.xs2,
    scrollSnapType: 'x mandatory', scrollPaddingInline: s.xs, overscrollBehaviorX: 'contain', scrollbarWidth: 'thin', scrollbarColor: `${tone.scrollThumb} transparent`,
  },
  notice: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, paddingInline: s.xs },
  // Out of flow while empty, so the pill keeps its row; still in the accessibility tree as a live region.
  noticeIdle: { position: 'absolute', width: '1px', height: '1px', overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  line: { margin: 0, textWrap: 'pretty' },
  refusal: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', columnGap: s.md, rowGap: s.xs, minWidth: 0, paddingInline: s.xs },
  refusalText: { flex: '1 1 16ch', minWidth: 0 },
  flash: { borderRadius: r.keyboard, marginInline: `calc(${s.xs} * -1)`, paddingInline: s.xs, animationName: flash, animationDuration: m.reveal, animationTimingFunction: 'ease-out' },
  attachment: {
    display: 'flex', alignItems: 'center', gap: s.md, flex: `0 0 ${g.fileMax}`, maxWidth: '75%', boxSizing: 'border-box', minWidth: 0,
    padding: s.md, borderRadius: r.sm, backgroundColor: surface.rowActive, scrollSnapAlign: 'start',
  },
  failed: { outlineWidth: g.hairline, outlineStyle: 'solid', outlineColor: tone.dangerFg, outlineOffset: `calc(${g.hairline} * -1)` },
  // Inner radius = tile radius − padding, floored at 4px; the inset hairline separates white or black edges from the tile.
  image: { width: g.band, height: g.band, flexShrink: 0, objectFit: 'cover', borderRadius: r.keyboard, outlineWidth: g.hairline, outlineStyle: 'solid', outlineColor: border.rail, outlineOffset: `calc(${g.hairline} * -1)` },
  text: { display: 'flex', flexDirection: 'column', flex: '1 1 auto', minWidth: 0, marginInlineEnd: s.xs },
  name: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  detail: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  problem: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: tone.dangerFg },
})
