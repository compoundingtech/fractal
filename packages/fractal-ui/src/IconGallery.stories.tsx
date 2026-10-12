import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { Icon } from './assistant-ui/composition/Icons'
import { iconGroups, iconNames, iconRegistry, iconSizes, sharedGlyphAllowance, type IconName } from './assistant-ui/composition/icon-registry'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { borderVars as border, geometryVars as g, iconVars as icon, surfaceVars as surface, textVars as ink, spaceVars as s, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { iconGalleryGeometry as gallery } from './icon-gallery-tokens.stylex'

type Width = 'narrow' | 'wide'

/** Every registry meaning at the three token sizes, grouped as in the registry. */
function IconGallery({ scheme = 'dark', width = 'wide' }: { scheme?: Scheme; width?: Width }) {
  return <main data-scheme={scheme} {...stylex.props(styles.root, ...baselineTheme, scheme === 'light' && lightTheme)}>
    <div data-gallery-width={width} {...stylex.props(styles.frame, width === 'narrow' ? styles.narrow : styles.wide)}>
      <header {...stylex.props(styles.header)}>
        <h1 {...stylex.props(styles.title)}>Icons</h1>
        <p {...stylex.props(styles.lede)}>{iconNames.length} meanings. Each row shows one glyph at 12, 14 and 16, its name, what it means and the drawing it uses.</p>
      </header>
      {iconGroups.map(group => <section key={group} {...stylex.props(styles.section)}>
        <h2 {...stylex.props(styles.group)}>{group}</h2>
        <ul {...stylex.props(styles.list, width === 'wide' && styles.listWide)}>
          {iconNames.filter(name => iconRegistry[name].group === group).map(name => <li key={name} data-icon-row={name} {...stylex.props(styles.row)}>
            {iconSizes.map(size => <span key={size} data-size-column={size} {...stylex.props(styles.cell)}><Icon name={name} size={size} /></span>)}
            <span {...stylex.props(styles.text)}>
              <code {...stylex.props(styles.name)}>{name}</code>
              <span {...stylex.props(styles.meaning)}>{iconRegistry[name].meaning}</span>
              <span {...stylex.props(styles.glyph)}>{iconRegistry[name].glyph}</span>
            </span>
          </li>)}
        </ul>
      </section>)}
    </div>
  </main>
}

const allowed = new Set(sharedGlyphAllowance.map(pair => [...pair].sort().join(' ')))
const rowsOf = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>('[data-icon-row]')]
const iconsOf = (root: HTMLElement) => [...root.querySelectorAll<SVGSVGElement>('[data-icon-row] svg[data-icon]')]
const px = (value: string) => Number.parseFloat(value)
/** Rendered markup per meaning at one size; any two meanings drawing the same thing must be an allowed pair. */
function glyphProblems(root: HTMLElement): string[] {
  const byMarkup = new Map<string, string[]>()
  for (const row of rowsOf(root)) {
    const svg = row.querySelector('[data-size-column="16"] svg')
    if (svg === null || svg.children.length === 0) return [`${row.dataset.iconRow} draws nothing`]
    const markup = `${svg.getAttribute('fill')}|${[...svg.children].map(node => [node.tagName, ...[...node.attributes].filter(attribute => attribute.name !== 'class').map(attribute => `${attribute.name}=${attribute.value}`)].join(' ')).join(';')}`
    byMarkup.set(markup, [...byMarkup.get(markup) ?? [], row.dataset.iconRow!])
  }
  return [...byMarkup.values()].filter(names => names.length > 1 && !(names.length === 2 && allowed.has([...names].sort().join(' ')))).map(names => names.join(' = '))
}
/** Stroke stays at the stroke token on screen, and dots at the dot token, whatever the glyph size. */
function strokeProblems(root: HTMLElement): string[] {
  const stroke = px(getComputedStyle(root).getPropertyValue('--icon-gallery-stroke'))
  const dot = px(getComputedStyle(root).getPropertyValue('--icon-gallery-dot'))
  if (!(stroke > 0) || !(dot > 0)) return ['stroke and dot tokens did not resolve']
  return iconsOf(root).flatMap(svg => {
    const scale = px(getComputedStyle(svg).width) / 24
    const problems = Math.abs(px(getComputedStyle(svg).strokeWidth) * scale - stroke) > 0.01 ? [`${svg.dataset.icon}@${svg.dataset.iconSize} stroke`] : []
    if (iconRegistry[svg.dataset.icon as IconName].paint !== 'dots') return problems
    return [...problems, ...[...svg.querySelectorAll('circle')].filter(circle => Math.abs(2 * px(getComputedStyle(circle).getPropertyValue('r')) * scale - dot) > 0.01).map(() => `${svg.dataset.icon}@${svg.dataset.iconSize} dot`)]
  })
}
/** Each glyph renders at its column's size token, square. */
function sizeProblems(root: HTMLElement): string[] {
  const tokens = new Map(iconSizes.map(size => [size, px(getComputedStyle(root).getPropertyValue(`--icon-gallery-size${size}`))]))
  if ([...tokens.values()].some(value => !(value > 0))) return ['size tokens did not resolve']
  return iconsOf(root).filter(svg => {
    const column = Number(svg.closest<HTMLElement>('[data-size-column]')?.dataset.sizeColumn) as 12 | 14 | 16
    const style = getComputedStyle(svg)
    return Number(svg.dataset.iconSize) !== column || px(style.width) !== tokens.get(column) || px(style.height) !== tokens.get(column)
  }).map(svg => `${svg.dataset.icon}@${svg.dataset.iconSize} size`)
}

