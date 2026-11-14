import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button } from 'react-aria-components'
import { accentVars as accent, borderVars as border, geometryVars, radiusVars as r, spaceVars, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { PairingStore, PairingView } from './browserPairingState.ts'

/**
 * Pairing for this browser, above the composer in the reading lane. It asks only where sending is not
 * already granted or Fractal refused a send, shows the code to approve in Fractal while it waits, and
 * offers to forget a paired browser. Hosts without browser pairing render nothing, leaving the composer as it was.
 */
export function BrowserPairing({ store, messageSend }: { readonly store: PairingStore; readonly messageSend: 'granted' | 'ungranted' }) {
  const subscribe = React.useCallback((listener: () => void) => {
    const unsubscribe = store.subscribe(listener)
    const release = store.acquire()
    return () => { unsubscribe(); release() }
  }, [store])
  const view = React.useSyncExternalStore(subscribe, store.snapshot, store.snapshot)
  const content = notice(view, messageSend, store)
  if (content === undefined) return null
  return <div {...stylex.props(styles.lane)}>
    <section aria-label="Browser pairing" data-testid="browser-pairing" data-pairing-state={view._tag} {...stylex.props(styles.notice)}>
      {/* One live region: a code appearing or a refusal arriving is announced without moving focus. */}
      <p role="status" {...stylex.props(styles.text)}>
        <span {...stylex.props(styles.title, content.danger === true && styles.danger)}>{content.title}</span>
        {content.code === undefined ? null : <span data-testid="pairing-code" {...stylex.props(styles.code)}>{content.code}</span>}
        {content.detail === undefined ? null : <span {...stylex.props(styles.detail)}>{content.detail}</span>}
      </p>
      {content.action === undefined ? null
        : <Button isDisabled={content.action.busy === true} onPress={content.action.run} {...stylex.props(styles.button, content.action.primary === true && styles.primary)}>{content.action.label}</Button>}
    </section>
  </div>
}

interface Notice {
  readonly title: string
  readonly danger?: boolean
  readonly code?: string
  readonly detail?: string
  readonly action?: { readonly label: string; readonly run: () => void; readonly primary?: boolean; readonly busy?: boolean }
}

const notice = (view: PairingView, messageSend: 'granted' | 'ungranted', store: PairingStore): Notice | undefined => {
  const request = { label: 'Enable sending on this browser', run: () => { void store.start() }, primary: true }
  const retry = { label: 'Try again', run: () => { void store.start() } }
  switch (view._tag) {
    case 'Checking':
    case 'Hidden':
      return undefined
    case 'Unpaired':
      // Another credential may already allow sending; only a browser that cannot send is asked to pair.
      if (view.error !== undefined) return { title: view.error, danger: true, action: request }
      // A refused send outranks the grants read at load: they no longer show whether this browser can send.
      if (view.sendRefused === true) return { title: 'Fractal did not accept sending from this browser.', danger: true, detail: 'Approve it in Fractal to send messages from here.', action: request }
      return messageSend === 'granted' ? undefined : { title: 'This browser can read but not send.', detail: 'Approve it in Fractal to send messages from here.', action: request }
    case 'Requesting':
      return { title: 'Requesting a pairing code…', action: { ...request, label: 'Requesting code…', busy: true } }
    case 'Pending':
      return view.displayCode === undefined
        ? { title: 'Approve in Fractal', detail: 'Waiting for approval. This page reloads once the browser is approved.' }
        : { title: 'Approve in Fractal', code: view.displayCode, detail: 'Approve only if Fractal shows the code in the browser you are using. Browser labels and login names are not proof of who requested access.' }
    case 'Ended':
      return view.outcome === 'denied'
        ? { title: 'Fractal declined this browser.', danger: true, detail: 'Request a new code if this was a mistake.', action: retry }
        : { title: 'The pairing code expired before it was approved.', detail: 'Request a new code and approve it in Fractal.', action: retry }
    case 'Paired':
      return {
        title: view.error ?? 'This browser is paired.',
        ...(view.error === undefined ? {} : { danger: true }),
        action: { label: 'Forget this browser', run: () => { void store.forget() } },
      }
    case 'Forgetting':
      return { title: 'Forgetting this browser…', action: { label: 'Forget this browser', run: () => {}, busy: true } }
    case 'Reloading':
      return { title: 'Reloading with this browser’s new access…' }
  }
}

const focusRing = { outlineWidth: geometryVars.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  lane: { width: '100%', maxWidth: geometryVars.lane, boxSizing: 'border-box', paddingInline: spaceVars.lg, paddingBottom: spaceVars.sm, marginInline: 'auto', flexShrink: 0 },
  notice: { display: 'flex', alignItems: 'center', gap: spaceVars.md, minWidth: 0 },
  text: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: spaceVars.sm, rowGap: spaceVars.xs2, flex: '1 1 0', minWidth: 0, margin: 0, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  title: { color: ink.fg, fontWeight: t.weightMedium },
  danger: { color: tone.dangerFg },
  code: { paddingInline: spaceVars.xs, borderRadius: r.keyboard, backgroundColor: surface.controlFill, borderWidth: geometryVars.hairline, borderStyle: 'solid', borderColor: border.borderStrong, color: ink.fg, fontFamily: t.fontMono, fontSize: t.codeSize, fontVariantNumeric: 'tabular-nums', letterSpacing: t.trackingWide, userSelect: 'all' },
  detail: { flexBasis: '100%', color: ink.fgSoft, overflowWrap: 'anywhere' },
  button: { flexShrink: 0, height: geometryVars.controlSm, paddingInline: spaceVars.md, borderRadius: r.sm, borderWidth: geometryVars.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, whiteSpace: 'nowrap', cursor: { default: 'pointer', ':disabled': 'default' }, opacity: { default: 1, ':disabled': 0.6 }, ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  primary: { borderColor: accent.primary, backgroundColor: accent.primary, color: accent.onPrimary, ':hover': { backgroundColor: accent.primaryHover } },
})
