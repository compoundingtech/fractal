import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fireEvent, userEvent, waitFor, within } from 'storybook/test'
import { useAui, useAuiState, type AppendMessage, type AttachmentAdapter } from '@assistant-ui/react'
import { Button } from 'react-aria-components'
import { Transcript, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import { EmbraceComposer } from './assistant-ui/EmbraceComposer'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { serializeDraft, type MentionToken } from './assistant-ui/embrace-composer/draft'
import { composerImagePolicy, isComposerImageType, type ComposerAvailability, type ComposerImage, type ComposerImagePolicy, type ComposerRecipientReadout, type ComposerReply } from './assistant-ui/embrace-composer/send'
import type { SendState, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { accentVars as accent, borderVars as border, geometryVars as g, radiusVars as r, surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'

/* Synthetic fixture identities; the kit receives every fact through props and reports intent through callbacks. */
const roster = [
  { ref: 'agent/fixture-review', label: 'Review seat', sessionId: 'session/fixture-a1' },
  { ref: 'agent/fixture-build', label: 'Build seat', sessionId: 'session/fixture-b1' },
  { ref: 'agent/fixture-docs', label: 'Docs seat', sessionId: 'session/fixture-c1' },
] as const
const fixtureReply: ComposerReply = { messageId: 'mail/fixture-41', sender: 'Build seat', subject: 'Nightly report', excerpt: 'Two checks are still red on the fixture branch.' }
const buildMention: MentionToken = { _tag: 'Mention', ref: 'agent/fixture-build', family: 'agent', label: 'Build seat' }

type AvailabilityTag = ComposerAvailability['_tag']
interface Scenario {
  readonly variant: 'C1' | 'C2'
  readonly roster: 'full' | 'none'
  /** The host can move the active session while the draft is open. */
  readonly sessionControl?: boolean
  /** Limits the fixture adapter declares; absent means Fractal's defaults. */
  readonly imagePolicy?: ComposerImagePolicy
  readonly reply?: boolean
  readonly subject?: boolean
  readonly images?: boolean
  readonly transcript?: boolean
  readonly availability: AvailabilityTag
}
const scenarios = {
  recipient: { variant: 'C1', roster: 'full', availability: 'Ready' },
  readout: { variant: 'C1', roster: 'none', sessionControl: true, availability: 'Ready' },
  mention: { variant: 'C2', roster: 'full', availability: 'Ready' },
  reply: { variant: 'C1', roster: 'full', reply: true, subject: true, images: true, availability: 'Ready' },
  images: { variant: 'C1', roster: 'full', images: true, availability: 'Ready' },
  limited: { variant: 'C1', roster: 'full', images: true, imagePolicy: { maxCount: 2, maxBytes: 5 * 1024 * 1024, mediaTypes: ['image/png', 'image/jpeg'] }, availability: 'Ready' },
  keyboard: { variant: 'C1', roster: 'full', images: true, availability: 'Ready' },
  states: { variant: 'C1', roster: 'full', images: true, transcript: true, availability: 'Ready' },
  offline: { variant: 'C1', roster: 'full', images: true, transcript: true, availability: 'Offline' },
  missingGrant: { variant: 'C1', roster: 'full', images: true, availability: 'MissingGrant' },
  revoked: { variant: 'C1', roster: 'full', images: true, transcript: true, availability: 'Revoked' },
  unavailable: { variant: 'C1', roster: 'full', availability: 'Unavailable' },
} as const satisfies Record<string, Scenario>
type ScenarioName = keyof typeof scenarios

/* Fixture upload service: the story plays the host, so upload progress only moves when the play says so. */
type UploadState = ComposerImage['state']
interface Uploads {
  readonly subscribe: (listener: () => void) => () => void
  readonly snapshot: () => ReadonlyMap<string, UploadState>
  readonly preview: (id: string) => string | undefined
  readonly start: (id: string, file: File) => void
  readonly retry: (id: string) => void
  readonly completeAll: () => void
  readonly forget: (id: string) => void
  readonly dispose: () => void
}
function createUploads(): Uploads {
  let states: ReadonlyMap<string, UploadState> = new Map()
  const previews = new Map<string, string>()
  const listeners = new Set<() => void>()
  const emit = (next: Map<string, UploadState>) => { states = next; listeners.forEach(listener => listener()) }
  return {
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    snapshot: () => states,
    preview: (id: string) => previews.get(id),
    start: (id: string, file: File) => {
      previews.set(id, URL.createObjectURL(file))
      emit(new Map(states).set(id, file.name.startsWith('fail') ? { _tag: 'Failed', detail: 'The connection dropped.' } : { _tag: 'Uploading', progress: 0.4 }))
    },
    retry: (id: string) => emit(new Map(states).set(id, { _tag: 'Uploading', progress: 0 })),
    completeAll: () => emit(new Map([...states].map(([id, state]) => [id, state._tag === 'Uploading' ? { _tag: 'Ready' } : state]))),
    forget: (id: string) => {
      const preview = previews.get(id)
      if (preview !== undefined) URL.revokeObjectURL(preview)
      previews.delete(id)
      const next = new Map(states)
      next.delete(id)
      emit(next)
    },
    dispose: () => { previews.forEach(preview => URL.revokeObjectURL(preview)); previews.clear() },
  }
}
let attachmentSequence = 0
const fixtureAttachments = (uploads: Uploads, policy: ComposerImagePolicy): AttachmentAdapter => ({
  accept: policy.mediaTypes.join(','),
  add: async ({ file }) => {
    const id = `attachment/fixture-${++attachmentSequence}`
    uploads.start(id, file)
    return { id, type: 'image', name: file.name, contentType: file.type, file, status: { type: 'requires-action', reason: 'composer-send' } }
  },
  send: async attachment => ({ ...attachment, status: { type: 'complete' }, content: [] }),
  remove: async attachment => { uploads.forget(attachment.id) },
})

const at = (index: number) => new Date(Date.parse('2026-03-02T09:00:00Z') + index * 60_000).toISOString()
const now = Date.parse('2026-03-02T09:30:00Z')
type UserRow = TextItem & { readonly role: 'user' }
const userRow = (id: string, text: string, sendState: SendState | undefined, index: number): UserRow =>
  ({ _tag: 'Text', id, role: 'user', text, attachments: [], streaming: false, at: at(index), sender: { kind: 'human', label: 'Operator' }, sendState })
const seededRows: readonly UserRow[] = [
  userRow('pending/fixture-1', 'Ship the fixture branch.', { _tag: 'Sent' }, 1),
  userRow('pending/fixture-2', 'Check the queue depth.', { _tag: 'Pending' }, 2),
  userRow('pending/fixture-3', 'Rerun the flaky check.', { _tag: 'Failed', reason: { _tag: 'Failed' }, detail: 'The relay did not answer.', code: 'relay_timeout', retryable: true }, 3),
  userRow('pending/fixture-4', 'Post the weekly digest.', { _tag: 'Failed', reason: { _tag: 'Ungranted' }, detail: 'The send grant expired.', code: 'grant_expired', retryable: false }, 4),
]
const turnFor = (row: UserRow): TranscriptTurn => ({ id: row.id, prompt: row, items: [], work: workLogTurnFromItems([row], { kindFor: () => 'read', running: false, failed: false, interrupted: false, startedAt: row.at, completeHistory: true }) })
const transcriptSync = { _tag: 'Live', since: now - 5000 } as const

interface Host {
  readonly scenario: Scenario
  readonly availability: ComposerAvailability
  readonly reply: ComposerReply | undefined
  readonly subject: string
  readonly onClearReply: () => void
  readonly onSubjectChange: (subject: string) => void
  readonly onRecipientChange: (ref: string) => void
  readonly record: (line: string) => void
  readonly afterSend: () => void
  /** Host-moved session for the readout; 0 means the draft's session is still active. */
  readonly sessionMoves: number
}

/** One recipient's runtime: drafts, images and transcript rows stay with it across navigation. */
function RecipientPane({ recipient, active, host }: { readonly recipient: (typeof roster)[number]; readonly active: boolean; readonly host: Host }) {
  const [rows, setRows] = React.useState<readonly UserRow[]>(() => host.scenario.transcript === true ? seededRows : [])
  const uploads = React.useMemo(createUploads, [])
  React.useEffect(() => uploads.dispose, [uploads])
  const hostRef = React.useRef(host)
  hostRef.current = host
  const policy = host.scenario.imagePolicy ?? composerImagePolicy
  const adapters = React.useMemo(() => ({ attachments: fixtureAttachments(uploads, policy) }), [uploads, policy])
  const options = React.useMemo(() => ({
    messages: rows, isRunning: false, adapters,
    onNew: async (message: AppendMessage) => {
      const text = message.content.flatMap(part => part.type === 'text' ? [part.text] : []).join('\n')
      const names = (message.attachments ?? []).map(attachment => attachment.name)
      const { subject, reply } = hostRef.current
      hostRef.current.record([`To ${recipient.ref}: ${text === '' ? 'no text' : `“${text}”`}`, names.length > 0 ? `images ${names.join(', ')}` : undefined, subject === '' ? undefined : `subject ${subject}`, reply === undefined ? undefined : `reply to ${reply.messageId}`].filter(Boolean).join('; '))
      hostRef.current.afterSend()
      setRows(previous => [...previous, userRow(`pending/fixture-${previous.length + 1}`, text === '' ? names.join(', ') : text, { _tag: 'Pending' }, previous.length + 1)])
    },
  }), [rows, adapters, recipient.ref])
  const retrySend = (id: string) => {
    host.record(`Retry ${id}`)
    setRows(previous => previous.map(row => row.id === id ? { ...row, sendState: { _tag: 'Pending' } } : row))
  }
  const deliver = () => setRows(previous => previous.map(row => row.sendState?._tag === 'Pending' ? { ...row, sendState: { _tag: 'Sent' } } : row))
  return <section hidden={!active} data-recipient-pane={recipient.ref} aria-label={`Conversation with ${recipient.label}`} {...stylex.props(styles.pane)}>
    <EmbraceRuntimeProvider options={options}>
      {host.scenario.transcript === true ? <div {...stylex.props(styles.transcript)}>
        <Transcript title={recipient.label} turns={rows.map(turnFor)} sync={transcriptSync} now={now} observedAt={now - 8000} onRetrySend={retrySend} sendAvailability={host.availability} />
      </div> : null}
      <div {...stylex.props(styles.fixtureBar)}>
        <Button onPress={uploads.completeAll} {...stylex.props(styles.button)}>Complete uploads</Button>
        {host.scenario.transcript === true ? <Button onPress={deliver} {...stylex.props(styles.button)}>Accept pending</Button> : null}
      </div>
      <div {...stylex.props(styles.dock)}><PaneComposer recipient={recipient} uploads={uploads} host={host} /></div>
    </EmbraceRuntimeProvider>
  </section>
}

function PaneComposer({ recipient, uploads, host }: { readonly recipient: (typeof roster)[number]; readonly uploads: Uploads; readonly host: Host }) {
  const aui = useAui()
  const attachments = useAuiState(state => state.composer.attachments)
  const states = React.useSyncExternalStore(uploads.subscribe, uploads.snapshot)
  const images = React.useMemo(() => attachments.flatMap((attachment): ComposerImage[] => {
    const mediaType = attachment.contentType
    if (mediaType === undefined || !isComposerImageType(mediaType)) return []
    return [{ id: attachment.id, name: attachment.name, mediaType, size: attachment.file?.size ?? 0, previewUrl: uploads.preview(attachment.id), state: states.get(attachment.id) ?? { _tag: 'Ready' } }]
  }), [attachments, states, uploads])
  const moved = host.scenario.sessionControl === true && host.sessionMoves > 0
  const readout: ComposerRecipientReadout = { ref: recipient.ref, label: recipient.label, sessionId: moved ? `session/fixture-a${host.sessionMoves + 1}` : recipient.sessionId, sessionChanged: moved }
  return <EmbraceComposer
    variant={host.scenario.variant}
    readingColumn
    recipient={readout}
    recipients={host.scenario.roster === 'full' ? roster : []}
    onRecipientChange={host.onRecipientChange}
    mentionCandidates={host.scenario.variant === 'C2' ? [buildMention] : undefined}
    onSendDraft={draft => { const mentions = serializeDraft(draft).mentions; if (mentions.length > 0) host.record(`Mentions ${mentions.join(', ')}`) }}
    images={host.scenario.images === true ? images : undefined}
    imagePolicy={host.scenario.imagePolicy}
    onRemoveImage={id => { void aui.composer.attachment({ id }).remove() }}
    onRetryImage={uploads.retry}
    availability={host.availability}
    reply={host.reply}
    onClearReply={host.onClearReply}
    subject={host.scenario.subject === true ? host.subject : undefined}
    onSubjectChange={host.scenario.subject === true ? host.onSubjectChange : undefined}
  />
}

const availabilityFor = (tag: AvailabilityTag, recover: () => void): ComposerAvailability => {
  switch (tag) {
    case 'Ready': return { _tag: 'Ready' }
    case 'Offline': return { _tag: 'Offline', detail: 'The relay connection is down.', onReconnect: recover }
    case 'MissingGrant': return { _tag: 'MissingGrant', detail: 'This device holds no send grant for Review seat.', onRecover: recover }
    case 'Revoked': return { _tag: 'Revoked', detail: 'The device key was revoked.', code: 'device_revoked', onRecover: recover }
    case 'Unavailable': return { _tag: 'Unavailable', detail: 'Sending is paused for maintenance.' }
  }
}

function ComposerSendStory({ scheme, scenario: name }: { readonly scheme: Scheme; readonly scenario: ScenarioName }) {
  const scenario: Scenario = scenarios[name]
  const [activeRef, setActiveRef] = React.useState<string>(roster[0].ref)
  const [visited, setVisited] = React.useState<ReadonlySet<string>>(() => new Set([roster[0].ref]))
  const [availabilityTag, setAvailabilityTag] = React.useState<AvailabilityTag>(scenario.availability)
  const [reply, setReply] = React.useState<ComposerReply | undefined>(scenario.reply === true ? fixtureReply : undefined)
  const [subject, setSubject] = React.useState('')
  const [log, setLog] = React.useState<readonly string[]>([])
  const [recipientChanges, setRecipientChanges] = React.useState<readonly string[]>([])
  const [recoveries, setRecoveries] = React.useState(0)
  const [sessionMoves, setSessionMoves] = React.useState(0)
  // The host owns recovery: the kit only reports intent. Offline reconnects at once; grants and enrollment wait for an explicit fixture step.
  const recover = () => { setRecoveries(count => count + 1); if (availabilityTag === 'Offline') setAvailabilityTag('Ready') }
  const host: Host = {
    scenario, reply, subject, sessionMoves,
    availability: availabilityFor(availabilityTag, recover),
    onClearReply: () => setReply(undefined),
    onSubjectChange: setSubject,
    onRecipientChange: ref => {
      setRecipientChanges(previous => [...previous, ref])
      setVisited(previous => new Set(previous).add(ref))
      setActiveRef(ref)
    },
    record: line => setLog(previous => [...previous, line]),
    afterSend: () => { setReply(undefined); setSubject('') },
  }
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {roster.filter(recipient => visited.has(recipient.ref)).map(recipient => <RecipientPane key={recipient.ref} recipient={recipient} active={recipient.ref === activeRef} host={host} />)}
    <section aria-label="Fixture host" {...stylex.props(styles.fixture)}>
      <p data-testid="fixture-recipient-changes">Recipient changes: {recipientChanges.length === 0 ? 'none' : recipientChanges.join(', ')}</p>
      <p data-testid="fixture-recoveries">Recovery requests: {recoveries}</p>
      {availabilityTag !== 'Ready' && availabilityTag !== 'Offline' ? <Button onPress={() => setAvailabilityTag('Ready')} {...stylex.props(styles.button)}>Finish setup</Button> : null}
      {scenario.sessionControl === true ? <Button onPress={() => setSessionMoves(count => count + 1)} {...stylex.props(styles.button)}>Move session</Button> : null}
      <ol aria-label="Host log" data-testid="fixture-log" {...stylex.props(styles.log)}>{log.map((line, index) => <li key={index}>{line}</li>)}</ol>
    </section>
  </main>
}

const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Composer send',
  component: ComposerSendStory,
  parameters: { scenario: { _tag: 'contract-fixture', reason: 'Host-controlled recipient, upload, recovery and delivery callbacks isolate the composer send contracts without network transport.' }, layout: 'fullscreen', docs: { description: { component: 'Composer send surface: recipient readout and selection, reply chip, optional subject, image tray, availability recovery and per-row send state. The story plays the host: uploads, retries, recovery and delivery move only through fixture callbacks; the kit makes no network calls.' } } },
  args: { scheme: 'dark', scenario: 'recipient' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, scenario: { options: Object.keys(scenarios), control: 'select' } },
} satisfies Meta<typeof ComposerSendStory>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'DeclaredImageLimits:contract-fixture',
  'ImageRetry:contract-fixture',
  'ImageTray:contract-fixture',
  'ImageTrayLight:contract-fixture',
  'KeyboardSend:contract-fixture',
  'KeyboardSendLight:contract-fixture',
  'MentionKeepsRecipient:contract-fixture',
  'MissingGrant:contract-fixture',
  'OfflineRecovery:contract-fixture',
  'OfflineRecoveryLight:contract-fixture',
  'RecipientReadout:contract-fixture',
  'RecipientSelection:contract-fixture',
  'RecipientSelectionLight:contract-fixture',
  'ReplyAndSubject:contract-fixture',
  'ReplyAndSubjectLight:contract-fixture',
  'RevokedRecovery:contract-fixture',
  'RevokedRecoveryLight:contract-fixture',
  'SendStates:contract-fixture',
  'SendStatesLight:contract-fixture',
  'Unavailable:contract-fixture',
]
type Story = StoryObj<typeof meta>

/* Play helpers. Every assertion after an interaction waits for the re-render. */
const pngBytes = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=='), char => char.charCodeAt(0))
const imageFile = (name: string, type = 'image/png', size?: number) => new File([size === undefined ? pngBytes : new Uint8Array(size)], name, { type })
const pane = (canvasElement: HTMLElement) => {
  const found = canvasElement.querySelector<HTMLElement>('[data-recipient-pane]:not([hidden])')
  if (found === null) throw new Error('No active recipient pane')
  return found
}
const messageField = (root: HTMLElement) => within(root).getByRole('textbox', { name: /^Message to / })
const sendButton = (root: HTMLElement) => within(root).getByRole('button', { name: 'Send' })
const hostLog = (canvasElement: HTMLElement) => [...canvasElement.querySelectorAll('[data-testid="fixture-log"] li')].map(item => item.textContent ?? '')
const sentLines = (canvasElement: HTMLElement) => hostLog(canvasElement).filter(line => line.startsWith('To '))
const settled = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
const pickFiles = (root: HTMLElement, files: readonly File[]) => {
  const input = root.querySelector<HTMLInputElement>('input[type="file"]')
  if (input === null) throw new Error('No image picker input')
  const data = new DataTransfer()
  files.forEach(file => data.items.add(file))
  input.files = data.files
  input.dispatchEvent(new Event('change', { bubbles: true }))
}
const pasteFiles = (target: HTMLElement, files: readonly File[]) => {
  const data = new DataTransfer()
  files.forEach(file => data.items.add(file))
  target.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }))
}
const tray = (root: HTMLElement) => within(root).getByRole('list', { name: /^Images, / })
const tileStates = (root: HTMLElement) => [...root.querySelectorAll('[data-image-state]')].map(tile => tile.getAttribute('data-image-state')?.toLowerCase())
const describedText = (element: Element) => (element.getAttribute('aria-describedby') ?? '').split(' ').map(id => document.getElementById(id)?.textContent ?? '').join(' ')
async function sound(canvasElement: HTMLElement) {
  await expect(canvasElement.textContent ?? '').not.toMatch(/unknown/i)
  await expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(window.innerWidth)
}
async function moveMenuFocusTo(label: string) {
  for (let step = 0; step < roster.length + 1 && document.activeElement?.textContent !== label; step++) await userEvent.keyboard('{ArrowDown}')
  await expect(document.activeElement?.textContent).toBe(label)
}