const meta = {
  title: 'Fractal UI/Icons',
  component: IconGallery,
  parameters: { layout: 'fullscreen' },
  args: { scheme: 'dark', width: 'wide' },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, width: { options: ['narrow', 'wide'], control: 'radio' } },
} satisfies Meta<typeof IconGallery>
export default meta
type Story = StoryObj<typeof meta>

export const Wide: Story = { play: async ({ canvasElement }) => {
  const root = canvasElement.querySelector<HTMLElement>('[data-gallery-width]')!
  await waitFor(() => expect(rowsOf(root).map(row => row.dataset.iconRow)).toEqual([...iconNames]))
  await expect(iconsOf(root)).toHaveLength(iconNames.length * iconSizes.length)

  // One meaning, one glyph. Control: a second row drawing the close glyph is rejected, then removed.
  await expect(glyphProblems(root)).toEqual([])
  const close = root.querySelector<HTMLElement>('[data-icon-row="close"]')!
  const copy = close.cloneNode(true) as HTMLElement
  copy.dataset.iconRow = 'reused-close'
  close.after(copy)
  await expect(glyphProblems(root)).toEqual(['close = reused-close'])
  copy.remove()
  await expect(glyphProblems(root)).toEqual([])

  // Stroke and dots on screen. Controls: a 16 glyph given the 12 stroke, and a 16 dot given the 12 radius.
  await expect(strokeProblems(root)).toEqual([])
  const small = root.querySelector<SVGSVGElement>('[data-icon-row="more"] [data-size-column="12"] svg')!
  const large = root.querySelector<SVGSVGElement>('[data-icon-row="more"] [data-size-column="16"] svg')!
  large.style.strokeWidth = getComputedStyle(small).strokeWidth
  await expect(strokeProblems(root)).toEqual(['more@16 stroke'])
  large.style.removeProperty('stroke-width')
  const dot = large.querySelector('circle')!
  dot.style.setProperty('r', getComputedStyle(small.querySelector('circle')!).getPropertyValue('r'))
  await expect(strokeProblems(root)).toEqual(['more@16 dot'])
  dot.style.removeProperty('r')
  await expect(strokeProblems(root)).toEqual([])

  // Size tokens. Control: a 16 glyph moved into the 12 column is rejected, then moved back.
  await expect(sizeProblems(root)).toEqual([])
  const home = large.parentElement!
  small.parentElement!.append(large)
  await expect(sizeProblems(root)).toEqual(['more@16 size'])
  home.append(large)
  await expect(sizeProblems(root)).toEqual([])
} }
export const Narrow: Story = { ...Wide, args: { width: 'narrow' } }
export const WideLight: Story = { ...Wide, args: { scheme: 'light' } }
export const NarrowLight: Story = { ...Wide, args: { scheme: 'light', width: 'narrow' } }

const styles = stylex.create({
  root: {
    // The size, stroke and dot tokens, exposed so the play compares rendered glyphs with the token values.
    '--icon-gallery-size12': icon.size12, '--icon-gallery-size14': icon.size14, '--icon-gallery-size16': icon.size16,
    '--icon-gallery-stroke': icon.stroke, '--icon-gallery-dot': icon.dot,
    minHeight: '100vh', boxSizing: 'border-box', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans,
  },
  frame: { boxSizing: 'border-box', maxWidth: '100%', padding: s.xl, display: 'flex', flexDirection: 'column', gap: s.section },
  narrow: { width: gallery.narrow },
  wide: { width: gallery.wide },
  header: { display: 'flex', flexDirection: 'column', gap: s.xs },
  title: { margin: s.zero, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightSemibold, color: ink.fg },
  lede: { margin: s.zero, fontSize: t.uiSize, lineHeight: t.uiLeading, color: ink.fgSoft },
  section: { display: 'flex', flexDirection: 'column', gap: s.md },
  group: { margin: s.zero, paddingBottom: s.xs, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightSemibold, color: ink.fgSoft },
  list: { margin: s.zero, padding: s.zero, listStyleType: 'none', display: 'grid', gridTemplateColumns: 'minmax(0, 1fr)', columnGap: s.section, rowGap: s.md },
  listWide: { gridTemplateColumns: `repeat(auto-fill, minmax(${gallery.column}, 1fr))` },
  row: { display: 'grid', gridTemplateColumns: `${g.controlSm} ${g.controlSm} ${g.controlSm} minmax(0, 1fr)`, alignItems: 'start', columnGap: s.sm, minWidth: 0 },
  cell: { display: 'flex', alignItems: 'center', justifyContent: 'center', height: g.controlSm, color: ink.fg },
  text: { display: 'flex', flexDirection: 'column', minWidth: 0, paddingTop: s.xs2, overflowWrap: 'anywhere' },
  name: { fontFamily: t.fontMono, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fg },
  meaning: { fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgSoft },
  glyph: { fontSize: t.denseSize, lineHeight: t.metaLeading, color: ink.fgMuted },
})
