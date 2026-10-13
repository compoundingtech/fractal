import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { QuotaSidebarFooter, quotaFooterStartsExpanded } from './QuotaSidebarFooter'
import { quotaFooterAccounts as accounts } from './quota-footer-fixtures'
import { expectNoNativeTitles } from './full-value-tooltip-play.ts'
import { baselineTheme } from './assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from './assistant-ui/composition-theme'
import { ThemePortal } from './assistant-ui/taste/ThemePortal'
import { geometryVars as geometry, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from './assistant-ui/composition-tokens.stylex'
import { quotaGeometry } from './quota-geometry.stylex.ts'

type Height = 'short' | 'tall'
type Width = 208 | 256 | 320 | 360
interface Args { scheme: Scheme; height: Height; width: Width }

/** Synthetic agent names; the roster takes the sidebar height the footer leaves. */
const roster = ['Atlas review', 'Birch planner', 'Cobalt triage', 'Delta notes', 'Ember build', 'Fern docs', 'Garnet sync', 'Hazel tests', 'Iris layout', 'Juniper scan', 'Kestrel deploy', 'Linden audit', 'Maple drafts', 'Nimbus watch', 'Onyx bench', 'Poplar index', 'Quill copy', 'Rowan merge', 'Sable fetch', 'Tamarack lint', 'Umber cache', 'Vale route', 'Willow parse', 'Yarrow sort']
const widths: readonly Width[] = [208, 256, 320, 360]
const status = { kind: 'current', text: 'Updated just now' } as const

function Frame({ scheme, children }: { scheme: Scheme; children: React.ReactNode }) {
  return <div data-scheme={scheme} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.frame)}><ThemePortal>{children}</ThemePortal></div>
}

function Sidebar({ label, height, width, onOpenAccount }: { label: string; height: Height; width: Width; onOpenAccount: (id: string) => void }) {
  return <aside aria-label={label} data-quota-story-sidebar={width} {...stylex.props(styles.sidebar, height === 'short' ? styles.short : styles.tall, width === 208 ? styles.width208 : width === 256 ? styles.width256 : width === 320 ? styles.width320 : styles.width360)}>
    <ul aria-label={`${label} agents`} {...stylex.props(styles.roster)}>{roster.map(name => <li key={name} {...stylex.props(styles.agent)}>{name}</li>)}</ul>
    <QuotaSidebarFooter rows={accounts} status={status} onOpenAccount={onOpenAccount} />
  </aside>
}

function Single({ scheme, height, width }: Args) {
  const [opened, setOpened] = React.useState<string>()
  return <Frame scheme={scheme}><div {...stylex.props(styles.stage)}>
    <Sidebar label="Sidebar" height={height} width={width} onOpenAccount={setOpened} />
    <p role="status" data-quota-story-opened={opened} {...stylex.props(styles.caption)}>{opened === undefined ? 'No account opened' : `Opened usage details for ${accounts.find(account => account.id === opened)?.label}`}</p>
  </div></Frame>
}

const meta = {
  title: 'Fractal UI/Quota Sidebar Footer',
  render: Single,
  parameters: { layout: 'fullscreen', docs: { description: { component: 'Quota in the sidebar footer, one 22px line per account in caller priority order: account, primary reading, reset, the secondary window from 20rem, then +N for the rest. Below 20rem bars drop before the account is cut. A React Aria disclosure starts expanded only when every row fits in half the sidebar; otherwise it starts collapsed and names the most pressing reading. Synthetic fixtures.' } } },
  args: { scheme: 'dark', height: 'tall', width: 256 },
  argTypes: { scheme: { options: ['dark', 'light'], control: 'radio' }, height: { options: ['short', 'tall'], control: 'radio' }, width: { options: widths, control: 'select' } },
} satisfies Meta<Args>
export default meta
type Story = StoryObj<typeof meta>

