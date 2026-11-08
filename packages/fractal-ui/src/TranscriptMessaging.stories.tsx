/** Transcript messaging v5: cards for people and agents, rows for machine results, plain output for the agent's own work, at 1280 and 440 in both themes. Synthetic host only. */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import type { MessageParticipant, MessageParticipants } from './assistant-ui/composition/MessageCard'
import { NarrowLayoutContext } from './assistant-ui/composition/NarrowLayout'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { Attachment, ConversationItem, EventItem, HarnessNotice, MessageItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const day = '2026-11-08'
const at = (time: string) => `${day}T${time}:00Z`
const now = Date.parse(at('10:45'))
const self = { ref: 'agent/example/harbor-sdk', label: 'harbor-sdk' } as const

// A person's avatar arrives from the host; the story carries it inline so no request leaves the page.
const inesAvatar = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" fill="#c2410c"/><circle cx="20" cy="16" r="7" fill="#fed7aa"/><path d="M6 40a14 14 0 0 1 28 0" fill="#fed7aa"/></svg>')}`
const P = {
  rowan: { label: 'Rowan Vale', ref: 'person/rowan-v', kind: 'person' },
  ines: { label: 'Ines Ortega', ref: 'person/ines-o', kind: 'person', avatarUrl: inesAvatar },
  sdk: { label: 'harbor-sdk', ref: self.ref, kind: 'agent', status: 'working' },
  keysmith: { label: 'keysmith', ref: 'agent/example/keysmith', kind: 'agent', status: 'idle' },
  reviewer: { label: 'reviewer', ref: 'agent/example/reviewer', kind: 'agent', status: 'needs' },
  disk: { label: 'disk-steward', ref: 'agent/fleet/disk-steward', kind: 'agent', status: 'working' },
  rcc: { label: 'release-captain-coordinator-europe-west', ref: 'agent/release/release-captain-coordinator-europe-west', kind: 'agent', status: 'needs' },
  nightly: { label: 'harbor-sdk-integration-tests-nightly', ref: 'agent/example/harbor-sdk-integration-tests-nightly', kind: 'agent', status: 'idle' },
} as const satisfies Readonly<Record<string, MessageParticipant>>

const participants: Record<string, MessageParticipants> = {}
const mail = (id: string, time: string, from: MessageParticipant, to: MessageParticipant, title: string | undefined, text: string | undefined, attachments?: readonly Attachment[]): MessageItem => {
  participants[id] = { from, to, involvesPerson: from.kind === 'person' || to.kind === 'person' }
  return { _tag: 'Message', id, messageId: `message/${id}`, author: from.ref, to: to.ref, at: at(time), ...(title === undefined ? {} : { title }), ...(text === undefined ? {} : { text }), ...(attachments === undefined ? {} : { attachments }) }
}
const notice = (id: string, time: string, kind: EventItem['kind'], title: string, text: string, data: HarnessNotice): EventItem => ({ _tag: 'Event', id, kind, title, text, at: at(time), sender: { kind: 'harness', label: 'OMP' }, data: {}, notice: data })
const prose = (id: string, time: string, text: string): TextItem => ({ _tag: 'Text', id, role: 'assistant', text, attachments: [], streaming: false, at: at(time) })
const file = (index: number, name: string, mediaType: string, size: number): Attachment => ({ id: `blob/${index}`, name, mediaType, size })

const longPlan = Array.from({ length: 10 }, (_, index) => `${index + 1}. Step ${index + 1} re-signs one manifest group and records its proof under the step heading.`).join('\n')
const evidence = [file(1, 'keys-3-diagram.png', 'image/png', 84 * 1024), file(2, 'box3-cleanup.log', 'text/plain', 212 * 1024), file(3, 'manifest-signatures-before-and-after-rotation.png', 'image/png', 1.2 * 1024 * 1024), file(4, 'reader-rename.diff', 'text/x-diff', 18 * 1024), file(5, 'manifests.csv', 'text/csv', 9 * 1024), file(6, 'box3-disk.png', 'image/png', 64 * 1024)]

const prompt: TextItem & { readonly role: 'user' } = { _tag: 'Text', id: 'v5/prompt', role: 'user', text: 'Rotate the reader keys, but keep the old reader for one release.', attachments: [], streaming: false, at: at('10:20'), sender: { kind: 'human', label: 'Rowan Vale' } }
const items: readonly ConversationItem[] = [
  prose('v5/prose', '10:24', 'Rotation step 1 is done. The old reader stays for one release; I opened #231 for its removal.'),
  mail('v5/brief', '10:28', P.keysmith, P.reviewer, 'Brief ready', 'The rotation brief is in doc/keys-3.', [evidence[0]!]),
  mail('v5/to-you', '10:29', P.keysmith, P.rowan, 'Need a call', 'Should step 3 wait for the audit?'),
  mail('v5/header-only', '10:29', P.keysmith, P.sdk, 'Older brief', undefined),
  mail('v5/plan', '10:30', P.keysmith, P.sdk, 'Rotation plan', longPlan),
  mail('v5/long-names', '10:31', P.rcc, P.nightly, 'Freeze window for every train owner in the europe-west region', 'Freeze starts 12:00 UTC. Hold merges to 9c41e0a descendants until I confirm.'),
  mail('v5/stop', '10:32', P.disk, P.sdk, 'Stop: cleanup scope', '**Stop.** The cleanup rule does not cover worktrees under `refs/heads/`.'),
  mail('v5/evidence', '10:32', P.disk, P.sdk, 'Cleanup evidence', 'Evidence for the box3 cleanup, as asked.', evidence),
  prose('v5/prose-2', '10:33', 'The stop arrived after I had deleted one of my four listed folders on box3.'),
  notice('v5/interjection', '10:40', 'interjection', 'Ines Ortega → harbor-sdk', 'Keep the public API unchanged.', { _tag: 'Interjection', from: 'Ines Ortega', to: 'harbor-sdk', payload: 'Keep the public API unchanged.', raw: '' }),
  mail('v5/ines', '10:40', P.ines, P.sdk, undefined, 'And report the timeout separately; do not hide it.'),
  notice('v5/irc', '10:41', 'irc', 'reviewer → harbor-sdk', 'I found no caller that needs the old fallback.', { _tag: 'Irc', from: 'reviewer', to: 'harbor-sdk', payload: 'I found no caller that needs the old fallback.', raw: '' }),
  notice('v5/keycheck', '10:42', 'subagent-result', 'KeyCheck · completed · 1m45s', 'No actionable defects found. Not a release approval.', { _tag: 'SubagentResult', taskId: 'KeyCheck', agent: 'reviewer', status: 'completed', duration: '1m45s', payload: '{"summary":"No actionable defects found."}', raw: '' }),
  notice('v5/job42', '10:42', 'background-job', 'Background job 42 failed', 'Browser launch failed before checks could run.', { _tag: 'BackgroundJob', jobId: 'job_42', label: 'Background job 42', state: 'failed', duration: '47s', payload: 'Browser launch failed before checks could run.\n\nexit 1', raw: '' }),
  notice('v5/audit', '10:43', 'subagent-result', 'ManifestAudit · timed out · 10m00s', 'Stopped after 10m without a verdict.', { _tag: 'SubagentResult', taskId: 'ManifestAudit', agent: 'task', status: 'failed (timed out)', duration: '10m00s', error: 'Stopped after 10m without a verdict.', payload: 'Stopped after 10m without a verdict.', raw: '' }),
  notice('v5/compaction', '10:43', 'compaction', 'Context compacted', 'Automatic · 4 sections', { _tag: 'Compaction', payload: '## Goal\nRotate the reader keys.', raw: '' }),
  prose('v5/prose-3', '10:44', 'Two items still wait on others: the Keysmith brief and the box3 confirmation.'),
]
const turn: TranscriptTurn = {
  id: 'v5/turn', prompt, items,
  work: workLogTurnFromItems([prompt, ...items], { kindFor: () => 'read', running: false, failed: false, interrupted: false, completeHistory: true, startedAt: prompt.at }),
  senderCaptions: Object.fromEntries(items.filter(item => item._tag === 'Text').map(item => [item.id, self.label])),
  participants,
}

type Width = 1280 | 440
function MessagingStory({ scheme, width }: { readonly scheme: Scheme; readonly width: Width }) {
  const options = React.useMemo(() => ({ messages: [prompt, ...items], isRunning: false, onNew: async () => {} }), [])
  return <NarrowLayoutContext.Provider value={width === 440}>
    <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
      <EmbraceRuntimeProvider options={options}><Transcript title="harbor-sdk" turns={[turn]} sync={{ _tag: 'Live', since: now - 5000 }} now={now} observedAt={now} self={self} /></EmbraceRuntimeProvider>
    </main>
  </NarrowLayoutContext.Provider>
}

const card = (root: HTMLElement, id: string) => root.querySelector<HTMLElement>(`[data-item-id="${id}"]`) ?? (() => { throw new Error(`no entry ${id}`) })()
const timeShown = (root: HTMLElement, id: string) => card(root, id).querySelector('[data-testid=message-time]')?.getAttribute('data-time-visible')
const play: Story['play'] = async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  const more = canvas.queryByRole('button', { name: 'Show all messages' })
  if (more !== null) await userEvent.click(more)
  await waitFor(() => expect(card(canvasElement, 'v5/brief')).toBeInTheDocument())
  // Stacks: the same sender to the same recipient folds under one head, 4px apart.
  await expect(card(canvasElement, 'v5/plan')).toHaveAttribute('data-stacked', 'true')
  await expect(card(canvasElement, 'v5/evidence')).toHaveAttribute('data-stacked', 'true')
  await expect(within(card(canvasElement, 'v5/plan')).queryByTestId('message-head')).toBeNull()
  const gap = card(canvasElement, 'v5/plan').getBoundingClientRect().top - card(canvasElement, 'v5/header-only').getBoundingClientRect().bottom
  await expect(Math.round(gap)).toBe(4)
  // Recipient: named only when it is not this transcript's agent.
  await expect(within(card(canvasElement, 'v5/brief')).getByTestId('message-recipient')).toHaveTextContent('reviewer')
  await expect(within(card(canvasElement, 'v5/to-you')).getByTestId('message-recipient')).toHaveTextContent('Rowan Vale')
  for (const id of ['v5/header-only', 'v5/stop', 'v5/interjection', 'v5/irc', 'v5/ines']) await expect(within(card(canvasElement, id)).queryByTestId('message-recipient')).toBeNull()
  // Time: shown when the minute changes, otherwise on hover.
  await expect(timeShown(canvasElement, 'v5/to-you')).toBe('true')
  await expect(timeShown(canvasElement, 'v5/header-only')).toBe('false')
  await userEvent.hover(card(canvasElement, 'v5/header-only'))
  await expect(getComputedStyle(card(canvasElement, 'v5/header-only').querySelector('[data-testid=message-time]')!).opacity).toBe('1')
  // People's interjections are cards; machine results are rows; the agent's own work stays plain output.
  await expect(card(canvasElement, 'v5/interjection')).toHaveAttribute('data-testid', 'message-card')
  await expect(within(card(canvasElement, 'v5/interjection')).getByTestId('message-kind')).toHaveTextContent('Interjection')
  for (const id of ['v5/keycheck', 'v5/job42', 'v5/audit']) await expect(card(canvasElement, id)).toHaveAttribute('data-testid', 'message-row')
  await expect(within(card(canvasElement, 'v5/job42')).queryByTestId('message-kind')).toBeNull()
  await expect(within(card(canvasElement, 'v5/keycheck')).getByTestId('message-kind')).toHaveTextContent('Subagent')
  await expect(card(canvasElement, 'v5/compaction')).toHaveAttribute('data-testid', 'harness-notice')
  await expect(within(card(canvasElement, 'v5/prose')).queryByTestId('message-sender')).toBeNull()
  // Failures and timeouts: red edge and red status word, never a red fill.
  for (const id of ['v5/job42', 'v5/audit']) {
    const row = card(canvasElement, id)
    await expect(row).toHaveAttribute('data-failed', 'true')
    await expect(getComputedStyle(row).backgroundColor).toBe('rgba(0, 0, 0, 0)')
    await expect(getComputedStyle(within(row).getByTestId('message-status')).color).not.toBe(getComputedStyle(within(card(canvasElement, 'v5/keycheck')).getByTestId('message-status')).color)
  }
  // Attachments: more than four show three and +N more.
  const files = within(card(canvasElement, 'v5/evidence'))
  await expect(files.getAllByTestId('transcript-attachment')).toHaveLength(3)
  await expect(files.getByTestId('transcript-attachments-more')).toHaveTextContent('+3 more')
  // Long names truncate; the time stays inside the card.
  const long = card(canvasElement, 'v5/long-names')
  const time = long.querySelector('[data-testid=message-time]')!.getBoundingClientRect()
  await expect(time.right).toBeLessThanOrEqual(long.getBoundingClientRect().right)
  await expect(time.width).toBeGreaterThan(0)
  // The 'to you' edge, the avatar's 8px status dot, a person's real avatar, and the light card surface.
  await expect(card(canvasElement, 'v5/to-you')).toHaveAttribute('data-message-look', 'to_you')
  await expect(card(canvasElement, 'v5/brief')).toHaveAttribute('data-message-look', 'between_others')
  await expect(card(canvasElement, 'v5/stop').querySelector('[data-testid=message-status-dot]')!.getBoundingClientRect().width).toBe(8)
  await expect(card(canvasElement, 'v5/ines').querySelector('[data-testid=message-avatar] img')).not.toBeNull()
  if (args.scheme === 'light') await expect(getComputedStyle(card(canvasElement, 'v5/brief')).backgroundColor).toBe('rgb(255, 255, 255)')
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Transcript messaging',
  component: MessagingStory,
  parameters: {
    scenario: { _tag: 'contract-fixture', reason: 'Synthetic mail, interjection, IRC and machine-result fixtures exercise v5 stacking, recipient suppression, time dedupe, failure rows and attachment overflow.' },
    layout: 'fullscreen',
    viewport: { options: {
      messaging1280: { name: 'Messaging desktop', styles: { width: '1280px', height: '2600px' }, type: 'desktop' },
      messaging440: { name: 'Messaging narrow', styles: { width: '440px', height: '3400px' }, type: 'mobile' },
    } },
  },
  globals: { viewport: { value: 'messaging1280', isRotated: false } },
  args: { scheme: 'dark', width: 1280 },
} satisfies Meta<typeof MessagingStory>
export default meta

// Reviewed scenario exemptions by story export and mode, co-located with the synthetic host contract.
export const scenarioExemptions: readonly string[] = [
  'Narrow440Dark:contract-fixture',
  'Narrow440Light:contract-fixture',
  'Wide1280Dark:contract-fixture',
  'Wide1280Light:contract-fixture',
]
type Story = StoryObj<typeof meta>
export const Wide1280Dark: Story = { play }
export const Wide1280Light: Story = { args: { scheme: 'light' }, play }
export const Narrow440Dark: Story = { args: { width: 440 }, globals: { viewport: { value: 'messaging440', isRotated: false } }, play }
export const Narrow440Light: Story = { args: { width: 440, scheme: 'light' }, globals: { viewport: { value: 'messaging440', isRotated: false } }, play }

const styles = stylex.create({
  root: { height: '100vh', width: '100%', minWidth: 0, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
})
