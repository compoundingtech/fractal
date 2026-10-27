import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, Modal, ModalOverlay } from 'react-aria-components'
import type { Attachment } from '../embrace-data/model'
import { NarrowLayoutContext } from './NarrowLayout'
import { Icon } from './Icons'
import { ThemePortal } from '../taste/ThemePortal'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, statusVars as status, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

/**
 * The host's read state for one attachment id. Only `Loaded` carries bytes the kit may show. `Unavailable` means the
 * host can never read this id or type, so the card stays a plain file card with nothing to press.
 */
export type TranscriptAttachmentImageState =
  | { readonly _tag: 'Deferred' }
  | { readonly _tag: 'Unavailable' }
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Unauthorized' }
  | { readonly _tag: 'Failed' }
  | { readonly _tag: 'Loaded'; readonly src: string }

/**
 * Host-owned attachment image reads. `state` is called during render with the attachment's id and media type and must
 * be synchronous; `load` is called only from a person's Show image or Retry press, with the message that carries the
 * attachment. The host owns the cache and the lifetime of any URL it returns.
 */
export interface TranscriptAttachmentImages {
  readonly state: (attachmentId: string, mediaType?: string) => TranscriptAttachmentImageState
  readonly load: (attachment: Attachment, messageId: string) => void
}

/** Supplied by `Transcript`; absent means no host can read attachment bytes, so every image stays a deferred card. */
export const TranscriptAttachmentImagesContext = React.createContext<TranscriptAttachmentImages | undefined>(undefined)

const deferred: TranscriptAttachmentImageState = { _tag: 'Deferred' }
const typeLabels: Readonly<Record<string, string>> = { 'image/png': 'PNG', 'image/jpeg': 'JPEG', 'image/gif': 'GIF', 'image/webp': 'WebP' }
const sizeFormatter = new Intl.NumberFormat([], { maximumFractionDigits: 1 })
const formatSize = (bytes: number | undefined) => {
  if (bytes === undefined || !Number.isFinite(bytes) || bytes < 0) return 'Size unavailable'
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB'] as const
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1 }
  return `${sizeFormatter.format(value)} ${units[unit]}`
}

/**
 * A sent or received message's attachments (user prompts, assistant answers and incoming mail bodies alike). The kit
 * never fetches one. Each is a quiet file card named `Image: <name>, <type>`
 * showing its kind and byte size (`Size unavailable` when the message carries none). With a host's
 * `attachmentImages`, an image card offers **Show image**, which asks the host to read it for `messageId` (the
 * attachment's own carrier message when it has one); the card then reads **Loading image**, **Not authorized**
 * (no retry) or **Couldn't load image** with **Retry**. An `Unavailable` image stays a plain card with no action.
 * A loaded image fits within 320×240 (in the narrow layout, the full column up to 240 tall) with its aspect ratio
 * kept, and opens full size in a dialog. Attachments carry no dimensions, so no space is reserved before it loads.
 * Markdown images are unaffected: they keep `resolveImage`.
 *
 * Focus never falls to the page. Show image stays mounted as a focusable, inert **Loading image** button while the
 * read runs (repeat presses are ignored) and becomes Retry in place, so a failed retry keeps focus on Retry. When the
 * action leaves (`Unauthorized`) focus moves to the card; when the card leaves (`Loaded`) it moves to the image's
 * Open button. Either move happens only if focus was still inside the card, so a person who moved on keeps their
 * place. Once a person asks, a status outside the card announces each change through the loaded image.
 */
export function TranscriptAttachments({ attachments, messageId }: { readonly attachments: readonly Attachment[]; readonly messageId: string }) {
  if (attachments.length === 0) return null
  return <div data-testid="transcript-attachments" {...stylex.props(styles.list)}>
    {attachments.map((attachment, index) => <TranscriptAttachment key={`${attachment.id}:${index}`} attachment={attachment} messageId={attachment.messageId ?? messageId} />)}
  </div>
}