/** The open recipient menu: the only accessible menu, controlled by a recipient trigger. MenuTrigger may name it from the trigger. */
const findRecipientMenu = () => waitFor(() => {
  const menu = within(document.body).getByRole('menu')
  expect(menu).toHaveAccessibleName(/recipient/i)
  expect(document.querySelector(`[aria-controls="${menu.id}"]`)).toHaveAttribute('data-composer-recipient')
  return menu
})

/** K1: the readout names the recipient and exposes ref and session; keyboard and pointer picks report once and keep each recipient's draft. */
export const RecipientSelection: Story = {
  play: async ({ canvasElement }) => {
    await document.fonts.ready
    const review = pane(canvasElement)
    const trigger = await within(review).findByRole('button', { name: 'To Review seat, change recipient' })
    await expect(describedText(trigger)).toContain('agent/fixture-review')
    // Label in name: the accessible name starts with the visible "To Review seat".
    await expect(trigger.getAttribute('aria-label')?.startsWith('To Review seat')).toBe(true)
    await expect(trigger.querySelector('[data-composer-readout]')).toHaveTextContent('Review seat')
    await expect(trigger.textContent).toMatch(/^To/)
    await expect(describedText(trigger)).toContain('session/fixture-a1')
    await userEvent.type(messageField(review), 'Draft for review')
    // Typing can move the footer between layouts; query the trigger the user would see now.
    const typedTrigger = within(review).getByRole('button', { name: 'To Review seat, change recipient' })
    typedTrigger.focus()
    await expect(document.activeElement).toBe(typedTrigger)
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(typedTrigger).toHaveAttribute('aria-expanded', 'true'))
    const menu = await findRecipientMenu()
    await expect(menu.id).toBe(typedTrigger.getAttribute('aria-controls'))
    await expect(within(menu).getByRole('menuitemradio', { name: 'Review seat' })).toHaveAttribute('aria-checked', 'true')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(within(document.body).queryByRole('menu')).toBeNull())
    await waitFor(() => expect(document.activeElement).toBe(within(review).getByRole('button', { name: 'To Review seat, change recipient' })))
    await expect(canvasElement.querySelector('[data-testid="fixture-recipient-changes"]')).toHaveTextContent('Recipient changes: none')
    await userEvent.keyboard('{Enter}')
    await findRecipientMenu()
    await moveMenuFocusTo('Build seat')
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(canvasElement.querySelector('[data-testid="fixture-recipient-changes"]')).toHaveTextContent('Recipient changes: agent/fixture-build'))
    const build = await waitFor(() => { const found = pane(canvasElement); if (found.dataset.recipientPane !== 'agent/fixture-build') throw new Error('Build pane not active'); return found })
    await expect(messageField(build)).toHaveValue('')
    await expect(sentLines(canvasElement)).toEqual([])
    // Picking the current recipient again reports nothing.
    await userEvent.click(within(build).getByRole('button', { name: 'To Build seat, change recipient' }))
    const again = await findRecipientMenu()
    await userEvent.click(within(again).getByRole('menuitemradio', { name: 'Build seat' }))
    await waitFor(() => expect(within(document.body).queryByRole('menu')).toBeNull())
    await expect(canvasElement.querySelector('[data-testid="fixture-recipient-changes"]')).toHaveTextContent('Recipient changes: agent/fixture-build')
    await userEvent.click(within(build).getByRole('button', { name: 'To Build seat, change recipient' }))
    const back = await findRecipientMenu()
    await userEvent.click(within(back).getByRole('menuitemradio', { name: 'Review seat' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-testid="fixture-recipient-changes"]')).toHaveTextContent('Recipient changes: agent/fixture-build, agent/fixture-review'))
    await waitFor(() => expect(pane(canvasElement).dataset.recipientPane).toBe('agent/fixture-review'))
    await expect(messageField(pane(canvasElement))).toHaveValue('Draft for review')
    await sound(canvasElement)
  },
}
export const RecipientSelectionLight: Story = { ...RecipientSelection, args: { scheme: 'light' } }

