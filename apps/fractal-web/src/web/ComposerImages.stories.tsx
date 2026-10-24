import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { EmbraceComposer, EmbraceRuntimeProvider, Transcript, assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme, compositionLightTheme, type ConversationRuntimeOptions, type TextItem, type TranscriptTurn } from '@fractal/ui/assistant-ui'
import { surfaceVars as surface, spaceVars as space, textVars as text } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { imageUploadAdapter } from './composerSend.ts'

/** The production adapter owns acceptance and size limits; only the upload transport is a controlled fixture. */
function ComposerImages({ scheme, slow = false, previewImages = false }: { readonly scheme: 'dark' | 'light'; readonly slow?: boolean; readonly previewImages?: boolean }) {
  const [receipts, setReceipts] = React.useState<readonly string[]>([])
  const [messages, setMessages] = React.useState<readonly (TextItem & { readonly role: 'user' })[]>([])
  const [blobUrls] = React.useState(() => new Map<string, string>())
  const [transportFinished, setTransportFinished] = React.useState(false)
  const [upload] = React.useState(() => {
    let finish!: () => void
    const pending = new Promise<void>(resolve => { finish = resolve })
    return { pending, finish }
  })
  const [adapter] = React.useState(() => imageUploadAdapter(async ({ bytes, mediaType }) => {
    if (slow) await upload.pending
    const file = bytes instanceof Blob ? bytes : new Blob([bytes instanceof ArrayBuffer ? bytes : new Uint8Array(bytes).buffer], { type: mediaType })
    const sha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', await file.arrayBuffer()))].map(byte => byte.toString(16).padStart(2, '0')).join('')
    const blob = `blob/${sha256}`
    const url = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = () => resolve(String(reader.result))
      reader.onerror = () => reject(reader.error)
      reader.readAsDataURL(file)
    })
    blobUrls.set(blob, url)
    setTransportFinished(true)
    return { _tag: 'Success', value: { blob, sha256, size: file.size, media_type: mediaType } }
  }))
  const options: ConversationRuntimeOptions = {
    messages, adapters: { attachments: adapter },
    onNew: async message => {
      const count = message.attachments?.length ?? 0
      setReceipts(previous => [...previous, `Sent ${count} ${count === 1 ? 'image' : 'images'}`])
      setMessages(previous => [...previous, {
        _tag: 'Text', id: `fixture-send/${previous.length + 1}`, role: 'user',
        text: message.content.filter(part => part.type === 'text').map(part => part.text).join('\n'),
        attachments: (message.attachments ?? []).map(attachment => {
          const part = attachment.content.find(part => part.type === 'file' && part.sourceType === 'id')
          return { id: part?.type === 'file' ? part.data : attachment.id, mediaType: attachment.contentType ?? 'application/octet-stream', name: attachment.name }
        }),
        streaming: false, at: '2026-10-24T06:21:11.000Z',
      }])
    },
  }
  const turns: readonly TranscriptTurn[] = messages.map(prompt => ({ id: prompt.id, prompt, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }))
  return <section aria-label={slow ? 'Controlled image upload' : 'Live image adapter'} data-scheme={scheme} {...stylex.props(styles.surface, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
    <EmbraceRuntimeProvider options={options}>
      {messages.length > 0 && <div {...stylex.props(styles.transcript)}><Transcript turns={turns} title="Image review" showHeader={false} sync={{ _tag: 'Live', since: 0 }} now={1792822871000} observedAt={1792822871000} {...(previewImages ? { resolveImage: (id: string) => { const src = blobUrls.get(id); return src === undefined ? { _tag: 'Defer' as const } : { _tag: 'Load' as const, src } } } : {})} /></div>}
      <EmbraceComposer variant="C1" readingColumn conversationLabel="Image review" imageAttachments={{ max: 4 }} />
    </EmbraceRuntimeProvider>
    {slow && <button onClick={upload.finish}>Complete fixture upload</button>}
    {slow && transportFinished && <p data-testid="fixture-upload-completed">Fixture upload completed</p>}
    <div role="log" aria-label="Fixture send receipts">{receipts.map((receipt, index) => <p key={index}>{receipt}</p>)}</div>
  </section>
}

const meta = {
  title: 'Fractal/App/Composer images', component: ComposerImages,
  args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Exercises the live image adapter with deterministic local upload transport and controlled completion, not a server connection.' } },
} satisfies Meta<typeof ComposerImages>
export default meta
type Story = StoryObj<typeof meta>

const pasteImages = async (root: HTMLElement, names: readonly string[]) => {
  const files = new DataTransfer()
  const bytes = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aRXsAAAAASUVORK5CYII='), character => character.charCodeAt(0))
  for (const name of names) files.items.add(new File([bytes], name, { type: 'image/png' }))
  await userEvent.click(within(root).getByRole('textbox', { name: 'Message' }))
  await userEvent.paste(files)
  await waitFor(() => expect(within(root).getByRole('img', { name: names.at(-1)! })).toBeVisible())
}

const refuseOversize = async (root: HTMLElement) => {
  const canvas = within(root)
  const files = new DataTransfer()
  files.items.add(new File([new Uint8Array(12 * 1024 * 1024)], 'large-photo.png', { type: 'image/png' }))
  await userEvent.click(canvas.getByRole('textbox', { name: 'Message' }))
  await userEvent.paste(files)
  const notice = canvas.getByTestId('composer-attachment-notice')
  await waitFor(() => expect(notice).toHaveTextContent('Images must be 10 MiB or smaller. Choose a smaller image and try again.'))
  await expect(notice).toHaveAttribute('aria-live', 'polite')
  await expect(canvas.queryByTestId('composer-attachments')).toBeNull()
  await expect(canvas.getByRole('button', { name: 'Attach image' })).toBeVisible()
  await expect(canvas.getByRole('button', { name: 'Send' })).toBeDisabled()
}
const showUploading = async (root: HTMLElement) => {
  const canvas = within(root)
  await pasteImages(root, ['workspace-review.png'])
  await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
  await waitFor(() => expect(canvas.getByTestId('composer-upload')).toHaveTextContent('Uploading workspace-review.png'))
  await expect(canvas.getByRole('button', { name: 'Cancel upload' })).toBeVisible()
  await expect(canvas.getByRole('button', { name: 'Send' })).toBeDisabled()
  await expect(canvas.getByRole('log', { name: 'Fixture send receipts' })).toBeEmptyDOMElement()
  // Leave the deterministic upload pending for the screenshot/a11y pass. Manual completion uses the fixture button.
}
const showSeveralImages = async (root: HTMLElement) => {
  const canvas = within(root)
  await pasteImages(root, ['workspace-overview.png', 'composer-permission-loss-detail.png', 'image-upload-refusal.png'])
  await expect(canvas.getAllByRole('img')).toHaveLength(3)
  await expect(canvas.getAllByRole('button', { name: /^Remove / })).toHaveLength(3)
  await expect(canvas.getByRole('button', { name: 'Attach image' })).toBeVisible()
  await expect(canvas.getByRole('button', { name: 'Send' })).toBeEnabled()
  await expect(root.scrollWidth).toBeLessThanOrEqual(root.clientWidth)
}
const showSentImage = async (root: HTMLElement) => {
  const canvas = within(root)
  await pasteImages(root, ['sent-layout.png'])
  await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
  await waitFor(() => expect(canvas.getByRole('log', { name: 'Fixture send receipts' })).toHaveTextContent('Sent 1 image'))
  await expect(canvas.getByRole('group', { name: 'Image: sent-layout.png, PNG' })).toBeVisible()
  await expect(canvas.queryByRole('img', { name: 'sent-layout.png' })).toBeNull()
}

export const SizeRefusal: Story = { tags: ['critical'], play: async ({ canvasElement }) => refuseOversize(canvasElement) }
export const UploadInProgress: Story = { tags: ['critical'], args: { slow: true }, play: async ({ canvasElement }) => showUploading(canvasElement) }
export const UploadCancelled: Story = {
  tags: ['critical'], args: { slow: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await showUploading(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: 'Cancel upload' }))
    await waitFor(() => expect(canvas.getByRole('img', { name: 'workspace-review.png' })).toBeVisible())
    await expect(canvas.getByRole('button', { name: 'Remove workspace-review.png' })).toBeVisible()
    await userEvent.click(canvas.getByRole('button', { name: 'Complete fixture upload' }))
    await waitFor(() => expect(canvas.getByTestId('fixture-upload-completed')).toBeVisible())
    await expect(canvas.getByRole('log', { name: 'Fixture send receipts' })).toBeEmptyDOMElement()
    await userEvent.click(canvas.getByRole('button', { name: 'Remove workspace-review.png' }))
    await expect(canvas.queryByTestId('composer-attachments')).toBeNull()
  },
}
export const SeveralImages: Story = { tags: ['critical'], play: async ({ canvasElement }) => showSeveralImages(canvasElement) }
export const SentImage: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => showSentImage(canvasElement),
}
/** Follow-up target only: the live app does not yet supply an authorized blob resolver (#3778). */
export const ResolvedImageFollowup: Story = {
  tags: ['critical'], args: { previewImages: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await pasteImages(canvasElement, ['sent-layout.png'])
    await userEvent.click(canvas.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Open sent-layout.png' })).toBeVisible())
    await userEvent.click(canvas.getByRole('button', { name: 'Open sent-layout.png' }))
    await expect(within(document.body).getByRole('dialog', { name: 'sent-layout.png' })).toBeVisible()
  },
}
export const LongFilename: Story = {
  tags: ['critical'],
  play: async ({ canvasElement }) => {
    await pasteImages(canvasElement, ['Screenshot 2026-10-24 at 06.21.11.png'])
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('button', { name: 'Remove Screenshot 2026-10-24 at 06.21.11.png' })).toBeVisible()
    await expect(canvasElement.scrollWidth).toBeLessThanOrEqual(canvasElement.clientWidth)
  },
}
export const AllStates: Story = {
  render: args => <><ComposerImages scheme={args.scheme} /><ComposerImages scheme={args.scheme} slow /><ComposerImages scheme={args.scheme} /><ComposerImages scheme={args.scheme} /></>,
  play: async ({ canvasElement }) => {
    const sections = canvasElement.querySelectorAll<HTMLElement>('section[aria-label="Live image adapter"], section[aria-label="Controlled image upload"]')
    await refuseOversize(sections[0]!)
    await showUploading(sections[1]!)
    await showSeveralImages(sections[2]!)
    await showSentImage(sections[3]!)
  },
}

const styles = stylex.create({
  surface: { display: 'flex', flexDirection: 'column', gap: space.md, padding: space.lg, minWidth: 0, backgroundColor: surface.canvas, color: text.fg },
  transcript: { height: '320px', minWidth: 0 },
})
