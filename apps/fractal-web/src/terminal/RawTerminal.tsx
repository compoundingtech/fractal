/**
 * The interactive terminal: a native Ghostty VT emulator (WASM) fed byte-exact from the raw PTY
 * socket, painted through the DOM renderer so selection, copy, find and screen readers keep
 * reading real text. Keys and pastes leave as DATA frames on the same socket.
 */

import { useAtomValue } from '@effect/atom-react'
import type { TerminalScreen } from '@smalltalk/st3-client/schema'
import type { PtyOutput, PtySession, PtySize, PtyState, PtyTarget } from '@st3/sdk/effect'
import * as stylex from '@stylexjs/stylex'
import { Cause, Effect, Option, Schema, Stream, SubscriptionRef } from 'effect'
import * as AsyncResult from 'effect/reactivity/AsyncResult'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'
import { Button } from 'react-aria-components'

import { Modal, ModalDialog, ModalHeader, ModalTitle } from '../ui-compat/components.tsx'
import { scale, tokens } from '../ui-compat/tokens.stylex.ts'

import { DomTerminal, TERMINAL_FONT_SIZE, TERMINAL_LINE_HEIGHT } from './DomTerminal.tsx'
import { makeGhosttyKeyboardOn, type TerminalKeyEncoder } from './ghosttyKeyboard.ts'
import { loadGhosttyNative, makeGhosttyTerminal, type GhosttyTerminal } from './ghosttyVt.ts'
import type { TerminalPalette } from './palette.ts'
import {
  admitPaste,
  admitText,
  browserTerminalKey,
  makeCompositionGate,
  routeKey,
  type TerminalAdmission,
} from './rawTerminalInput.ts'
import { gridFor, makeResizeDebouncer, type RawPreparation, type RawTerminalPort } from './rawTerminalState.ts'
import { TERMINAL_FONT_FAMILY, terminalFontsReady } from './terminalFonts.ts'

/** The emulator or its asset could not be prepared; the pane falls back to the projected screen. */
class RawTerminalUnprepared extends Schema.TaggedError<RawTerminalUnprepared>()('RawTerminalUnprepared', {
  message: Schema.String,
}) {}

/** What the pane renders from the raw session: its connection state and the emulator's screen. */
export interface RawSnapshot {
  readonly state: PtyState
  /** `undefined` until the first SCREEN replay or output has been written. */
  readonly screen: TerminalScreen | undefined
}

/** Frame-coalesced view of the emulator for React. */
export interface RawView {
  readonly subscribe: (listener: () => void) => () => void
  readonly current: () => RawSnapshot
  /** Scroll the viewport by lines (negative is up) or back to the active area. */
  readonly scroll: (delta: number | 'bottom') => void
}

/** One prepared raw terminal: emulator, encoder and live session sharing one WASM instance. */
export interface RawTerminalHandle {
  readonly session: PtySession
  readonly encoder: TerminalKeyEncoder
  readonly terminal: GhosttyTerminal
  readonly view: RawView
}

/**
 * Bytes go into the emulator as they arrive; React sees at most one snapshot per animation frame.
 * A SCREEN frame is the server's replay of the current screen: the emulator is reset first.
 */
const makeRawView = ({
  terminal,
  meta,
}: {
  readonly terminal: GhosttyTerminal
  readonly meta: { readonly terminalId: string; readonly incarnation: string; readonly fallbackTitle: string }
}) => {
  let snapshot: RawSnapshot = { state: { _tag: 'Connecting', attempt: 1 }, screen: undefined }
  let painted = false
  let frame: number | undefined
  const listeners = new Set<() => void>()
  const publish = (next: RawSnapshot) => {
    snapshot = next
    for (const listener of listeners) listener()
  }
  const paint = () => {
    frame = undefined
    if (!painted) return
    const screen = terminal.snapshot(meta)
    if (screen !== snapshot.screen) publish({ ...snapshot, screen })
  }
  const schedule = () => {
    frame ??= requestAnimationFrame(paint)
  }
  return {
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    current: () => snapshot,
    scroll: (delta: number | 'bottom') => {
      terminal.scroll(delta)
      schedule()
    },
    output: (output: PtyOutput) => {
      switch (output._tag) {
        case 'Screen':
          terminal.reset()
          terminal.write(output.bytes)
          painted = true
          break
        case 'Data':
          terminal.write(output.bytes)
          painted = true
          break
        case 'Geometry':
          terminal.resize({ cols: output.cols, rows: output.rows })
          break
        case 'Exit':
          break
      }
      schedule()
    },
    state: (state: PtyState) => {
      if (state !== snapshot.state) publish({ ...snapshot, state })
    },
    dispose: () => {
      if (frame !== undefined) cancelAnimationFrame(frame)
      frame = undefined
      listeners.clear()
    },
  }
}