/** K1: with nobody else to pick, the readout stays a labelled, focusable fact; each session change is announced politely, once. */
export const RecipientReadout: Story = {
  args: { scenario: 'readout' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const readout = await within(root).findByRole('group', { name: 'To Review seat' })
    await expect(within(root).queryByRole('button', { name: /change recipient/ })).toBeNull()
    await expect(describedText(readout)).toContain('session/fixture-a1')
    readout.focus()
    await expect(document.activeElement).toBe(readout)
    const announcement = within(root).getByTestId('composer-session-announcement')
    await expect(announcement).toHaveAttribute('role', 'status')
    await expect(announcement).not.toHaveAttribute('aria-live', 'assertive')
    await expect(announcement.textContent).toBe('')
    await expect(within(root).queryByTestId('composer-session-note')).toBeNull()
    const records: MutationRecord[] = []
    const changes = new MutationObserver(list => { records.push(...list) })
    const drain = () => { records.push(...changes.takeRecords()); return records.splice(0) }
    changes.observe(announcement, { subtree: true, childList: true, characterData: true })
    try {
      await userEvent.click(within(canvasElement).getByRole('button', { name: 'Move session' }))
      await waitFor(() => expect(announcement).toHaveTextContent('Session changed. This message continues session session/fixture-a2.'))
      await expect(within(root).getByTestId('composer-session-note')).toHaveTextContent('Session changed. This message continues session session/fixture-a2.')
      await expect(drain()).toHaveLength(1)
      // Unrelated re-renders leave the region untouched, so nothing is announced twice.
      await userEvent.type(messageField(root), 'Still the same draft')
      await waitFor(() => expect(messageField(root)).toHaveValue('Still the same draft'))
      await settled()
      await expect(drain()).toHaveLength(0)
      await expect(within(root).getByTestId('composer-session-announcement')).toBe(announcement)
      await userEvent.click(within(canvasElement).getByRole('button', { name: 'Move session' }))
      await waitFor(() => expect(announcement).toHaveTextContent('Session changed. This message continues session session/fixture-a3.'))
      await expect(drain()).toHaveLength(1)
    } finally { changes.disconnect() }
    await expect(describedText(within(root).getByRole('group', { name: 'To Review seat' }))).toContain('session/fixture-a3')
    await expect(messageField(root)).toHaveValue('Still the same draft')
    await sound(canvasElement)
  },
}