function TranscriptAttachment({ attachment, messageId }: { readonly attachment: Attachment; readonly messageId: string }) {
  const images = React.useContext(TranscriptAttachmentImagesContext)
  const narrow = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(false)
  // Announcements start with the person's own press, so remounted or cached cards stay quiet.
  const [requested, setRequested] = React.useState(false)
  const card = React.useRef<HTMLSpanElement | null>(null)
  const openButton = React.useRef<HTMLButtonElement>(null)
  // Set while a focused card or action is detached; the next layout effect hands focus to what replaced it.
  const focusLost = React.useRef(false)
  const trackFocus = React.useCallback((element: HTMLElement | null) => {
    if (!element) return
    return () => { if (element.contains(element.ownerDocument.activeElement)) focusLost.current = true }
  }, [])
  const cardRef = React.useCallback((element: HTMLSpanElement | null) => {
    card.current = element
    const release = trackFocus(element)
    return () => { release?.(); card.current = null }
  }, [trackFocus])
  React.useLayoutEffect(() => {
    if (!focusLost.current) return
    focusLost.current = false
    const target = openButton.current ?? card.current
    target?.focus({ preventScroll: true })
  })
  const image = attachment.mediaType.startsWith('image/')
  const name = attachment.name ?? (image ? 'Unnamed image' : 'Unnamed file')
  const type = typeLabels[attachment.mediaType] ?? attachment.mediaType
  const host = image ? images : undefined
  const state = host?.state(attachment.id, attachment.mediaType) ?? deferred
  const readable = host !== undefined && state._tag !== 'Unavailable'
  const announcement = !requested ? '' : state._tag === 'Loading' ? `Loading image ${name}` : state._tag === 'Unauthorized' ? `Not authorized to show ${name}` : state._tag === 'Failed' ? `Couldn't load ${name}` : state._tag === 'Loaded' ? `Loaded image ${name}` : ''
  const action = !readable ? undefined : state._tag === 'Deferred' ? { label: 'Show image', name: `Show image ${name}`, testId: 'transcript-attachment-show' } : state._tag === 'Failed' ? { label: 'Retry', name: `Retry loading ${name}`, testId: 'transcript-attachment-retry' } : state._tag === 'Loading' ? { label: 'Loading image', name: `Loading image ${name}`, testId: 'transcript-attachment-loading' } : undefined
  const pending = state._tag === 'Loading'
  const load = () => {
    if (host === undefined || pending) return
    setRequested(true)
    host.load(attachment, messageId)
  }
  return <>
    {/* Outside the card, so the result is still announced after the loaded image replaces it. */}
    {readable && <span role="status" aria-live="polite" data-testid="transcript-attachment-status" {...stylex.props(styles.visuallyHidden)}>{announcement}</span>}
    {state._tag !== 'Loaded'
      ? <span ref={cardRef} role="group" aria-label={`${image ? 'Image' : 'File'}: ${name}, ${type}`} aria-busy={pending || undefined} tabIndex={readable ? -1 : undefined} data-testid="transcript-attachment" data-attachment-state={state._tag === 'Unavailable' ? 'deferred' : state._tag.toLowerCase()} {...stylex.props(styles.card, narrow && styles.cardNarrow)}>
        <span {...stylex.props(styles.cardIdentity)}>
          <Icon name={image ? 'image' : 'attach'} />
          <span {...stylex.props(styles.cardName)}>{name}</span>
          <span {...stylex.props(styles.cardDetail)}>{`· ${image && type !== attachment.mediaType ? `${type} image` : type} · ${formatSize(attachment.size)}`}</span>
        </span>
        {readable && <span {...stylex.props(styles.cardStatus)}>
          {(state._tag === 'Unauthorized' || state._tag === 'Failed') && <span {...stylex.props(state._tag === 'Failed' && styles.statusFailed)}>{state._tag === 'Unauthorized' ? 'Not authorized' : "Couldn't load image"}</span>}
          {/* One native button from Show image through Loading to Retry keeps focus in place; inert while pending, like Stop. */}
          {action !== undefined && <button type="button" ref={trackFocus} aria-label={action.name} aria-disabled={pending || undefined} data-testid={action.testId} onClick={load} {...stylex.props(styles.action, narrow && styles.actionNarrow, pending && styles.actionPending)}>{action.label}</button>}
        </span>}
      </span>
      : <>
        <Button ref={openButton} aria-label={`Open ${name}`} data-testid="transcript-attachment-open" data-attachment-state="loaded" onPress={() => setOpen(true)} {...stylex.props(styles.open, narrow && styles.openNarrow)}>
          <img src={state.src} alt={name} referrerPolicy="no-referrer" {...stylex.props(styles.image, styles.thumbnail)} />
        </Button>
        {/* Portal inside the themed surface so the viewer keeps the host's light or dark tokens. */}
        <ThemePortal><ModalOverlay isOpen={open} onOpenChange={setOpen} isDismissable {...stylex.props(styles.overlay)}>
          <Modal {...stylex.props(styles.modal)}>
            <Dialog aria-label={name} data-testid="transcript-image-viewer" {...stylex.props(styles.dialog)}>
              <div {...stylex.props(styles.viewerBar)}>
                <span {...stylex.props(styles.cardName)}>{name}</span>
                <Button aria-label="Close image" onPress={() => setOpen(false)} {...stylex.props(styles.close, narrow && styles.closeNarrow)}><Icon name="x" /></Button>
              </div>
              <img src={state.src} alt={name} referrerPolicy="no-referrer" {...stylex.props(styles.image, styles.full)} />
            </Dialog>
          </Modal>
        </ModalOverlay></ThemePortal>
      </>}
  </>
}

