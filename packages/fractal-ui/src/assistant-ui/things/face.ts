// Controlled presentation; no fixture imports, no routes, no stores.
/**
 * One face per thing. Every kind projects its decoded model into this shape once, and the kit draws every dense
 * representation (row, fact row, inline chip, panel section, hover-card body) from it. Kinds never draw rows of their
 * own, so status, age, icon, colour and truncation cannot drift per kind.
 */
import type * as React from 'react'
import type { IconName } from '../composition/Icons'

/** Neutral state mark: ● filled (doing something), ○ hollow (exists, at rest), × ended, nothing when unknown. */
export type MarkShape = 'filled' | 'hollow' | 'ended' | 'none'
export interface ThingMark {
  readonly shape: MarkShape
  /** The wire word, humanised: `working`, `merged`, `exited 1`. A row's first meta to give way. */
  readonly word: string
}

/** Colour means act, and only act: amber when you must act, red when it failed. */
export type ActTone = 'act' | 'failed'
export interface ThingAct {
  readonly tone: ActTone
  /** The specific ask or cause: `pick a naming scheme`, `exit 101`, `checks failing`. Never one word per tone. */
  readonly label: string
}

export type Known<A> = { readonly _tag: 'Known'; readonly value: A } | { readonly _tag: 'Unknown'; readonly reason: string }

export interface ThingFact {
  readonly label: string
  readonly value: Known<string>
  readonly mono?: boolean
}

export interface ThingFace {
  /** Human kind label, never a wire kind: `Pull request`, not `vcs.pull-request`. */
  readonly kind: string
  readonly icon: IconName
  readonly title: string
  /** `path` titles are mono and keep their last segment when they truncate. */
  readonly titleStyle?: 'prose' | 'path'
  /**
   * The id people say out loud, beside the title: `#214` for a pull request, issue or CI run. Only those kinds set
   * it; pty handles, subagent types and other handles live in the card's facts.
   */
  readonly id?: string
  readonly mark: ThingMark
  readonly act?: ThingAct
  /**
   * The kind's own moment, shown bare on rows (`4m`) and with its verb on cards (`updated 4m`): missions their last
   * update, terminals and subagents their start, pull requests and issues their last GitHub update, CI runs their
   * finish (start while running).
   */
  readonly at?: { readonly at: number; readonly verb: string }
  /** One sentence for the card: last progress, latest line, failing check. */
  readonly summary?: string
  readonly progress?: { readonly done: number; readonly total: number }
  readonly facts?: readonly ThingFact[]
}

/** What a representation knows before it has a face: enough to hold the row's shape. */
export interface ThingHint {
  readonly kind: string
  readonly icon: IconName
  readonly title?: string
}

/** Every representation draws all four states the same way. */
export type Observed<A> =
  | { readonly _tag: 'Loading'; readonly hint: ThingHint }
  /**
   * `stale`: the last observation may be out of date; the age reads `seen 25m` and every colour stays.
   * `readOnly`: you can see it but not act on it, so an amber ask is drawn without colour.
   */
  | { readonly _tag: 'Known'; readonly value: A; readonly observedAt: number; readonly freshness: 'live' | 'stale'; readonly readOnly?: boolean }
  | { readonly _tag: 'Unavailable'; readonly hint: ThingHint; readonly reason: string }
  /** The record arrived but did not decode: the hint is shown with the issue. */
  | { readonly _tag: 'Invalid'; readonly hint: ThingHint; readonly issue: string }

interface OpenModifiers {
  readonly metaKey: boolean
  readonly ctrlKey: boolean
}

/**
 * Structurally the workspace's `OpenableProps`, widened for pointer drags: click, Cmd-click, Enter, Alt-Enter and a
 * drag in one spread. A host drags either with `useEntityDrag`'s `onPointerDown` or with native `draggable` +
 * `onDragStart`. Absent means the thing has no durable ref: it is a fact, not a destination, and the row draws no
 * hover fill, pointer, focus stop, hover card or drag.
 */
export interface ThingOpenable {
  readonly onClick: (event: OpenModifiers) => void
  readonly onKeyDown: (event: OpenModifiers & { readonly key: string; readonly altKey: boolean; readonly shiftKey: boolean; readonly preventDefault: () => void }) => void
  readonly onPointerDown?: (event: React.PointerEvent<Element>) => void
  readonly draggable?: true
  readonly onDragStart?: (event: { readonly dataTransfer: Pick<DataTransfer, 'setData' | 'effectAllowed'> }) => void
}
