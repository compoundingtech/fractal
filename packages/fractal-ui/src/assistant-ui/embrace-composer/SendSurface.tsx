import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { useAui, useAuiState } from '@assistant-ui/react'
import { Button, FileTrigger, Focusable, Input, Menu, MenuItem, MenuTrigger, Popover, TextField, Tooltip, TooltipTrigger } from 'react-aria-components'
import { Icon } from '../composition/Icons'
import { NarrowLayoutContext } from '../composition/NarrowLayout'
import { surfaceVars as surface, textVars as text, borderVars as border, accentVars as accent, interactionVars as interaction, typeVars as t, radiusVars as r, spaceVars as s, geometryVars as g } from '../composition-tokens.stylex'
import {
  describeImageCount, describeImageTypes, formatImageLimit, formatImageSize, screenComposerImages,
  type ComposerImagePolicy,
  type ComposerAvailability, type ComposerImage, type ComposerRecipientReadout, type ComposerReply,
} from './send'

/** Picker and paste share one intake; both reach the host runtime only through `addAttachment`. */
export interface ComposerImageIntake {
  readonly available: boolean
  readonly disabled: boolean
  readonly policy: ComposerImagePolicy
  readonly refused?: string
  readonly add: (files: readonly File[]) => void
}
export const ComposerImageIntakeContext = React.createContext<ComposerImageIntake | undefined>(undefined)

export function useComposerImageIntake(retainedImages: number, disabled: boolean, policy: ComposerImagePolicy): ComposerImageIntake {
  const aui = useAui()
  const available = useAuiState(state => state.thread.capabilities.attachments)
  const attached = useAuiState(state => state.composer.attachments.length)
  const sent = useAuiState(state => state.thread.messages.length)
  const retained = Math.max(retainedImages, attached)
  // A refusal speaks for the tray it was made against. Once that tray is in place, removing an image or
  // sending the draft retires it for good; a later restore of the same images does not bring it back.
  const [refusal, setRefusal] = React.useState<{ readonly text: string; readonly retained: number; readonly sent: number; readonly reached: boolean }>()
  React.useEffect(() => {
    if (refusal === undefined) return
    if (sent !== refusal.sent || (refusal.reached && retained < refusal.retained)) setRefusal(undefined)
    else if (!refusal.reached && retained >= refusal.retained) setRefusal({ ...refusal, reached: true })
  }, [refusal, retained, sent])
  const add = React.useCallback((files: readonly File[]) => {
    if (!available || disabled || files.length === 0) return
    const screened = screenComposerImages(files, retained, policy)
    const after = retained + screened.accepted.length
    setRefusal(screened.refused === undefined ? undefined : { text: screened.refused, retained: after, sent, reached: false })
    for (const file of screened.accepted) {
      aui.composer.addAttachment(file).catch((error: unknown) => {
        const reason = error instanceof Error && error.message !== '' ? error.message : 'It could not be added.'
        setRefusal(previous => ({ text: [previous?.text, `${file.name}: ${reason}`].filter(Boolean).join(' '), retained: Math.min(previous?.retained ?? after, after - 1), sent, reached: false }))
      })
    }
  }, [aui, available, disabled, retained, sent, policy])
  const refused = refusal !== undefined && sent === refusal.sent && (refusal.reached ? retained >= refusal.retained : true) ? refusal.text : undefined
  return React.useMemo(() => ({ available, disabled, policy, refused, add }), [available, disabled, policy, refused, add])
}

export function ComposerAttachImage({ compact = false }: { readonly compact?: boolean }) {
  const intake = React.useContext(ComposerImageIntakeContext)
  const narrow = React.useContext(NarrowLayoutContext)
  if (intake === undefined || !intake.available) return null
  const { policy } = intake
  return <TooltipTrigger delay={150} closeDelay={0}>
    <FileTrigger acceptedFileTypes={[...policy.mediaTypes]} allowsMultiple={policy.maxCount > 1} onSelect={list => { if (list !== null) intake.add([...list]) }}>
      <Button aria-label="Attach image" isDisabled={intake.disabled} {...stylex.props(styles.iconButton, compact && styles.iconCompact, narrow && styles.touchControl)}><Icon name="attach" /></Button>
    </FileTrigger>
    <Tooltip {...stylex.props(styles.tooltip)}>Attach up to {describeImageCount(policy.maxCount)} ({describeImageTypes(policy)}), {formatImageLimit(policy.maxBytes)} each</Tooltip>
  </TooltipTrigger>
}

