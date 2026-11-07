import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor, within } from 'storybook/test'
import { lightTheme, type Scheme } from '../../assistant-ui/composition-theme'
import {
  accentVars, borderVars, elevationVars, interactionVars, motionVars, radiusVars, spaceVars, statusVars, surfaceVars, textVars, typeVars,
} from '../../assistant-ui/composition-tokens.stylex'

/** The semantic token families the app paints with. Values are read back from the rendered page. */
const colourFamilies = [
  { name: 'Surface', vars: surfaceVars },
  { name: 'Text', vars: textVars },
  { name: 'Border', vars: borderVars },
  { name: 'Accent', vars: accentVars },
  { name: 'Interaction', vars: interactionVars },
  { name: 'Status', vars: statusVars },
] as const

const entries = (vars: object): readonly (readonly [string, string])[] =>
  Object.entries(vars).filter((entry): entry is [string, string] => typeof entry[1] === 'string' && entry[1].startsWith('var('))

/** Reads the token's computed value at this element, so the page shows what the theme resolves to. */
const Resolved = ({ token }: { readonly token: string }) => {
  const ref = React.useRef<HTMLElement>(null)
  const [value, setValue] = React.useState('')
  React.useLayoutEffect(() => {
    const name = /^var\((--[^,)]+)/.exec(token)?.[1]
    if (ref.current && name) setValue(getComputedStyle(ref.current).getPropertyValue(name).trim())
  })
  return <code ref={ref} data-token-value="" {...stylex.props(styles.value)}>{value}</code>
}

const Page = ({ scheme, title, children }: { readonly scheme: Scheme; readonly title: string; readonly children: React.ReactNode }) =>
  <main data-scheme={scheme} {...stylex.props(styles.page, scheme === 'light' && lightTheme)}>
    <h1 {...stylex.props(styles.title)}>{title}</h1>
    {children}
  </main>

const ColourPage = ({ scheme }: { readonly scheme: Scheme }) => <Page scheme={scheme} title="Colour">
  {colourFamilies.map(({ name, vars }) => <section key={name} aria-label={name}>
    <h2 {...stylex.props(styles.heading)}>{name}</h2>
    <ul {...stylex.props(styles.grid)}>
      {entries(vars).map(([key, token]) => <li key={key} data-token={`${name.toLowerCase()}.${key}`} {...stylex.props(styles.card)}>
        <span aria-hidden="true" {...stylex.props(styles.swatch, styles.fill(token))} />
        <span {...stylex.props(styles.name)}>{key}</span>
        <Resolved token={token} />
      </li>)}
    </ul>
  </section>)}
</Page>

const typeSamples = [
  { key: 'ui', size: typeVars.uiSize, leading: typeVars.uiLeading },
  { key: 'body', size: typeVars.bodySize, leading: typeVars.bodyLeading },
  { key: 'heading', size: typeVars.headingSize, leading: typeVars.headingLeading },
  { key: 'meta', size: typeVars.metaSize, leading: typeVars.metaLeading },
  { key: 'code', size: typeVars.codeSize, leading: typeVars.codeLeading },
  { key: 'dense', size: typeVars.denseSize, leading: typeVars.metaLeading },
] as const

const TypePage = ({ scheme }: { readonly scheme: Scheme }) => <Page scheme={scheme} title="Type">
  <ul {...stylex.props(styles.stack)}>
    {typeSamples.map(({ key, size, leading }) => <li key={key} data-token={`type.${key}`} {...stylex.props(styles.row)}>
      <span {...stylex.props(styles.name)}>{key}</span>
      <span {...stylex.props(styles.sample(size, leading), key === 'code' && styles.mono)}>The agent is reviewing three changed files.</span>
      <Resolved token={size} />
    </li>)}
  </ul>
</Page>

const SpacePage = ({ scheme }: { readonly scheme: Scheme }) => <Page scheme={scheme} title="Space and radius">
  <section aria-label="Space">
    <h2 {...stylex.props(styles.heading)}>Space</h2>
    <ul {...stylex.props(styles.stack)}>
      {entries(spaceVars).map(([key, token]) => <li key={key} data-token={`space.${key}`} {...stylex.props(styles.row)}>
        <span {...stylex.props(styles.name)}>{key}</span><span aria-hidden="true" {...stylex.props(styles.bar(token))} /><Resolved token={token} />
      </li>)}
    </ul>
  </section>
  <section aria-label="Radius">
    <h2 {...stylex.props(styles.heading)}>Radius</h2>
    <ul {...stylex.props(styles.grid)}>
      {entries(radiusVars).map(([key, token]) => <li key={key} data-token={`radius.${key}`} {...stylex.props(styles.card)}>
        <span aria-hidden="true" {...stylex.props(styles.swatch, styles.rounded(token))} /><span {...stylex.props(styles.name)}>{key}</span><Resolved token={token} />
      </li>)}
    </ul>
  </section>
</Page>