/** K3: the adapter's declared limits, not the kit defaults, drive the picker filter, the refusals and the copy. */
export const DeclaredImageLimits: Story = {
  args: { scenario: 'limited' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const attach = await within(root).findByRole('button', { name: 'Attach image' })
    await expect(root.querySelector('input[type="file"]')).toHaveAttribute('accept', 'image/png,image/jpeg')
    // RAC opens hover tooltips only in pointer modality; a real click on the field establishes it.
    const field = messageField(root)
    await userEvent.click(field)
    await userEvent.hover(attach)
    await waitFor(() => expect(within(document.body).getByRole('tooltip')).toHaveTextContent('Attach up to 2 images (PNG or JPEG), 5 MB each'))
    await userEvent.unhover(attach)
    await waitFor(() => expect(within(document.body).queryByRole('tooltip')).toBeNull())
    pasteFiles(field, [imageFile('one.png'), imageFile('two.jpg', 'image/jpeg'), imageFile('three.png')])
    await waitFor(() => expect(tray(root)).toHaveAccessibleName('Images, 2 of 2'))
    await expect(within(root).getByRole('alert')).toHaveTextContent('A message carries at most 2 images.')
    await userEvent.click(within(root).getByRole('button', { name: 'Remove two.jpg' }))
    await waitFor(() => expect(tray(root)).toHaveAccessibleName('Images, 1 of 2'))
    pasteFiles(field, [imageFile('six.png', 'image/png', 6 * 1024 * 1024), imageFile('loop.gif', 'image/gif')])
    await waitFor(() => expect(within(root).getByRole('alert')).toHaveTextContent('six.png is 6.0 MB; Fractal takes images up to 5 MB. loop.gif is image/gif; Fractal takes PNG or JPEG.'))
    await expect(tray(root)).toHaveAccessibleName('Images, 1 of 2')
    // The Fractal default (four images, 10 MB) never leaks into the copy when the adapter declares less.
    await expect(root.textContent ?? '').not.toMatch(/10 MB|at most 4/)
    await sound(canvasElement)
  },
}

