import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { flushSync } from 'react-dom'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Button } from 'react-aria-components'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { EmbraceMarkdownPreview } from './assistant-ui/EmbraceToolPreview'
import { Markdown, completeStreamingTail } from './assistant-ui/composition/Markdown'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { MarkdownCanvas, languages, streamChunks, inlineStreamStages, styles } from './markdown.story-canvas'


const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal UI/Markdown', component: MarkdownCanvas,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'CommonMark + GFM prose with semantic emphasis, nested lists, aligned scrollable tables, titled links, and preserved resource references. Code fences retain local wrap/copy state across streamed updates, copy unformatted source through the Clipboard API, and lazily load explicit refractor grammars whose tokens use the existing semantic theme. Unknown languages remain plain text. Streaming mode completes unfinished inline prose without changing settled literal punctuation or fenced source.' } } },
  args: { scenario: 'rich-text', scheme: 'dark' },
  argTypes: { scenario: { options: ['rich-text', 'languages', 'streaming', 'inline-streaming', 'all'], control: 'radio' }, scheme: { options: ['dark', 'light'], control: 'radio' } },
} satisfies Meta<typeof MarkdownCanvas>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'AllStatesLight:contract-fixture',
  'CopyFailure:contract-fixture',
  'InputForegroundDark1440:contract-fixture',
  'InputForegroundDark440:contract-fixture',
  'InputForegroundLight1440:contract-fixture',
  'InputForegroundLight440:contract-fixture',
  'Languages:contract-fixture',
  'LanguagesLight:contract-fixture',
  'Light:contract-fixture',
  'RichText:contract-fixture',
  'StreamingCodeBlock:contract-fixture',
  'StreamingCodeBlockLight:contract-fixture',
  'StreamingControlsPersistence:contract-fixture',
  'StreamingControlsPersistenceLight:contract-fixture',
  'StreamingHeading:contract-fixture',
  'StreamingHeadingLight:contract-fixture',
  'StreamingInlineMarkup:contract-fixture',
  'StreamingParagraph:contract-fixture',
  'StreamingParagraphLight:contract-fixture',
  'StreamingTable:contract-fixture',
  'StreamingTableLight:contract-fixture',
  'StreamingTightList:contract-fixture',
  'StreamingTightListLight:contract-fixture',
]
type Story = StoryObj<typeof meta>
const contractFixture = { parameters: { scenario: { _tag: 'contract-fixture', reason: 'Exact local markdown syntax and token streaming stages exercise parser and renderer contracts.' } } } satisfies Story

const settle = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
const requireElement = <T extends Element>(root: Element, selector: string): T => {
  const element = root.querySelector<T>(selector)
  if (element === null) throw new Error(`Missing ${selector}`)
  return element
}

