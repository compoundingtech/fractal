import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Button } from 'react-aria-components'
import { Transcript, type TranscriptHistory, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { TextItem, ToolCallItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { interactionVars as interaction, borderVars as border, geometryVars as g, radiusVars as r, surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { committedTranscriptHistory } from './story-play'
import type { SyncStatus } from './assistant-ui/sync-presentation/sync-status'
const now = Date.parse('2026-01-15T12:30:00Z')
const minute = (index: number) => new Date(Date.parse('2026-01-15T12:00:00Z') + index * 60_000).toISOString()
const prompts = ['Summarize the failing selection check.', 'Keep visible rows grouped after filtering.', 'Explain why the empty state flickers.', 'Check the count after a row is hidden.']
const answers = ['The selection check compares the projected row with the stored selection and reports the first mismatch.', 'Visible rows now stay grouped: the filter runs before grouping, so hidden rows never split a group.', 'The empty state rendered before the first snapshot arrived. It now waits for the initial snapshot.', 'The count follows the visible rows, so hiding a row lowers it by one and selection stays on the same row.']

function turn(index: number, prompt: string | undefined, answer: string | undefined, { failed = false, tool }: { failed?: boolean; tool?: ToolCallItem } = {}): TranscriptTurn {
  const id = `turn/${index}`
  const user: (TextItem & { role: 'user' }) | undefined = prompt === undefined ? undefined : { _tag: 'Text', id: `${id}/prompt`, role: 'user', text: prompt, attachments: [], streaming: false, at: minute(index), sender: { kind: 'human', label: 'Operator' }, sendState: failed ? { _tag: 'Failed', reason: { _tag: 'Failed' }, detail: 'The send request timed out.' } : undefined }
  const reply: TextItem | undefined = answer === undefined ? undefined : { _tag: 'Text', id: `${id}/answer`, role: 'assistant', text: answer, attachments: [], streaming: false, at: minute(index), sender: { kind: 'agent', label: 'Assistant' } }
  const items = [...(tool === undefined ? [] : [tool]), ...(reply === undefined ? [] : [reply])]
  return { id, prompt: user, items, work: workLogTurnFromItems(user === undefined ? items : [user, ...items], { kindFor: () => 'read', running: false, failed: false, interrupted: false, startedAt: minute(index), completeHistory: true }) }
}
function toolCall(index: number, output?: string): ToolCallItem {
  return { _tag: 'ToolCall', id: `turn/${index}/tool`, callId: `call-${index}`, name: 'read_file', input: { path: 'src/rows.ts' }, status: 'success', callSeen: true, at: minute(index), ...(output === undefined ? {} : { result: { content: output, isError: false, at: minute(index) } }) }
}
const historyTurn = (index: number, revise: { answer?: string; tool?: ToolCallItem } = {}) => turn(index, prompts[index % prompts.length], revise.answer ?? answers[index % answers.length], { tool: revise.tool ?? (index === 22 ? toolCall(22) : undefined) })
export const initialTurns = [...Array.from({ length: 40 }, (_, index) => historyTurn(index)), turn(40, 'Ship the grouped rows.', undefined, { failed: true })]
/** The host's initial turns, older-history status and deterministic clock. */
interface PressHistory { readonly turns: readonly TranscriptTurn[]; readonly history: TranscriptHistory; readonly now: number }
const fixtureHistory: PressHistory = { turns: initialTurns, history: { _tag: 'Complete' }, now }
const replacing = (next: TranscriptTurn) => (previous: readonly TranscriptTurn[]) => previous.map(entry => entry.id === next.id ? next : entry)

export function PressStory({ scheme, initial = fixtureHistory, revisions = true }: { scheme: Scheme; initial?: PressHistory; revisions?: boolean }) {
  const [turns, setTurns] = React.useState(initial.turns)
  const [retries, setRetries] = React.useState(0)
  const next = React.useRef(initial.turns.length)
  const sync = React.useMemo((): SyncStatus => ({ _tag: 'Live', since: initial.now - 5000 }), [initial.now])
  const messages = React.useMemo(() => turns.flatMap(entry => entry.prompt === undefined ? entry.items : [entry.prompt, ...entry.items]), [turns])
  const options = React.useMemo(() => ({ messages, isRunning: false, onNew: async () => {} }), [messages])
  // A host republishing its snapshot decodes fresh objects for the same rows.
  const republish = React.useCallback(() => setTurns(previous => structuredClone(previous)), [])
  const appendReply = React.useCallback(() => {
    const index = next.current++
    setTurns(previous => [...previous, turn(index, undefined, answers[index % answers.length])])
  }, [])
  const insertReply = React.useCallback(() => {
    const index = next.current++
    setTurns(previous => [...previous.slice(0, -1), turn(index, undefined, 'The visible rows are grouped and ready for review.  \nThe failed send keeps its original place.'), previous[previous.length - 1]!])
  }, [])
  const retry = React.useCallback(() => setRetries(count => count + 1), [])
  // Same length, different words: a length-only revision would miss it.
  const reviseAnswer = React.useCallback(() => setTurns(replacing(historyTurn(23, { answer: [...answers[23 % answers.length]!].reverse().join('') }))), [])
  const attachResult = React.useCallback(() => setTurns(replacing(historyTurn(22, { tool: toolCall(22, 'export const rows = groupVisible(filter(rows))') }))), [])
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><EmbraceRuntimeProvider options={options}>
    <div {...stylex.props(styles.toolbar)}><Button onPress={republish} {...stylex.props(styles.button)}>Republish snapshot</Button><Button onPress={appendReply} {...stylex.props(styles.button)}>Append agent reply</Button>{revisions && <><Button onPress={insertReply} {...stylex.props(styles.button)}>Insert reply above failed send</Button><Button onPress={reviseAnswer} {...stylex.props(styles.button)}>Revise answer</Button><Button onPress={attachResult} {...stylex.props(styles.button)}>Attach tool result</Button></>}<span>Retries <output data-testid="retry-count">{retries}</output></span></div>
    <div {...stylex.props(styles.transcript)}><Transcript title="Row projection" turns={turns} history={initial.history} sync={sync} now={initial.now} observedAt={initial.now - 8000} onRetrySend={retry} /></div>
  </EmbraceRuntimeProvider></main>
}
export async function settleFrames() {
  // The package lib is ES2022 (no Promise.withResolvers). Three frames cover the viewport's scheduled write and capture.
  for (let frame = 0; frame < 3; frame++) await new Promise(resolve => requestAnimationFrame(resolve))
}
export async function ready(canvasElement: HTMLElement, abortSignal: AbortSignal) {
  await document.fonts.ready
  const canvas = within(canvasElement)
  // Press stability exercises older rows, which tail-first mounting deliberately defers.
  await userEvent.click(await canvas.findByRole('button', { name: 'Show all messages' }))
  const viewport = await committedTranscriptHistory(canvasElement, initialTurns.length, abortSignal)
  await waitFor(() => expect(viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop).toBeLessThanOrEqual(2))
  return { viewport, canvas, jump: canvas.getByText('New messages ↓'), retries: canvas.getByTestId('retry-count') }
}
export async function readerScrollsUp(viewport: HTMLElement) {
  viewport.dispatchEvent(new WheelEvent('wheel', { deltaY: -viewport.scrollHeight }))
  viewport.scrollTop = 0
  await settleFrames()
}
const styles = stylex.create({
  root: { height: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  transcript: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column' },
  toolbar: { display: 'flex', alignItems: 'center', gap: s.md, padding: s.md, flexShrink: 0, fontSize: t.metaSize },
  button: { minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
})