/** K1: an @mention inserts a token and never changes who the message goes to. */
export const MentionKeepsRecipient: Story = {
  args: { scenario: 'mention' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const field = await within(root).findByRole('textbox', { name: 'Message to Review seat' })
    await userEvent.click(field)
    await userEvent.keyboard('Loop in @Bui')
    const options = await within(document.body).findByRole('listbox', { name: 'Mention a subject' })
    await userEvent.click(within(options).getByRole('option', { name: /Build seat/ }))
    await waitFor(() => expect(field).toHaveTextContent('@Build seat'))
    await expect(within(root).getByRole('button', { name: 'To Review seat, change recipient' })).toBeInTheDocument()
    await expect(canvasElement.querySelector('[data-testid="fixture-recipient-changes"]')).toHaveTextContent('Recipient changes: none')
    await userEvent.click(sendButton(root))
    await waitFor(() => expect(sentLines(canvasElement)).toHaveLength(1))
    await expect(sentLines(canvasElement)[0]).toMatch(/^To agent\/fixture-review: /)
    await expect(hostLog(canvasElement)).toContain('Mentions agent/fixture-build')
    await expect(pane(canvasElement).dataset.recipientPane).toBe('agent/fixture-review')
    await sound(canvasElement)
  },
}

/** K2: the reply chip and subject are host-controlled; clearing the reply keeps text and images, Enter in the subject never sends. */
export const ReplyAndSubject: Story = {
  args: { scenario: 'reply' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const chip = await within(root).findByRole('group', { name: 'Replying to Build seat' })
    await expect(describedText(chip)).toContain('mail/fixture-41')
    const field = messageField(root)
    await userEvent.type(field, 'Thanks, merging now')
    pickFiles(root, [imageFile('harbor.png')])
    await waitFor(() => expect(tileStates(root)).toEqual(['uploading']))
    await userEvent.click(within(root).getByRole('button', { name: 'Complete uploads' }))
    await waitFor(() => expect(tileStates(root)).toEqual(['ready']))
    const subject = within(root).getByRole('textbox', { name: 'Subject' })
    await userEvent.type(subject, 'Weekly notes{Enter}')
    await waitFor(() => expect(document.activeElement).toBe(field))
    await expect(subject).toHaveValue('Weekly notes')
    await expect(sentLines(canvasElement)).toEqual([])
    await userEvent.click(within(chip).getByRole('button', { name: 'Clear reply to Build seat' }))
    await waitFor(() => expect(within(root).queryByRole('group', { name: 'Replying to Build seat' })).toBeNull())
    await expect(document.activeElement).toBe(field)
    await expect(field).toHaveValue('Thanks, merging now')
    await expect(within(tray(root)).getByText('harbor.png')).toBeInTheDocument()
    await userEvent.click(sendButton(root))
    await waitFor(() => expect(sentLines(canvasElement)).toEqual(['To agent/fixture-review: “Thanks, merging now”; images harbor.png; subject Weekly notes']))
    await waitFor(() => expect(within(root).getByRole('textbox', { name: 'Subject' })).toHaveValue(''))
    await sound(canvasElement)
  },
}
export const ReplyAndSubjectLight: Story = { ...ReplyAndSubject, args: { scheme: 'light', scenario: 'reply' } }

