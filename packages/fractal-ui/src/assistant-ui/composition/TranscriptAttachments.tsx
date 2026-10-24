import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Dialog, Modal, ModalOverlay } from 'react-aria-components'
import type { Attachment } from '../embrace-data/model'
import { MarkdownImagePolicy, type MarkdownImageResolution } from './Markdown'
import { NarrowLayoutContext } from './NarrowLayout'
import { Icon } from './Icons'
import { ThemePortal } from '../taste/ThemePortal'
import { borderVars as border, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'

const deferred: MarkdownImageResolution = { _tag: 'Defer' }
const typeLabels: Readonly<Record<string, string>> = { 'image/png': 'PNG', 'image/jpeg': 'JPEG', 'image/gif': 'GIF', 'image/webp': 'WebP' }

/**
 * A sent message's attachments. The kit never fetches one. By default each is a quiet file card named
 * `Image: <name>, <type>`; attachments carry no byte size, so the card says so rather than estimating one.
 * A host that can show the bytes resolves the attachment id through the transcript's `resolveImage`: the image
 * then fits within 320×240 (in the narrow layout, the full column up to 240 tall) with its aspect ratio kept,
 * and opens full size in a dialog. Attachments carry no dimensions, so no space is reserved before it loads.
 */
export function TranscriptAttachments({ attachments }: { readonly attachments: readonly Attachment[] }) {
  if (attachments.length === 0) return null
  return <div data-testid="transcript-attachments" {...stylex.props(styles.list)}>
    {attachments.map((attachment, index) => <TranscriptAttachment key={`${attachment.id}:${index}`} attachment={attachment} />)}
  </div>
}

function TranscriptAttachment({ attachment }: { readonly attachment: Attachment }) {
  const { resolveImage } = React.useContext(MarkdownImagePolicy)
  const narrow = React.useContext(NarrowLayoutContext)
  const [open, setOpen] = React.useState(false)
  const image = attachment.mediaType.startsWith('image/')
  const name = attachment.name ?? (image ? 'Unnamed image' : 'Unnamed file')
  const type = typeLabels[attachment.mediaType] ?? attachment.mediaType
  const resolution = image ? resolveImage?.(attachment.id) ?? deferred : deferred
  if (resolution._tag === 'Defer') return <span role="group" aria-label={`${image ? 'Image' : 'File'}: ${name}, ${type}`} data-testid="transcript-attachment" data-attachment-state="deferred" {...stylex.props(styles.card)}>
    <Icon name="attach" />
    <span {...stylex.props(styles.cardName)}>{name}</span>
    <span {...stylex.props(styles.cardDetail)}>{type} · Size unavailable</span>
  </span>
  return <>
    <Button aria-label={`Open ${name}`} data-testid="transcript-attachment-open" onPress={() => setOpen(true)} {...stylex.props(styles.open, narrow && styles.openNarrow)}>
      <img src={resolution.src} alt={name} referrerPolicy="no-referrer" {...stylex.props(styles.image, styles.thumbnail)} />
    </Button>
    {/* Portal inside the themed surface so the viewer keeps the host's light or dark tokens. */}
    <ThemePortal><ModalOverlay isOpen={open} onOpenChange={setOpen} isDismissable {...stylex.props(styles.overlay)}>
      <Modal {...stylex.props(styles.modal)}>
        <Dialog aria-label={name} data-testid="transcript-image-viewer" {...stylex.props(styles.dialog)}>
          <div {...stylex.props(styles.viewerBar)}>
            <span {...stylex.props(styles.cardName)}>{name}</span>
            <Button aria-label="Close image" onPress={() => setOpen(false)} {...stylex.props(styles.close, narrow && styles.closeNarrow)}><Icon name="x" /></Button>
          </div>
          <img src={resolution.src} alt={name} referrerPolicy="no-referrer" {...stylex.props(styles.image, styles.full)} />
        </Dialog>
      </Modal>
    </ModalOverlay></ThemePortal>
  </>
}

const styles = stylex.create({
  list: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.sm, minWidth: 0, marginTop: s.xs },
  card: { display: 'inline-flex', alignItems: 'center', gap: s.sm, minWidth: 0, maxWidth: '100%', minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fgMuted, fontSize: t.metaSize, lineHeight: t.metaLeading },
  cardName: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fg, fontSize: t.metaSize },
  cardDetail: { flexShrink: 0 },
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