function streamingTailStory(text: string, terminalSelector: string): Story {
  return {
    ...contractFixture,
    render: ({ scheme }) => <main data-testid="markdown-story" {...stylex.props(styles.canvas, ...baselineTheme, scheme === 'light' && lightTheme)}><ThemePortal><div {...stylex.props(styles.lane)}><Markdown text={text} streaming /></div></ThemePortal></main>,
    play: async ({ canvasElement }) => {
      await document.fonts.ready
      const markdown = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown"]')
      const tails = markdown.querySelectorAll<HTMLElement>('[data-streaming-tail="true"]')
      if (tails.length !== 1) throw new Error(`Expected one streaming caret after ${terminalSelector}, found ${tails.length}`)
      const tail = tails[0]!
      const caret = getComputedStyle(tail, '::after')
      if (caret.content !== '""' || caret.display !== 'inline-block' || parseFloat(caret.width) <= 0 || parseFloat(caret.height) <= 0 || caret.backgroundColor === 'rgba(0, 0, 0, 0)') throw new Error('Streaming caret exists but is not painted')
      if (terminalSelector === 'p') {
        if (tail !== markdown.lastElementChild || tail.tagName !== 'P') throw new Error('Paragraph caret must stay inside the final paragraph, not on its own line')
        const range = document.createRange()
        range.selectNodeContents(tail)
        const lines = range.getClientRects()
        if (lines.length !== 1 || tail.getBoundingClientRect().height > parseFloat(getComputedStyle(tail).lineHeight) + 1) throw new Error('Paragraph caret introduced another line')
      } else if (tail !== markdown.lastElementChild || tail.tagName !== 'SPAN' || !tail.previousElementSibling?.matches(terminalSelector)) throw new Error(`Streaming fallback must follow the terminal ${terminalSelector}`)
    },
  }
}
export const StreamingCodeBlock: Story = streamingTailStory('The current implementation is:\n\n```typescript\nexport const selected = "row-3";\n```', '[data-testid="markdown-code-block"]')
export const StreamingHeading: Story = streamingTailStory('The next section is:\n\n## Selection details', 'h2')
export const StreamingTable: Story = streamingTailStory('Current resources:\n\n| Resource | State |\n| --- | --- |\n| Selection model | Ready |', '[role="region"]')
export const StreamingTightList: Story = streamingTailStory('Current checks:\n\n- Preserve selection identity.\n- Restore keyboard focus.', 'ul')
export const StreamingParagraph: Story = streamingTailStory('Selection identity stays stable.\n\nThe current result is ready.\n', 'p')
export const StreamingCodeBlockLight: Story = { ...StreamingCodeBlock, args: { scheme: 'light' } }
export const StreamingHeadingLight: Story = { ...StreamingHeading, args: { scheme: 'light' } }
export const StreamingTableLight: Story = { ...StreamingTable, args: { scheme: 'light' } }
export const StreamingTightListLight: Story = { ...StreamingTightList, args: { scheme: 'light' } }
export const StreamingParagraphLight: Story = { ...StreamingParagraph, args: { scheme: 'light' } }
export const RichText: Story = { ...contractFixture, tags: ['critical'],
  play: async ({ canvasElement }) => {
    const root = requireElement(canvasElement, '[data-testid="markdown"]')
    const title = requireElement<HTMLAnchorElement>(root, 'h1 a')
    if (title.textContent !== 'Selection and ordering' || title.title !== 'Selection guide' || !title.getAttribute('href')?.includes('selection_(stable)')) throw new Error('Linked heading lost its formatted label, balanced URL, or title')
    if (root.querySelectorAll('strong').length < 4 || root.querySelectorAll('em').length < 4) throw new Error('Bold and italic markup did not produce semantic elements')
    if (root.querySelector('ol[start="3"] > li > ul > li > ol') === null) throw new Error('Nested mixed lists lost their structure or ordered start')
    if (root.querySelectorAll('table tbody tr').length !== 3 || !root.textContent?.includes('Escaped | pipe')) throw new Error('GFM table parsing leaked delimiters or dropped rows')
    if (root.querySelector('[data-testid="custom-inline-reference"]') === null) throw new Error('Custom inline reference renderer was lost')
    const resource = [...root.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === 'src/model.ts')
    if (resource === undefined) throw new Error('Resource chip was lost')
    resource.click()
    await settle()
    if (canvasElement.querySelector('[data-testid="opened-resource"]')?.textContent !== 'Opened src/model.ts') throw new Error('Resource chip lost its open callback')
  },
}
export const Languages: Story = { ...contractFixture, tags: ['critical'],
  args: { scenario: 'languages' },
  play: async ({ canvasElement }) => {
    const fences = [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="markdown-code-block"]')]
    if (fences.length !== languages.length) throw new Error('Language coverage lost a fence')
    for (let index = 0; index < languages.length; index++) {
      const [language, code] = languages[index]!
      const fence = fences[index]!
      if (fence.dataset.language !== (language || 'text')) throw new Error(`Fence ${index} lost its language label`)
      if (index < languages.length - 2) {
        for (let frame = 0; fence.querySelector('[data-syntax-token]') === null && frame < 600; frame++) await settle()
        if (fence.querySelector('[data-syntax-token]') === null) throw new Error(`${language} did not produce real syntax tokens`)
      } else if (fence.querySelector('[data-syntax-token], script') !== null) throw new Error('Unknown language should remain safe literal text')
      if (fence.querySelector('code')?.textContent !== `${code}\n`) throw new Error(`${language || 'text'} highlighting changed the source text`)
    }
  },
}
export const Light: Story = { ...RichText, args: { scheme: 'light' } }
export const LanguagesLight: Story = { ...Languages, args: { scenario: 'languages', scheme: 'light' } }
export const AllStatesLight: Story = { ...contractFixture, args: { scenario: 'all', scheme: 'light' } }
export const StreamingControlsPersistence: Story = { ...contractFixture, tags: ['critical'],
  args: { scenario: 'streaming' },
  play: async ({ canvasElement }) => {
    await document.fonts.ready
    await settle()
    const fence = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown-code-block"]')
    const wrap = requireElement<HTMLButtonElement>(fence, '[aria-label="Wrap code"]')
    const copy = requireElement<HTMLButtonElement>(fence, '[aria-label="Copy code"]')
    const append = requireElement<HTMLButtonElement>(canvasElement, '[data-testid="append-stream"]')
    for (let frame = 0; fence.querySelector('[data-syntax-token]') === null && frame < 600; frame++) await settle()
    if (fence.querySelector('[data-syntax-token]') === null) throw new Error('Lazy TypeScript grammar did not finish loading before the interaction')
    const clipboardDescriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
    const copied: string[] = []
    // Deterministic clipboard transport for the story only; production uses the real Clipboard API.
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (value: string) => { copied.push(value) } } })
    try {
      flushSync(() => wrap.click())
      await settle()
      if (wrap.getAttribute('aria-pressed') !== 'true') throw new Error('Wrap toggle did not enable wrapping')
      flushSync(() => copy.click())
      await settle()
      if (fence.dataset.copyState !== 'copied' || copied[0] !== fence.querySelector('code')?.textContent) throw new Error('Copy did not write the raw rendered code')
      if (String(copy.textContent) !== 'Copied' || copy.getAttribute('aria-label') !== 'Copy code: Copied') throw new Error('Copied feedback did not match the accessible control name')
      for (let stage = 1; stage < streamChunks.length; stage++) {
        flushSync(() => append.click())
        await settle()
        const current = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown-code-block"]')
        if (current !== fence || current.dataset.wrap !== 'true' || current.dataset.copyState !== 'copied' || current.querySelector('[aria-label="Wrap code"]') !== wrap || current.querySelector('[aria-label="Copy code"]') !== copy) throw new Error(`Streaming chunk ${stage} remounted the fence or reset local controls`)
        if (copy.textContent !== 'Copy') throw new Error('Streaming code change falsely retained a current-version Copied label')
      }
      flushSync(() => copy.click())
      await settle()
      if (copied.at(-1) !== fence.querySelector('code')?.textContent || copy.textContent !== 'Copied') throw new Error('Copy after streaming did not use the latest code')
      if (copy.getAttribute('aria-label') !== 'Copy code: Copied') throw new Error('Copy after streaming did not update the accessible control name')
      const second = canvasElement.querySelectorAll<HTMLElement>('[data-testid="markdown-code-block"]')[1]
      if (second?.dataset.wrap !== 'false') throw new Error('A new fence inherited another fence’s local wrap state')
    } finally {
      if (clipboardDescriptor === undefined) Reflect.deleteProperty(navigator, 'clipboard')
      else Object.defineProperty(navigator, 'clipboard', clipboardDescriptor)
    }
  },
}
export const StreamingControlsPersistenceLight: Story = { ...StreamingControlsPersistence, args: { scenario: 'streaming', scheme: 'light' } }
export const StreamingInlineMarkup: Story = { ...contractFixture, tags: ['critical'],
  args: { scenario: 'inline-streaming' },
  play: async ({ canvasElement }) => {
    const adversarial = '['.repeat(50_000)
    const started = performance.now()
    const completed = completeStreamingTail(adversarial)
    const durationMs = performance.now() - started
    canvasElement.dataset.streamingTailDuration = String(durationMs)
    if (durationMs >= 50) throw new Error(`50k opening brackets exceeded the 50ms streaming-tail budget: ${durationMs}ms`)
    if (completed !== '['.repeat(49_999)) throw new Error('Streaming tail must remove only the last unmatched opening bracket')
    if (completeStreamingTail('Read [**label**](https://example.com') !== 'Read **label**') throw new Error('Incomplete destination did not retain its formatted label')
    if (completeStreamingTail('\\[literal') !== '\\[literal') throw new Error('Escaped opening bracket must remain literal')
    for (const [input, expected] of [
      ['Read [label](https://[::1', 'Read label'],
      ['Read [outer [inner]](https://[::1', 'Read outer [inner]'],
      ['Read [label](https://example.com/a(b)', 'Read label'],
      ['Read [label](https://example.com/a\\)b', 'Read label'],
      ['Read [label](https://example.com/a\\\\)', 'Read [label](https://example.com/a\\\\)'],
      ['Read [escaped \\[label\\]](https://[::1', 'Read escaped \\[label\\]'],
      ['Read \\[literal', 'Read \\[literal'],
    ] as const) if (completeStreamingTail(input) !== expected) throw new Error(`Streaming link-tail mismatch for ${JSON.stringify(input)}`)
    await document.fonts.ready
    const append = requireElement<HTMLButtonElement>(canvasElement, '[data-testid="append-stream"]')
    for (let stage = 0; stage < inlineStreamStages.length; stage++) {
      const markdown = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown"]')
      if (markdown.textContent?.match(/[*`\[\]]/) || markdown.textContent?.includes('https://')) throw new Error(`Inline streaming stage ${stage} leaked raw markdown`)
      if (markdown.querySelector('strong')?.textContent !== 'Stable') throw new Error(`Inline streaming stage ${stage} lost bold semantics`)
      if (stage >= 1 && markdown.querySelector('em')?.textContent !== 'ordered') throw new Error('Partial italic lost semantic formatting')
      if (stage >= 2 && ![...markdown.querySelectorAll('button')].some(button => button.textContent === 'src/model.ts')) throw new Error('Partial inline code lost its resource chip')
      if (stage === inlineStreamStages.length - 1) {
        if (markdown.querySelector('a')?.getAttribute('href') !== 'https://example.com/guide') throw new Error('Completed streamed link did not become actionable')
        break
      }
      flushSync(() => append.click())
      await settle()
    }
  },
}

export const CopyFailure: Story = { ...contractFixture, tags: ['critical'],
  args: { scenario: 'streaming' },
  play: async ({ canvasElement }) => {
    const fence = requireElement<HTMLElement>(canvasElement, '[data-testid="markdown-code-block"]')
    const copy = requireElement<HTMLButtonElement>(fence, '[aria-label="Copy code"]')
    await document.fonts.ready
    for (let frame = 0; fence.querySelector('[data-syntax-token]') === null && frame < 600; frame++) await settle()
    if (fence.querySelector('[data-syntax-token]') === null) throw new Error('Lazy grammar did not finish loading before the copy failure interaction')
    const descriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => { throw new DOMException('Clipboard permission denied', 'NotAllowedError') } } })
    try {
      flushSync(() => copy.click())
      // The rejected write settles asynchronously: wait for the copy to leave ready/copying, not a fixed two frames.
      for (let frame = 0; (fence.dataset.copyState === 'ready' || fence.dataset.copyState === 'copying') && frame < 600; frame++) await settle()
      const status = fence.querySelector('[role="status"]')?.textContent ?? ''
      if (fence.dataset.copyState !== 'failed' || !status.includes('Check clipboard permissions') || copy.disabled) {
        throw new Error(`Copy failure must report the permission problem and leave retry available (state ${fence.dataset.copyState}, status ${JSON.stringify(status)}, disabled ${copy.disabled})`)
      }
    } finally {
      if (descriptor === undefined) Reflect.deleteProperty(navigator, 'clipboard')
      else Object.defineProperty(navigator, 'clipboard', descriptor)
    }
  },
}

/** C27/E04–E12: the input boundary forwards foreground ink, while default seat Prose stays soft. */
function InputInkCanvas({ scheme = 'dark', narrow = false }: { scheme?: 'dark' | 'light'; narrow?: boolean }) {
  const [foreground, setForeground] = React.useState(true)
  const [revision, rerender] = React.useState(0)
  return <main data-revision={revision} {...stylex.props(styles.canvas, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div {...stylex.props(styles.toolbar)}><Button onPress={() => rerender(value => value + 1)}>Rerender input</Button><Button onPress={() => setForeground(false)}>Show soft control</Button><Button onPress={() => setForeground(true)}>Restore input</Button></div>
    <section {...stylex.props(styles.lane, narrow && styles.inputNarrow)}><EmbraceMarkdownPreview tone={foreground ? 'foreground' : 'default'} markdown={'The input anchors this turn.\n\n- Review the retained snapshot.\n\n| Fact | Value |\n| --- | --- |\n| Snapshot | Retained |'} /></section>
  </main>
}
export const InputForegroundDark1440: Story = { ...contractFixture, render: args => <InputInkCanvas scheme={args.scheme} />, play: async ({ canvasElement, args }) => {
  const canvas = within(canvasElement)
  await userEvent.click(canvas.getByRole('button', { name: 'Rerender input' }))
  await waitFor(() => expect(canvasElement.querySelector('main')).toHaveAttribute('data-revision', '1'))
  const expected = args.scheme === 'light' ? 'rgb(39, 39, 42)' : 'rgb(245, 245, 245)'
  const paints = () => [...canvasElement.querySelectorAll<HTMLElement>('[data-testid="markdown"] p, [data-testid="markdown"] li, [data-testid="markdown"] table')].map(element => getComputedStyle(element).color)
  await expect(paints()).toEqual([expected, expected, expected])
  await userEvent.click(canvas.getByRole('button', { name: 'Show soft control' }))
  await waitFor(() => expect(canvas.getByTestId('markdown')).toHaveAttribute('data-tone', 'default'))
  await expect(paints().every(color => color !== expected)).toBe(true)
  await userEvent.click(canvas.getByRole('button', { name: 'Restore input' }))
  await waitFor(() => expect(canvas.getByTestId('markdown')).toHaveAttribute('data-tone', 'foreground'))
  await expect(paints()).toEqual([expected, expected, expected])
} }
export const InputForegroundLight1440: Story = { ...InputForegroundDark1440, args: { scheme: 'light' } }
export const InputForegroundDark440: Story = { ...InputForegroundDark1440, render: args => <InputInkCanvas scheme={args.scheme} narrow /> }
export const InputForegroundLight440: Story = { ...InputForegroundDark440, args: { scheme: 'light' } }

