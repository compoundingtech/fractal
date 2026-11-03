import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { EmbraceRuntimeProvider, Transcript, assistantDarkTheme, liveAccentTheme, compositionLightTheme, type TextItem, type TranscriptAttachmentImages, type TranscriptTurn } from '@fractal/ui/assistant-ui'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import type { Attachment, ConversationItem } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-data/model.ts'
import { surfaceVars as surface, spaceVars as space, textVars as text } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import type { AttachmentPort, ConversationPortResult } from '../data/source.ts'
import type { BlobChunk } from '@fractal/client/schema'
import { createTranscriptImages } from './transcriptImages.ts'

/** A 240×150 PNG of a sketched review layout; it fits the 320×240 thumbnail box unscaled. */
const PNG_BASE64 = 'iVBORw0KGgoAAAANSUhEUgAAAPAAAACWCAIAAABvmpKCAAABy0lEQVR42u3dsQ2AIBRFUWaxsLSysrIiTkDtCJYM4AzO5FguYAIlgZPcCX5O90gI6xalbgpOIKAloCWgJaAFtAS0BLQEtAS0gJaAloCWgJaAFtAS0FIDoKd5kboJaAEtAS0BLQEtoCWgJaAloCWgBbQ0Guh8P2oqiIEGGmiggQZaQAMtoIEGWkADDTTQQAMtoIEW0EADDTTQQHucJAEtAS0BLaAloCWgJaAloAW0BLQEtNQC6HReUjGgBTTQAhpoAQ20gBbQQAtooAW0YUWGFQloCWgJaAloAS0BLRlWNOI4ArSABlpAAw000AIaaAENtIAGWmODloCWgJaAFtCuIKAloCWgJaAFtAS0BLQEtAS0gC4V86vf6m+4H0mVAQ000EADDbSABhpooIEGGmiggQYaaKCBFtBAAy2ggQYaaKCBBhpooAU00EALaKCBBhpooIEGGmiggQYaaAENNNBAAw000EADDTTQQAMtoIEGGmiggQYaaKCBBhpooAU00EADDTTQQAMNNNBAAw20gAYaaKCBBlpAAw000EADDTTQQAMNNNBAC2iggQa6JdBSV5/XS0BLQEtAC2hXENAS0BLQEtACWgJaAloCWqrpA6KzhibHIN5aAAAAAElFTkSuQmCC'
const PNG_SHA256 = '4d254b9599fd1216e6a2ae8d0010a365bd3d93f2495402cd1201c17095fdb3e3'
const PNG_SIZE = 516
const NAME = 'review-layout.png'
const MESSAGE_ID = 'message/transcript-images-review'
const MAILBOX_MESSAGE_ID = 'message/transcript-images-mailbox'
/** The PNG signature and IHDR chunk header, then nothing: the cache's checks pass, the browser's decoder refuses it. */
const UNDECODABLE_PNG_BASE64 = 'iVBORw0KGgoAAAANSUhEUg=='
const UNDECODABLE_PNG_SIZE = 16

type Fixture = 'loading' | 'unauthorized' | 'failed' | 'loaded' | 'unavailable' | 'decode-failed'
type ChunkRequest = Parameters<AttachmentPort['chunk']>[0]
type Width = 'desktop' | 'phone'
/** Which row carries the image: the person's prompt, an assistant answer, or the paired body of incoming agent mail. */
type Row = 'prompt' | 'assistant' | 'mailbox'

/** `unavailable` sends a type the host never reads; every other fixture sends the PNG. */
const attachmentFor = (fixture: Fixture) => fixture === 'unavailable'
  ? { name: 'review-layout.svg', mediaType: 'image/svg+xml', label: 'Image: review-layout.svg, image/svg+xml' }
  : { name: NAME, mediaType: 'image/png', label: `Image: ${NAME}, PNG` }

/**
 * The production transcript image cache over a controlled chunk transport: `loading` never settles, `unauthorized`
 * refuses as ungranted, `failed` refuses on every attempt so Retry is observable, `loaded` returns the PNG in one chunk,
 * `decode-failed` first returns PNG-signed bytes the browser cannot decode and the real PNG on Retry, and `unavailable`
 * is a type the cache refuses before any read. With `gated`, each read waits for the fixture's Release reads control,
 * so a play can hold Loading and then let it finish. Nothing reads until the person asks.
 *
 * `row` moves the attachment off the prompt. `assistant` is a native assistant answer: no carrier message, so the read
 * names none. `mailbox` is received agent mail as the timeline projects it, a header row and its paired body, where the
 * body carries the attachment with the mail's own message id; the read must use that id, not the row's.
 */
