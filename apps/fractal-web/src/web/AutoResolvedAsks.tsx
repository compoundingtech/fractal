import type { AutoResolvedAsk } from '@fractal/client'
import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import type { AutoResolvedRead, OverrideAction } from '../conversation/alerts/alert-action-port.ts'
import type { Override } from '../conversation/alerts/useAutoResolved.ts'

// The Alerts tab's Auto-resolved section: asks a default answered at their deadline in the last seven days. While
// Fractal still offers it, the person answers one with one of its other named answers in a single click.

/** The verbatim consequence every client shows before an override. */
export const overrideCopy = (asker: string): string =>
  `Your answer replaces the default. Work already done on the default is not undone; ${asker} is told.`

type Answer = NonNullable<NonNullable<AutoResolvedAsk['request']>['answers']>[number]

/** The named answers that would replace the default, or none when Fractal does not offer an override. */
export const overrideAnswers = (ask: AutoResolvedAsk): readonly Answer[] =>
  ask.action_parameters?.['work.override'] === undefined
    ? []
    : (ask.request?.answers ?? []).filter((answer) => answer.id !== ask.default_answer?.id)

type RowState =
  | { readonly _tag: 'Sending'; readonly answer: string }
  | { readonly _tag: 'Sent'; readonly answer: string }
  | { readonly _tag: 'Refused'; readonly detail: string }

const NONE: readonly AutoResolvedAsk[] = []

/** The section, from a read: nothing while unread, refused to a non-person, or empty; a failed read says so. */
export const AutoResolvedSection = ({ read, now, onOverride, onSelect }: {
  readonly read: AutoResolvedRead | undefined
  readonly now: number
  /** Absent on a device without control access: rows still show what the default was. */
  readonly onOverride?: Override
  /** Opens the exact resolved source for an independent discussion. */
  readonly onSelect?: (id: string) => void
}): React.ReactNode => {
  const id = React.useId()
  const heading = React.useRef<HTMLHeadingElement>(null)
  const list = React.useRef<HTMLUListElement>(null)
  const [states, setStates] = React.useState<ReadonlyMap<string, RowState>>(new Map())
  const asks = read?._tag === 'Observed' ? read.asks : NONE
  // Once it listed asks, the section stays when the last one leaves, so focus and the person's place stay in it.
  const [listed, setListed] = React.useState(false)
  if (asks.length > 0 && !listed) setListed(true)
  // A row leaves on the read after its override is accepted. If focus was in it, it moves to the next row, else the
  // previous one, else the heading, so the person stays in the section.
  const focused = React.useRef<string | undefined>(undefined)
  const shown = React.useRef<readonly string[]>([])
  React.useLayoutEffect(() => {
    const before = shown.current
    const ids = asks.map((ask) => ask.resolution_id)
    shown.current = ids
    const lost = focused.current
    if (lost === undefined || ids.includes(lost)) return
    focused.current = undefined
    const active = document.activeElement
    if (active !== null && active !== document.body) return
    const at = before.indexOf(lost)
    const next = [...before.slice(at + 1), ...before.slice(0, Math.max(at, 0)).reverse()].find((each) => ids.includes(each))
    const row = next === undefined ? undefined : [...(list.current?.querySelectorAll<HTMLElement>('[data-auto-resolved-id]') ?? [])].find((each) => each.dataset.autoResolvedId === next)
    ;(row?.querySelector<HTMLElement>('button') ?? heading.current)?.focus()
  }, [asks])
  // An empty read hides the section only when it is complete: one that may be missing asks says so even with none listed.
  if (read === undefined || (read._tag === 'Unavailable' && read.reason === 'absent') || (read._tag === 'Observed' && asks.length === 0 && !listed && read.complete)) return null
  const set = (key: string, state: RowState) => setStates((prior) => new Map(prior).set(key, state))
  const override = (ask: AutoResolvedAsk, answer: OverrideAction['answer'], label: string) => {
    if (onOverride === undefined || states.get(ask.resolution_id)?._tag === 'Sending') return
    set(ask.resolution_id, { _tag: 'Sending', answer: label })
    void onOverride(ask, answer).then((result) => {
      set(ask.resolution_id, result._tag === 'Sent' ? { _tag: 'Sent', answer: label } : { _tag: 'Refused', detail: result.detail })
    })
  }
  return (
    <section aria-labelledby={id} data-auto-resolved="" {...stylex.props(styles.section)}>
      <h3 ref={heading} id={id} tabIndex={-1} {...stylex.props(styles.heading)}>Auto-resolved</h3>
      {read._tag === 'Unavailable'
        ? <p role="status" {...stylex.props(styles.note)}>Auto-resolved asks could not be loaded; reconnect and try again.</p>
        : <>
          {asks.length === 0
            ? listed ? <p {...stylex.props(styles.note)}>Nothing else was auto-resolved.</p> : null
            : <ul ref={list} aria-labelledby={id} {...stylex.props(styles.list)}>
              {asks.map((ask) => (
                <AutoResolvedRow
                  key={ask.resolution_id}
                  ask={ask}
                  now={now}
                  state={states.get(ask.resolution_id)}
                  onFocusWithin={() => { focused.current = ask.resolution_id }}
                  {...(onSelect === undefined ? {} : { onSelect: () => onSelect(`auto-resolved:${ask.resolution_id}`) })}
                  {...(onOverride === undefined ? {} : { onOverride: (answer, label) => override(ask, answer, label) })}
                />
              ))}
            </ul>}
          {read.complete ? null : <p {...stylex.props(styles.note)}>Older auto-resolved asks may be missing from this list.</p>}
        </>}
    </section>
  )
}

