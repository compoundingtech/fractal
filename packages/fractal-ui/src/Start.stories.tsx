import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, within } from 'storybook/test'
import { colorVars as color, typeVars as type } from './assistant-ui/composition-tokens.stylex'
import { lightTheme } from './assistant-ui/composition-theme'
import { darkTheme } from './assistant-ui/embrace-theme'

const storyLink = (id: string) => `./?path=/story/${id}`

const areas = [
  { name: 'Start', about: 'This page.' },
  { name: 'Screens', about: 'Real product compositions over scenario worlds. Start your review here.' },
  { name: 'Surfaces', about: 'One product area at a time: transcript, composer, sidebar, details.' },
  { name: 'Components', about: 'Controls, overlays, time, Markdown, diffs.' },
  { name: 'Foundations', about: 'Colour, type, space, motion.' },
] as const

const toolbar = [
  { name: 'Scenario', about: 'Swaps the world behind a story. Screens that do not pin a world follow it.' },
  { name: 'Theme', about: 'Dark or light. Story default keeps what the story declares.' },
  { name: 'Viewport', about: 'Phone, narrow, tablet, laptop or desktop width.' },
] as const

const StartPage = ({ scheme }: { readonly scheme: 'dark' | 'light' }) => <main data-scheme={scheme} {...stylex.props(scheme === 'dark' ? darkTheme : lightTheme, styles.page)}>
  <div {...stylex.props(styles.column)}>
    <h1 {...stylex.props(styles.title)}>Fractal Storybook</h1>
    <p {...stylex.props(styles.lead)}>The design review surface for the Fractal app and its UI kit. Each story is one state; theme and width come from the toolbar.</p>

    <h2 {...stylex.props(styles.heading)}>Start with</h2>
    <ul {...stylex.props(styles.list)}>
      <li><a target="_top" href={storyLink('screens-workspace--populated')} {...stylex.props(styles.link)}>Workspace</a>: the whole app shell</li>
      <li><a target="_top" href={storyLink('screens-conversation--populated')} {...stylex.props(styles.link)}>Conversation</a>: one thread and its composer</li>
    </ul>

    <h2 {...stylex.props(styles.heading)}>Toolbar</h2>
    <dl {...stylex.props(styles.terms)}>
      {toolbar.map(({ name, about }) => <div key={name} {...stylex.props(styles.term)}><dt {...stylex.props(styles.name)}>{name}</dt><dd {...stylex.props(styles.about)}>{about}</dd></div>)}
    </dl>

    <h2 {...stylex.props(styles.heading)}>Structure</h2>
    <dl {...stylex.props(styles.terms)}>
      {areas.map(({ name, about }) => <div key={name} {...stylex.props(styles.term)}><dt {...stylex.props(styles.name)}>{name}</dt><dd {...stylex.props(styles.about)}>{about}</dd></div>)}
    </dl>
    <p {...stylex.props(styles.about)}>Stories are moving into these areas in stages. Until then, the rest live under Fractal UI, Fractal and wf.</p>

    <h2 {...stylex.props(styles.heading)}>Hidden depth</h2>
    <p {...stylex.props(styles.about)}>States that exist for the play gate rather than for review carry the <code {...stylex.props(styles.code)}>gate</code> tag and are hidden by default. To see them, clear the exclusion in the tag filter above the sidebar tree. They still run in every gate.</p>
  </div>
</main>

/** The landing page: where to begin a review and how the Storybook is organized. */
const meta = {
  id: 'start',
  title: 'Start',
  component: StartPage,
  excludeStories: ['scenarioExemptions'],
  args: { scheme: 'dark' },
  argTypes: { scheme: { control: 'radio', options: ['dark', 'light'] } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'A static guide to the Storybook, with no product data.' } },
} satisfies Meta<typeof StartPage>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Start:primitive']

export const Start: StoryObj<typeof meta> = {
  name: 'Start',
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByRole('heading', { level: 1, name: 'Fractal Storybook' })).toBeVisible()
    await expect(canvas.getByRole('link', { name: 'Workspace' })).toHaveAttribute('href', storyLink('screens-workspace--populated'))
  },
}

const styles = stylex.create({
  page: { minHeight: '100vh', backgroundColor: color.canvas, color: color.fg, fontFamily: type.fontSans, fontSize: type.uiSize, lineHeight: type.bodyLeading, paddingBlock: '48px', paddingInline: '24px', boxSizing: 'border-box' },
  column: { maxWidth: '640px', marginInline: 'auto' },
  title: { margin: 0, fontSize: '24px', lineHeight: '32px', fontWeight: type.weightSemibold },
  lead: { marginBlock: '8px 0', color: color.fgSoft },
  heading: { marginBlock: '32px 8px', fontSize: type.headingSize, lineHeight: type.headingLeading, fontWeight: type.weightSemibold },
  list: { margin: 0, paddingInlineStart: '20px' },
  link: { color: color.fg, textDecoration: 'underline', textUnderlineOffset: '2px' },
  terms: { margin: 0 },
  term: { display: 'flex', flexWrap: 'wrap', columnGap: '12px', paddingBlock: '4px' },
  name: { flex: '0 0 96px', fontWeight: type.weightMedium },
  about: { margin: 0, flex: '1 1 240px', color: color.fgSoft },
  code: { fontFamily: type.fontMono, fontSize: type.codeSize },
})
