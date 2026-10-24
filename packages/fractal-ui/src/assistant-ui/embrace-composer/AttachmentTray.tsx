import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { AttachmentPrimitive, ComposerPrimitive, useAui, useAuiEvent, useAuiState } from '@assistant-ui/react'
import { Button } from 'react-aria-components'
import { interactionVars as interaction, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'

/**
 * The composer's pending attachments, each removable before send. A host limit (`max`) is named in the tray
 * instead of refusing the send later, and so is the host's `disabledReason` while images cannot be uploaded:
 * the staged images stay listed and removable so the draft can still go out as text. Renders nothing while the
 * draft has no attachment.
 */
export function ComposerAttachmentTray({ max, disabledReason }: { readonly max?: number; readonly disabledReason?: string }) {
  const count = useAuiState(state => state.composer.attachments.length)
  if (count === 0) return null
  const excess = max === undefined ? 0 : count - max
  return <div data-testid="composer-attachments" {...stylex.props(styles.tray)}>
    <div {...stylex.props(styles.list)}><ComposerPrimitive.Attachments>{() => <AttachmentPreview />}</ComposerPrimitive.Attachments></div>
    {disabledReason !== undefined && <div role="alert" {...stylex.props(styles.problem)}>{disabledReason}</div>}
    {excess > 0 && <div role="alert" {...stylex.props(styles.problem)}>{`A message can carry at most ${max} ${max === 1 ? 'image' : 'images'}. Remove ${excess} to send.`}</div>}
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
 * Refused picker and paste adds for the thread composer, from the public `composer.attachmentAddError` event.
 * Refusals of one pick or paste are listed together; the next accepted add, a removal or a send clears them.
 */
export function useAttachmentRefusals(enabled: boolean): readonly AttachmentRefusal[] {
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
  return enabled ? refusals : []
}

/**
 * Announces refused adds even while nothing is staged. The live region stays mounted (visually hidden while
 * empty) so the first refusal is announced; each refusal is a new line, so a repeated refusal is announced again.
 */
export function ComposerAttachmentNotice({ refusals }: { readonly refusals: readonly AttachmentRefusal[] }) {
  return <div aria-live="polite" data-testid="composer-attachment-notice" {...stylex.props(styles.notice, refusals.length === 0 && styles.noticeIdle)}>
    {refusals.map(refusal => <p key={refusal.id} {...stylex.props(styles.problem, styles.noticeLine)}>{refusal.message}</p>)}
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
      <span {...stylex.props(styles.name)}><AttachmentPrimitive.Name /></span>
      {problem !== undefined ? <span role="alert" {...stylex.props(styles.problem)}>{problem}</span> : uploading ? <span role="status" {...stylex.props(styles.detail)}>Uploading…</span> : null}
    </span>
    <AttachmentPrimitive.Remove asChild><Button aria-label={`Remove ${attachment.name}`} {...stylex.props(styles.button)}><Icon name="x" /></Button></AttachmentPrimitive.Remove>
  </AttachmentPrimitive.Root>
}

const styles = stylex.create({
  tray: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  list: { display: 'flex', gap: s.md, flexWrap: 'wrap', minWidth: 0 },
  notice: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0 },
  // Out of flow while empty, so the pill keeps its row; still in the accessibility tree as a live region.
  noticeIdle: { position: 'absolute', width: '1px', height: '1px', overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap' },
  noticeLine: { margin: 0 },
  attachment: { display: 'flex', alignItems: 'center', gap: s.md, padding: s.md, borderRadius: r.sm, backgroundColor: surface.rowActive, minWidth: 0 },
  failed: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: tone.dangerFg },
  image: { width: g.band, height: g.band, objectFit: 'cover', borderRadius: r.sm },
  text: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  name: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, color: ink.fg },
  detail: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  problem: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: tone.dangerFg },
  button: { display: 'inline-flex', justifyContent: 'center', alignItems: 'center', flexShrink: 0, minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.controlFill, color: ink.fg, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
})
