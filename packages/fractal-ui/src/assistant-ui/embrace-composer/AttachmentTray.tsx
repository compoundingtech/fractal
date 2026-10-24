import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { AttachmentPrimitive, ComposerPrimitive, useAuiState } from '@assistant-ui/react'
import { Button } from 'react-aria-components'
import { interactionVars as interaction, borderVars as border, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../composition-tokens.stylex'
import { Icon } from '../composition/Icons'

/**
 * The composer's pending attachments, each removable before send. A host limit (`max`) is named in the tray
 * instead of refusing the send later; renders nothing while the draft has no attachment.
 */
export function ComposerAttachmentTray({ max }: { readonly max?: number }) {
  const count = useAuiState(state => state.composer.attachments.length)
  if (count === 0) return null
  const excess = max === undefined ? 0 : count - max
  return <div data-testid="composer-attachments" {...stylex.props(styles.tray)}>
    <div {...stylex.props(styles.list)}><ComposerPrimitive.Attachments>{() => <AttachmentPreview />}</ComposerPrimitive.Attachments></div>
    {excess > 0 && <div role="alert" {...stylex.props(styles.problem)}>{`A message can carry at most ${max} ${max === 1 ? 'image' : 'images'}. Remove ${excess} to send.`}</div>}
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
  attachment: { display: 'flex', alignItems: 'center', gap: s.md, padding: s.md, borderRadius: r.sm, backgroundColor: surface.rowActive, minWidth: 0 },
  failed: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: tone.dangerFg },
  image: { width: g.band, height: g.band, objectFit: 'cover', borderRadius: r.sm },
  text: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  name: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, color: ink.fg },
  detail: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  problem: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: tone.dangerFg },
  button: { display: 'inline-flex', justifyContent: 'center', alignItems: 'center', flexShrink: 0, minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.control, backgroundColor: surface.controlFill, color: ink.fg, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
})
