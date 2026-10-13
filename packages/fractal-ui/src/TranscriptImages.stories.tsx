import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { Transcript, type TranscriptTurn } from './assistant-ui/composition/Transcript'
import type { TranscriptImage, TranscriptImageLoad } from './assistant-ui/composition/Images'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import type { ConversationItem, TextItem } from './assistant-ui/embrace-data/model'
import { workLogTurnFromItems } from './assistant-ui/taste/work-log'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'

const now = Date.parse('2026-01-15T12:01:00Z')
const at = '2026-01-15T12:00:00Z'
type Loader = (spec: Paint) => () => Promise<TranscriptImageLoad>
interface Paint { readonly w: number; readonly h: number; readonly hue: number; readonly label: string }
/** Synthetic bitmaps drawn in the page: no network, no files, no green hues. */
const paint: Loader = ({ w, h, hue, label }) => () => new Promise((resolve, reject) => {
  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const context = canvas.getContext('2d')!
  const gradient = context.createLinearGradient(0, 0, w, h)
  gradient.addColorStop(0, `hsl(${hue} 55% 38%)`)
  gradient.addColorStop(1, `hsl(${hue + 30} 60% 18%)`)
  context.fillStyle = gradient
  context.fillRect(0, 0, w, h)
  context.strokeStyle = 'rgba(255, 255, 255, 0.35)'
  context.lineWidth = Math.max(2, w / 200)
  for (let x = 0; x < w; x += w / 8) context.strokeRect(x + w / 32, h / 6, w / 12, h / 2)
  context.fillStyle = '#ffffff'
  context.font = `${Math.round(h / 8)}px sans-serif`
  context.fillText(label, w / 20, h * 0.85)
  canvas.toBlob(blob => blob === null ? reject(new Error('Canvas did not encode')) : resolve({ _tag: 'Loaded', blob }), 'image/png')
})
const pending: Loader = () => () => new Promise(() => {})

interface ImageSpec extends Paint { readonly id: string; readonly anchor: string; readonly origin: TranscriptImage['origin']; readonly title: string; readonly bytes: number }
const specs: readonly ImageSpec[] = [
  { id: 'img-1', anchor: 'turn-a/prompt', origin: 'input', title: 'From Operator · web', w: 1200, h: 750, hue: 220, label: 'wide layout', bytes: 245_000 },
  { id: 'img-2', anchor: 'turn-a/prompt', origin: 'input', title: 'From Operator · web', w: 800, h: 600, hue: 260, label: 'sidebar', bytes: 120_000 },
  { id: 'img-3', anchor: 'turn-a/prompt', origin: 'input', title: 'From Operator · web', w: 640, h: 960, hue: 300, label: 'narrow', bytes: 98_000 },
  { id: 'img-4', anchor: 'turn-a/shot', origin: 'tool', title: 'Screenshot · layout-wide.png', w: 1600, h: 1000, hue: 200, label: 'capture', bytes: 287_000 },
  { id: 'img-5', anchor: 'turn-a/answer', origin: 'prose', title: 'Annotated header spacing', w: 1000, h: 400, hue: 20, label: 'annotated', bytes: 76_000 },
  { id: 'img-6', anchor: 'turn-b/read-1', origin: 'tool', title: 'Read · panel-1.png', w: 900, h: 600, hue: 240, label: 'panel 1', bytes: 110_000 },
  { id: 'img-7', anchor: 'turn-b/read-1', origin: 'tool', title: 'Read · panel-2.png', w: 900, h: 600, hue: 270, label: 'panel 2', bytes: 112_000 },
  { id: 'img-8', anchor: 'turn-b/read-2', origin: 'tool', title: 'Read · panel-3.png', w: 900, h: 600, hue: 330, label: 'panel 3', bytes: 108_000 },
  { id: 'img-9', anchor: 'turn-b/read-2', origin: 'tool', title: 'Read · panel-4.png', w: 900, h: 600, hue: 30, label: 'panel 4', bytes: 104_000 },
]
const metaFor = (spec: ImageSpec) => [spec.origin === 'input' ? 'Operator' : 'Assistant', spec.origin === 'input' ? 'input' : spec.origin === 'tool' ? 'tool result' : 'reply', `${spec.w} × ${spec.h}`, `${Math.round(spec.bytes / 1000)} KB`]
const imageFrom = (spec: ImageSpec, load: Loader = paint): TranscriptImage => ({ id: spec.id, anchor: spec.anchor, origin: spec.origin, load: load(spec), width: spec.w, height: spec.h, bytes: spec.bytes, mediaType: 'image/png', caption: { title: spec.title, meta: metaFor(spec) } })

