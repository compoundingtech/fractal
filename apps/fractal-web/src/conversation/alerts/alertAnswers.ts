import type * as Native from '@fractal/client/schema'
import * as React from 'react'

// Answer visibility is independent of the gateway's action and stream latency, as in the Fractal terminal UI
// (crates/fractal-tui/src/ui/answers.rs). An answer leaves the screen when it is sent, not when the server confirms it:
// the alert stays hidden through stale stream frames, is done only when the action was accepted AND the feed no longer
// holds the alert, and comes back with an error flash when refused, never taking focus or anyone's draft with it.
// The person's choice and words live here too, keyed by alert, so they survive moving between views.

export type AnswerResult = { readonly _tag: 'Sent'; readonly settled?: boolean } | { readonly _tag: 'Refused'; readonly detail: string }

/** What a choice is about: an alert as the person read it. */
export type AnswerScope = { readonly id: string; readonly revision: string }

export type PendingAnswer = {
  readonly item: Native.Attention
  /** What the person answered, in words. */
  readonly label: string
  readonly phase: 'sending' | 'sent' | 'confirmed'
}

type Draft = {
  readonly words: string
  /** The revision `selected` was made on: a new revision may ask something new. Words outlive it, they are the person's. */
  readonly revision?: string
  readonly selected?: Readonly<Record<string, readonly string[]>>
  readonly step?: number
}

export type AlertFlash = { readonly id: string; readonly title: string; readonly detail: string }

export type AlertAnswersState = {
  readonly pending: ReadonlyMap<string, PendingAnswer>
  readonly drafts: ReadonlyMap<string, Draft>
  readonly failed: ReadonlyMap<string, { readonly revision: string; readonly detail: string }>
  readonly flash?: AlertFlash
  /** Sources that settle an answer themselves (a replay never drops it from its feed). */
  readonly settled: ReadonlySet<string>
}

type Landing = { readonly landed: () => unknown; readonly cancel: () => void }

const RECORD_MS = 4_000

const without = <V,>(map: ReadonlyMap<string, V>, key: string): ReadonlyMap<string, V> => {
  const next = new Map(map)
  next.delete(key)
  return next
}
const withEntry = <V,>(map: ReadonlyMap<string, V>, key: string, value: V): ReadonlyMap<string, V> => new Map(map).set(key, value)

/**
 * Whether the person's answer still covers this alert. A harness can ask anew behind the same alert id (a new episode):
 * that is a new prompt waiting on them, not a stale frame of the one they answered.
 */
export const hidesAlert = (state: AlertAnswersState, item: Native.Attention): boolean => {
  if (state.settled.has(item.id)) return true
  const pending = state.pending.get(item.id)
  return pending !== undefined && (item.attention_kind === 'harness-prompt'
    ? pending.item.episode === item.episode
    : pending.item.revision === item.revision)
}

export type AlertAnswers = ReturnType<typeof createAlertAnswers>

