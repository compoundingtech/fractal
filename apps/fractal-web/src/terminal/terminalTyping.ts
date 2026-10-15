/**
 * Typing that does not ride a raw ATTACH: ordered `terminal.input` sessions through the gateway,
 * started by the pane's own focus, click or first key instead of a separate enable control, and
 * the native key encoder for a pane that shows the projected screen.
 */

import { RegistryContext, useAtomValue } from '@effect/atom-react'
import { Effect } from 'effect'
import * as AsyncResult from 'effect/reactivity/AsyncResult'
import * as Atom from 'effect/reactivity/Atom'
import * as React from 'react'

import { TerminalInputUnsendable } from './actionTerminalInput.ts'
import { makeGhosttyKeyboardOn, type TerminalKeyEncoder } from './ghosttyKeyboard.ts'
import { loadGhosttyNative } from './ghosttyVt.ts'
import type { TerminalInputPort, TerminalInputPortFactory } from './orderedTerminalInput.ts'
import type { TerminalTyping } from './RawTerminal.tsx'

const paused =
  'Input is paused while this terminal is hidden, disconnected or stale. It resumes when a live screen returns.'
const unprepared = 'Terminal input could not be prepared. No keystrokes were sent.'

interface GatewaySession {
  readonly port: Promise<TerminalInputPort | undefined>
  /** The session refused, lost or could not prepare a write; only a new gesture starts another. */
  ended: boolean
  readonly stop: () => void
}

/**
 * One gateway input session per pane while `enabled`. The first focus, click or key starts it; a
 * different terminal or incarnation, a hidden or stale screen, or a lost grant ends it. A refused or
 * uncertain delivery ends it with its reason, and typing resumes only on the next focus or click.
 */
export const useGatewayTyping = ({
  factory,
  subject,
  terminalRef,
  incarnation,
  enabled,
}: {
  readonly factory: TerminalInputPortFactory | undefined
  /** The subject address whose terminal feed the pane shows; the port re-reads it per post. */
  readonly subject: string
  readonly terminalRef: string
  readonly incarnation: string
  /** The pane is visible, its projected screen is live, and the device holds `terminal.input`. */
  readonly enabled: boolean
}): Pick<TerminalTyping, 'send' | 'arm' | 'notice'> => {
  const registry = React.useContext(RegistryContext)
  const session = React.useRef<GatewaySession | undefined>(undefined)
  const [notice, setNotice] = React.useState<string | undefined>(undefined)
  React.useEffect(
    () => () => {
      session.current?.stop()
      session.current = undefined
      setNotice(undefined)
    },
    [factory, registry, subject, terminalRef, incarnation, enabled],
  )
  const start = (): GatewaySession | undefined => {
    if (!enabled || factory === undefined) return undefined
    setNotice(undefined)
    let unsubscribe = () => {}
    let stopped = false
    const entry: GatewaySession = {
      ended: false,
      stop: () => {
        stopped = true
        entry.ended = true
        unsubscribe()
        void entry.port.then((port) => port?.close())
      },
      port: factory({ subject, terminalRef, incarnation, registry }).then(
        (port) => {
          if (stopped) {
            port.close()
            return undefined
          }
          unsubscribe = registry.subscribe(port.state, (state) => {
            if (state._tag !== 'Closed' || session.current !== entry) return
            entry.ended = true
            setNotice(state.reason)
          })
          port.open()
          return port
        },
        () => {
          entry.ended = true
          if (session.current === entry) setNotice(unprepared)
          return undefined
        },
      ),
    }
    session.current = entry
    return entry
  }
  return {
    notice,
    arm: () => {
      if (session.current === undefined || session.current.ended) start()
    },
    send: (bytes) => {
      if (!enabled) return paused
      // A key is itself the gesture that starts the first session; an ended one waits for a click.
      const entry = session.current ?? start()
      if (entry === undefined) return unprepared
      if (entry.ended) return notice ?? 'Input is off. Click the terminal to type again.'
      void entry.port
        .then((port) => port?.send(bytes))
        .catch((error: unknown) => {
          // Refusals end the session and name themselves through its state; a local refusal does not.
          if (error instanceof TerminalInputUnsendable && session.current === entry)
            setNotice(`${error.message}. Nothing was sent.`)
        })
      return undefined
    },
  }
}

/** What the standalone key encoder can say about itself. */
export type StandaloneEncoder =
  | { readonly _tag: 'Loading' }
  | { readonly _tag: 'Ready'; readonly encoder: TerminalKeyEncoder }
  | { readonly _tag: 'Failed'; readonly message: string }

// One encoder for the page while any projected pane types through the gateway; released when none does.
const standaloneEncoder = Atom.make(
  Effect.acquireRelease(
    Effect.tryPromise(async () => makeGhosttyKeyboardOn(await loadGhosttyNative())),
    (encoder) => Effect.sync(() => encoder.dispose()),
  ),
).pipe(Atom.setIdleTTL(0))
const idle: Atom.Atom<AsyncResult.AsyncResult<TerminalKeyEncoder, unknown>> = Atom.make(
  AsyncResult.initial<TerminalKeyEncoder, unknown>(),
).pipe(Atom.keepAlive)

/** The native key encoder for a pane without an emulator (the projected screen); loaded only while `active`. */
export const useStandaloneEncoder = (active: boolean): StandaloneEncoder => {
  const atom: Atom.Atom<AsyncResult.AsyncResult<TerminalKeyEncoder, unknown>> = active ? standaloneEncoder : idle
  const result = useAtomValue(atom)
  if (AsyncResult.isSuccess(result)) return { _tag: 'Ready', encoder: result.value }
  if (AsyncResult.isFailure(result)) return { _tag: 'Failed', message: 'The terminal key encoder could not be loaded. No keystrokes were sent.' }
  return { _tag: 'Loading' }
}