const toolCall = (id: string, name: string, path: string, content: string): ConversationItem => ({ _tag: 'ToolCall', id, callId: id, name, input: { path }, status: 'success', result: { content, mediaType: 'text/plain', isError: false, at: '2026-01-15T12:00:09Z' }, callSeen: true, at: '2026-01-15T12:00:02Z' })
function turnOf(prefix: string, promptText: string, calls: readonly ConversationItem[], answerText: string): TranscriptTurn {
  const prompt: TextItem & { role: 'user' } = { _tag: 'Text', id: `${prefix}/prompt`, role: 'user', text: promptText, attachments: [], streaming: false, at, sender: { kind: 'human', label: 'Operator' } }
  const answer: TextItem = { _tag: 'Text', id: `${prefix}/answer`, role: 'assistant', text: answerText, attachments: [], streaming: false, at: '2026-01-15T12:00:20Z', sender: { kind: 'agent', label: 'Assistant' } }
  const items = [...calls, answer]
  return { id: prefix, prompt, items, work: workLogTurnFromItems([prompt, ...items], { kindFor: () => 'read', running: false, failed: false, interrupted: false, durationMs: 18000, startedAt: at, completeHistory: true }) }
}
const turns: readonly TranscriptTurn[] = [
  turnOf('turn-a', 'Here are three captures of the layout. Check the header spacing.', [toolCall('turn-a/shot', 'screenshot', 'layout-wide.png', 'Captured image [image/png]')], 'The header spacing is uneven at the wide breakpoint; the annotated capture marks the gap.'),
  turnOf('turn-b', 'Compare the four panel exports.', [toolCall('turn-b/read-1', 'read', 'panel-1.png', 'Read image file [image/png]'), toolCall('turn-b/read-2', 'read', 'panel-3.png', 'Read image file [image/png]')], 'All four panels share the same grid; panel 3 has a clipped label.'),
]
const foldedTurns: readonly TranscriptTurn[] = [turns[0]!]

function ImagesTranscript({ scheme, data, images, label }: { readonly scheme: Scheme; readonly data: readonly TranscriptTurn[]; readonly images: readonly TranscriptImage[]; readonly label: string }) {
  const messages = React.useMemo(() => data.flatMap(turn => turn.prompt === undefined ? turn.items : [turn.prompt, ...turn.items]), [data])
  const options = React.useMemo(() => ({ messages, isRunning: false, onNew: async () => {} }), [messages])
  return <main aria-label={label} data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <EmbraceRuntimeProvider options={options}><Transcript title="Layout review" turns={data} sync={{ _tag: 'Live', since: now - 5000 }} now={now} observedAt={now - 8000} onOpenTool={() => {}} images={images} /></EmbraceRuntimeProvider>
    <p data-testid="contrast-probe" {...stylex.props(styles.probe)}>Probe</p>
  </main>
}

/** WCAG ratio of an element's painted ink against its first opaque ancestor background. */
function contrastOf(element: Element): number {
  const foreground = getComputedStyle(element).color.match(/[\d.]+/g)!.map(Number)
  let opacity = 1
  let background: number[] | undefined
  for (let node: Element | null = element; node !== null; node = node.parentElement) {
    const css = getComputedStyle(node)
    opacity *= Number(css.opacity)
    const color = css.backgroundColor.match(/[\d.]+/g)!.map(Number)
    if (background === undefined && color.length >= 3 && (color[3] ?? 1) === 1) background = color
  }
  if (background === undefined) throw new Error('Contrast requires an opaque background')
  const alpha = (foreground[3] ?? 1) * opacity
  const painted = foreground.slice(0, 3).map((channel, index) => channel * alpha + background![index]! * (1 - alpha))
  const luminance = (channels: readonly number[]) => channels.slice(0, 3).reduce((sum, channel, index) => {
    const srgb = channel / 255
    return sum + [0.2126, 0.7152, 0.0722][index]! * (srgb <= 0.04045 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4)
  }, 0)
  const fg = luminance(painted), bg = luminance(background)
  return (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05)
}
/** Every listed element meets AA; the dimmed probe proves the measurement can fail. */
async function expectReadable(elements: readonly Element[], probe: HTMLElement) {
  for (const element of elements) await expect(contrastOf(element)).toBeGreaterThanOrEqual(4.5)
  probe.style.opacity = '0.2'
  await expect(contrastOf(probe)).toBeLessThan(4.5)
  probe.style.opacity = ''
}
const viewer = () => within(document.body).findByTestId('image-viewer')
async function expectCounter(text: string) {
  await waitFor(() => expect(document.querySelector('[data-testid="viewer-counter"]')).toHaveTextContent(text))
}
const noUnknown = async (root: Element) => expect(root.textContent ?? '').not.toMatch(/\bUnknown\b/)