export const sessionChangeNote = (recipient: ComposerRecipientReadout) =>
  `Session changed. This message continues ${recipient.sessionId === undefined ? 'the current session' : `session ${recipient.sessionId}`}.`

const availabilityAnnouncement = (availability: ComposerAvailability | undefined) =>
  availability === undefined || availability._tag === 'Ready' ? '' : `${availabilityCopy[availability._tag].title} ${availability.detail}`

/** Names every image that is not ready; progress is left out, so the text changes only when a state does. */
const imageAnnouncement = (images: readonly ComposerImage[] | undefined) => {
  if (images === undefined || images.length === 0) return ''
  const pending = images.flatMap(image => image.state._tag === 'Uploading' ? [`${image.name}: uploading.`] : image.state._tag === 'Failed' ? [`${image.name}: upload failed, ${image.state.detail}`] : [])
  return pending.length > 0 ? pending.join(' ') : `${describeImageCount(images.length)} ready to send.`
}

/**
 * Always-mounted polite regions. Visible notices mount already filled, which screen readers may not
 * announce; these change text only when the session, availability or an image state changes.
 */
export function ComposerLiveStatus({ recipient, availability, images }: { readonly recipient?: ComposerRecipientReadout; readonly availability?: ComposerAvailability; readonly images?: readonly ComposerImage[] }) {
  return <>
    <span role="status" data-testid="composer-session-announcement" {...stylex.props(styles.visuallyHidden)}>{recipient?.sessionChanged === true ? sessionChangeNote(recipient) : ''}</span>
    <span role="status" data-testid="composer-availability-announcement" {...stylex.props(styles.visuallyHidden)}>{availabilityAnnouncement(availability)}</span>
    <span role="status" data-testid="composer-image-announcement" {...stylex.props(styles.visuallyHidden)}>{imageAnnouncement(images)}</span>
  </>
}

/** `To <label>`; the canonical ref and active session stay available to assistive technology. */
export function ComposerRecipientControl({ recipient, recipients, onRecipientChange, compact = false }: {
  readonly recipient: ComposerRecipientReadout
  readonly recipients?: readonly { readonly ref: string; readonly label: string }[]
  readonly onRecipientChange?: (ref: string) => void
  readonly compact?: boolean
}) {
  const detailId = React.useId()
  const others = recipients?.filter(choice => choice.ref !== recipient.ref) ?? []
  const picker = onRecipientChange !== undefined && recipients !== undefined && others.length > 0
  const detail = `${recipient.ref}${recipient.sessionId === undefined ? '' : ` · session ${recipient.sessionId}`}${onRecipientChange !== undefined && !picker ? ' · No other recipients available' : ''}`
  // The space keeps the visible text "To <label>" a prefix of the accessible name (label in name).
  const label = <><span aria-hidden="true" {...stylex.props(styles.to)}>To</span>{' '}<span data-composer-readout {...stylex.props(styles.recipientLabel)}>{recipient.label}</span></>
  return <>
    <span id={detailId} {...stylex.props(styles.visuallyHidden)}>Recipient {recipient.ref}{recipient.sessionId === undefined ? '' : `, session ${recipient.sessionId}`}.{onRecipientChange !== undefined && !picker ? ' No other recipients available.' : ''}</span>
    {picker ? <MenuTrigger>
      <TooltipTrigger delay={150} closeDelay={0}>
        <Button aria-label={`To ${recipient.label}, change recipient`} aria-describedby={detailId} data-composer-recipient={recipient.ref} {...stylex.props(styles.recipient, styles.recipientButton, compact && styles.recipientCompact)}>
          {label}<Icon name="chevron-down" size={12} />
        </Button>
        <Tooltip {...stylex.props(styles.tooltip)}>{detail}</Tooltip>
      </TooltipTrigger>
      <Popover placement="top start" {...stylex.props(styles.popup)}>
        <Menu aria-label="Recipient" selectionMode="single" disallowEmptySelection selectedKeys={[recipient.ref]}
          onSelectionChange={keys => { const key = keys === 'all' ? undefined : [...keys][0]; if (key !== undefined && String(key) !== recipient.ref) onRecipientChange(String(key)) }}>
          {recipients.map(choice => <MenuItem key={choice.ref} id={choice.ref} textValue={choice.label} {...stylex.props(styles.option)}>
            {({ isSelected }) => <><span {...stylex.props(styles.check)}>{isSelected ? <Icon name="check" size={12} /> : null}</span><span {...stylex.props(styles.optionLabel)}>{choice.label}</span></>}
          </MenuItem>)}
        </Menu>
      </Popover>
    </MenuTrigger> : <TooltipTrigger delay={150} closeDelay={0}>
      <Focusable><span tabIndex={0} role="group" aria-label={`To ${recipient.label}`} aria-describedby={detailId} data-composer-recipient={recipient.ref} {...stylex.props(styles.recipient, compact && styles.recipientCompact)}>{label}</span></Focusable>
      <Tooltip {...stylex.props(styles.tooltip)}>{detail}</Tooltip>
    </TooltipTrigger>}
  </>
}