/** K3: picker and paste share one intake; a fifth image, an oversized file and an unsupported type are refused aloud; images alone send. */
export const ImageTray: Story = {
  args: { scenario: 'images' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    await expect(await within(root).findByRole('button', { name: 'Attach image' })).toBeEnabled()
    pickFiles(root, [imageFile('harbor.png'), imageFile('ridge.jpg', 'image/jpeg'), imageFile('tide.gif', 'image/gif'), imageFile('dune.webp', 'image/webp')])
    await waitFor(() => expect(tray(root)).toHaveAccessibleName('Images, 4 of 4'))
    await expect(tileStates(root)).toEqual(['uploading', 'uploading', 'uploading', 'uploading'])
    await expect(within(root).getByTestId('composer-image-wait')).toHaveTextContent('Waiting for 4 images to finish uploading.')
    await expect(sendButton(root)).toBeDisabled()
    const field = messageField(root)
    pasteFiles(field, [imageFile('fifth.png')])
    await waitFor(() => expect(within(root).getByRole('alert')).toHaveTextContent('A message carries at most 4 images.'))
    await expect(tray(root)).toHaveAccessibleName('Images, 4 of 4')
    await userEvent.click(within(root).getByRole('button', { name: 'Remove dune.webp' }))
    await waitFor(() => expect(tray(root)).toHaveAccessibleName('Images, 3 of 4'))
    await expect(document.activeElement).toBe(field)
    pasteFiles(field, [imageFile('large.png', 'image/png', composerImagePolicy.maxBytes + 1024 * 1024), imageFile('scan.bmp', 'image/bmp')])
    await waitFor(() => expect(within(root).getByRole('alert')).toHaveTextContent('large.png is 11.0 MB; Fractal takes images up to 10 MB. scan.bmp is image/bmp; Fractal takes PNG, JPEG, GIF or WebP.'))
    await expect(tray(root)).toHaveAccessibleName('Images, 3 of 4')
    pasteFiles(field, [imageFile('fifth.png')])
    await waitFor(() => expect(tray(root)).toHaveAccessibleName('Images, 4 of 4'))
    await waitFor(() => expect(within(root).queryByRole('alert')).toBeNull())
    await userEvent.click(within(root).getByRole('button', { name: 'Complete uploads' }))
    await waitFor(() => expect(sendButton(root)).toBeEnabled())
    await expect(field).toHaveValue('')
    await userEvent.click(sendButton(root))
    await waitFor(() => expect(sentLines(canvasElement)).toEqual(['To agent/fixture-review: no text; images harbor.png, ridge.jpg, tide.gif, fifth.png']))
    await waitFor(() => expect(within(root).queryByRole('list', { name: /^Images, / })).toBeNull())
    await sound(canvasElement)
  },
}
export const ImageTrayLight: Story = { ...ImageTray, args: { scheme: 'light', scenario: 'images' } }

