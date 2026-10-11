import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, fn, userEvent, within } from 'storybook/test'
import axe from 'axe-core'
import { HarnessNotice } from './assistant-ui/composition/HarnessNotice'
import { harnessWeightExamples } from './assistant-ui/composition/HarnessNotice.fixtures'
import type { HarnessWeight } from './assistant-ui/embrace-data/model'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { surfaceVars as surface, textVars as ink, borderVars as border, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { QuestionForm, QuestionSummary, NeedsYouBar, NeedsYouInbox } from './assistant-ui/QuestionForm'
import { questionRequest } from './question-fixtures'

/** q106 pairing: the production HarnessNotice at both weights over the same eight fictional events. */
const variants: readonly { readonly weight: HarnessWeight; readonly name: string; readonly note: string }[] = [
  { weight: 'row', name: 'B1 · Quiet rows', note: 'Low interruption; longer payloads need deliberate disclosure.' },
  { weight: 'card', name: 'B2 · Compact cards', note: 'Clearest event boundaries; costs more vertical space.' },
]

export function HarnessWeightComparison({ scheme = 'dark' }: { readonly scheme?: Scheme }) {
  return <section aria-label="Harness event weight comparison" data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {variants.map(variant => <section key={variant.weight} aria-label={variant.name} data-testid={`harness-weight-${variant.weight}`} {...stylex.props(styles.column)}>
      <header {...stylex.props(styles.header)}><h2 {...stylex.props(styles.title)}>{variant.name}</h2><p {...stylex.props(styles.note)}>{variant.note}</p></header>
      <p {...stylex.props(styles.prose)}>The retry change is ready for review. I’m collecting the review and background check results.</p>
      <div {...stylex.props(styles.list)}>{harnessWeightExamples.map(item => <HarnessNotice key={item.id} item={item} weight={variant.weight} />)}</div>
      <section aria-label="Questions specimens" {...stylex.props(styles.list)}>
        <div data-component-name="QuestionForm" data-weight-class="card"><QuestionForm request={questionRequest} onOpenTerminal={fn()} /></div>
        <div data-component-name="QuestionSummary" data-weight-class="row"><QuestionSummary request={{ ...questionRequest, state: { _tag: 'answered', answers: { single: { selected: ['thread'] } }, by: 'Reviewer', at: 'a moment ago' } }} /></div>
        <div data-component-name="NeedsYouBar" data-weight-class="shell"><NeedsYouBar requests={[questionRequest]} expanded={false} onToggle={fn()} /></div>
        <div data-component-name="NeedsYouInbox" data-weight-class="shell"><NeedsYouInbox requests={[questionRequest]} onShowInThread={fn()} onOpenTerminal={fn()} /></div>
      </section>
    </section>)}
  </section>
}

const meta = { title: 'Fractal/Explore/Harness event weight', component: HarnessWeightComparison, excludeStories: ['HarnessWeightComparison'], parameters: { layout: 'fullscreen' }, args: { scheme: 'dark' }, argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' } } } satisfies Meta<typeof HarnessWeightComparison>
export default meta
type Story = StoryObj<typeof meta>

export const Dark: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    for (const weight of ['row', 'card']) {
      const column = within(canvas.getByTestId(`harness-weight-${weight}`))
      await expect(column.getAllByTestId('harness-notice')).toHaveLength(8)
      await expect(column.queryByText(/<system-notice>|<irc|<task-result|## Goal/)).toBeNull()
      for (const [name, weightClass] of [['QuestionForm', 'card'], ['QuestionSummary', 'row'], ['NeedsYouBar', 'shell'], ['NeedsYouInbox', 'shell']] as const) {
        await expect(canvas.getByTestId(`harness-weight-${weight}`).querySelector(`[data-component-name="${name}"]`)).toHaveAttribute('data-weight-class', weightClass)
      }
      const specimens = canvas.getByTestId(`harness-weight-${weight}`)
      await expect(within(specimens.querySelector('[data-component-name="QuestionForm"]') as HTMLElement).getByRole('button', { name: 'Open terminal' })).toBeVisible()
      await expect(within(specimens.querySelector('[data-component-name="QuestionSummary"]') as HTMLElement).getByText('Answered')).toBeVisible()
      await expect(within(specimens.querySelector('[data-component-name="NeedsYouBar"]') as HTMLElement).getByRole('button', { name: /is waiting on you/ })).toHaveAttribute('aria-expanded', 'false')
      await expect(within(specimens.querySelector('[data-component-name="NeedsYouInbox"]') as HTMLElement).getByRole('region', { name: 'Needs you inbox' }).querySelector('details')).not.toHaveAttribute('open')
    }
    const read = within(canvas.getByTestId('harness-weight-row')).getByRole('button', { name: 'Read summary' })
    await userEvent.click(read)
    await expect(read).toHaveAttribute('aria-expanded', 'true')
    const contrast = await axe.run(canvasElement, { runOnly: { type: 'rule', values: ['color-contrast'] } })
    await expect(contrast.violations).toEqual([])
  },
}
export const Light: Story = { ...Dark, args: { scheme: 'light' } }

const styles = stylex.create({
  root: { display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(22rem, 1fr))', gap: s.section, minHeight: '100vh', padding: s.section, backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans },
  column: { display: 'flex', flexDirection: 'column', gap: s.lg, minWidth: 0 },
  header: { paddingBottom: s.md, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: border.border },
  title: { margin: 0, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: 600 },
  note: { margin: 0, color: ink.fgMuted, fontSize: t.metaSize },
  prose: { margin: 0, fontSize: t.bodySize, lineHeight: t.bodyLeading, color: ink.fgSoft },
  list: { display: 'flex', flexDirection: 'column', gap: s.md },
})