const meta = {
  title: 'Fractal UI/Transcript images',
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark' as Scheme },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } },
  render: ({ scheme }) => <ImagesTranscript scheme={scheme} data={turns} images={specs.map(spec => imageFrom(spec))} label="Transcript with images" />,
} satisfies Meta<{ scheme: Scheme }>
export default meta
type Story = StoryObj<typeof meta>

export const ManyImages: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await waitFor(() => expect(canvasElement.querySelectorAll('[data-testid="transcript-image"][data-image-state="ready"]')).toHaveLength(4))
  const inputs = canvasElement.querySelector('[data-image-placement="input"]')!
  await expect(within(inputs as HTMLElement).getAllByRole('button', { name: /^View image: From Operator/ })).toHaveLength(3)
  const foldA = canvas.getByRole('button', { name: /Worked for 18s · 1 image/ })
  const foldB = canvas.getByRole('button', { name: /Worked for 18s · 4 images/ })
  await expect(foldA).toHaveAttribute('aria-expanded', 'false')
  // Folded work keeps its screenshots visible as thumbs; several images cap at three.
  const foldThumbsB = within(foldB.closest('[data-testid="work-log-fold"]') as HTMLElement).getAllByRole('button', { name: /^View image: Read · panel-/ })
  await expect(foldThumbsB).toHaveLength(3)
  await userEvent.click(foldThumbsB[0]!)
  const dialog = await viewer()
  await expect(dialog).toHaveAccessibleName('Image 6 of 9: Read · panel-1.png')
  await expectCounter('6 of 9')
  // Prev crosses from the tool screenshots into the previous turn's reply image.
  await userEvent.click(within(dialog).getByRole('button', { name: 'Previous image' }))
  await expectCounter('5 of 9')
  await waitFor(() => expect(within(dialog).getByTestId('viewer-image')).toHaveAttribute('alt', 'Annotated header spacing'))
  await expect([...dialog.querySelectorAll('[data-testid="viewer-caption"]')].find(element => (element as HTMLElement).offsetParent !== null)).toHaveTextContent('Assistant · reply · 1000 × 400 · 76 KB')
  if (window.innerWidth >= 640) {
    const film = within(dialog).getByRole('listbox', { name: 'Images in this thread' })
    const options = within(film).getAllByRole('option')
    await expect(options).toHaveLength(9)
    await expect(options[4]).toHaveAttribute('aria-selected', 'true')
    await userEvent.click(options[0]!)
    await expectCounter('1 of 9')
    await expect(within(dialog).queryByRole('button', { name: 'Previous image' })).toBeNull()
  }
  await expectReadable([within(dialog).getByTestId('viewer-counter'), ...dialog.querySelectorAll('[data-testid="viewer-caption"] span')].filter(element => (element as HTMLElement).offsetParent !== null), canvas.getByTestId('contrast-probe'))
  await noUnknown(document.body)
  await userEvent.click(within(dialog).getByRole('button', { name: 'Close image viewer' }))
  await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
  await waitFor(() => expect(foldThumbsB[0]).toHaveFocus())
} }