/** K3: a failed upload holds send until it is retried or removed; retry reports intent and the host decides. */
export const ImageRetry: Story = {
  args: { scenario: 'images' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const field = await within(root).findByRole('textbox', { name: 'Message to Review seat' })
    await userEvent.type(field, 'Two sketches')
    pickFiles(root, [imageFile('fail-sketch.png'), imageFile('harbor.png')])
    await waitFor(() => expect(tileStates(root)).toEqual(['failed', 'uploading']))
    await expect(within(root).getByTestId('composer-image-wait')).toHaveTextContent('Retry or remove the image that did not upload to send.')
    await expect(sendButton(root)).toBeDisabled()
    await expect(within(root).queryByRole('button', { name: 'Retry upload of harbor.png' })).toBeNull()
    await userEvent.click(within(root).getByRole('button', { name: 'Retry upload of fail-sketch.png' }))
    await waitFor(() => expect(tileStates(root)).toEqual(['uploading', 'uploading']))
    await userEvent.click(within(root).getByRole('button', { name: 'Complete uploads' }))
    await waitFor(() => expect(sendButton(root)).toBeEnabled())
    pasteFiles(field, [imageFile('fail-scan.png')])
    await waitFor(() => expect(tileStates(root)).toEqual(['ready', 'ready', 'failed']))
    await expect(sendButton(root)).toBeDisabled()
    await userEvent.click(within(root).getByRole('button', { name: 'Remove fail-scan.png' }))
    await waitFor(() => expect(tileStates(root)).toEqual(['ready', 'ready']))
    await waitFor(() => expect(sendButton(root)).toBeEnabled())
    await userEvent.click(sendButton(root))
    await waitFor(() => expect(sentLines(canvasElement)).toEqual(['To agent/fixture-review: “Two sketches”; images fail-sketch.png, harbor.png']))
    await sound(canvasElement)
  },
}

/** K5: Enter during IME composition never sends; Shift+Enter breaks the line; Enter sends exactly once. */
export const KeyboardSend: Story = {
  args: { scenario: 'keyboard' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const field = await within(root).findByRole('textbox', { name: 'Message to Review seat' })
    await userEvent.click(field)
    await userEvent.keyboard('nihao')
    fireEvent.compositionStart(field)
    fireEvent.keyDown(field, { key: 'Enter', code: 'Enter', keyCode: 229, isComposing: true })
    await settled()
    await expect(sentLines(canvasElement)).toEqual([])
    await expect(field).toHaveValue('nihao')
    fireEvent.compositionEnd(field)
    await userEvent.keyboard('{Shift>}{Enter}{/Shift}second line')
    await waitFor(() => expect(field).toHaveValue('nihao\nsecond line'))
    await expect(sentLines(canvasElement)).toEqual([])
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(sentLines(canvasElement)).toEqual(['To agent/fixture-review: “nihao\nsecond line”']))
    await waitFor(() => expect(field).toHaveValue(''))
    await userEvent.keyboard('{Enter}')
    await settled()
    await expect(sentLines(canvasElement)).toHaveLength(1)
    await sound(canvasElement)
  },
}
export const KeyboardSendLight: Story = { ...KeyboardSend, args: { scheme: 'light', scenario: 'keyboard' } }

const rowFor = (root: HTMLElement, id: string) => {
  const row = root.querySelector<HTMLElement>(`[data-testid="user-message"][data-item-id="${id}"]`)
  if (row === null) throw new Error(`No row ${id}`)
  return row
}

/** K4: rows read Sending…, Sent or Failed; only retryable failures offer Retry, and retry reports the row once. */
export const SendStates: Story = {
  args: { scenario: 'states' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    await waitFor(() => expect(rowFor(root, 'pending/fixture-1')).toHaveTextContent('Sent'))
    await expect(within(rowFor(root, 'pending/fixture-2')).getByRole('status')).toHaveTextContent('Sending…')
    const terminal = rowFor(root, 'pending/fixture-4')
    await expect(within(terminal).queryByRole('button', { name: 'Retry' })).toBeNull()
    await userEvent.click(within(terminal).getByRole('button', { expanded: false }))
    await waitFor(() => expect(terminal).toHaveTextContent('Code grant_expired'))
    await userEvent.type(messageField(root), 'Newer draft')
    await userEvent.click(within(rowFor(root, 'pending/fixture-3')).getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(within(rowFor(root, 'pending/fixture-3')).getByRole('status')).toHaveTextContent('Sending…'))
    await expect(hostLog(canvasElement).filter(line => line.startsWith('Retry '))).toEqual(['Retry pending/fixture-3'])
    await expect(messageField(root)).toHaveValue('Newer draft')
    await userEvent.click(within(root).getByRole('button', { name: 'Accept pending' }))
    await waitFor(() => expect(within(rowFor(root, 'pending/fixture-3')).getByRole('status')).toHaveTextContent('Sent'))
    await expect(within(rowFor(root, 'pending/fixture-2')).getByRole('status')).toHaveTextContent('Sent')
    await userEvent.click(sendButton(root))
    await waitFor(() => expect(within(rowFor(root, 'pending/fixture-5')).getByRole('status')).toHaveTextContent('Sending…'))
    await sound(canvasElement)
  },
}
export const SendStatesLight: Story = { ...SendStates, args: { scheme: 'light', scenario: 'states' } }