const availabilityCopy = {
  Offline: { title: 'Offline — reconnect to send.', action: 'Reconnect' },
  MissingGrant: { title: 'Sending messages is not permitted here.', action: 'Check access' },
  Revoked: { title: 'This device is no longer authorized to send.', action: 'Set up again' },
  Unavailable: { title: "Messages can't be sent here.", action: undefined },
} as const

/** Exactly one recovery affordance; the draft stays editable and nothing is sent in the background. */
export function ComposerAvailabilityNotice({ availability }: { readonly availability: Exclude<ComposerAvailability, { readonly _tag: 'Ready' }> }) {
  const copy = availabilityCopy[availability._tag]
  const recover = availability._tag === 'Offline' ? availability.onReconnect : availability._tag === 'Unavailable' ? undefined : availability.onRecover
  return <div data-testid="composer-availability" data-availability={availability._tag} {...stylex.props(styles.notice)}>
    <div {...stylex.props(styles.noticeText)}>
      <span {...stylex.props(styles.noticeTitle)}>{copy.title}</span>
      <span {...stylex.props(styles.muted)}>{availability.detail} Your draft is kept.{availability._tag === 'Revoked' && availability.code !== undefined ? ` Code ${availability.code}.` : ''}</span>
    </div>
    {recover !== undefined && copy.action !== undefined ? <Button onPress={recover} {...stylex.props(styles.textButton)}>{copy.action}</Button> : null}
  </div>
}

export function ComposerReplyChip({ reply, onClear }: { readonly reply: ComposerReply; readonly onClear?: () => void }) {
  const idId = React.useId()
  const context = reply.subject ?? reply.excerpt
  return <div role="group" aria-label={`Replying to ${reply.sender}`} aria-describedby={idId} data-testid="composer-reply" data-reply-message={reply.messageId} {...stylex.props(styles.chip)}>
    <span {...stylex.props(styles.chipText)}>Replying to <span {...stylex.props(styles.strong)}>{reply.sender}</span>{context === undefined ? null : ` · ${context}`}</span>
    <span id={idId} {...stylex.props(styles.visuallyHidden)}>Message {reply.messageId}</span>
    {onClear !== undefined ? <Button aria-label={`Clear reply to ${reply.sender}`} onPress={onClear} {...stylex.props(styles.textButton)}>Clear reply</Button> : null}
  </div>
}

