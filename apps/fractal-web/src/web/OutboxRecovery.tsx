import * as React from 'react'
import { flushSync } from 'react-dom'
import * as stylex from '@stylexjs/stylex'
import * as Aria from 'react-aria-components'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'
import { accentVars as accent, borderVars as border, geometryVars, radiusVars as r, spaceVars, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource } from '../data/react.tsx'
import type { OutboxRecoveryCopy, OutboxRecoveryItem, OutboxRecoveryPort } from '../data/source.ts'
import { offeredActions, recoveryStoreFor, type RecoveryFailure, type RecoveryOperation, type RecoveryStore } from './outboxRecoveryState.ts'

export const RECOVERY_TITLE = 'Outbox on this device'
export const RECOVERY_UNAVAILABLE = 'Outbox on this device is unavailable. Its messages cannot be shown right now.'

/** The workspace's one recovery surface, above every pane, so thread-only and startup views reach it too. */
export function LiveOutboxRecovery() {
  const port = useDataSource().outboxRecovery
  return port === undefined ? null : <OutboxRecovery port={port} />
}

/** The exact unconfirmed-delivery sentence: an unknown original may have arrived and must never be sent again. */
export const unconfirmedHeadline = (subject: OutboxRecoveryItem['subject']) => `Fractal has not confirmed this ${subject}. Do not send it again.`

const headline = (item: OutboxRecoveryItem): string => {
  switch (item.kind) {
    case 'unknown': return unconfirmedHeadline(item.subject)
    case 'not-applied': return `Fractal did not apply this ${item.subject}.`
    case 'pending': return `This ${item.subject} has not been sent.`
    case 'corrupt': return 'This outbox entry is damaged and cannot be sent.'
    case 'identity-mismatch': return 'This entry belongs to a different Fractal identity and cannot be sent from here.'
  }
}

const discardConsequence = (item: OutboxRecoveryItem): string => {
  switch (item.kind) {
    case 'unknown': return 'This removes the copy on this device only. It does not cancel the original, which may already have arrived.'
    case 'pending':
    case 'not-applied': return `This removes the ${item.subject} from this device. It will not be sent.`
    case 'corrupt':
    case 'identity-mismatch': return 'This removes the entry from this device. It does not cancel anything Fractal may already have received.'
  }
}

const actionLabel: Readonly<Record<RecoveryOperation, { readonly idle: string; readonly busy: string; readonly failed: string; readonly stale: string }>> = {
  copy: { idle: 'Copy', busy: 'Copying…', failed: 'Copy did not open: this message could not be read from the outbox.', stale: 'Copy did not open: this entry changed in another tab or window. It shows its current state.' },
  discard: { idle: 'Discard', busy: 'Discarding…', failed: 'Discard did not finish: outbox unavailable; retry later.', stale: 'Not discarded: this entry changed in another tab or window. It shows its current state.' },
  retry: { idle: 'Retry', busy: 'Retrying…', failed: 'Retry did not finish. This entry shows its current state.', stale: 'Not retried: this entry changed in another tab or window. It shows its current state.' },
  send: { idle: 'Revalidate and send', busy: 'Revalidating…', failed: 'Not sent: the recipient or this browser’s access did not revalidate, or the outbox is unavailable.', stale: 'Not sent: this entry changed in another tab or window. It shows its current state.' },
}

/** A Discard confirmation pins the attempt state the person read; `changed` once that state moved on before Discard ran. */
type Confirming = { readonly observed: OutboxRecoveryItem; readonly changed: boolean; readonly discarding: boolean }

/**
 * Messages and threads this browser's outbox holds but can no longer send by itself. Each row names its attempt's
 * state and offers only what that state allows: an unconfirmed original is copied into a separate new attempt (never
 * retried), a not-applied one retries as a fresh attempt, a pending one revalidates before sending, and anything
 * damaged or foreign is discarded. Discard always asks first. Storage that cannot be read is a notice without
 * actions; the outbox's change wakeups only re-read the list.
 */