/** Every row on one 22px line in aligned columns 8px apart; readings, reset and +N are never cut; only the account truncates. Synchronous, so negative controls can expect it to throw. */
function assertFooter(footer: HTMLElement) {
  const width = footer.getBoundingClientRect().width
  const wide = footer.hasAttribute('data-quota-footer-wide')
  expect(wide).toBe(width >= 320)
  const rows = [...footer.querySelectorAll<HTMLElement>('[data-quota-footer-row]')]
  expect(rows.map(row => row.dataset.quotaFooterRow)).toEqual(accounts.map(account => account.id))
  const kinds = wide ? ['account', 'primary', 'reset', 'secondary', 'more'] : ['account', 'primary', 'reset', 'more']
  const columns = [...rows[0]!.children].map(cell => cell.getBoundingClientRect().left)
  for (const row of rows) {
    const id = row.dataset.quotaFooterRow!
    const account = accounts.find(candidate => candidate.id === id)!
    expect(Math.round(row.getBoundingClientRect().height), `${id} row height`).toBe(22)
    const cells = [...row.children] as HTMLElement[]
    expect(cells.map(cell => cell.dataset.quotaFooterCell)).toEqual(kinds)
    cells.forEach((cell, index) => {
      const box = cell.getBoundingClientRect()
      expect(Math.abs(box.left - columns[index]!), `${id} ${kinds[index]} column`).toBeLessThan(1)
      if (index === 0) return
      expect(box.left - cells[index - 1]!.getBoundingClientRect().right, `${id} gap before ${kinds[index]}`).toBeGreaterThanOrEqual(7.9)
      expect(cell.scrollWidth, `${id} ${kinds[index]} is cut`).toBeLessThanOrEqual(cell.clientWidth)
    })
    const name = row.querySelector<HTMLElement>('[data-quota-footer-account]')!
    const range = name.ownerDocument.createRange()
    range.selectNodeContents(name)
    expect(new Set([...range.getClientRects()].map(rect => Math.round(rect.top))).size, `${id} account lines`).toBe(1)
    expect(row.querySelector('[data-quota-footer-reset]')?.textContent).toBe(account.compactReset)
    if (!wide) expect(row.querySelectorAll('[data-quota-footer-bar]')).toHaveLength(0)
    if (wide && account.governing.fraction !== undefined) expect(row.querySelector('[data-quota-footer-cell="primary"] [data-quota-footer-bar]')).not.toBeNull()
    // Every fixture row has primary, secondary and monthly windows: +N is the monthly, plus the secondary once it has no column.
    expect(row.querySelector('[data-quota-footer-more]')?.textContent).toMatch(new RegExp(`^\\+${wide ? 1 : 2}\\b`))
  }
}

const settle = { timeout: 5000 }
const findFooters = (canvasElement: HTMLElement, count: number) => waitFor(() => {
  const found = [...canvasElement.querySelectorAll<HTMLElement>('[data-quota-footer]')]
  expect(found).toHaveLength(count)
  return found
})
const parts = (footer: HTMLElement) => ({
  trigger: footer.querySelector<HTMLElement>('[data-quota-footer-trigger]')!,
  panel: footer.querySelector<HTMLElement>('[data-quota-footer-panel]')!,
  scroller: footer.querySelector<HTMLElement>('[data-quota-footer-scroller]')!,
  sidebar: footer.parentElement!,
})
const expectExpanded = (footer: HTMLElement, expanded: boolean, choice: 'space' | 'user') => waitFor(() => {
  const { trigger, panel } = parts(footer)
  expect(trigger).toHaveAttribute('aria-expanded', String(expanded))
  expect(panel.hasAttribute('hidden')).toBe(!expanded)
  expect(footer).toHaveAttribute('data-quota-footer-choice', choice)
}, settle)

/** Hover and keyboard focus each show one tooltip with the account's full evidence; the button keeps it as its description. */
async function expectAccountTooltip(button: HTMLElement, full: string) {
  const tooltips = () => [...button.ownerDocument.querySelectorAll<HTMLElement>('[role="tooltip"]')]
  const one = () => waitFor(() => {
    expect(tooltips()).toHaveLength(1)
    expect(tooltips()[0]).toHaveTextContent(full)
    const box = tooltips()[0]!.getBoundingClientRect()
    expect(box.width).toBeGreaterThan(box.height)
  }, settle)
  const none = () => waitFor(() => expect(tooltips()).toHaveLength(0), settle)
  button.scrollIntoView({ block: 'center', inline: 'nearest' })
  await userEvent.pointer({ keys: '[MouseLeft]', target: button.ownerDocument.body })
  await userEvent.hover(button)
  await one()
  await userEvent.unhover(button)
  await none()
  await userEvent.keyboard('{F13}')
  button.focus()
  await one()
  button.blur()
  await none()
}