const notice = (root: HTMLElement) => within(root).getByTestId('composer-availability')

/** K4: offline keeps the draft editable, holds send and row Retry, and offers one Reconnect; reconnecting never sends by itself. */
export const OfflineRecovery: Story = {
  args: { scenario: 'offline' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const box = await waitFor(() => notice(root))
    await expect(box).toHaveTextContent('Offline — reconnect to send.')
    await expect(box).toHaveTextContent('The relay connection is down. Your draft is kept.')
    await expect(within(box).getAllByRole('button').map(button => button.textContent)).toEqual(['Reconnect'])
    const field = messageField(root)
    await userEvent.type(field, 'Queue this later{Enter}')
    await settled()
    await expect(field).toHaveValue('Queue this later')
    await expect(sendButton(root)).toBeDisabled()
    await expect(sentLines(canvasElement)).toEqual([])
    await expect(within(rowFor(root, 'pending/fixture-3')).queryByRole('button', { name: 'Retry' })).toBeNull()
    await userEvent.click(within(box).getByRole('button', { name: 'Reconnect' }))
    await waitFor(() => expect(within(root).queryByTestId('composer-availability')).toBeNull())
    await waitFor(() => expect(sendButton(root)).toBeEnabled())
    await expect(within(rowFor(root, 'pending/fixture-3')).getByRole('button', { name: 'Retry' })).toBeInTheDocument()
    await expect(sentLines(canvasElement)).toEqual([])
    await expect(field).toHaveValue('Queue this later')
    await sound(canvasElement)
  },
}
export const OfflineRecoveryLight: Story = { ...OfflineRecovery, args: { scheme: 'light', scenario: 'offline' } }

/** K4: a revoked device names its code and offers one Set up again; the host, not the kit, decides when sending resumes. */
export const RevokedRecovery: Story = {
  args: { scenario: 'revoked' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const box = await waitFor(() => notice(root))
    await expect(box).toHaveTextContent('This device is no longer authorized to send.')
    await expect(box).toHaveTextContent('The device key was revoked. Your draft is kept. Code device_revoked.')
    await expect(within(box).getAllByRole('button').map(button => button.textContent)).toEqual(['Set up again'])
    await expect(within(rowFor(root, 'pending/fixture-3')).queryByRole('button', { name: 'Retry' })).toBeNull()
    await userEvent.type(messageField(root), 'Draft before setup')
    await userEvent.click(within(box).getByRole('button', { name: 'Set up again' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-testid="fixture-recoveries"]')).toHaveTextContent('Recovery requests: 1'))
    await expect(notice(root)).toBeInTheDocument()
    await expect(sendButton(root)).toBeDisabled()
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Finish setup' }))
    await waitFor(() => expect(sendButton(root)).toBeEnabled())
    await expect(messageField(root)).toHaveValue('Draft before setup')
    await expect(sentLines(canvasElement)).toEqual([])
    await sound(canvasElement)
  },
}
export const RevokedRecoveryLight: Story = { ...RevokedRecovery, args: { scheme: 'light', scenario: 'revoked' } }

/** K4: a missing grant offers one Check access; an unavailable relay offers nothing to press. */
export const MissingGrant: Story = {
  args: { scenario: 'missingGrant' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const box = await waitFor(() => notice(root))
    await expect(box).toHaveTextContent('Sending messages is not permitted here.')
    await expect(within(box).getAllByRole('button').map(button => button.textContent)).toEqual(['Check access'])
    await userEvent.click(within(box).getByRole('button', { name: 'Check access' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-testid="fixture-recoveries"]')).toHaveTextContent('Recovery requests: 1'))
    await expect(notice(root)).toBeInTheDocument()
    await sound(canvasElement)
  },
}
export const Unavailable: Story = {
  args: { scenario: 'unavailable' },
  play: async ({ canvasElement }) => {
    const root = pane(canvasElement)
    const box = await waitFor(() => notice(root))
    await expect(box).toHaveTextContent('Sending is paused for maintenance. Your draft is kept.')
    await expect(within(box).queryAllByRole('button')).toEqual([])
    await userEvent.type(messageField(root), 'Still typing')
    await waitFor(() => expect(messageField(root)).toHaveValue('Still typing'))
    await expect(sendButton(root)).toBeDisabled()
    await sound(canvasElement)
  },
}

const styles = stylex.create({
  root: { minHeight: '100vh', width: '100%', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  pane: { flex: '1 1 auto', minHeight: 0, display: 'flex', flexDirection: 'column' },
  transcript: { display: 'flex', flexDirection: 'column', minHeight: 0 },
  dock: { flexShrink: 0, paddingBlock: s.lg },
  fixtureBar: { display: 'flex', flexWrap: 'wrap', gap: s.md, paddingInline: s.lg },
  fixture: { display: 'flex', flexDirection: 'column', gap: s.sm, padding: s.lg, borderTopWidth: g.hairline, borderTopStyle: 'solid', borderTopColor: border.border, color: ink.fgSoft, fontSize: t.metaSize },
  log: { margin: 0, paddingInlineStart: s.lg, overflowWrap: 'anywhere' },
  button: { alignSelf: 'flex-start', minHeight: g.controlMd, paddingInline: s.md, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: r.sm, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary } },
})