export function ComposerSubjectField({ subject, onSubjectChange, isReadOnly, onEnter }: { readonly subject: string; readonly onSubjectChange: (subject: string) => void; readonly isReadOnly: boolean; readonly onEnter: () => void }) {
  return <TextField aria-label="Subject" value={subject} onChange={onSubjectChange} isReadOnly={isReadOnly} {...stylex.props(styles.subjectField)}>
    <Input data-composer-subject placeholder="Subject (optional)"
      onKeyDown={event => { if (event.key === 'Enter' && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); onEnter() } }}
      {...stylex.props(styles.subject)} />
  </TextField>
}

const imageStateText = (image: ComposerImage) => image.state._tag === 'Ready' ? formatImageSize(image.size)
  : image.state._tag === 'Uploading' ? `Uploading${image.state.progress === undefined ? '' : ` ${Math.round(Math.min(1, Math.max(0, image.state.progress)) * 100)}%`}…`
  : `Upload failed: ${image.state.detail}`

export function ComposerImageTray({ images, maxCount, onRemoveImage, onRetryImage, disabled }: {
  readonly images: readonly ComposerImage[]
  readonly maxCount: number
  readonly onRemoveImage?: (id: string) => void
  readonly onRetryImage?: (id: string) => void
  readonly disabled: boolean
}) {
  if (images.length === 0) return null
  return <ul aria-label={`Images, ${images.length} of ${maxCount}`} data-testid="composer-images" {...stylex.props(styles.tray)}>
    {images.map(image => <li key={image.id} data-image-id={image.id} data-image-state={image.state._tag} {...stylex.props(styles.tile)}>
      <span aria-hidden="true" {...stylex.props(styles.thumb)}>{image.previewUrl === undefined ? image.mediaType.slice('image/'.length).toUpperCase() : <img src={image.previewUrl} alt="" {...stylex.props(styles.thumbImage)} />}</span>
      <span {...stylex.props(styles.tileBody)}>
        <span {...stylex.props(styles.tileName)}>{image.name}</span>
        <span {...stylex.props(styles.tileState)}>{imageStateText(image)}</span>
        <span {...stylex.props(styles.tileActions)}>
          {image.state._tag === 'Failed' && onRetryImage !== undefined ? <Button aria-label={`Retry upload of ${image.name}`} isDisabled={disabled} onPress={() => onRetryImage(image.id)} {...stylex.props(styles.textButton)}>Retry</Button> : null}
          {onRemoveImage !== undefined ? <Button aria-label={`Remove ${image.name}`} isDisabled={disabled} onPress={() => onRemoveImage(image.id)} {...stylex.props(styles.textButton)}>Remove</Button> : null}
        </span>
      </span>
    </li>)}
  </ul>
}