const inactive: Atom.Atom<AsyncResult.AsyncResult<RawTerminalHandle, RawTerminalUnprepared>> = Atom.make(
  AsyncResult.initial<RawTerminalHandle, RawTerminalUnprepared>(),
).pipe(Atom.keepAlive)
const unsubscribed = () => () => {}

/** The raw session of a pane while it is visible, and what the pane can say about it. */
export interface RawTerminal {
  readonly preparation: RawPreparation
  readonly handle: RawTerminalHandle | undefined
  readonly snapshot: RawSnapshot | undefined
}

/**
 * Own one raw session for `target` while `active`. The atom's scope owns the emulator, encoder and
 * socket; leaving, hiding or retargeting the pane closes it (DETACH) and nothing is replayed.
 */
export const useRawTerminal = ({
  port,
  target,
  active,
  size,
  title,
}: {
  readonly port: RawTerminalPort | undefined
  readonly target: PtyTarget | undefined
  readonly active: boolean
  /** The attach size: the current shared geometry until the element is measured. */
  readonly size: PtySize
  readonly title: string
}): RawTerminal => {
  const terminalId = target?.terminalId
  const incarnation = target?.incarnation
  // The attach size and fallback title are read once per session, from the render that targets it;
  // later geometry travels as RESIZE frames and the title comes from the emulator.
  const resource = React.useMemo(() => {
    if (port === undefined || terminalId === undefined || incarnation === undefined || !active) return inactive
    return Atom.make(
      Effect.gen(function* () {
        const native = yield* Effect.tryPromise({
          try: () => loadGhosttyNative(),
          catch: (cause) =>
            new RawTerminalUnprepared({ message: `The terminal emulator could not be loaded: ${String(cause)}` }),
        })
        const terminal = yield* Effect.acquireRelease(
          Effect.try({
            try: () => makeGhosttyTerminal(native, size),
            catch: (cause) =>
              new RawTerminalUnprepared({ message: `The terminal emulator could not start: ${String(cause)}` }),
          }),
          (created) => Effect.sync(() => created.dispose()),
        )
        const encoder = yield* Effect.acquireRelease(
          Effect.try({
            try: () => makeGhosttyKeyboardOn(native),
            catch: (cause) =>
              new RawTerminalUnprepared({ message: `The terminal key encoder could not start: ${String(cause)}` }),
          }),
          (created) => Effect.sync(() => created.dispose()),
        )
        const view = makeRawView({ terminal, meta: { terminalId, incarnation, fallbackTitle: title } })
        yield* Effect.addFinalizer(() => Effect.sync(view.dispose))
        const session = yield* port.open({ target: { terminalId, incarnation }, size, onOutput: view.output })
        yield* SubscriptionRef.changes(session.state).pipe(
          Stream.runForEach((state) => Effect.sync(() => view.state(state))),
          Effect.forkScoped,
        )
        return { session, encoder, terminal, view } satisfies RawTerminalHandle
      }),
    ).pipe(Atom.setIdleTTL(0))
  }, [port, terminalId, incarnation, active])
  const result = useAtomValue(resource)
  const handle = AsyncResult.isSuccess(result) ? result.value : undefined
  const subscribe = React.useMemo(() => handle?.view.subscribe ?? unsubscribed, [handle])
  const snapshot = React.useSyncExternalStore(subscribe, () => handle?.view.current())
  const preparation: RawPreparation =
    port === undefined
      ? { _tag: 'Unsupported' }
      : AsyncResult.isFailure(result)
        ? {
            _tag: 'Unprepared',
            message: Option.getOrUndefined(Cause.findErrorOption(result.cause))?.message ?? Cause.pretty(result.cause),
          }
        : snapshot === undefined
          ? { _tag: 'Preparing' }
          : { _tag: 'Session', state: snapshot.state }
  return { preparation, handle, snapshot }
}