export const FoldedScreenshot: Story = {
  render: ({ scheme }) => <ImagesTranscript scheme={scheme} data={foldedTurns} images={specs.filter(spec => spec.anchor.startsWith('turn-a/')).map(spec => imageFrom(spec))} label="Folded screenshot" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const fold = await canvas.findByRole('button', { name: /Worked for 18s · 1 image/ })
    const foldRow = fold.closest('[data-testid="work-log-fold"]') as HTMLElement
    const thumb = await within(foldRow).findByRole('button', { name: 'View image: Screenshot · layout-wide.png' })
    await waitFor(() => expect(foldRow.querySelector('[data-testid="image-row-thumb"]')).toHaveAttribute('data-image-state', 'ready'))
    const box = foldRow.querySelector('[data-testid="image-row-thumb"]')!.getBoundingClientRect()
    await expect([Math.round(box.width), Math.round(box.height)]).toEqual([22, 16])
    await userEvent.click(thumb)
    const dialog = await viewer()
    await expectCounter('4 of 5')
    await userEvent.click(within(dialog).getByRole('button', { name: 'Close image viewer' }))
    await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
    // Opening the fold moves the thumb into its row: decoration inside the row button, with the row's facts.
    await userEvent.click(fold)
    await waitFor(() => expect(fold).toHaveAttribute('aria-expanded', 'true'))
    await expect(foldRow.querySelector('[data-testid="image-row-thumbs"]')).toBeNull()
    const row = await canvas.findByRole('button', { name: /screenshot.*1600×1000 · 280 KB/ })
    await expect(within(row).queryByRole('button')).toBeNull()
    // The settled preview shows the opened body: one image fit within 480×320.
    const opened = await waitFor(() => {
      const element = canvasElement.querySelector('[data-image-placement="opened"] [data-testid="transcript-image"]')
      if (element === null) throw new Error('Opened image not rendered')
      return element
    })
    const openedBox = opened.getBoundingClientRect()
    await expect(openedBox.width).toBeLessThanOrEqual(480.5)
    await expect(openedBox.height).toBeLessThanOrEqual(320.5)
    await expect(Math.abs(openedBox.width / openedBox.height - 1.6)).toBeLessThan(0.02)
    await userEvent.click(within(opened as HTMLElement).getByRole('button', { name: 'View image: Screenshot · layout-wide.png' }))
    const reopened = await viewer()
    if (window.innerWidth >= 640) {
      await userEvent.click(within(reopened).getByRole('button', { name: 'Show in transcript' }))
      await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
      await waitFor(() => expect(document.activeElement).toBe(canvasElement.querySelector('[data-image-anchor="turn-a/shot"]')))
    } else {
      await expect(within(reopened).getByTestId('viewer-zoom')).not.toBeVisible()
      await userEvent.keyboard('{Escape}')
      await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
    }
    await noUnknown(canvasElement)
  },
}

export const Loading: Story = {
  render: ({ scheme }) => <ImagesTranscript scheme={scheme} data={turns} images={specs.map(spec => imageFrom(spec, pending))} label="Images loading" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvasElement.querySelectorAll('[data-testid="transcript-image"][data-image-state="loading"]')).toHaveLength(4))
    await expect(canvasElement.querySelector('img')).toBeNull()
    // The reply image reserves its final 1000×400 box before any byte arrives.
    const prose = canvasElement.querySelector('[data-image-placement="prose"] [data-testid="transcript-image"]')!
    const box = prose.getBoundingClientRect()
    await expect(box.height).toBeGreaterThan(0)
    await expect(Math.abs(box.width / box.height - 2.5)).toBeLessThan(0.02)
    await expect(box.width).toBeLessThanOrEqual(1000)
    const tile = within(prose as HTMLElement).getByRole('button', { name: 'View image: Annotated header spacing' })
    await expect(prose).toHaveAttribute('aria-busy', 'true')
    await expect(prose.querySelector('[data-testid="image-skeleton"]')).not.toBeNull()
    await userEvent.click(tile)
    const dialog = await viewer()
    await expect(within(dialog).getByRole('status', { name: 'Loading Annotated header spacing' })).toBeInTheDocument()
    await expect(within(dialog).getByTestId('viewer-zoom-readout')).toBeDisabled()
    await expectReadable([within(dialog).getByTestId('viewer-counter'), ...[...dialog.querySelectorAll('[data-testid="viewer-caption"] > *')].filter(element => (element as HTMLElement).offsetParent !== null)], canvas.getByTestId('contrast-probe'))
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
    await noUnknown(canvasElement)
  },
}