const styles = stylex.create({
  visuallyHidden: { position: 'absolute', width: g.hairline, height: g.hairline, padding: 0, margin: `calc(-1 * ${g.hairline})`, overflow: 'hidden', clipPath: 'inset(50%)', whiteSpace: 'nowrap', borderWidth: 0 },
  // Same box as the toolbar button this replaces, so footer density and pill/slab measurement are unchanged.
  iconButton: {
    minHeight: g.controlMd, flexShrink: 0, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.controlBorder,
    borderRadius: r.control, backgroundColor: surface.controlFill, color: text.fg,
    paddingBlock: s.xs, paddingInline: s.md, fontFamily: t.fontSans, fontSize: t.metaSize,
    cursor: { default: 'pointer', ':disabled': 'not-allowed' }, opacity: { default: 1, ':disabled': 0.64 },
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  iconCompact: { paddingInline: s.xs },
  // Narrow hosts: an icon-only control is never narrower than it is tall.
  touchControl: { minWidth: g.touchTarget },
  tooltip: { width: 'max-content', maxWidth: `min(${g.tooltipMax}, calc(100vw - ${s.lg}))`, boxSizing: 'border-box', padding: s.sm, borderRadius: r.sm, backgroundColor: surface.raised, color: text.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  recipient: {
    display: 'inline-flex', alignItems: 'center', gap: s.xs, minWidth: 0, maxWidth: g.crumbMax, flexShrink: 1, minHeight: g.controlMd,
    fontFamily: t.fontSans, fontSize: t.metaSize, color: text.fg, whiteSpace: 'nowrap', borderRadius: r.control,
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  recipientButton: { borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.controlBorder, backgroundColor: surface.controlFill, paddingInline: s.sm, cursor: 'pointer' },
  recipientCompact: { fontSize: t.denseSize },
  to: { color: text.fgSoft, flexShrink: 0 },
  recipientLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' },
  popup: { maxWidth: `min(${g.tooltipMax}, 90vw)`, maxHeight: g.commandMaxHeight, overflowY: 'auto', backgroundColor: surface.raised, color: text.fg, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.md, padding: s.xs, fontFamily: t.fontSans, fontSize: t.metaSize },
  option: { minHeight: g.controlLg, display: 'flex', alignItems: 'center', gap: s.xs, paddingInline: s.sm, borderRadius: r.sm, cursor: 'pointer', outlineStyle: 'none', ':focus': { backgroundColor: surface.rowActive } },
  check: { display: 'inline-flex', width: g.iconSm, flexShrink: 0 },
  optionLabel: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  notice: { display: 'flex', alignItems: 'center', gap: s.md, flexWrap: 'wrap', padding: s.sm, borderRadius: r.md, backgroundColor: surface.rowActive, fontFamily: t.fontSans },
  noticeText: { display: 'flex', flexDirection: 'column', gap: s.xs2, flex: '1 1 0', minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  noticeTitle: { color: text.fg, fontWeight: t.weightMedium },
  muted: { color: text.fgSoft },
  strong: { color: text.fg, fontWeight: t.weightMedium },
  textButton: {
    display: 'inline-flex', alignItems: 'center', flexShrink: 0, minHeight: g.controlSm, paddingInline: s.sm,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm,
    backgroundColor: surface.controlFill, color: text.fg, fontFamily: t.fontSans, fontSize: t.metaSize,
    cursor: { default: 'pointer', ':disabled': 'not-allowed' }, opacity: { default: 1, ':disabled': 0.64 },
    ':hover': { backgroundColor: surface.rowHover },
    ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  chip: { display: 'flex', alignItems: 'center', gap: s.sm, minWidth: 0, paddingBlock: s.xs, paddingInline: s.sm, borderRadius: r.md, backgroundColor: surface.rowActive, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  chipText: { flex: '1 1 0', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: text.fgSoft },
  subjectField: { display: 'flex', minWidth: 0 },
  subject: {
    width: '100%', minWidth: 0, boxSizing: 'border-box', minHeight: g.controlMd, paddingInline: s.xs,
    borderWidth: 0, borderBottomWidth: g.hairline, borderStyle: 'solid', borderColor: border.border, backgroundColor: surface.transparent,
    color: text.fg, fontFamily: t.fontSans, fontSize: t.uiSize, outlineStyle: 'none',
    '::placeholder': { color: text.fgMuted },
    ':focus-visible': { borderColor: accent.primary },
  },
  tray: { display: 'flex', flexWrap: 'wrap', gap: s.sm, margin: 0, padding: 0, listStyle: 'none', minWidth: 0 },
  tile: { display: 'flex', alignItems: 'flex-start', gap: s.sm, flex: `1 1 ${g.crumbMax}`, maxWidth: g.fileMax, minWidth: 0, padding: s.xs, borderRadius: r.md, backgroundColor: surface.rowActive },
  thumb: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.band, height: g.band, flexShrink: 0, overflow: 'hidden', borderRadius: r.sm, backgroundColor: surface.codeBg, color: text.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize },
  thumbImage: { width: '100%', height: '100%', objectFit: 'cover' },
  tileBody: { display: 'flex', flexDirection: 'column', gap: s.xs2, minWidth: 0, flex: '1 1 0' },
  tileName: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: text.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  tileState: { color: text.fgSoft, fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  tileActions: { display: 'flex', gap: s.xs, flexWrap: 'wrap' },
})