export function OutboxRecovery({ port }: { readonly port: OutboxRecoveryPort }) {
  const store = recoveryStoreFor(port)
  const subscribe = React.useCallback((listener: () => void) => {
    const unsubscribe = store.subscribe(listener)
    const release = store.acquire()
    return () => { unsubscribe(); release() }
  }, [store])
  const view = React.useSyncExternalStore(subscribe, store.snapshot, store.snapshot)
  const [confirming, setConfirming] = React.useState<Confirming>()
  const [composing, setComposing] = React.useState<{ readonly item: OutboxRecoveryItem; readonly copy: OutboxRecoveryCopy }>()
  // After a copy is saved, the composer closes and this says where its outcome lives; it claims no delivery.
  const [copied, setCopied] = React.useState<OutboxRecoveryItem['subject']>()
  const section = React.useRef<HTMLElement>(null)
  const items = view.list._tag === 'Listed' ? view.list.items : []
  // Identity is the attempt id: a confirmation follows that attempt and closes if the outbox no longer holds it.
  const live = confirming === undefined ? undefined : items.find(item => item.id === confirming.observed.id)
  // What the dialog shows: a re-read at another revision replaces the opened state before anything runs, so Discard
  // stays bound to the revision on screen. Closed once the outbox no longer holds the attempt.
  const shown: Confirming | undefined = confirming === undefined || live === undefined ? undefined
    : !confirming.discarding && live.revision !== confirming.observed.revision ? { observed: live, changed: true, discarding: false }
    : confirming
  // Focus left on a removed row returns to the list (or stays where the person moved it).
  const settleFocus = () => {
    const active = document.activeElement
    if (active === null || active === document.body) section.current?.focus()
  }
  const act = (item: OutboxRecoveryItem, action: RecoveryOperation) => {
    switch (action) {
      case 'discard': setConfirming({ observed: item, changed: false, discarding: false }); return
      case 'retry': void store.retry(item.id, item.revision).then(settleFocus); return
      case 'send': void store.send(item.id, item.revision).then(settleFocus); return
      case 'copy': void store.copy(item.id, item.revision).then(outcome => { if (outcome._tag === 'Done') { setCopied(undefined); setComposing({ item, copy: outcome.value }) } }); return
    }
  }
  /**
   * Discards exactly the revision the dialog shows. A stale answer never retries: the dialog stays open on the attempt's
   * current state and consequence, and only a second, deliberate Discard acts on it.
   */
  const discard = (observed: OutboxRecoveryItem) => {
    setConfirming({ observed, changed: shown?.changed === true, discarding: true })
    void store.discard(observed.id, observed.revision).then(outcome => {
      if (outcome._tag === 'Stale') {
        const list = store.snapshot().list
        const current = list._tag === 'Listed' ? list.items.find(item => item.id === observed.id) : undefined
        setConfirming(current === undefined ? undefined : { observed: current, changed: true, discarding: false })
        return
      }
      // Close before settling focus, so a removed row's focus lands on the list rather than the closing dialog.
      flushSync(() => setConfirming(undefined))
      settleFocus()
    })
  }
  const body = view.list._tag === 'Checking' ? null
    : view.list._tag === 'Unavailable' ? <div {...stylex.props(styles.lane)}>
      <section aria-label={RECOVERY_TITLE} data-outbox-recovery="unavailable" {...stylex.props(styles.recovery)}>
        <p role="status" {...stylex.props(styles.text)}>{RECOVERY_UNAVAILABLE}</p>
      </section>
    </div>
      : items.length === 0 ? null : <div {...stylex.props(styles.lane)}>
        <section ref={section} tabIndex={-1} aria-label={RECOVERY_TITLE} data-outbox-recovery="list" {...stylex.props(styles.recovery, styles.focusTarget)}>
          <p aria-hidden="true" {...stylex.props(styles.title)}>{RECOVERY_TITLE}<span {...stylex.props(styles.count)}> · {items.length}</span></p>
          <p role="status" data-outbox-copy-notice {...stylex.props(styles.text)}>{copied === undefined ? null : `The copy is saved as its own new ${copied}. If Fractal does not confirm it, it is listed here. Do not send it again.`}</p>
          <ul {...stylex.props(styles.list)}>
            {items.map(item => <OutboxRecoveryRow key={item.id} item={item} actions={offeredActions(item, port)} busy={view.busy.get(item.id)} failed={view.failed.get(item.id)} onAction={act} />)}
          </ul>
        </section>
      </div>
  return <>
    {body}
    <ThemePortal>
      <DiscardConfirmation confirming={shown} onCancel={() => setConfirming(undefined)} onConfirm={discard} />
      {composing === undefined ? null : <CopyComposer key={composing.copy.copyKey} item={composing.item} copy={composing.copy} store={store}
        onClose={recorded => { setComposing(undefined); if (recorded) setCopied(composing.item.subject) }} />}
    </ThemePortal>
  </>
}