const MotionPage = ({ scheme }: { readonly scheme: Scheme }) => <Page scheme={scheme} title="Motion and elevation">
  <section aria-label="Motion">
    <h2 {...stylex.props(styles.heading)}>Motion</h2>
    <ul {...stylex.props(styles.stack)}>
      {entries(motionVars).map(([key, token]) => <li key={key} data-token={`motion.${key}`} {...stylex.props(styles.row)}><span {...stylex.props(styles.name)}>{key}</span><Resolved token={token} /></li>)}
    </ul>
  </section>
  <section aria-label="Elevation">
    <h2 {...stylex.props(styles.heading)}>Elevation</h2>
    <ul {...stylex.props(styles.grid)}>
      {entries(elevationVars).map(([key, token]) => <li key={key} data-token={`elevation.${key}`} {...stylex.props(styles.card, styles.shadow(token))}><span {...stylex.props(styles.name)}>{key}</span></li>)}
    </ul>
  </section>
</Page>

const pages = { colour: ColourPage, type: TypePage, space: SpacePage, motion: MotionPage } as const
const Foundations = ({ page, scheme }: { readonly page: keyof typeof pages; readonly scheme: Scheme }) => {
  const Selected = pages[page]
  return <Selected scheme={scheme} />
}

const meta = {
  title: 'Foundations/Tokens',
  component: Foundations,
  excludeStories: ['scenarioExemptions'],
  args: { page: 'colour', scheme: 'dark' },
  argTypes: { page: { table: { disable: true } }, scheme: { control: 'radio', options: ['dark', 'light'] } },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'primitive', reason: 'The semantic token families, with no product data.' } },
} satisfies Meta<typeof Foundations>
export default meta
type Story = StoryObj<typeof meta>

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = ['Colour:primitive', 'Motion:primitive', 'Space:primitive', 'Type:primitive']

/** Every token on the page resolves to a value under the current theme. */
const resolvesAll: NonNullable<Story['play']> = async ({ canvasElement }) => {
  const values = [...canvasElement.querySelectorAll<HTMLElement>('[data-token-value]')]
  await expect(values.length).toBeGreaterThan(0)
  await waitFor(() => expect(values.filter(value => value.textContent === '').map(value => value.closest('[data-token]')?.getAttribute('data-token'))).toEqual([]))
}

export const Colour: Story = { name: 'Colour', args: { page: 'colour' }, play: async context => {
  await resolvesAll(context)
  await expect(within(context.canvasElement).getByRole('region', { name: 'Surface' })).toBeVisible()
} }
export const Type: Story = { args: { page: 'type' }, play: resolvesAll }
export const Space: Story = { name: 'Space and radius', args: { page: 'space' }, play: resolvesAll }
export const Motion: Story = { name: 'Motion and elevation', args: { page: 'motion' }, play: resolvesAll }

const styles = stylex.create({
  page: { minHeight: '100vh', boxSizing: 'border-box', paddingBlock: spaceVars.section, paddingInline: spaceVars.xl, backgroundColor: surfaceVars.canvas, color: textVars.fg, fontFamily: typeVars.fontSans, fontSize: typeVars.uiSize, lineHeight: typeVars.uiLeading },
  title: { marginBlock: 0, fontSize: typeVars.headingSize, lineHeight: typeVars.headingLeading, fontWeight: typeVars.weightSemibold },
  heading: { marginBlock: `${spaceVars.section} ${spaceVars.md}`, fontSize: typeVars.uiSize, fontWeight: typeVars.weightMedium, color: textVars.fgMuted },
  grid: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(160px, 1fr))', gap: spaceVars.md, margin: 0, padding: 0, listStyle: 'none' },
  stack: { display: 'grid', gap: spaceVars.sm, margin: 0, padding: 0, listStyle: 'none' },
  card: { display: 'grid', gap: spaceVars.xs, padding: spaceVars.md, borderRadius: radiusVars.md, borderWidth: 1, borderStyle: 'solid', borderColor: borderVars.border, backgroundColor: surfaceVars.raised },
  row: { display: 'grid', gridTemplateColumns: '96px minmax(0, 1fr) auto', alignItems: 'center', gap: spaceVars.lg },
  swatch: { height: '40px', borderRadius: radiusVars.sm, borderWidth: 1, borderStyle: 'solid', borderColor: borderVars.borderStrong },
  fill: (color: string) => ({ backgroundColor: color }),
  rounded: (radius: string) => ({ borderRadius: radius, backgroundColor: surfaceVars.message }),
  bar: (width: string) => ({ width, height: '8px', backgroundColor: accentVars.primary }),
  shadow: (shadow: string) => ({ boxShadow: shadow, minHeight: '56px' }),
  sample: (fontSize: string, lineHeight: string) => ({ fontSize, lineHeight }),
  mono: { fontFamily: typeVars.fontMono },
  name: { fontWeight: typeVars.weightMedium },
  value: { fontFamily: typeVars.fontMono, fontSize: typeVars.metaSize, color: textVars.fgMuted, overflowWrap: 'anywhere' },
})