const styles = stylex.create({
  list: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.sm, minWidth: 0, marginTop: s.xs },
  // Focusable by script only: it holds focus while the read leaves no action to press.
  card: { display: 'inline-flex', alignItems: 'center', columnGap: s.md, minWidth: 0, maxWidth: '100%', minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading, outlineStyle: 'none', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  // Phones: the read status and its action wrap below the name instead of squeezing it.
  cardNarrow: { flexWrap: 'wrap', paddingBlock: s.xs },
  cardIdentity: { display: 'inline-flex', alignItems: 'center', gap: s.sm, minWidth: 0 },
  cardName: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fg, fontSize: t.metaSize },
  cardDetail: { flexShrink: 0, fontVariantNumeric: 'tabular-nums' },
  cardStatus: { display: 'inline-flex', alignItems: 'center', gap: s.sm, flexShrink: 0, whiteSpace: 'nowrap' },
  statusFailed: { color: status.dangerFg },
  action: { display: 'inline-flex', alignItems: 'center', minHeight: g.controlSm, paddingBlock: 0, paddingInline: s.xs, borderWidth: 0, borderRadius: r.control, backgroundColor: surface.transparent, color: status.runningFg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  actionNarrow: { minHeight: g.touchTarget },
  // Reads as the quiet status it replaces; still focusable, so focus stays put while the read runs.
  actionPending: { color: ink.fgMuted, cursor: 'default', ':hover': { backgroundColor: surface.transparent } },
  visuallyHidden: { position: 'absolute', width: 1, height: 1, padding: 0, margin: -1, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
  // The button carries the width cap so it shrinks to the fitted image and its focus ring hugs it.
  open: { display: 'block', width: 'fit-content', maxWidth: `min(100%, ${g.imageMaxWidth})`, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, cursor: 'zoom-in', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  openNarrow: { maxWidth: '100%' },
  // Inset hairline separates images whose edges match the canvas, in either theme.
  image: { display: 'block', borderRadius: r.sm, outlineWidth: g.hairline, outlineStyle: 'solid', outlineColor: `color-mix(in srgb, ${ink.fg} 10%, transparent)`, outlineOffset: `calc(-1 * ${g.hairline})` },
  thumbnail: { width: 'auto', height: 'auto', maxWidth: '100%', maxHeight: g.imageMaxHeight },
  overlay: { position: 'fixed', inset: 0, zIndex: 30, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: s.xl, backgroundColor: surface.scrim },
  modal: { display: 'flex', maxWidth: '100%', maxHeight: '100%', minWidth: 0, backgroundColor: surface.canvas, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, color: ink.fg },
  dialog: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, padding: s.md, outlineStyle: 'none' },
  viewerBar: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.md, minWidth: 0 },
  close: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0, width: g.controlMd, height: g.controlMd, padding: 0, borderWidth: 0, borderRadius: r.control, backgroundColor: surface.transparent, color: ink.fgMuted, cursor: 'pointer', ':hover': { color: ink.fg, backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  closeNarrow: { width: g.touchTarget, height: g.touchTarget },
  full: { maxWidth: `calc(100vw - 2 * ${s.xl} - 2 * ${s.md} - 2 * ${g.hairline})`, maxHeight: `calc(100dvh - 2 * ${s.xl} - 2 * ${s.md} - 2 * ${g.hairline} - ${g.controlMd} - ${s.sm})`, objectFit: 'contain' },
})