const retried = fn()
const download = fn()
function failedImages(): readonly TranscriptImage[] {
  let attempts = 0
  const flaky: Loader = spec => () => {
    attempts += 1
    retried(attempts)
    return attempts === 1 ? Promise.reject(new Error('network')) : paint(spec)()
  }
  return specs.map(spec => spec.id === 'img-1' ? imageFrom(spec, flaky)
    : spec.id === 'img-4' ? imageFrom(spec, () => () => Promise.resolve({ _tag: 'Gone' }))
    : spec.id === 'img-5' ? { ...imageFrom(spec, () => () => Promise.resolve({ _tag: 'TooLarge', bytes: 40 * 1024 * 1024 })), onDownload: download }
    : spec.id === 'img-2' ? imageFrom(spec, () => () => Promise.resolve({ _tag: 'Unsupported' }))
    : imageFrom(spec))
}
function FailedStory({ scheme }: { readonly scheme: Scheme }) {
  const [images] = React.useState(failedImages)
  return <ImagesTranscript scheme={scheme} data={turns} images={images} label="Images failing" />
}
export const Failed: Story = {
  render: ({ scheme }) => <FailedStory scheme={scheme} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    retried.mockClear()
    download.mockClear()
    const failed = await waitFor(() => {
      const element = canvasElement.querySelector('[data-testid="transcript-image"][data-image-id="img-1"][data-image-state="failed"]')
      if (element === null) throw new Error('Failed tile not rendered')
      return element as HTMLElement
    })
    await expect(failed).toHaveTextContent('Couldn’t load image')
    const gone = await waitFor(() => {
      const element = canvasElement.querySelector('[data-image-anchor~="turn-a/shot"] [data-image-state="gone"]')
      if (element === null) throw new Error('Gone thumb not rendered')
      return element
    })
    await expect(gone).not.toBeNull()
    await waitFor(() => expect(canvasElement.querySelector('[data-image-id="img-5"]')).toHaveTextContent('Image is 40 MB'))
    await expect(canvasElement.querySelector('[data-image-id="img-2"]')).toHaveTextContent('This image type can’t be shown')
    // Unsupported without a host download offers no dead button.
    await expect(within(canvasElement.querySelector('[data-image-id="img-2"]') as HTMLElement).queryByRole('button')).toBeNull()
    await userEvent.click(canvas.getByRole('button', { name: 'Download Annotated header spacing' }))
    await expect(download).toHaveBeenCalledTimes(1)
    // No status tile is an empty box.
    for (const element of canvasElement.querySelectorAll('[data-testid="transcript-image"]')) await expect((element.querySelector('img') !== null || (element.textContent ?? '').trim().length > 0) || element.getAttribute('data-image-state') === 'loading').toBe(true)
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry loading From Operator · web' }))
    await waitFor(() => expect(canvasElement.querySelector('[data-testid="transcript-image"][data-image-id="img-1"]')).toHaveAttribute('data-image-state', 'ready'))
    await expect(retried).toHaveBeenCalledTimes(2)
    // In the viewer, a gone image keeps its place and prev/next still work.
    await userEvent.click(within(canvasElement.querySelector('[data-image-id="img-1"]') as HTMLElement).getByRole('button', { name: 'View image: From Operator · web' }))
    const dialog = await viewer()
    await expectCounter('1 of 9')
    await userEvent.keyboard('{ArrowRight}{ArrowRight}{ArrowRight}')
    await expectCounter('4 of 9')
    await waitFor(() => expect(within(dialog).getByRole('group', { name: 'Screenshot · layout-wide.png: Image no longer available · transcript changed' })).toBeInTheDocument())
    await userEvent.click(within(dialog).getByRole('button', { name: 'Next image' }))
    await expectCounter('5 of 9')
    await expectReadable([...canvasElement.querySelectorAll('[data-testid="transcript-image"] [data-image-state] > span')].filter(element => element.textContent !== ''), canvas.getByTestId('contrast-probe'))
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
    await noUnknown(canvasElement)
  },
}

export const Narrow440: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await waitFor(() => expect(canvasElement.querySelectorAll('[data-testid="transcript-image"][data-image-state="ready"]')).toHaveLength(4))
  const lane = canvas.getByTestId('transcript-scroll')
  await expect(lane.scrollWidth).toBeLessThanOrEqual(lane.clientWidth)
  for (const image of canvasElement.querySelectorAll('[data-testid="transcript-image"]')) {
    const box = image.getBoundingClientRect()
    await expect(box.right).toBeLessThanOrEqual(lane.getBoundingClientRect().right + 0.5)
  }
  const prose = canvasElement.querySelector('[data-image-placement="prose"] [data-testid="transcript-image"]')!.getBoundingClientRect()
  await expect(prose.height).toBeLessThanOrEqual(400.5)
  await userEvent.click(canvas.getAllByRole('button', { name: 'View image: From Operator · web' })[1]!)
  const dialog = await viewer()
  await expectCounter('2 of 9')
  const narrow = window.innerWidth < 640
  const film = within(dialog).getByTestId('viewer-filmstrip')
  const zoom = within(dialog).getByTestId('viewer-zoom')
  const close = within(dialog).getByRole('button', { name: 'Close image viewer' })
  await expect(close).toBeVisible()
  await expect(within(dialog).getByTestId('viewer-counter')).toBeVisible()
  if (narrow) {
    // Under 640 the bar keeps Close and the counter; the caption moves under the stage and swipes step.
    await expect(film).not.toBeVisible()
    await expect(zoom).not.toBeVisible()
    const captions = [...dialog.querySelectorAll('[data-testid="viewer-caption"]')].filter(element => (element as HTMLElement).offsetParent !== null)
    await expect(captions).toHaveLength(1)
    await expect(captions[0]!.getBoundingClientRect().top).toBeGreaterThan(within(dialog).getByTestId('viewer-stage').getBoundingClientRect().top)
    const stage = within(dialog).getByTestId('viewer-stage')
    stage.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, clientX: 320, clientY: 400, pointerId: 1 }))
    stage.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, button: 0, clientX: 200, clientY: 404, pointerId: 1 }))
    await expectCounter('3 of 9')
  } else {
    await expect(film).toBeVisible()
    await expect(zoom).toBeVisible()
  }
  await expect(dialog.getBoundingClientRect().width).toBeLessThanOrEqual(window.innerWidth)
  await userEvent.click(close)
  await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
  await noUnknown(canvasElement)
} }