export const Tall: Story = {
  play: async ({ canvasElement }) => {
    const [footer] = await findFooters(canvasElement, 1)
    const { trigger, scroller, sidebar } = parts(footer)
    await expectExpanded(footer, true, 'space')
    assertFooter(footer)
    await expect(scroller.scrollHeight).toBeLessThanOrEqual(scroller.clientHeight)
    await expect(footer.getBoundingClientRect().height).toBeLessThanOrEqual(sidebar.clientHeight / 2 + 0.5)
    await expectNoNativeTitles(footer)
    await expect(trigger).toHaveAccessibleName(/^Usage.*most pressing.*1w 100%.*cx\/harbor-lab$/)
    await expect(footer.textContent).not.toMatch(/unknown/i)

    const north = footer.querySelector<HTMLElement>('[data-quota-footer-account="north"]')!
    await expect(north).toHaveAccessibleName('gh/north-studio')
    await expect(north).toHaveAccessibleDescription('North studio · Reset in 1h 12m · Opens usage details')
    await expectAccountTooltip(north, 'gh/north-studio · North studio · Reset in 1h 12m')
    await userEvent.click(north)
    await waitFor(() => expect(canvasElement.querySelector('[data-quota-story-opened]')).toHaveAttribute('data-quota-story-opened', 'north'), settle)

    const more = within(footer).getByRole('button', { name: '+2 more windows for North studio' })
    await userEvent.click(more)
    const dialog = await within(document.body).findByRole('dialog', { name: 'North studio · more windows' }, settle)
    await expect(within(dialog).getByText('41%')).toBeVisible()
    await expect(within(dialog).getByText('18%')).toBeVisible()
    await expect(dialog.querySelectorAll('[data-quota-footer-reading]')).toHaveLength(2)
    await userEvent.click(within(dialog).getByRole('button', { name: 'Close' }))
    await waitFor(() => expect(dialog).not.toBeInTheDocument(), settle)
    assertFooter(footer)

    // Negative controls: a cut reading, a missing cell, a closed gap and a wrapped account each fail the geometry check.
    const harborPrimary = footer.querySelector<HTMLElement>('[data-quota-footer-row="harbor"] [data-quota-footer-cell="primary"]')!
    harborPrimary.style.width = '12px'
    expect(() => assertFooter(footer)).toThrow()
    harborPrimary.style.removeProperty('width')
    const reset = footer.querySelector<HTMLElement>('[data-quota-footer-row="north"] [data-quota-footer-cell="reset"]')!
    const parent = reset.parentElement!, next = reset.nextSibling
    reset.remove()
    expect(() => assertFooter(footer)).toThrow()
    parent.insertBefore(reset, next)
    const table = footer.querySelector<HTMLElement>('[data-quota-footer-table]')!
    table.style.columnGap = '0'
    expect(() => assertFooter(footer)).toThrow()
    table.style.removeProperty('column-gap')
    const longest = footer.querySelector<HTMLElement>('[data-quota-footer-account="ventures"]')!
    longest.style.whiteSpace = 'normal'
    expect(() => assertFooter(footer)).toThrow()
    longest.style.removeProperty('white-space')
    assertFooter(footer)

    // The user's keyboard toggle collapses to the one-line trigger and holds.
    trigger.focus()
    await userEvent.keyboard('{Enter}')
    await expectExpanded(footer, false, 'user')
    await expect(footer.getBoundingClientRect().height).toBeLessThanOrEqual(trigger.getBoundingClientRect().height + 1.5)
    await userEvent.keyboard(' ')
    await expectExpanded(footer, true, 'user')
    assertFooter(footer)
  },
}