const AutoResolvedRow = ({ ask, now, state, onFocusWithin, onOverride, onSelect }: {
  readonly ask: AutoResolvedAsk
  readonly now: number
  readonly state: RowState | undefined
  readonly onFocusWithin: () => void
  readonly onOverride?: (answer: OverrideAction['answer'], label: string) => void
  readonly onSelect?: () => void
}): React.ReactNode => {
  const touch = React.useContext(NarrowLayoutContext)
  const copyId = React.useId()
  const wordsId = React.useId()
  // A request for changes carries the person's words (the ask rejects one without them): its button opens a field and
  // Send answers once there are words. Every other answer is sent on its one click.
  const [drafting, setDrafting] = React.useState<Answer>()
  const [words, setWords] = React.useState('')
  // The asker as this client names agents everywhere (`alertFrom`): its id without the `agent/` prefix.
  const asker = ask.requester_id.replace(/^agent\//, '')
  const age = ago(ask.resolved_at, now)
  const answers = onOverride === undefined ? [] : overrideAnswers(ask)
  // The row's answers hold while one is in flight, and once one is sent; a refusal frees them again. They stay focusable
  // (aria-disabled, not disabled) so focus is not dropped while the answer travels.
  const busy = state?._tag === 'Sending' || state?._tag === 'Sent'
  const applied = ask.default_answer === undefined ? 'Resolved at its deadline' : `Default applied: ${ask.default_answer.label}`
  return (
    <li data-auto-resolved-id={ask.resolution_id} data-override-state={state?._tag.toLowerCase()} onFocus={onFocusWithin} {...stylex.props(styles.row)}>
      <span {...stylex.props(styles.line)}>
        {onSelect === undefined ? <span {...stylex.props(styles.title)}>{ask.title}</span>
          : <Aria.Button onPress={onSelect} {...stylex.props(styles.title, styles.open)}>{ask.title}</Aria.Button>}{' '}
        <span aria-label={`resolved ${age} ago`} {...stylex.props(styles.metric)}>{age}</span>
      </span>
      <span {...stylex.props(styles.meta)}>{applied} · {asker}</span>
      {answers.length === 0 ? null : <>
        <div role="group" aria-label={`Answer “${ask.title}” instead`} aria-describedby={copyId} {...stylex.props(styles.answers)}>
          {answers.map((answer) => {
            const takesWords = answer.outcome === 'request_changes'
            const open = drafting?.id === answer.id
            return (
              <Aria.Button
                key={answer.id}
                aria-disabled={busy || undefined}
                {...(takesWords ? { 'aria-expanded': open, ...(open ? { 'aria-controls': wordsId } : {}) } : {})}
                onPress={() => {
                  if (busy) return
                  if (takesWords) setDrafting(open ? undefined : answer)
                  else onOverride?.({ id: answer.id }, answer.label)
                }}
                {...stylex.props(styles.answer, touch && styles.touch, open && styles.answerOpen, busy && styles.answerBusy)}
              >
                {answer.label}
              </Aria.Button>
            )
          })}
        </div>
        {drafting === undefined ? null : (
          <div id={wordsId} {...stylex.props(styles.words)}>
            <Aria.TextField aria-label={`What should change, for “${drafting.label}”`} value={words} onChange={setWords} {...stylex.props(styles.wordsField)}>
              <Aria.TextArea autoFocus rows={2} placeholder="What should change?" {...stylex.props(styles.field, touch && styles.fieldTouch)} />
            </Aria.TextField>
            <Aria.Button
              isDisabled={words.trim() === ''}
              aria-disabled={busy || undefined}
              onPress={() => { if (!busy) onOverride?.({ id: drafting.id, text: words.trim() }, drafting.label) }}
              {...stylex.props(styles.answer, styles.send, touch && styles.touch, busy && styles.answerBusy)}
            >
              Send “{drafting.label}”
            </Aria.Button>
          </div>
        )}
        <p id={copyId} {...stylex.props(styles.copy)}>{overrideCopy(asker)}</p>
      </>}
      {state === undefined ? null
        : state._tag === 'Refused' ? <p role="alert" {...stylex.props(styles.status, styles.statusError)}>{state.detail}</p>
          : <p role="status" {...stylex.props(styles.status)}>{state._tag === 'Sending' ? `Sending “${state.answer}”…` : `Sent “${state.answer}”; ${asker} is told.`}</p>}
    </li>
  )
}

const focus = { ':focus-visible': { outline: `2px solid ${c.primary}`, outlineOffset: '-2px' } }
const styles = stylex.create({
  section: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingBlock: s.md, borderTop: `1px solid ${c.border}` },
  heading: { margin: 0, paddingInline: s.row, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: c.fgMuted, outline: 'none', ...focus },
  list: { margin: 0, padding: 0, listStyle: 'none', display: 'flex', flexDirection: 'column', gap: s.xs2 },
  row: { display: 'flex', flexDirection: 'column', gap: s.xs, paddingBlock: s.sm, paddingInline: s.row, borderRadius: 4, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, color: c.fg },
  line: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0 },
  title: { flexGrow: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  open: { borderWidth: 0, padding: 0, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, textAlign: 'start', cursor: 'pointer', ...focus },
  metric: { flexShrink: 0, color: c.fgSoft, fontSize: t.metaSize, fontVariantNumeric: 'tabular-nums' },
  meta: { color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  answers: { display: 'flex', flexWrap: 'wrap', gap: s.xs },
  answer: {
    display: 'inline-flex', alignItems: 'center', minHeight: g.controlSm, paddingInline: s.md, borderWidth: 1, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.sm,
    backgroundColor: c.controlFill, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, fontWeight: t.weightMedium, cursor: 'pointer', whiteSpace: 'nowrap',
    ':hover': { backgroundColor: c.rowHover }, ...focus,
  },
  answerBusy: { opacity: 0.45, cursor: 'default', ':hover': { backgroundColor: c.controlFill } },
  answerOpen: { borderColor: c.primary },
  words: { display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: s.xs },
  wordsField: { display: 'flex', alignSelf: 'stretch' },
  field: {
    flexGrow: 1, minWidth: 0, minHeight: g.controlMd, maxHeight: g.editorMax, boxSizing: 'border-box', resize: 'vertical', paddingBlock: s.xs, paddingInline: s.md,
    borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, backgroundColor: 'transparent', color: c.fg,
    fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, '::placeholder': { color: c.fgSoft }, ':focus-visible': { outline: 'none', borderColor: c.primary },
  },
  fieldTouch: { minHeight: g.touchTarget, fontSize: t.touchInputSize, lineHeight: t.uiLeading },
  send: { ':disabled': { opacity: 0.45, cursor: 'default' } },
  touch: { minHeight: g.touchTarget },
  copy: { margin: 0, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  status: { margin: 0, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading, overflowWrap: 'anywhere' },
  statusError: { color: c.dangerFg },
  note: { margin: 0, paddingInline: s.row, color: c.fgSoft, fontSize: t.metaSize, lineHeight: t.metaLeading },
})
