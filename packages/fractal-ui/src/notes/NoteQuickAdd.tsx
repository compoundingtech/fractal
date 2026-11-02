import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Dialog } from 'react-aria-components'
import { VisuallyHidden } from 'react-aria'
import { Checkbox, Popover } from '../assistant-ui/composition/Controls'
import { geometryVars as g, spaceVars as s } from '../assistant-ui/composition-tokens.stylex'
import { NoteComposer, type NoteComposerProps } from './NoteComposer'
import { shortcutContextBlocked } from './shortcut-guard'

/** One quick-add: its fixed target, the laid-out element the popover anchors to, and where focus returns on close. */
export interface NoteQuickAddRequest {
  readonly target: string
  readonly anchor: Element
  readonly returnFocus: HTMLElement | null
}

export interface NoteQuickAddState {
  /** The latest request; it stays after closing so the popover can animate out. */
  readonly request: NoteQuickAddRequest | undefined
  readonly isOpen: boolean
  /** Opens on `target`, anchored to `anchor`. The element focused now gets focus back on close. */
  readonly open: (target: string, anchor: Element) => void
  readonly close: () => void
}

/**
 * Workspace-scoped quick-add state: one request at a time, whichever entry opened it (the `n` shortcut or a
 * row's "Add note…" menu item). A menu item unmounts as its menu closes, so focus then returns to the anchor.
 */
export function useNoteQuickAdd(): NoteQuickAddState {
  const [state, setState] = React.useState<{ readonly request: NoteQuickAddRequest | undefined; readonly isOpen: boolean }>({ request: undefined, isOpen: false })
  const open = React.useCallback((target: string, anchor: Element) => {
    const active = anchor.ownerDocument.activeElement
    const returnFocus = !(active instanceof HTMLElement) || active === anchor.ownerDocument.body ? null
      : active.closest('[role="menu"]') === null ? active
        : anchor instanceof HTMLElement ? anchor : null
    setState({ request: { target, anchor, returnFocus }, isOpen: true })
  }, [])
  const close = React.useCallback(() => setState(current => current.isOpen ? { ...current, isOpen: false } : current), [])
  return { request: state.request, isOpen: state.isOpen, open, close }
}

/**
 * Binds unmodified lowercase `n` on the document once per workspace. `onShortcut` returns whether it opened
 * something; only then is the key consumed. The shared guard leaves the key to fields, typeahead collections
 * (the sidebar tree), open overlays and IME composition.
 */
export function useNoteShortcut(enabled: boolean, onShortcut: () => boolean) {
  const latest = React.useRef(onShortcut)
  React.useLayoutEffect(() => { latest.current = onShortcut })
  React.useEffect(() => {
    if (!enabled) return
    const listener = (event: KeyboardEvent) => {
      if (event.key !== 'n' || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.repeat || shortcutContextBlocked(event)) return
      if (latest.current()) event.preventDefault()
    }
    document.addEventListener('keydown', listener, true)
    return () => document.removeEventListener('keydown', listener, true)
  }, [enabled])
}

export interface NoteQuickAddProps extends Omit<NoteComposerProps, 'autoFocus' | 'onClose' | 'target' | 'targetLabel' | 'surface' | 'onPendingChange'> {
  readonly state: Pick<NoteQuickAddState, 'request' | 'isOpen' | 'close'>
  /** The request target's display name: the agent title, or "this session". */
  readonly targetLabel: string
  /** The workspace's durable `n` preference; the popover offers the toggle. */
  readonly shortcutEnabled: boolean
  readonly onShortcutEnabledChange: (enabled: boolean) => void
}

/**
 * The quick-add popover, mounted once per workspace. While a create is in flight it cannot be dismissed
 * (Escape, outside press or the composer's own Escape), so its outcome always reaches the open composer.
 * The host owns one draft per target, shared with every other entry point.
 */
export function NoteQuickAdd({ state, targetLabel, shortcutEnabled, onShortcutEnabledChange, ...composer }: NoteQuickAddProps) {
  const { request, isOpen, close } = state
  const [pending, setPending] = React.useState(false)
  const [announcement, setAnnouncement] = React.useState('')
  const anchor = React.useMemo(() => ({ current: request?.anchor ?? null }), [request])
  React.useEffect(() => setPending(false), [request])
  const dismiss = () => { if (!pending) close() }
  return <>
    {/* Persistent: the composer's own status unmounts with the popover. */}
    <VisuallyHidden><span role="status" aria-live="polite" data-testid="note-quick-add-status">{announcement}</span></VisuallyHidden>
    {request !== undefined && <Popover
      triggerRef={anchor}
      isOpen={isOpen}
      onOpenChange={next => { if (!next) dismiss() }}
      isKeyboardDismissDisabled={pending}
      shouldCloseOnInteractOutside={() => !pending}
      placement="bottom start"
      style={styles.popover}
    >
      <Dialog aria-label={`Add note to ${targetLabel}`} data-note-target={request.target} {...stylex.props(styles.dialog)}>
        <FocusReturn to={request.returnFocus} />
        <NoteComposer
          key={request.target}
          {...composer}
          target={request.target}
          targetLabel={targetLabel}
          surface="bare"
          autoFocus
          onPendingChange={setPending}
          onClose={dismiss}
          onSubmit={async value => {
            await composer.onSubmit(value)
            setAnnouncement(`Note added to ${targetLabel}.`)
            close()
          }}
        />
        <Checkbox isSelected={shortcutEnabled} onChange={onShortcutEnabledChange} style={styles.shortcut}>N opens note composer</Checkbox>
      </Dialog>
    </Popover>}
  </>
}

/**
 * React Aria returns focus to the element focused when the popover mounted. When that element is gone (a menu
 * item), focus falls to the body or to the tree row holding the menu; once React Aria has run, put it on the
 * request's return target instead.
 */
function FocusReturn({ to }: { readonly to: HTMLElement | null }) {
  React.useEffect(() => () => {
    requestAnimationFrame(() => {
      if (to === null || !to.isConnected) return
      const active = to.ownerDocument.activeElement
      if (active === null || active === to.ownerDocument.body || (active !== to && active.contains(to))) to.focus({ preventScroll: true })
    })
  }, [to])
  return null
}

const styles = stylex.create({
  popover: { width: g.polishNarrow, maxWidth: `calc(100vw - ${s.xl} - ${s.md})`, boxSizing: 'border-box', padding: s.sm },
  dialog: { display: 'flex', flexDirection: 'column', gap: s.sm, minWidth: 0, outlineStyle: 'none' },
  shortcut: { alignSelf: 'flex-start' },
})
