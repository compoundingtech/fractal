import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { Transcript } from './assistant-ui/composition/Transcript'
import { EmbraceRuntimeProvider } from './assistant-ui/EmbraceRuntime'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { geometryVars as g, surfaceVars as surface, spaceVars as s } from './assistant-ui/composition-tokens.stylex'
import type { TextItem } from './assistant-ui/embrace-data/model'

const at = '2026-01-15T12:00:00Z'
const clock = Date.parse(at) + 12000
const prompt = (id: string, body: string): TextItem & { role: 'user' } => ({ _tag: 'Text', id, role: 'user', text: body, attachments: [], streaming: false, at })
const empty = { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false }
const prompts = [
  prompt('md/daemon', 'A mission step is ready: step-run/demo-1/work. Run `st work claim step-run/demo-1/work --as agent/demo/one` to read and claim it.'),
  prompt('md/lines', 'Please check these:\nthe **first** line\nand `Array<string>` next to a bare Array<string>\n\n- one item\n- [a link](https://example.test)'),
  prompt('md/long', Array.from({ length: 14 }, (_, index) => `Line ${index + 1} with \`code\` and **bold** that keeps its formatting when collapsed.`).join('\n')),
]
const mail: TextItem = { _tag: 'Text', id: 'md/mail', role: 'system', text: 'Mail from another agent, read `a < b` and:\n\n1. the **first** step\n2. the second step', attachments: [], streaming: false, at }
const turns = prompts.map((item, index) => ({ id: item.id, prompt: item, items: index === 0 ? [mail] : [], work: empty }))
const styles = stylex.create({ root: { width: g.polishWide, maxWidth: '100%', height: g.transcriptFrameHeight, backgroundColor: surface.canvas, padding: s.md } })
function PromptsStory({ scheme }: { readonly scheme: Scheme }) {
  return <main {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}><EmbraceRuntimeProvider options={{ messages: [...prompts, mail], onNew: async () => {} }}><Transcript title="Markdown prompts" turns={turns} sync={{ _tag: 'Live', since: clock }} now={clock} observedAt={clock} /></EmbraceRuntimeProvider></main>
}
const meta = { excludeStories: ['scenarioExemptions'], title: 'Fractal UI/Markdown prompts', component: PromptsStory, parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'Invented prompts prove the Markdown contract of the prompt bar: command as code, kept lines, kept tag-like words, a formatted collapsed prompt and delivered mail.' } }, args: { scheme: 'dark' as Scheme } } satisfies Meta<typeof PromptsStory>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Dark:contract-fixture', 'Light:contract-fixture']

const rendersMarkdown = async ({ canvasElement }: { readonly canvasElement: HTMLElement }) => {
  const canvas = within(canvasElement)
  await expect(canvas.getByText('st work claim step-run/demo-1/work --as agent/demo/one').tagName).toBe('CODE')
  await expect(canvasElement.textContent).not.toContain('`st work')
  await expect(canvasElement.textContent).toContain('next to a bare Array<string>')
  await canvasElement.ownerDocument.fonts.ready
  await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
  const message = canvasElement.querySelector<HTMLElement>('[data-item-id="md/lines"]')!
  const body = message.querySelector<HTMLElement>('[data-testid="markdown"]')!.parentElement!
  await expect(body.scrollHeight).toBeLessThanOrEqual(body.clientHeight)
  await expect(within(message).queryByRole('button', { name: 'Show full message' })).toBeNull()
}
export const Dark: StoryObj<typeof meta> = { play: rendersMarkdown }
export const Light: StoryObj<typeof meta> = { args: { scheme: 'light' }, play: rendersMarkdown }
