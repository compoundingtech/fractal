import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { AttachmentPrimitive, ComposerPrimitive, useAui, useAuiEvent, useAuiState, type Attachment } from '@assistant-ui/react'
import { Button } from 'react-aria-components'
import { interactionVars as interaction, borderVars as border, geometryVars as g, motionVars as m, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'

/**
 * The composer's attachments: those staged in the draft, each removable before send, and, while a send uploads its
 * images, the submission's images, shown at once with an activity bar so the row keeps its height. Chips grow to fit
 * their name up to `attachmentMax`; a longer name keeps its end and extension and shortens the middle. From 600px
 * wide chips wrap; below it they share one horizontally scrolling row, so several images never push the field out of
 * view above a phone keyboard. Renders nothing while there is neither. `onCancelUpload` stops the whole send (the
 * runtime cancels a submission, not one of its images): one image's × cancels it, several share one Cancel upload.
 * Why a draft cannot be sent is the host's `ComposerSendRefusal`, not the tray's.
 */
export function ComposerAttachmentTray({ onCancelUpload }: { readonly onCancelUpload?: ((from: Element) => void) | undefined }) {
  const count = useAuiState(state => state.composer.attachments.length)
  const submitted = useAuiState(state => state.composer.submission?.attachments) ?? noAttachments
  if (count === 0 && submitted.length === 0) return null
  return <div data-testid="composer-attachments" {...stylex.props(styles.tray)}>
    {submitted.map(attachment => <div key={attachment.id} data-attachment-status={attachment.status.type} data-uploading="true" {...stylex.props(styles.attachment)}>
      <AttachmentChip attachment={attachment} uploading />
      {submitted.length === 1 && onCancelUpload !== undefined ? <Button aria-label={`Cancel upload of ${attachment.name}`} onPress={event => onCancelUpload(event.target)} {...stylex.props(composerControl.ghost, composerControl.icon, composerControl.touchHit)}><Icon name="x" /></Button> : null}
    </div>)}
    {submitted.length > 1 && onCancelUpload !== undefined ? <Button aria-label={`Cancel upload of ${submitted.length} images`} onPress={event => onCancelUpload(event.target)} {...stylex.props(composerControl.ghost, composerControl.outline, composerControl.touchHit, styles.trayAction)}>Cancel upload</Button> : null}
    <ComposerPrimitive.Attachments>{() => <AttachmentPreview />}</ComposerPrimitive.Attachments>
  </div>
}

const noAttachments: readonly Attachment[] = []

/**
 * Names a send's upload for assistive tech while the tray's chips show it. It stays mounted (visually hidden) so the
 * first upload is announced.
 */
export function ComposerUploadStatus() {
  const submitted = useAuiState(state => state.composer.submission?.attachments) ?? noAttachments
  const label = submitted.length === 0 ? undefined : submitted.length === 1 ? `Uploading ${submitted[0]!.name}…` : `Uploading ${submitted.length} images…`
  return <div role="status" data-testid="composer-upload" {...stylex.props(styles.noticeIdle)}>{label}</div>
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
  // An upload that failed returns the attachment to the draft with the reason; the draft keeps it until removed.
  const problem = attachment.status.type === 'incomplete' ? attachment.status.message ?? 'This image could not be uploaded.' : undefined
  return <AttachmentPrimitive.Root data-attachment-status={attachment.status.type} {...stylex.props(styles.attachment, problem !== undefined && styles.failed)}>
    <AttachmentChip attachment={attachment} problem={problem} uploading={attachment.status.type === 'running'} announce />
    <AttachmentPrimitive.Remove asChild><Button aria-label={`Remove ${attachment.name}`} {...stylex.props(composerControl.ghost, composerControl.icon, composerControl.touchHit)}><Icon name="x" /></Button></AttachmentPrimitive.Remove>
  </AttachmentPrimitive.Root>
}

// Long names keep this many trailing characters (the end of the name and its extension) and shorten before them,
// so screenshots that share a prefix stay distinguishable.
const nameTail = 12

/** A chip's thumbnail, name and state; the caller owns the chip and its trailing control. */
function AttachmentChip({ attachment, problem, uploading, announce = false }: {
  readonly attachment: Pick<Attachment, 'name' | 'file'>
  readonly problem?: string | undefined
  readonly uploading: boolean
  /** Announce the upload from the chip; a submission's upload is announced once by `ComposerUploadStatus`. */
  readonly announce?: boolean
}) {
  const file = attachment.file
  const [url, setUrl] = React.useState<string>()
  React.useEffect(() => {
    if (file === undefined) return
    const next = URL.createObjectURL(file)
    setUrl(next)
    return () => URL.revokeObjectURL(next)
  }, [file])
  const name = attachment.name
  const split = name.length > nameTail + 4
  return <>
    {url !== undefined && <img src={url} alt={name} {...stylex.props(styles.image)} />}
    <span {...stylex.props(styles.text)}>
      <span title={name} {...stylex.props(styles.name)}>
        <span {...stylex.props(styles.nameHead)}>{split ? name.slice(0, -nameTail) : name}</span>
        {split ? <span {...stylex.props(styles.nameTail)}>{name.slice(-nameTail)}</span> : null}
      </span>
      {problem !== undefined ? <span role="alert" {...stylex.props(styles.problem, styles.line)}>{problem}</span>
        : uploading ? <>
          <span role={announce ? 'status' : undefined} {...stylex.props(styles.detail)}>Uploading…</span>
          <span aria-hidden="true" {...stylex.props(styles.activity)}><span {...stylex.props(styles.activitySegment)} /></span>
        </> : null}
    </span>
  </>
}

const flash = stylex.keyframes({ from: { backgroundColor: tone.dangerMuted }, to: { backgroundColor: surface.transparent } })
const sweep = stylex.keyframes({ from: { transform: 'translateX(-100%)' }, to: { transform: 'translateX(400%)' } })

/**
 * The composer's secondary controls (Attach, Remove, inline recovery): one ghost treatment in both themes, with a
 * hover/pressed wash. `outline` gives a text action (Remove image, Cancel upload) the kit's outlined control fill so
 * it reads as a button beside copy. `touchHit` extends the hit area to the 44px touch target around the visible 28px
 * control.
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
  outline: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: { default: surface.controlFill, ':hover': interaction.wash, ':active': interaction.wash } },
  touchHit: { '::before': { content: '""', position: 'absolute', inset: `min(0px, calc((100% - ${g.touchTarget}) / 2))` } },
  // Keeps its place and focus but reads as unavailable; activation is guarded by the caller.
  blocked: { color: ink.fgMuted, cursor: 'not-allowed', backgroundColor: { default: surface.transparent, ':hover': surface.transparent } },
})

const styles = stylex.create({
  // Tiles align with the field's text inset; block padding keeps focus rings clear of the scroll clip. From 600px
  // tiles wrap so every one stays in view; narrower, one row scrolls and a cut-off tile shows there is more.
  tray: {
    display: 'flex', flexWrap: { default: 'nowrap', '@media (min-width: 600px)': 'wrap' }, gap: s.md, minWidth: 0,
    overflowX: { default: 'auto', '@media (min-width: 600px)': 'visible' }, overflowY: { default: 'hidden', '@media (min-width: 600px)': 'visible' },
    paddingInline: s.xs, paddingBlock: s.xs2,
    scrollSnapType: { default: 'x mandatory', '@media (min-width: 600px)': 'none' }, scrollPaddingInline: s.xs, overscrollBehaviorX: 'contain', scrollbarWidth: 'thin', scrollbarColor: `${tone.scrollThumb} transparent`,
  },
  trayAction: { alignSelf: 'center' },
  notice: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, paddingInline: s.xs },
  // Out of flow while empty, so the pill keeps its row; still in the accessibility tree as a live region.
  noticeIdle: { position: 'absolute', width: '1px', height: '1px', overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  line: { margin: 0, textWrap: 'pretty' },
  refusal: { display: 'flex', alignItems: 'center', flexWrap: 'wrap', columnGap: s.md, rowGap: s.xs, minWidth: 0, paddingInline: s.xs },
  refusalText: { flex: '1 1 16ch', minWidth: 0 },
  flash: { borderRadius: r.keyboard, marginInline: `calc(${s.xs} * -1)`, paddingInline: s.xs, animationName: flash, animationDuration: m.reveal, animationTimingFunction: 'ease-out' },
  // Sized to its name up to `attachmentMax`. In the scrolling row a tile never shrinks, and the 75% cap keeps the next one peeking.
  attachment: {
    display: 'flex', alignItems: 'center', gap: s.md, boxSizing: 'border-box', minWidth: 0,
    flex: { default: '0 0 auto', '@media (min-width: 600px)': '0 1 auto' },
    maxWidth: { default: `min(${g.attachmentMax}, 75%)`, '@media (min-width: 600px)': `min(${g.attachmentMax}, 100%)` },
    padding: s.md, borderRadius: r.sm, backgroundColor: surface.rowActive, scrollSnapAlign: 'start',
  },
  failed: { outlineWidth: g.hairline, outlineStyle: 'solid', outlineColor: tone.dangerFg, outlineOffset: `calc(${g.hairline} * -1)` },
  // Inner radius = tile radius − padding, floored at 4px; the inset hairline separates white or black edges from the tile.
  image: { width: g.band, height: g.band, flexShrink: 0, objectFit: 'cover', borderRadius: r.keyboard, outlineWidth: g.hairline, outlineStyle: 'solid', outlineColor: border.rail, outlineOffset: `calc(${g.hairline} * -1)` },
  text: { display: 'flex', flexDirection: 'column', flex: '1 1 auto', minWidth: 0, marginInlineEnd: s.xs },
  // Head and tail split a long name so the ellipsis falls in its middle; `pre` keeps a space at the split. The head
  // gives way first; on a chip too narrow for the tail alone (wide glyphs on a phone), the tail is capped to the name
  // and ends in its own ellipsis rather than painting past the text into the trailing control.
  name: { display: 'flex', minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  nameHead: { flex: '0 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'pre' },
  nameTail: { flex: '0 0 auto', minWidth: 0, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'pre' },
  detail: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  problem: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: tone.dangerFg },
  // Indeterminate: the upload reports no byte progress. A still segment under reduced motion.
  activity: { display: 'block', height: g.progressSm, marginBlockStart: s.xs, overflow: 'hidden', borderRadius: r.full, backgroundColor: interaction.wash },
  activitySegment: {
    display: 'block', width: '25%', height: '100%', borderRadius: r.full, backgroundColor: ink.fgMuted,
    animationName: sweep, animationDuration: m.pulse, animationTimingFunction: m.ease, animationIterationCount: 'infinite',
    '@media (prefers-reduced-motion: reduce)': { animationName: 'none', transform: 'translateX(150%)' },
  },
})