export const Short: Story = {
  args: { height: 'short' },
  play: async ({ canvasElement }) => {
    const [footer] = await findFooters(canvasElement, 1)
    const { trigger, scroller, sidebar } = parts(footer)
    await expectExpanded(footer, false, 'space')
    const headerHeight = trigger.getBoundingClientRect().bottom - footer.getBoundingClientRect().top
    await expect(footer.getBoundingClientRect().height).toBeLessThanOrEqual(headerHeight + 0.5)
    await expect(footer.querySelector('[data-quota-footer-pressing]')?.textContent).toContain('1w')
    await expect(trigger).toHaveAccessibleName(/^Usage.*most pressing.*1w 100%.*cx\/harbor-lab$/)
    // The rule, with a control: the 13 rows need 13 × 22px plus the status line; this sidebar's half is too small, four times its height is not.
    const rowsHeight = (accounts.length + 1) * 22
    await expect(quotaFooterStartsExpanded({ panelHeight: rowsHeight, headerHeight, sidebarHeight: sidebar.clientHeight })).toBe(false)
    await expect(quotaFooterStartsExpanded({ panelHeight: rowsHeight, headerHeight, sidebarHeight: sidebar.clientHeight * 4 })).toBe(true)

    // Until the user chooses, the default follows the sidebar's height in both directions.
    sidebar.style.height = '900px'
    await expectExpanded(footer, true, 'space')
    assertFooter(footer)
    sidebar.style.removeProperty('height')
    await expectExpanded(footer, false, 'space')

    // Expanded by the user on the short sidebar, the rows scroll inside half its height.
    await userEvent.click(trigger)
    await expectExpanded(footer, true, 'user')
    await expect(scroller.scrollHeight).toBeGreaterThan(scroller.clientHeight)
    await expect(footer.getBoundingClientRect().height).toBeLessThanOrEqual(sidebar.clientHeight / 2 + 0.5)
    assertFooter(footer)
    const last = footer.querySelector<HTMLElement>(`[data-quota-footer-account="${accounts.at(-1)!.id}"]`)!
    await userEvent.keyboard('{F13}')
    last.focus()
    await waitFor(() => {
      const box = last.getBoundingClientRect(), view = scroller.getBoundingClientRect()
      expect(box.top).toBeGreaterThanOrEqual(view.top - 0.5)
      expect(box.bottom).toBeLessThanOrEqual(view.bottom + 0.5)
    }, settle)
    last.blur()

    // The choice holds after the footer re-measures a shorter and then the restored sidebar.
    const remeasured = () => waitFor(() => expect(getComputedStyle(scroller).maxBlockSize).toBe(`${Math.floor(sidebar.clientHeight / 2 - headerHeight)}px`), settle)
    sidebar.style.height = '200px'
    await remeasured()
    await expectExpanded(footer, true, 'user')
    sidebar.style.removeProperty('height')
    await remeasured()
    await expectExpanded(footer, true, 'user')
    trigger.focus()
    await userEvent.keyboard(' ')
    await expectExpanded(footer, false, 'user')
    await expectNoNativeTitles(footer)
  },
}

/** 208 and 256 fold the secondary into +N and drop bars; from 320 the secondary has its own column and bars return. */
export const Widths: Story = {
  render: ({ scheme }) => <Frame scheme={scheme}><div {...stylex.props(styles.matrix)}>{widths.map(width => <Sidebar key={width} label={`Sidebar ${width}`} height="tall" width={width} onOpenAccount={() => {}} />)}</div></Frame>,
  play: async ({ canvasElement }) => {
    const footers = await findFooters(canvasElement, widths.length)
    for (const footer of footers) {
      await expectExpanded(footer, true, 'space')
      await expect(Math.round(footer.getBoundingClientRect().width)).toBe(Number(footer.parentElement!.dataset.quotaStorySidebar))
      assertFooter(footer)
    }
    await expect(footers.map(footer => footer.hasAttribute('data-quota-footer-wide'))).toEqual([false, false, true, true])
    await expectNoNativeTitles(canvasElement)
  },
}

/** Both schemes side by side: the short sidebar starts collapsed, the tall one expanded. */
export const Overview: Story = {
  render: () => <div {...stylex.props(styles.matrix)}>{(['dark', 'light'] as const).flatMap(scheme => (['short', 'tall'] as const).map(height => <Frame key={`${scheme}-${height}`} scheme={scheme}>
    <Sidebar label={`${scheme} ${height} sidebar`} height={height} width={256} onOpenAccount={() => {}} />
  </Frame>))}</div>,
  play: async ({ canvasElement }) => {
    const footers = await findFooters(canvasElement, 4)
    for (const [index, footer] of footers.entries()) await expectExpanded(footer, index % 2 === 1, 'space')
  },
}

const styles = stylex.create({
  frame: { backgroundColor: surface.canvas, color: ink.fg, padding: s.xl, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading },
  stage: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl },
  matrix: { display: 'flex', flexWrap: 'wrap', alignItems: 'flex-start', gap: s.xl },
  sidebar: { display: 'flex', flexDirection: 'column', boxSizing: 'border-box', maxWidth: '100%', overflow: 'hidden', backgroundColor: surface.sidebar, outline: quotaGeometry.hair },
  short: { height: quotaGeometry.footerStoryShortHeight },
  tall: { height: quotaGeometry.footerStoryTallHeight },
  width208: { width: geometry.sidebarMin },
  width256: { width: geometry.sidebarDefault },
  width320: { width: quotaGeometry.drawerMediumWidth },
  width360: { width: quotaGeometry.footerStoryWideWidth },
  roster: { flexGrow: 1, minHeight: 0, overflow: 'hidden', margin: 0, paddingBlock: s.sm, paddingInline: s.lg, listStyle: 'none' },
  agent: { height: geometry.controlMd, display: 'flex', alignItems: 'center', color: ink.fgSoft, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' },
  caption: { margin: 0, color: ink.fgMuted },
})