export const createAlertAnswers = () => {
  let state: AlertAnswersState = { pending: new Map(), drafts: new Map(), failed: new Map(), settled: new Set() }
  let seen: ReadonlySet<string> | undefined
  const landings = new Map<string, Landing>()
  const listeners = new Set<() => void>()
  const set = (next: AlertAnswersState) => { state = next; for (const listener of listeners) listener() }
  const draft = (id: string): Draft => state.drafts.get(id) ?? { words: '' }
  const putDraft = (id: string, next: Draft) => set({ ...state, drafts: withEntry(state.drafts, id, next) })
  const confirm = (id: string) => {
    const pending = state.pending.get(id)
    if (pending === undefined || pending.phase === 'confirmed') return
    set({ ...state, pending: withEntry(state.pending, id, { ...pending, phase: 'confirmed' }), drafts: without(state.drafts, id), failed: without(state.failed, id) })
    globalThis.setTimeout(() => {
      if (state.pending.get(id)?.phase === 'confirmed') set({ ...state, pending: without(state.pending, id) })
    }, RECORD_MS)
  }
  const land = (id: string) => {
    const landing = landings.get(id)
    if (landing === undefined) return
    landings.delete(id)
    landing.landed()
  }
  const finish = (answer: PendingAnswer, result: AnswerResult) => {
    const id = answer.item.id
    const pending = state.pending.get(id)
    // The same alert can ask again while an earlier answer is still in flight. Its completion owns only its entry.
    if (pending !== answer) return
    if (result._tag === 'Refused') {
      // Rollback: the alert returns where the feed holds it. Selection, focus and every draft stay as they are.
      set({
        ...state,
        pending: without(state.pending, id),
        failed: withEntry(state.failed, id, { revision: pending.item.revision, detail: result.detail }),
        flash: { id, title: pending.item.title, detail: result.detail },
      })
      return
    }
    set({ ...state, pending: withEntry(state.pending, id, { ...pending, phase: 'sent' }), ...(result.settled === true ? { settled: new Set([...state.settled, id]) } : {}) })
    if (result.settled === true || seen?.has(id) === false) confirm(id)
  }
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    hidden: (item: Native.Attention): boolean => hidesAlert(state, item),
    /** Hides the alert now, runs the answer once, and settles it from the result. */
    send: (item: Native.Attention, label: string, run: () => Promise<AnswerResult>): void => {
      if (hidesAlert(state, item)) return
      const failed = without(state.failed, item.id)
      const answer: PendingAnswer = { item, label, phase: 'sending' }
      set({ ...state, failed, pending: withEntry(state.pending, item.id, answer), ...(state.flash?.id === item.id ? { flash: undefined } : {}) })
      run().then(
        (result) => finish(answer, result),
        () => finish(answer, { _tag: 'Refused', detail: 'The answer could not be confirmed; check the alert before answering again. It was not sent again.' }),
      )
    },
    /** Every authoritative frame. An answer is done once it was accepted and the frame no longer holds its alert. */
    observe: (items: readonly Native.Attention[]): void => {
      seen = new Set(items.filter((item) => item.state === 'open').map((item) => item.id))
      for (const id of [...landings.keys()]) if (!seen.has(id)) land(id)
      for (const [id, pending] of state.pending) if (pending.phase === 'sent' && !seen.has(id)) confirm(id)
    },
    /** `alert_landed`: times the authoritative feed clearing, not the local hide. */
    watchLanding: (id: string, landing: Landing): void => {
      landings.set(id, landing)
      if (seen?.has(id) === false) land(id)
    },
    words: (id: string): string => draft(id).words,
    setWords: (id: string, words: string): void => putDraft(id, { ...draft(id), words }),
    /** The chosen answers on this revision, or undefined when the person has not chosen on it. */
    selected: (item: AnswerScope, step = 'main'): readonly string[] | undefined => {
      const current = draft(item.id)
      return current.revision === item.revision ? current.selected?.[step] : undefined
    },
    setSelected: (item: AnswerScope, ids: readonly string[], step = 'main'): void => {
      const current = draft(item.id)
      const kept = current.revision === item.revision ? current.selected : undefined
      putDraft(item.id, { ...current, revision: item.revision, selected: { ...kept, [step]: ids } })
    },
    step: (item: AnswerScope): number => {
      const current = draft(item.id)
      return current.revision === item.revision ? current.step ?? 0 : 0
    },
    setStep: (item: AnswerScope, step: number): void => {
      const current = draft(item.id)
      putDraft(item.id, { ...current, revision: item.revision, selected: current.revision === item.revision ? current.selected : undefined, step })
    },
    /** Why the last answer on this revision failed. */
    failure: (item: AnswerScope): string | undefined => {
      const failed = state.failed.get(item.id)
      return failed?.revision === item.revision ? failed.detail : undefined
    },
    clearFlash: (): void => { if (state.flash !== undefined) set({ ...state, flash: undefined }) },
  }
}

/** The page's one store; a story or test provides its own. */
export const AlertAnswersContext = React.createContext<AlertAnswers>(createAlertAnswers())

/** The store, re-rendering on any change to it. */
export const useAlertAnswersStore = (): AlertAnswers => {
  const store = React.useContext(AlertAnswersContext)
  React.useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot)
  return store
}