function OutboxRecoveryRow({ item, actions, busy, failed, onAction }: {
  readonly item: OutboxRecoveryItem
  readonly actions: readonly RecoveryOperation[]
  readonly busy: RecoveryOperation | undefined
  readonly failed: RecoveryFailure | undefined
  readonly onAction: (item: OutboxRecoveryItem, action: RecoveryOperation) => void
}) {
  const id = React.useId()
  // An unconfirmed attempt shows no delivery status: only that it must not be sent again.
  const status = item.kind === 'unknown' || item.status === '' ? undefined : item.status
  const preview = item.text === undefined || item.text === '' ? undefined : item.text
  // Each action's name carries what it acts on: the message itself where shown, else its state.
  const subject = preview !== undefined ? `${id}-preview` : status !== undefined ? `${id}-status` : `${id}-headline`
  return <li data-outbox-item={item.id} data-outbox-kind={item.kind} data-outbox-subject={item.subject} {...stylex.props(styles.row)}>
    <span {...stylex.props(styles.text, styles.rowText)}>
      {/* The attempt's state is a live status: a wakeup that changes it (pending → unknown) is announced, an unchanged re-read is not. */}
      <span role="status" aria-atomic="true" data-outbox-state {...stylex.props(styles.rowState)}>
        <span id={`${id}-headline`} data-outbox-headline {...stylex.props(styles.headline, item.kind === 'unknown' && styles.attention)}>{headline(item)}</span>
        {status === undefined ? null : <span id={`${id}-status`} data-outbox-status>{status}</span>}
      </span>
      {preview === undefined ? null : <span id={`${id}-preview`} data-outbox-preview {...stylex.props(styles.preview)}>{preview}</span>}
      {failed === undefined ? null : <span role="status" data-outbox-failure={failed.operation} data-outbox-failure-cause={failed.cause} {...stylex.props(styles.failure)}>{actionLabel[failed.operation][failed.cause === 'stale' ? 'stale' : 'failed']}</span>}
    </span>
    {actions.length === 0 ? null : <span {...stylex.props(styles.actions)}>
      {actions.map(action => <Aria.Button key={action} id={`${id}-${action}`} aria-labelledby={`${id}-${action} ${subject}`} data-outbox-action={action} isDisabled={busy !== undefined} onPress={() => onAction(item, action)} {...stylex.props(styles.button)}>
        {busy === action ? actionLabel[action].busy : actionLabel[action].idle}
      </Aria.Button>)}
    </span>}
  </li>
}

/**
 * Discard is irreversible for the device's copy and says what it does not undo. Keep holds initial focus, and takes it
 * back whenever the attempt changes under the dialog: the alert names the new state, the consequence is the current
 * one, and Discard acts only on that shown revision.
 */
function DiscardConfirmation({ confirming, onCancel, onConfirm }: { readonly confirming: Confirming | undefined; readonly onCancel: () => void; readonly onConfirm: (observed: OutboxRecoveryItem) => void }) {
  const id = React.useId()
  const keep = React.useRef<HTMLButtonElement>(null)
  const item = confirming?.observed
  const changed = confirming?.changed === true
  const discarding = confirming?.discarding === true
  React.useEffect(() => { if (changed) keep.current?.focus() }, [changed, item])
  return <Aria.ModalOverlay isOpen={item !== undefined} isDismissable={!discarding} isKeyboardDismissDisabled={discarding} onOpenChange={open => { if (!open) onCancel() }} {...stylex.props(styles.overlay)}>
    <Aria.Modal {...stylex.props(styles.modal, styles.confirmModal)}>
      <Aria.Dialog role="alertdialog" aria-describedby={changed ? `${id}-changed ${id}-consequence` : `${id}-consequence`} data-outbox-discard-confirmation data-outbox-discard-changed={changed ? '' : undefined} {...stylex.props(styles.dialog)}>
        {item === undefined ? null : <>
          <Aria.Heading slot="title" {...stylex.props(styles.dialogTitle)}>Discard from this device?</Aria.Heading>
          {changed ? <p id={`${id}-changed`} role="alert" data-outbox-discard-changed-notice {...stylex.props(styles.dialogText, styles.changed)}>
            {`This ${item.subject} changed in another tab or window. Nothing was discarded. ${headline(item)}`}
          </p> : null}
          <p id={`${id}-consequence`} data-outbox-discard-consequence {...stylex.props(styles.dialogText)}>{discardConsequence(item)}</p>
          <div {...stylex.props(styles.footer)}>
            <Aria.Button ref={keep} autoFocus isDisabled={discarding} onPress={onCancel} {...stylex.props(styles.button)}>Keep</Aria.Button>
            <Aria.Button isDisabled={discarding} onPress={() => onConfirm(item)} {...stylex.props(styles.button, styles.danger)}>{discarding ? 'Discarding…' : 'Discard'}</Aria.Button>
          </div>
        </>}
      </Aria.Dialog>
    </Aria.Modal>
  </Aria.ModalOverlay>
}