const selectionInside = (element: HTMLElement | null) => {
  const selection = document.getSelection()
  return (
    element !== null &&
    selection !== null &&
    !selection.isCollapsed &&
    selection.anchorNode !== null &&
    selection.focusNode !== null &&
    element.contains(selection.anchorNode) &&
    element.contains(selection.focusNode)
  )
}

/** Props of the interactive surface. */
export interface RawTerminalSurfaceProps {
  readonly handle: RawTerminalHandle
  readonly screen: TerminalScreen
  readonly palette: TerminalPalette
  readonly live: boolean
  /** Whether keystrokes currently go to the terminal (the input at the cursor has focus). */
  readonly onTypingChange: (typing: boolean) => void
  /** Browser KeyboardLayoutMap, when available. */
  readonly layout?: ReadonlyMap<string, string>
}

/** Fonts must be ready before the cell is measured, or the first RESIZE carries a wrong grid. */
export const RawTerminalSurface = (props: RawTerminalSurfaceProps) => (
  <React.Suspense fallback={<p role="status">Loading terminal fonts…</p>}>
    <LoadedSurface {...props} />
  </React.Suspense>
)

const LoadedSurface = ({ handle, screen, palette, live, onTypingChange, layout }: RawTerminalSurfaceProps) => {
  React.use(terminalFontsReady())
  const id = React.useId()
  const gate = React.useMemo(makeCompositionGate, [])
  const surface = React.useRef<HTMLDivElement | null>(null)
  const field = React.useRef<HTMLTextAreaElement | null>(null)
  const probe = React.useRef<HTMLSpanElement | null>(null)
  const wheel = React.useRef(0)
  // Focus handoff guards: a pointer press keeps drag-selection on the grid; Ctrl-\ parks focus there.
  const pointerDown = React.useRef(false)
  const leaving = React.useRef(false)
  const [composing, setComposing] = React.useState(false)
  const [notice, setNotice] = React.useState<string | undefined>(undefined)
  // A large paste waits here, already encoded for the modes it was pasted under.
  const [heldPaste, setHeldPaste] = React.useState<Uint8Array | undefined>(undefined)
  const { session, encoder, terminal, view } = handle
  const send = (bytes: Uint8Array) => {
    if (bytes.length === 0) return
    if (!session.send(bytes)) {
      setNotice('The terminal is not connected. Nothing was sent, and keys are never replayed.')
      return
    }
    setNotice(undefined)
    view.scroll('bottom')
  }
  const admit = (admission: TerminalAdmission) => {
    if (admission._tag === 'Send') send(admission.bytes)
    else if (admission._tag === 'Confirm') setHeldPaste(admission.bytes)
    else setNotice(admission.notice)
  }
  const settlePaste = (confirmed: boolean) => {
    const bytes = heldPaste
    setHeldPaste(undefined)
    if (confirmed && bytes !== undefined) send(bytes)
    field.current?.focus({ preventScroll: true })
  }
  const keyRoute = (event: React.KeyboardEvent) =>
    routeKey({
      event: {
        key: event.key,
        code: event.code,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        shiftKey: event.shiftKey,
        altKey: event.altKey,
        isComposing: event.nativeEvent.isComposing || gate.composing(),
        keyCode: event.keyCode,
      },
      typing: document.activeElement === field.current,
      hasSelection: selectionInside(surface.current),
    })
  // React's onBeforeInput is synthesized from other events; the native event carries inputType.
  const beforeInput = React.useEffectEvent((event: InputEvent) => {
    const decision = gate.beforeInput({ inputType: event.inputType, data: event.data, isComposing: event.isComposing })
    if (decision._tag === 'Pass') return
    event.preventDefault()
    if (decision._tag === 'Drop') setNotice('Dropped text is not sent to the terminal. Paste it instead.')
    else if (decision.text !== undefined) admit(admitText({ encoder, text: decision.text, modes: terminal.modes() }))
  })
  // Unmounting a focused field fires no blur; the pane must not keep saying "Typing".
  const released = React.useEffectEvent(() => onTypingChange(false))
  const fieldRef = React.useCallback((node: HTMLTextAreaElement | null) => {
    field.current = node
    if (node === null) return
    const listener = (event: InputEvent) => beforeInput(event)
    node.addEventListener('beforeinput', listener)
    return () => {
      node.removeEventListener('beforeinput', listener)
      field.current = null
      released()
    }
  }, [])
  // The PTY follows the element: measure the viewport's content box against the font's cell and
  // send one debounced RESIZE per settled size. The server applies the smallest writable size.
  const measureRef = React.useCallback(
    (node: HTMLDivElement | null) => {
      surface.current = node
      const viewport = node?.parentElement
      if (node === null || viewport === null || viewport === undefined) return
      const debouncer = makeResizeDebouncer({ send: session.resize })
      const observer = new ResizeObserver(([entry]) => {
        const cellWidth = (probe.current?.getBoundingClientRect().width ?? 0) / 10
        if (entry === undefined) return
        const size = gridFor({
          width: entry.contentRect.width,
          height: entry.contentRect.height,
          cellWidth,
          cellHeight: TERMINAL_LINE_HEIGHT,
        })
        if (size !== undefined) debouncer.observe(size)
      })
      observer.observe(viewport)
      return () => {
        observer.disconnect()
        debouncer.dispose()
        surface.current = null
      }
    },
    [session],
  )
  const cursor = screen.cursor
  return (
    <div
      ref={measureRef}
      data-terminal-input={live ? 'live' : 'paused'}
      {...stylex.props(styles.surface)}
      onFocus={(event) => {
        // Keyboard-first typing: a grid focused while live hands focus to the cursor field at once,
        // so an IME's first keydown already composes in an editable element.
        if (leaving.current) {
          leaving.current = false
          return
        }
        if (!live || pointerDown.current) return
        if (event.target.getAttribute('aria-roledescription') === 'terminal') field.current?.focus({ preventScroll: true })
      }}
      onMouseDown={() => {
        pointerDown.current = true
      }}
      onKeyDown={(event) => {
        const route = keyRoute(event)
        if (route === 'leave') {
          event.preventDefault()
          leaving.current = true
          surface.current?.querySelector<HTMLElement>('[aria-roledescription="terminal"]')?.focus()
          return
        }
        if (route !== 'encode') return
        gate.key()
        event.preventDefault()
        send(encoder.key({ event: browserTerminalKey(event.nativeEvent, 'press', layout), modes: terminal.modes() }))
        if (document.activeElement !== field.current) field.current?.focus({ preventScroll: true })
      }}
      onKeyUp={(event) => {
        if (keyRoute(event) !== 'encode') return
        send(encoder.key({ event: browserTerminalKey(event.nativeEvent, 'release', layout), modes: terminal.modes() }))
      }}
      onPaste={(event) => {
        if (gate.composing()) return
        event.preventDefault()
        admit(admitPaste({ encoder, text: event.clipboardData.getData('text/plain'), modes: terminal.modes() }))
      }}
      onMouseUp={() => {
        pointerDown.current = false
        // A drag leaves a selection to copy; a plain click starts typing at the cursor.
        if (!selectionInside(surface.current)) field.current?.focus({ preventScroll: true })
      }}
      onWheel={(event) => {
        wheel.current += event.deltaMode === 1 ? event.deltaY : event.deltaY / TERMINAL_LINE_HEIGHT
        const lines = Math.trunc(wheel.current)
        if (lines === 0) return
        wheel.current -= lines
        view.scroll(lines)
      }}
    >
      <span
        ref={probe}
        aria-hidden
        {...stylex.props(styles.probe)}
        style={{ fontFamily: TERMINAL_FONT_FAMILY, fontSize: `${TERMINAL_FONT_SIZE}px` }}
      />
      <DomTerminal
        screen={screen}
        palette={palette}
        describedBy={`${id}-help`}
        overlay={
          <textarea
            ref={fieldRef}
            aria-label="Terminal input"
            aria-describedby={`${id}-help`}
            tabIndex={-1}
            rows={1}
            autoCapitalize="off"
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            {...stylex.props(styles.field, composing && styles.composing)}
            style={{
              top: cursor.row * TERMINAL_LINE_HEIGHT,
              left: `${cursor.column}ch`,
              width: composing ? `${Math.max(1, screen.columns - cursor.column)}ch` : '1ch',
              fontFamily: TERMINAL_FONT_FAMILY,
              color: palette.foreground,
              backgroundColor: composing ? palette.background : 'transparent',
            }}
            onFocus={() => onTypingChange(true)}
            onBlur={() => onTypingChange(false)}
            onCompositionStart={() => {
              gate.start()
              setComposing(true)
            }}
            onCompositionEnd={(event) => {
              setComposing(false)
              const text = gate.end(event.data)
              event.currentTarget.value = ''
              if (text !== undefined) admit(admitText({ encoder, text, modes: terminal.modes() }))
            }}
            onInput={(event) => {
              if (!gate.composing()) event.currentTarget.value = ''
            }}
          />
        }
      />
      <p id={`${id}-help`} {...stylex.props(styles.help)}>
        Click the terminal or start typing to send keys. Ctrl-\ returns focus to the page; Tab then moves on.
      </p>
      {notice !== undefined && (
        <p role="alert" {...stylex.props(styles.notice)}>
          {notice}
        </p>
      )}
      <Modal
        isOpen={heldPaste !== undefined}
        onOpenChange={(open) => {
          if (!open) settlePaste(false)
        }}
      >
        <ModalDialog aria-label="Confirm terminal paste">
          <ModalHeader>
            <ModalTitle>Paste into the terminal?</ModalTitle>
          </ModalHeader>
          <p {...stylex.props(styles.confirmText)}>
            {`This paste is ${Math.ceil((heldPaste?.length ?? 0) / 1024)} KB. The terminal receives it as typed input.`}
          </p>
          <div {...stylex.props(styles.actions)}>
            <Button onPress={() => settlePaste(false)} {...stylex.props(styles.button)}>
              Cancel
            </Button>
            <Button onPress={() => settlePaste(true)} {...stylex.props(styles.button)}>
              Paste
            </Button>
          </div>
        </ModalDialog>
      </Modal>
    </div>
  )
}