export const ViewerKeyboard: Story = { play: async ({ canvasElement }) => {
  const canvas = within(canvasElement)
  await waitFor(() => expect(canvasElement.querySelectorAll('[data-testid="transcript-image"][data-image-state="ready"]')).toHaveLength(4))
  const first = canvas.getAllByRole('button', { name: 'View image: From Operator · web' })[0]!
  first.focus()
  await userEvent.keyboard('{Enter}')
  const dialog = await viewer()
  const close = within(dialog).getByRole('button', { name: 'Close image viewer' })
  await waitFor(() => expect(close).toHaveFocus())
  await expectCounter('1 of 9')
  await expect(within(dialog).queryByRole('button', { name: 'Previous image' })).toBeNull()
  await userEvent.keyboard('{ArrowRight}')
  await expectCounter('2 of 9')
  await waitFor(() => expect(within(dialog).getByTestId('viewer-announcement')).toHaveTextContent('Image 2 of 9: From Operator · web'))
  await userEvent.keyboard('{End}')
  await expectCounter('9 of 9')
  await expect(within(dialog).queryByRole('button', { name: 'Next image' })).toBeNull()
  await userEvent.keyboard('{Home}')
  await expectCounter('1 of 9')
  // Stepping with the edge button that disappears keeps focus inside the dialog.
  await userEvent.click(within(dialog).getByRole('button', { name: 'Next image' }))
  await expectCounter('2 of 9')
  await userEvent.click(within(dialog).getByRole('button', { name: 'Previous image' }))
  await expectCounter('1 of 9')
  await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true))
  await waitFor(() => expect(within(dialog).getByTestId('viewer-image')).toHaveAttribute('alt', 'From Operator · web'))
  const readout = within(dialog).getByTestId('viewer-zoom-readout')
  await waitFor(() => expect(readout).toBeEnabled())
  await expect(readout).toHaveAccessibleName('Zoom: Fit')
  await userEvent.keyboard('1')
  await waitFor(() => expect(readout).toHaveTextContent('100%'))
  await userEvent.keyboard('+')
  await waitFor(() => expect(readout).toHaveTextContent('125%'))
  await userEvent.keyboard('-')
  await waitFor(() => expect(readout).toHaveTextContent('100%'))
  await userEvent.keyboard('0')
  await waitFor(() => expect(readout).toHaveTextContent('Fit'))
  // Tab cycles within the dialog.
  for (let index = 0; index < 12; index += 1) {
    await userEvent.tab()
    await expect(dialog.contains(document.activeElement)).toBe(true)
  }
  // A focused edge control may be showing its tooltip, which takes the first Esc; the dialog itself has none.
  dialog.focus()
  await userEvent.keyboard('{Escape}')
  await waitFor(() => expect(document.querySelector('[data-testid="image-viewer"]')).toBeNull())
  await waitFor(() => expect(first).toHaveFocus())
  await noUnknown(canvasElement)
} }

const styles = stylex.create({
  root: { height: '100vh', width: '100%', minWidth: 0, boxSizing: 'border-box', display: 'flex', flexDirection: 'column', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  probe: { position: 'absolute', bottom: 0, left: 0, margin: 0, color: ink.fgMuted, backgroundColor: surface.canvas, fontSize: t.metaSize },
})