/**
 * A separate composer for a copy of an unconfirmed original: it never touches the conversation's own composer or
 * draft, warns that the original may have arrived, and sends a genuinely new attempt with the original's retained
 * images. The original stays in the outbox until discarded.
 */
function CopyComposer({ item, copy, store, onClose }: { readonly item: OutboxRecoveryItem; readonly copy: OutboxRecoveryCopy; readonly store: RecoveryStore; readonly onClose: (recorded: boolean) => void }) {
  const id = React.useId()
  const [text, setText] = React.useState(copy.text)
  // A refusal saved nothing, so the person may edit and try again under the same copy key. A stale original saved
  // nothing either, but the copy no longer matches what the person read, so Send stays off. Any other answer closes.
  const [state, setState] = React.useState<{ readonly _tag: 'Editing' } | { readonly _tag: 'Sending' } | { readonly _tag: 'Refused'; readonly detail: string } | { readonly _tag: 'Stale' }>({ _tag: 'Editing' })
  const empty = text.trim() === '' && copy.images.length === 0
  const sending = state._tag === 'Sending'
  const stale = state._tag === 'Stale'
  const send = () => {
    setState({ _tag: 'Sending' })
    void store.sendCopy(item.id, copy.copyKey, text).then(outcome => { if (outcome._tag === 'Recorded') onClose(true); else setState(outcome) })
  }
  return <Aria.ModalOverlay isOpen isDismissable={false} isKeyboardDismissDisabled={sending} onOpenChange={open => { if (!open) onClose(false) }} {...stylex.props(styles.overlay)}>
    <Aria.Modal {...stylex.props(styles.modal, styles.composerModal)}>
      <Aria.Dialog aria-describedby={`${id}-warning`} data-outbox-copy-composer {...stylex.props(styles.dialog)}>
        <Aria.Heading slot="title" {...stylex.props(styles.dialogTitle)}>Send a copy as a new {item.subject}</Aria.Heading>
        <p id={`${id}-warning`} data-outbox-copy-warning {...stylex.props(styles.dialogText, styles.warning)}>
          The original may have arrived. Fractal has not confirmed it, so this copy is sent as a separate new {item.subject} and both may appear.
        </p>
        <Aria.TextField value={text} onChange={setText} isDisabled={sending} {...stylex.props(styles.field)}>
          <Aria.Label {...stylex.props(styles.fieldLabel)}>Copy of the {item.subject}</Aria.Label>
          <Aria.TextArea rows={5} {...stylex.props(styles.textArea)} />
        </Aria.TextField>
        {copy.images.length === 0 ? null : <div {...stylex.props(styles.images)}>
          <p id={`${id}-images`} {...stylex.props(styles.fieldLabel)}>{copy.images.length === 1 ? 'Sends with 1 image from the original' : `Sends with ${copy.images.length} images from the original`}</p>
          <ul aria-labelledby={`${id}-images`} data-outbox-copy-images {...stylex.props(styles.imageList)}>
            {copy.images.map((image, index) => <li key={`${index}:${image.name}`} {...stylex.props(styles.imageName)}>{image.name}</li>)}
          </ul>
        </div>}
        {empty ? <p {...stylex.props(styles.dialogText, styles.hint)}>Write a message to send a copy.</p> : null}
        {state._tag === 'Refused' ? <p role="alert" data-outbox-copy-refused {...stylex.props(styles.dialogText, styles.failure)}>Fractal refused this copy and nothing new was saved: {state.detail} Your text is still here.</p> : null}
        {stale ? <p role="alert" data-outbox-copy-stale {...stylex.props(styles.dialogText, styles.failure)}>The original changed in another tab or window, so nothing new was saved. Your text is still here; close to see its current state.</p> : null}
        <div {...stylex.props(styles.footer)}>
          <Aria.Button isDisabled={sending} onPress={() => onClose(false)} {...stylex.props(styles.button)}>{stale ? 'Close' : 'Cancel'}</Aria.Button>
          <Aria.Button isDisabled={sending || empty || stale} onPress={send} {...stylex.props(styles.button, styles.primary)}>{sending ? 'Sending copy…' : 'Send copy'}</Aria.Button>
        </div>
      </Aria.Dialog>
    </Aria.Modal>
  </Aria.ModalOverlay>
}

