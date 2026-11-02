import * as stylex from '@stylexjs/stylex'
import * as React from 'react'
import * as Aria from 'react-aria-components'

import { colorVars as c, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { AlertButton } from './AlertButton.tsx'
import { unavailableHarnessAnswers, type HarnessAnswerPort, type HarnessAnswers } from './harness-answer-port.ts'
import type { HarnessAsk, HarnessOption, HarnessQuestion } from './harness-question.ts'

// An omp `ask` the harness waits on, answered by option: one radio group or checkbox group per
// question, tabbed when there are several. Only the options' positions are answers; a free-text
// "Other" or note is answered in the terminal. While the web cannot deliver answers, Send stays
// disabled with the reason beside it, and Open terminal is always there. Nothing here says the
// ask was answered: only the harness moving on clears it.

const UNAVAILABLE = 'Answering from the web is not available yet. Answer in the seat\'s terminal.'

const styles = stylex.create({
  alert: { display: 'flex', flexDirection: 'column', gap: s.sm, fontFamily: t.fontSans, fontSize: t.metaSize, color: c.fg },
  title: { fontWeight: 600 },
  tabList: { display: 'flex', flexWrap: 'wrap', gap: s.xs2, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: c.border, paddingBottom: s.xs2 },
  tab: {
    display: 'inline-flex', alignItems: 'center', minHeight: g.controlSm, paddingInline: s.sm, borderRadius: r.control,
    color: c.fgMuted, maxWidth: '16rem', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', cursor: 'pointer', outline: 'none',
    ':hover': { color: c.fg },
    ':is([data-selected])': { backgroundColor: c.rowActive, color: c.fg },
    ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' },
  },
  panel: { outline: 'none', ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
  fieldset: { margin: 0, padding: 0, borderWidth: 0, minWidth: 0, display: 'flex', flexDirection: 'column', gap: s.xs },
  legend: { padding: 0, marginBottom: s.xs, fontWeight: 600 },
  hint: { fontWeight: 400, color: c.fgMuted },
  option: { display: 'flex', flexDirection: 'column', gap: s.xs2, padding: s.xs, borderRadius: r.control, ':hover': { backgroundColor: c.rowHover } },
  choice: { display: 'flex', alignItems: 'baseline', gap: s.xs, cursor: 'pointer', overflowWrap: 'anywhere' },
  input: { margin: 0, width: g.checkbox, height: g.checkbox, flexShrink: 0, alignSelf: 'center', accentColor: c.primary, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  recommended: { color: c.fgMuted, fontSize: t.metaSize },
  // Indented under the label, past the input.
  aside: { marginInlineStart: `calc(${g.checkbox} + ${s.xs})`, color: c.fgMuted, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  preview: {
    marginBlock: 0, marginInlineStart: `calc(${g.checkbox} + ${s.xs})`, padding: s.xs, borderRadius: r.keyboard, backgroundColor: c.raised,
    borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, fontFamily: t.fontMono, fontSize: t.metaSize,
    whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', maxHeight: g.previewMax, overflowY: 'auto',
  },
  actions: { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: s.sm },
  note: { color: c.fgMuted },
})

/** One option: a native radio or checkbox named by its label, described by its description and preview. */
const Option = ({ question, option, name, checked, onChange }: {
  readonly question: HarnessQuestion
  readonly option: HarnessOption
  readonly name: string
  readonly checked: boolean
  readonly onChange: () => void
}): React.ReactNode => {
  const id = `${name}-${option.position}`
  const described = [option.description ? `${id}-description` : '', option.preview ? `${id}-preview` : ''].filter(Boolean).join(' ')
  return (
    <div {...stylex.props(styles.option)}>
      <label {...stylex.props(styles.choice)}>
        <input
          type={question._tag === 'MultiSelect' ? 'checkbox' : 'radio'}
          name={name}
          value={option.position}
          checked={checked}
          onChange={onChange}
          aria-describedby={described || undefined}
          {...stylex.props(styles.input)}
        />
        <span>{option.label}</span>
        {question.recommended === option.position ? <span {...stylex.props(styles.recommended)}>(recommended)</span> : null}
      </label>
      {/* The harness's text, shown as text: React escapes it and nothing parses it as HTML or Markdown. */}
      {option.description ? <div id={`${id}-description`} {...stylex.props(styles.aside)}>{option.description}</div> : null}
      {option.preview ? <pre id={`${id}-preview`} {...stylex.props(styles.preview)}>{option.preview}</pre> : null}
    </div>
  )
}

/** One question's options; a radio group for one answer, a checkbox group for any. */
const Question = ({ question, name, picked, onPick }: {
  readonly question: HarnessQuestion
  readonly name: string
  readonly picked: readonly number[]
  readonly onPick: (positions: readonly number[]) => void
}): React.ReactNode => {
  const multi = question._tag === 'MultiSelect'
  return (
    <fieldset {...stylex.props(styles.fieldset)}>
      <legend {...stylex.props(styles.legend)}>
        {question.title} <span {...stylex.props(styles.hint)}>{multi ? '(choose any)' : '(choose one)'}</span>
      </legend>
      {question.options.map((option) => (
        <Option key={option.position} question={question} option={option} name={name}
          checked={picked.includes(option.position)}
          onChange={() => onPick(!multi ? [option.position] : picked.includes(option.position)
            ? picked.filter((each) => each !== option.position)
            : [...picked, option.position].sort((a, b) => a - b))} />
      ))}
    </fieldset>
  )
}

/**
 * An omp ask's questions, answered by option. Nothing is chosen up front, the recommended option
 * included. Send is disabled while the port is unavailable; Open terminal always stays.
 */
export const HarnessAnswerAlert = ({
  ask,
  port = unavailableHarnessAnswers,
  onOpenTerminal,
}: {
  readonly ask: HarnessAsk
  readonly port?: HarnessAnswerPort
  readonly onOpenTerminal?: () => void
}): React.ReactNode => {
  const base = React.useId()
  const noteId = `${base}-note`
  const [answers, setAnswers] = React.useState<HarnessAnswers>({})
  const [tab, setTab] = React.useState<string | undefined>(ask.questions[0]?.id)
  const [status, setStatus] = React.useState<string>()
  const [sending, setSending] = React.useState(false)
  const picked = (question: HarnessQuestion) => answers[question.id]?.positions ?? []
  const fields = ask.questions.map((each) => (
    <Question key={each.id} question={each} name={`${base}-${each.id}`} picked={picked(each)}
      onPick={(positions) => setAnswers((previous) => ({ ...previous, [each.id]: { positions } }))} />
  ))
  const complete = ask.questions.length > 0 && ask.questions.every((question) => question._tag === 'MultiSelect' || picked(question).length === 1)
  const unavailable = port.availability === 'unavailable'
  const send = () => {
    if (unavailable || !complete) return
    setSending(true)
    void port.answerHarnessQuestion({ callId: ask.callId, answers }).then((result) => {
      setSending(false)
      // The port reports only why an answer did not go; it never reports one as delivered.
      setStatus(result.detail)
    })
  }
  return (
    <section role="group" aria-label={ask.title} {...stylex.props(styles.alert)}>
      <div {...stylex.props(styles.title)}>◆ {ask.title}</div>
      {ask.questions.length > 1 ? (
        <Aria.Tabs selectedKey={tab} onSelectionChange={(key) => setTab(String(key))}>
          <Aria.TabList aria-label="Questions" {...stylex.props(styles.tabList)}>
            {ask.questions.map((each) => (
              <Aria.Tab key={each.id} id={each.id} {...stylex.props(styles.tab)}>
                {each.title}{picked(each).length > 0 ? ' ✓' : ''}
              </Aria.Tab>
            ))}
          </Aria.TabList>
          {ask.questions.map((each, index) => (
            <Aria.TabPanel key={each.id} id={each.id} {...stylex.props(styles.panel)}>{fields[index]}</Aria.TabPanel>
          ))}
        </Aria.Tabs>
      ) : fields[0] ?? null}
      <div {...stylex.props(styles.actions)}>
        <AlertButton tone="primary" isDisabled={unavailable || !complete || sending} aria-describedby={noteId} onPress={send}>
          Send answers
        </AlertButton>
        <AlertButton tone="secondary" isDisabled={onOpenTerminal === undefined} aria-describedby={onOpenTerminal === undefined ? `${base}-terminal` : undefined} onPress={onOpenTerminal}>
          Open terminal
        </AlertButton>
      </div>
      <div id={noteId} {...stylex.props(styles.note)}>{unavailable ? UNAVAILABLE : 'Choose an option for every question, then send.'}</div>
      {onOpenTerminal === undefined ? <div id={`${base}-terminal`} {...stylex.props(styles.note)}>This seat's terminal is not available in this view.</div> : null}
      {status ? <div role="status" {...stylex.props(styles.note)}>{status}</div> : null}
    </section>
  )
}