function TranscriptImages({ scheme, width = 'desktop', fixture, gated = false, row = 'prompt' }: { readonly scheme: 'dark' | 'light'; readonly width?: Width; readonly fixture: Fixture; readonly gated?: boolean; readonly row?: Row }) {
  const [requests, setRequests] = React.useState<readonly ChunkRequest[]>([])
  const [{ images, release }] = React.useState(() => {
    const waiting: Array<() => void> = []
    let attempts = 0
    const answer = (): Promise<ConversationPortResult<BlobChunk>> => {
      attempts += 1
      switch (fixture) {
        case 'loading': return Promise.withResolvers<never>().promise
        case 'unauthorized': return Promise.resolve({ _tag: 'Refused', reason: 'ungranted', detail: 'This session may not read attachment bytes.' })
        case 'failed': return Promise.resolve({ _tag: 'Refused', reason: 'failed', detail: 'The gateway could not read the attachment.' })
        case 'loaded': return Promise.resolve({ _tag: 'Success', value: { data: PNG_BASE64, offset: 0, sha256: PNG_SHA256, size: PNG_SIZE } })
        case 'decode-failed': return Promise.resolve({ _tag: 'Success', value: attempts === 1
          ? { data: UNDECODABLE_PNG_BASE64, offset: 0, sha256: PNG_SHA256, size: UNDECODABLE_PNG_SIZE }
          : { data: PNG_BASE64, offset: 0, sha256: PNG_SHA256, size: PNG_SIZE } })
        // The cache refuses this type before reading; a read here is a contract failure the play reports.
        case 'unavailable': return Promise.resolve({ _tag: 'Refused', reason: 'failed', detail: 'Unsupported type was read.' })
      }
    }
    const port: Pick<AttachmentPort, 'chunk'> = {
      chunk: request => {
        setRequests(previous => [...previous, request])
        if (!gated) return answer()
        const gate = Promise.withResolvers<void>()
        waiting.push(gate.resolve)
        return gate.promise.then(answer)
      },
    }
    return { images: createTranscriptImages(port), release: () => { for (const open of waiting.splice(0)) open() } }
  })
  React.useLayoutEffect(() => () => images.dispose(), [images])
  const snapshot = React.useSyncExternalStore(images.subscribe, images.snapshot)
  // A new host object per cache snapshot re-renders the transcript whenever a read changes state. The cache decides
  // which ids and types it can never read; everything else renders from the subscribed snapshot.
  const attachmentImages = React.useMemo((): TranscriptAttachmentImages => ({
    state: (id, mediaType) => { const read = images.state(id, mediaType); return read._tag === 'Unavailable' ? read : snapshot.get(id) ?? read },
    load: (attachment: Attachment, messageId: string) => { void images.load(attachment, messageId) },
  }), [images, snapshot])
  const { prompt, items, imageRow, carrier } = rowFixture(row, attachmentFor(fixture))
  const turns: readonly TranscriptTurn[] = [{ id: prompt.id, prompt, items, work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }]
  const received = row === 'prompt' ? '' : `${row} `
  return <NarrowLayoutContext.Provider value={width === 'phone'}>
    <section aria-label={`Transcript image ${received}${fixture} ${width}`} data-fixture={fixture} data-row={row} data-image-row={imageRow} data-carrier={carrier} data-scheme={scheme} {...stylex.props(styles.surface, width === 'phone' ? styles.phone : styles.desktop, scheme === 'dark' && assistantDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
      <div {...stylex.props(styles.transcript)}>
        <EmbraceRuntimeProvider options={{ messages: [prompt, ...items], onNew: async () => {} }}>
        <Transcript turns={turns} title="Image review" landmarkContext={`${received}${fixture}, ${width}`} showHeader={false} sync={{ _tag: 'Live', since: 0 }} now={1792909271000} observedAt={1792909271000} attachmentImages={attachmentImages} />
        </EmbraceRuntimeProvider>
      </div>
      <ol aria-label="Fixture chunk reads" {...stylex.props(styles.reads)}>{requests.map((request, index) => <li key={index} data-sha256={request.sha256} data-message={request.message} data-offset={request.offset ?? 0}>{`Read ${request.sha256.slice(0, 8)} at ${request.offset ?? 0}`}</li>)}</ol>
      {gated && <button type="button" onClick={release} {...stylex.props(styles.release)}>Release reads</button>}
    </section>
  </NarrowLayoutContext.Provider>
}

const at = (second: number) => `2026-10-25T06:21:${String(second).padStart(2, '0')}.000Z`
const reviewer = { kind: 'agent', label: 'reviewer' } as const
/** The turn for one row variant, the id of the row that carries the image, and the carrier message its read must name. */
interface RowFixture {
  readonly prompt: TextItem & { readonly role: 'user' }
  readonly items: readonly ConversationItem[]
  readonly imageRow: string
  readonly carrier: string | undefined
}
const rowFixture = (row: Row, attachment: { readonly name: string; readonly mediaType: string }): RowFixture => {
  const image: Attachment = { id: `blob/${PNG_SHA256}`, mediaType: attachment.mediaType, name: attachment.name, size: PNG_SIZE }
  switch (row) {
    case 'prompt': return {
      prompt: { _tag: 'Text', id: MESSAGE_ID, role: 'user', text: 'Here is the layout I mean.', attachments: [image], streaming: false, at: at(11) },
      items: [], imageRow: MESSAGE_ID, carrier: MESSAGE_ID,
    }
    case 'assistant': {
      const answer = 'timeline-entry/transcript-images/assistant-answer'
      return {
        prompt: { _tag: 'Text', id: 'timeline-entry/transcript-images/prompt', role: 'user', text: 'Render the layout you proposed.', attachments: [], streaming: false, at: at(11) },
        items: [{ _tag: 'Text', id: answer, role: 'assistant', text: 'Here is the rendered layout.', attachments: [image], streaming: false, at: at(14) }],
        imageRow: answer, carrier: undefined,
      }
    }
    case 'mailbox': {
      const body = 'timeline-entry/s1/2222222222222222-content'
      return {
        prompt: { _tag: 'Text', id: 'timeline-entry/transcript-images/prompt', role: 'user', text: 'Ask the reviewer for their layout.', attachments: [], streaming: false, at: at(11) },
        items: [
          { _tag: 'Message', id: 'timeline-entry/s1/2222222222222222-message', messageId: MAILBOX_MESSAGE_ID, from: 'agent/reviewer', to: 'agent/example', title: 'Layout screenshot', at: at(14), sender: reviewer },
          { _tag: 'Text', id: body, role: 'assistant', text: 'This is the layout I meant.', attachments: [{ ...image, messageId: MAILBOX_MESSAGE_ID }], streaming: false, at: at(14), sender: reviewer },
        ],
        imageRow: body, carrier: MAILBOX_MESSAGE_ID,
      }
    }
  }
}

const meta = { excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/Transcript images', component: TranscriptImages,
  args: { scheme: 'dark', width: 'desktop', fixture: 'loaded', gated: false, row: 'prompt' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: ['desktop', 'phone'], control: 'radio' }, fixture: { options: ['loading', 'unauthorized', 'failed', 'loaded', 'unavailable', 'decode-failed'], control: 'radio' }, gated: { control: 'boolean' }, row: { options: ['prompt', 'assistant', 'mailbox'], control: 'radio' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Exercises the production transcript image cache over a deterministic local chunk transport (pending, refused, failing, complete, undecodable then complete, and released on demand) on prompt, assistant and mailbox rows, not a server connection.' } },
} satisfies Meta<typeof TranscriptImages>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStates:contract-fixture',
  'AssistantImage:contract-fixture',
  'AssistantImagePhone:contract-fixture',
  'Contract:contract-fixture',
  'DecodeFailed:contract-fixture',
  'DecodeFailedPhone:contract-fixture',
  'Failed:contract-fixture',
  'FailedPhone:contract-fixture',
  'KeyboardFailed:contract-fixture',
  'KeyboardFailedPhone:contract-fixture',
  'KeyboardLoaded:contract-fixture',
  'KeyboardLoadedPhone:contract-fixture',
  'KeyboardNavigateAway:contract-fixture',
  'KeyboardNavigateAwayPhone:contract-fixture',
  'KeyboardUnauthorized:contract-fixture',
  'KeyboardUnauthorizedPhone:contract-fixture',
  'Loaded:contract-fixture',
  'LoadedPhone:contract-fixture',
  'Loading:contract-fixture',
  'LoadingPhone:contract-fixture',
  'MailboxImage:contract-fixture',
  'MailboxImagePhone:contract-fixture',
  'Unauthorized:contract-fixture',
  'UnauthorizedPhone:contract-fixture',
  'Unavailable:contract-fixture',
  'UnavailablePhone:contract-fixture',
]
type Story = StoryObj<typeof meta>

const card = (root: HTMLElement) => within(root).getByRole('group', { name: `Image: ${NAME}, PNG` })
/** The fixture section a play runs in: the story canvas holds one, AllStates passes each section itself. */
const fixtureSection = (root: HTMLElement) => root.matches('section[data-fixture]') ? root : root.querySelector<HTMLElement>('section[data-fixture]')!
const reads = (root: HTMLElement) => within(root).getByRole('list', { name: 'Fixture chunk reads' })
const status = (root: HTMLElement) => within(root).getByTestId('transcript-attachment-status')
/** Fixture-only: `click()` settles the held reads without moving the person's focus. */
const releaseReads = (root: HTMLElement) => within(root).getByRole('button', { name: 'Release reads' }).click()
const focusStaysOnPage = async () => {
  await expect(document.activeElement).not.toBe(document.body)
  await expect(document.activeElement).not.toBeNull()
}

/** The image stays a deferred card with no read until the person asks for it; then exactly one exact read starts. */
const showImage = async (root: HTMLElement) => {
  const canvas = within(root)
  await expect(card(root)).toHaveAttribute('data-attachment-state', 'deferred')
  await expect(reads(root)).toBeEmptyDOMElement()
  await expect(canvas.queryByRole('button', { name: `Open ${NAME}` })).toBeNull()
  await userEvent.click(canvas.getByRole('button', { name: `Show image ${NAME}` }))
  await expectOneRead(root)
}
const expectOneRead = async (root: HTMLElement) => {
  await waitFor(() => expect(reads(root).children).toHaveLength(1))
  await expectRead(root, reads(root).children[0]!)
}
/** Every read names the exact blob at offset 0 and the row's carrier message, or no message when the row has none. */
const expectRead = async (root: HTMLElement, read: Element) => {
  const carrier = fixtureSection(root).dataset.carrier
  await expect(read).toHaveAttribute('data-sha256', PNG_SHA256)
  if (carrier === undefined) await expect(read).not.toHaveAttribute('data-message')
  else await expect(read).toHaveAttribute('data-message', carrier)
  await expect(read).toHaveAttribute('data-offset', '0')
}
const fitsColumn = async (root: HTMLElement) => {
  await expect(root.scrollWidth).toBeLessThanOrEqual(root.clientWidth)
}

/** Show image stays in place as an inert Loading image button, so the focus it had is kept. */
const expectLoading = async (root: HTMLElement) => {
  const loading = card(root)
  await waitFor(() => expect(loading).toHaveAttribute('data-attachment-state', 'loading'))
  await expect(loading).toHaveAttribute('aria-busy', 'true')
  const pending = within(loading).getByRole('button', { name: `Loading image ${NAME}` })
  await expect(pending).toHaveAttribute('aria-disabled', 'true')
  await expect(pending).toHaveTextContent('Loading image')
  await expect(pending).toBeVisible()
  await expect(status(root)).toHaveTextContent(`Loading image ${NAME}`)
  return pending
}
const showLoading = async (root: HTMLElement) => {
  await showImage(root)
  const pending = await expectLoading(root)
  await expect(pending).toHaveFocus()
  await fitsColumn(root)
  // The fixture read never settles, so the screenshot and a11y pass see the loading card.
}
const showUnauthorized = async (root: HTMLElement) => {
  await showImage(root)
  const refused = card(root)
  await waitFor(() => expect(refused).toHaveAttribute('data-attachment-state', 'unauthorized'))
  await expect(within(refused).getByText('Not authorized')).toBeVisible()
  await expect(within(refused).queryByRole('button')).toBeNull()
  await expect(refused).toHaveFocus()
  await expect(reads(root).children).toHaveLength(1)
  await fitsColumn(root)
}
const showFailed = async (root: HTMLElement) => {
  await showImage(root)
  const failed = card(root)
  await waitFor(() => expect(failed).toHaveAttribute('data-attachment-state', 'failed'))
  await expect(within(failed).getByText("Couldn't load image")).toBeVisible()
  await userEvent.click(within(failed).getByRole('button', { name: `Retry loading ${NAME}` }))
  await waitFor(() => expect(reads(root).children).toHaveLength(2))
  await expectRead(root, reads(root).children[1]!)
  await waitFor(() => expect(card(root)).toHaveAttribute('data-attachment-state', 'failed'))
  const retry = within(card(root)).getByRole('button', { name: `Retry loading ${NAME}` })
  await expect(retry).toBeVisible()
  await expect(retry).toHaveFocus()
  await fitsColumn(root)
}
/** The decoded image replaces the card and takes the focus the card had, fitted within 320×240. */
const expectLoadedImage = async (root: HTMLElement) => {
  const canvas = within(root)
  const open = await canvas.findByRole('button', { name: `Open ${NAME}` })
  await expect(open).toHaveAttribute('data-attachment-state', 'loaded')
  await expect(canvas.queryByTestId('transcript-attachment')).toBeNull()
  await waitFor(() => expect(open).toHaveFocus())
  await expect(status(root)).toHaveTextContent(`Loaded image ${NAME}`)
  const thumbnail = within(open).getByRole('img', { name: NAME }) as HTMLImageElement
  await waitFor(() => expect(thumbnail.complete && thumbnail.naturalWidth).toBe(240))
  const box = thumbnail.getBoundingClientRect()
  await expect(box.width).toBeLessThanOrEqual(320)
  await expect(box.height).toBeLessThanOrEqual(240)
}
const showLoaded = async (root: HTMLElement) => {
  await showImage(root)
  await expectLoadedImage(root)
  await expect(reads(root).children).toHaveLength(1)
  await fitsColumn(root)
}
/**
 * A received row (an assistant answer, or the paired body of agent mail under its header row) shows the same deferred
 * card inside the agent's prose row, reads only on request for the row's carrier, and opens the same viewer.
 */
const showReceived = async (root: HTMLElement) => {
  const section = fixtureSection(root)
  const owner = card(root).closest<HTMLElement>('[data-testid="agent-message"]')
  await expect(owner).not.toBeNull()
  await expect(owner).toHaveAttribute('data-item-id', section.dataset.imageRow)
  await expect(owner).toHaveAttribute('data-item-kind', 'Text')
  await expect(within(section).getAllByTestId('transcript-attachments')).toHaveLength(1)
  if (section.dataset.row === 'mailbox') {
    const header = section.querySelector<HTMLElement>('[data-testid="agent-message"][data-item-kind="Message"]')
    await expect(header).not.toBeNull()
    await expect(within(header!).getByTestId('message-sender')).toHaveTextContent('reviewer')
    await expect(header!.compareDocumentPosition(owner!) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
  }
  await showLoaded(root)
  await expect(within(owner!).getByRole('button', { name: `Open ${NAME}` })).toHaveFocus()
  await openAndCloseViewer(root)
}
/**
 * PNG-signed bytes that the browser's own decoder refuses end as Failed with Retry, never a broken image; the pending
 * URL never reaches an img. Retry reads again and the real PNG then loads.
 */
const showDecodeFailed = async (root: HTMLElement) => {
  await showImage(root)
  const failed = card(root)
  await waitFor(() => expect(failed).toHaveAttribute('data-attachment-state', 'failed'))
  await expect(within(failed).getByText("Couldn't load image")).toBeVisible()
  await expect(status(root)).toHaveTextContent(`Couldn't load ${NAME}`)
  await expect(within(root).queryByRole('img', { name: NAME })).toBeNull()
  const retry = within(failed).getByRole('button', { name: `Retry loading ${NAME}` })
  await expect(retry).toHaveFocus()
  await userEvent.click(retry)
  await waitFor(() => expect(reads(root).children).toHaveLength(2))
  await expectRead(root, reads(root).children[1]!)
  await expectLoadedImage(root)
  await expect(reads(root).children).toHaveLength(2)
  await fitsColumn(root)
}
/** The cache refuses the type before reading: a plain card with nothing to press, no status and no read. */
const showUnavailable = async (root: HTMLElement) => {
  const plain = within(root).getByRole('group', { name: attachmentFor('unavailable').label })
  await expect(plain).toHaveAttribute('data-attachment-state', 'deferred')
  await expect(plain).not.toHaveAttribute('tabindex')
  await expect(within(plain).queryByRole('button')).toBeNull()
  await expect(within(plain).getByText('review-layout.svg')).toBeVisible()
  await expect(within(root).queryByTestId('transcript-attachment-status')).toBeNull()
  await expect(within(root).queryByText(/Not authorized|Couldn't load/)).toBeNull()
  await expect(reads(root)).toBeEmptyDOMElement()
  await fitsColumn(root)
}
/** Opens the viewer and closes it again, so the final screenshot is the thumbnail with focus back on it. Viewing never reads. */
const openAndCloseViewer = async (root: HTMLElement) => {
  const readCount = reads(root).children.length
  const open = within(root).getByRole('button', { name: `Open ${NAME}` })
  await userEvent.click(open)
  const viewer = await within(document.body).findByRole('dialog', { name: NAME })
  await expect(within(viewer).getByRole('img', { name: NAME })).toBeVisible()
  await userEvent.click(within(viewer).getByRole('button', { name: 'Close image' }))
  await waitFor(() => expect(within(document.body).queryByRole('dialog', { name: NAME })).toBeNull())
  await waitFor(() => expect(open).toHaveFocus())
  await expect(reads(root).children).toHaveLength(readCount)
}

type Key = 'Enter' | 'Space'
const press = (key: Key) => userEvent.keyboard(key === 'Enter' ? '{Enter}' : ' ')
/** Keyboard-only: focus Show image, press it, and hold the read in Loading with focus on the same control. */
const keyboardShow = async (root: HTMLElement, key: Key) => {
  const show = within(card(root)).getByRole('button', { name: `Show image ${NAME}` })
  show.focus()
  await expect(show).toHaveFocus()
  await press(key)
  await expectOneRead(root)
  const pending = await expectLoading(root)
  await expect(pending).toBe(show)
  await expect(pending).toHaveFocus()
  // A repeat press while the read runs is ignored.
  await press(key)
  await expect(reads(root).children).toHaveLength(1)
  await focusStaysOnPage()
  return pending
}
const keyboardLoaded = async (root: HTMLElement, key: Key) => {
  await keyboardShow(root, key)
  releaseReads(root)
  const open = await within(root).findByRole('button', { name: `Open ${NAME}` })
  await expect(within(root).queryByTestId('transcript-attachment')).toBeNull()
  await waitFor(() => expect(open).toHaveFocus())
  await expect(status(root)).toHaveTextContent(`Loaded image ${NAME}`)
  await expect(reads(root).children).toHaveLength(1)
  await fitsColumn(root)
}
const keyboardFailed = async (root: HTMLElement, showKey: Key, retryKey: Key) => {
  const action = await keyboardShow(root, showKey)
  releaseReads(root)
  await waitFor(() => expect(card(root)).toHaveAttribute('data-attachment-state', 'failed'))
  const retry = within(card(root)).getByRole('button', { name: `Retry loading ${NAME}` })
  await expect(retry).toBe(action)
  await expect(retry).toHaveFocus()
  await expect(status(root)).toHaveTextContent(`Couldn't load ${NAME}`)
  await press(retryKey)
  await waitFor(() => expect(reads(root).children).toHaveLength(2))
  await expect(await expectLoading(root)).toHaveFocus()
  releaseReads(root)
  await waitFor(() => expect(card(root)).toHaveAttribute('data-attachment-state', 'failed'))
  await expect(within(card(root)).getByRole('button', { name: `Retry loading ${NAME}` })).toHaveFocus()
  await expect(status(root)).toHaveTextContent(`Couldn't load ${NAME}`)
  await focusStaysOnPage()
  await fitsColumn(root)
}
const keyboardUnauthorized = async (root: HTMLElement, key: Key) => {
  await keyboardShow(root, key)
  releaseReads(root)
  const refused = card(root)
  await waitFor(() => expect(refused).toHaveAttribute('data-attachment-state', 'unauthorized'))
  await expect(within(refused).queryByRole('button')).toBeNull()
  // The action left with nothing to retry; the card itself keeps focus.
  await expect(refused).toHaveFocus()
  await expect(status(root)).toHaveTextContent(`Not authorized to show ${NAME}`)
  await focusStaysOnPage()
  await fitsColumn(root)
}
/** The person moves on while the read runs; completion announces the image but never takes focus back. */
const keyboardNavigateAway = async (root: HTMLElement, key: Key) => {
  await keyboardShow(root, key)
  await userEvent.tab({ shift: true })
  const elsewhere = document.activeElement
  await expect(elsewhere).not.toBe(document.body)
  await expect(card(root).contains(elsewhere)).toBe(false)
  releaseReads(root)
  const open = await within(root).findByRole('button', { name: `Open ${NAME}` })
  await expect(status(root)).toHaveTextContent(`Loaded image ${NAME}`)
  await expect(open).not.toHaveFocus()
  await expect(elsewhere).toHaveFocus()
}

export const Loading: Story = { tags: ['critical'], args: { fixture: 'loading' }, play: async ({ canvasElement }) => showLoading(canvasElement) }
export const LoadingPhone: Story = { tags: ['critical'], args: { fixture: 'loading', width: 'phone' }, play: async ({ canvasElement }) => showLoading(canvasElement) }
export const Loaded: Story = {
  tags: ['critical'], args: { fixture: 'loaded' },
  play: async ({ canvasElement }) => { await showLoaded(canvasElement); await openAndCloseViewer(canvasElement) },
}
export const LoadedPhone: Story = {
  tags: ['critical'], args: { fixture: 'loaded', width: 'phone' },
  play: async ({ canvasElement }) => { await showLoaded(canvasElement); await openAndCloseViewer(canvasElement) },
}
export const Unauthorized: Story = { tags: ['critical'], args: { fixture: 'unauthorized' }, play: async ({ canvasElement }) => showUnauthorized(canvasElement) }
export const UnauthorizedPhone: Story = { tags: ['critical'], args: { fixture: 'unauthorized', width: 'phone' }, play: async ({ canvasElement }) => showUnauthorized(canvasElement) }
export const Failed: Story = { tags: ['critical'], args: { fixture: 'failed' }, play: async ({ canvasElement }) => showFailed(canvasElement) }
export const FailedPhone: Story = { tags: ['critical'], args: { fixture: 'failed', width: 'phone' }, play: async ({ canvasElement }) => showFailed(canvasElement) }
export const Unavailable: Story = { tags: ['critical'], args: { fixture: 'unavailable' }, play: async ({ canvasElement }) => showUnavailable(canvasElement) }
export const UnavailablePhone: Story = { tags: ['critical'], args: { fixture: 'unavailable', width: 'phone' }, play: async ({ canvasElement }) => showUnavailable(canvasElement) }
export const DecodeFailed: Story = { tags: ['critical'], args: { fixture: 'decode-failed' }, play: async ({ canvasElement }) => showDecodeFailed(canvasElement) }
export const DecodeFailedPhone: Story = { tags: ['critical'], args: { fixture: 'decode-failed', width: 'phone' }, play: async ({ canvasElement }) => showDecodeFailed(canvasElement) }

// Received rows: the same cards, reads and viewer on an assistant answer and on the paired body of incoming mail.
export const AssistantImage: Story = { tags: ['critical'], args: { fixture: 'loaded', row: 'assistant' }, play: async ({ canvasElement }) => showReceived(canvasElement) }
export const AssistantImagePhone: Story = { tags: ['critical'], args: { fixture: 'loaded', row: 'assistant', width: 'phone' }, play: async ({ canvasElement }) => showReceived(canvasElement) }
export const MailboxImage: Story = { tags: ['critical'], args: { fixture: 'loaded', row: 'mailbox' }, play: async ({ canvasElement }) => showReceived(canvasElement) }
export const MailboxImagePhone: Story = { tags: ['critical'], args: { fixture: 'loaded', row: 'mailbox', width: 'phone' }, play: async ({ canvasElement }) => showReceived(canvasElement) }

// Keyboard focus lifecycle over held reads. Each width presses Enter and Space between its stories.
export const KeyboardLoaded: Story = { tags: ['critical'], args: { fixture: 'loaded', gated: true }, play: async ({ canvasElement }) => keyboardLoaded(canvasElement, 'Enter') }
export const KeyboardLoadedPhone: Story = { tags: ['critical'], args: { fixture: 'loaded', gated: true, width: 'phone' }, play: async ({ canvasElement }) => keyboardLoaded(canvasElement, 'Space') }
export const KeyboardFailed: Story = { tags: ['critical'], args: { fixture: 'failed', gated: true }, play: async ({ canvasElement }) => keyboardFailed(canvasElement, 'Enter', 'Space') }
export const KeyboardFailedPhone: Story = { tags: ['critical'], args: { fixture: 'failed', gated: true, width: 'phone' }, play: async ({ canvasElement }) => keyboardFailed(canvasElement, 'Space', 'Enter') }
export const KeyboardUnauthorized: Story = { tags: ['critical'], args: { fixture: 'unauthorized', gated: true }, play: async ({ canvasElement }) => keyboardUnauthorized(canvasElement, 'Space') }
export const KeyboardUnauthorizedPhone: Story = { tags: ['critical'], args: { fixture: 'unauthorized', gated: true, width: 'phone' }, play: async ({ canvasElement }) => keyboardUnauthorized(canvasElement, 'Enter') }
export const KeyboardNavigateAway: Story = { tags: ['critical'], args: { fixture: 'loaded', gated: true }, play: async ({ canvasElement }) => keyboardNavigateAway(canvasElement, 'Enter') }
export const KeyboardNavigateAwayPhone: Story = { tags: ['critical'], args: { fixture: 'loaded', gated: true, width: 'phone' }, play: async ({ canvasElement }) => keyboardNavigateAway(canvasElement, 'Space') }

/**
 * The commit lane checks the complete attachment contract with one Storybook bootstrap, remounting a fresh viewer
 * cache between probes. Individual stories keep the full scheme/width/key-depth matrix in the merge lane.
 * Loaded remains a separate two-width commit case because its thumbnail geometry is a width-specific contract.
 */
export const Contract: Story = {
  tags: ['critical'],
  play: async ({ mount, args }) => {
    const probes: readonly {
      readonly fixture: Fixture
      readonly row?: Row
      readonly gated?: boolean
      readonly run: (root: HTMLElement) => Promise<void>
    }[] = [
      { fixture: 'loading', run: showLoading },
      { fixture: 'loaded', run: async root => { await showLoaded(root); await openAndCloseViewer(root) } },
      { fixture: 'unauthorized', run: showUnauthorized },
      { fixture: 'failed', run: showFailed },
      { fixture: 'unavailable', run: showUnavailable },
      { fixture: 'decode-failed', run: showDecodeFailed },
      { fixture: 'loaded', row: 'assistant', run: showReceived },
      { fixture: 'loaded', row: 'mailbox', run: showReceived },
      { fixture: 'loaded', gated: true, run: root => keyboardLoaded(root, 'Enter') },
      { fixture: 'failed', gated: true, run: root => keyboardFailed(root, 'Enter', 'Space') },
      { fixture: 'unauthorized', gated: true, run: root => keyboardUnauthorized(root, 'Space') },
      { fixture: 'loaded', gated: true, run: root => keyboardNavigateAway(root, 'Enter') },
    ]
    for (const [index, probe] of probes.entries()) {
      const canvas = await mount(<TranscriptImages key={index} {...args} fixture={probe.fixture} row={probe.row} gated={probe.gated} />)
      await probe.run(canvas.getByRole('region', { name: /^Transcript image / }))
    }
  },
}

const fixtures: readonly Fixture[] = ['loading', 'loaded', 'unauthorized', 'failed', 'unavailable', 'decode-failed']
const show: Readonly<Record<Fixture, (root: HTMLElement) => Promise<void>>> = { loading: showLoading, loaded: showLoaded, unauthorized: showUnauthorized, failed: showFailed, unavailable: showUnavailable, 'decode-failed': showDecodeFailed }
/** Every state at the desktop and phone widths, each over its own cache and transport. */
export const AllStates: Story = {
  render: args => <>{(['desktop', 'phone'] as const).flatMap(width => fixtures.map(fixture => <TranscriptImages key={`${width}:${fixture}`} scheme={args.scheme} width={width} fixture={fixture} />))}</>,
  play: async ({ canvasElement }) => {
    for (const section of canvasElement.querySelectorAll<HTMLElement>('section[data-fixture]')) await show[section.dataset.fixture as Fixture](section)
  },
}

const styles = stylex.create({
  surface: { display: 'flex', flexDirection: 'column', gap: space.md, padding: space.lg, minWidth: 0, maxWidth: '100%', boxSizing: 'border-box', backgroundColor: surface.canvas, color: text.fg },
  desktop: { width: '1280px' },
  phone: { width: '390px' },
  transcript: { height: '320px', minWidth: 0 },
  reads: { margin: 0, paddingInlineStart: space.lg, color: text.fgMuted, fontSize: '12px' },
  release: { alignSelf: 'flex-start' },
})