const focusRing = { outlineWidth: geometryVars.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  lane: { width: '100%', maxWidth: geometryVars.lane, boxSizing: 'border-box', paddingInline: spaceVars.lg, marginInline: 'auto', flexShrink: 0 },
  recovery: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, minWidth: 0, margin: 0, paddingBlock: spaceVars.xs2, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  focusTarget: { outlineStyle: { default: 'none', ':focus-visible': 'solid' }, outlineWidth: geometryVars.focusRing, outlineColor: accent.primary, outlineOffset: '2px' },
  title: { margin: 0, color: ink.fgMuted, fontWeight: t.weightMedium },
  count: { fontVariantNumeric: 'tabular-nums' },
  // A long outbox scrolls inside its band instead of pushing the conversation away.
  list: { display: 'flex', flexDirection: 'column', gap: spaceVars.sm, maxHeight: 'min(40vh, 320px)', overflowY: 'auto', scrollbarGutter: 'stable', margin: 0, padding: 0, listStyle: 'none' },
  row: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', columnGap: spaceVars.md, rowGap: spaceVars.xs, minWidth: 0 },
  text: { margin: 0, color: ink.fgMuted, overflowWrap: 'anywhere' },
  rowText: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, flex: '1 1 240px', minWidth: 0, color: ink.fgSoft },
  rowState: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, minWidth: 0 },
  headline: { color: ink.fg, fontWeight: t.weightMedium },
  attention: { color: tone.attention },
  preview: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: ink.fgMuted },
  failure: { color: tone.attention },
  actions: { display: 'flex', flexWrap: 'wrap', gap: spaceVars.sm, flexShrink: 0 },
  button: { flexShrink: 0, height: geometryVars.controlSm, paddingInline: spaceVars.md, borderRadius: r.sm, borderWidth: geometryVars.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, whiteSpace: 'nowrap', cursor: { default: 'pointer', ':disabled': 'default' }, opacity: { default: 1, ':disabled': 0.6 }, ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  primary: { borderColor: accent.primary, backgroundColor: accent.primary, color: accent.onPrimary, ':hover': { backgroundColor: accent.primaryHover } },
  danger: { color: tone.dangerFg },
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', alignItems: 'center', justifyContent: 'center', padding: spaceVars.lg, boxSizing: 'border-box', backgroundColor: surface.scrim },
  modal: { width: '100%', boxSizing: 'border-box', maxHeight: '100%', overflowY: 'auto', borderRadius: r.md, borderWidth: geometryVars.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.raised, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, outline: 'none' },
  confirmModal: { maxWidth: 400 },
  composerModal: { maxWidth: 520 },
  dialog: { display: 'flex', flexDirection: 'column', gap: spaceVars.md, padding: spaceVars.xl, minWidth: 0, outline: 'none' },
  dialogTitle: { margin: 0, fontSize: t.uiSize, fontWeight: t.weightMedium, color: ink.fg },
  dialogText: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgSoft, overflowWrap: 'anywhere' },
  warning: { color: tone.attention },
  changed: { color: tone.attention, fontWeight: t.weightMedium },
  hint: { color: ink.fgMuted },
  field: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs, minWidth: 0 },
  fieldLabel: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  textArea: { boxSizing: 'border-box', width: '100%', minHeight: 96, resize: 'vertical', padding: spaceVars.md, borderRadius: r.sm, borderWidth: geometryVars.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, outline: 'none', ':focus-visible': { borderColor: accent.primary } },
  images: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs, minWidth: 0 },
  imageList: { display: 'flex', flexDirection: 'column', gap: spaceVars.xs2, margin: 0, padding: 0, listStyle: 'none' },
  imageName: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize, color: ink.fgSoft },
  footer: { display: 'flex', justifyContent: 'flex-end', flexWrap: 'wrap', gap: spaceVars.sm },
})