const styles = stylex.create({
  surface: { position: 'relative' },
  probe: { position: 'absolute', visibility: 'hidden', width: '10ch', height: 0, pointerEvents: 'none' },
  // The IME and the platform caret anchor here, at the terminal cursor; it paints nothing until
  // a composition shows its preedit text in place.
  field: {
    position: 'absolute',
    zIndex: 1,
    height: `${TERMINAL_LINE_HEIGHT}px`,
    margin: 0,
    padding: 0,
    borderWidth: 0,
    resize: 'none',
    overflow: 'hidden',
    whiteSpace: 'pre',
    fontSize: `${TERMINAL_FONT_SIZE}px`,
    lineHeight: `${TERMINAL_LINE_HEIGHT}px`,
    caretColor: 'transparent',
    opacity: 0,
    outline: 'none',
  },
  composing: {
    opacity: 1,
    textDecorationLine: 'underline',
  },
  help: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    margin: 0,
  },
  notice: { margin: 0, marginTop: scale.space1, fontSize: '0.75rem', color: tokens['--ds-red-900'] },
  actions: { display: 'flex', alignItems: 'center', gap: scale.space2 },
  confirmText: { margin: 0, marginBlock: scale.space2, lineHeight: 1.5 },
  button: {
    height: '1.5rem',
    paddingInline: scale.space2,
    fontSize: '0.75rem',
    color: tokens['--ds-gray-1000'],
    backgroundColor: {
      default: 'transparent',
      ':is([data-hovered])': tokens['--ds-gray-alpha-100'],
    },
    borderWidth: '1px',
    borderStyle: 'solid',
    borderColor: tokens['--ds-gray-alpha-400'],
    cursor: 'pointer',
    outlineStyle: { default: 'none', ':is([data-focus-visible])': 'solid' },
    outlineColor: tokens['--ds-focus-color'],
    outlineWidth: '2px',
  },
})
