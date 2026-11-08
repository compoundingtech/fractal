import type { ComponentProps } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, waitFor } from 'storybook/test'
import { AppStorySpecimen, type AppStoryArgs } from '../fixtures/AppStoryHarness.tsx'
import { assertProductionAppState } from '../fixtures/appStoryAssertions.ts'
import { appStoryAgent, appStoryStates } from '../fixtures/appStoryStates.ts'
import { fixtureProjections } from '../fixtures/projections.ts'
import { LiveAgentWorkspace } from './LiveAgentWorkspace.tsx'
import { writeCellsLikeAnotherWindow } from '../state/cell/testing.ts'

/** The production workspace module, as the app mounts it, over one availability state of the fixture world. */
const meta = {
  excludeStories: ['scenarioExemptions'],
  title: 'Fractal/App/LiveAgentWorkspace',
  component: LiveAgentWorkspace,
  args: { state: 'populated', scheme: 'dark' },
  argTypes: {
    state: { control: 'select', options: appStoryStates },
    scheme: { control: 'radio', options: ['dark', 'light'] },
  },
  parameters: { layout: 'fullscreen', scenario: { _tag: 'contract-fixture', reason: 'The production LiveAgentWorkspace module, with its lazily loaded ConversationPane, over fixtureSource feeds of the deterministic fixture world pins loading, empty, populated, failed-tool, offline, unavailable and streaming repair states without a gateway.' } },
  // The workspace keeps its scheme, selected agent and panes in persisted UI state; every story starts from the same
  // state. Storage events make the already-running cell store adopt it, as it does another window's write.
  beforeEach: ({ args }) => {
    const agentRef = appStoryAgent(args.state)
    writeCellsLikeAnotherWindow({
      'round2.scheme': { value: args.scheme },
      'round2.agent': { value: agentRef },
      'panels.leftSidebar': { value: { open: true, ratio: 256 / 1440 } },
      [`round2.panes.${agentRef}`]: { value: [] },
    })
  },
  render: ({ state }) => <AppStorySpecimen state={state}>{() => <LiveAgentWorkspace initialRef={appStoryAgent(state)} persistGeometry={false} />}</AppStorySpecimen>,
  play: async ({ canvasElement, args }) => {
    // On a cold dev server the lazily loaded pane takes longer than waitFor's default second.
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 60000 })
    canvasElement.dataset.proof = 'passed'
  },
} satisfies Meta<ComponentProps<typeof LiveAgentWorkspace> & AppStoryArgs>
export default meta

// Reviewed scenario exemptions by story export and mode; scenario.story-check.test.ts requires an exact match.
export const scenarioExemptions: readonly string[] = [
  'Empty:contract-fixture',
  'FailedTool:contract-fixture',
  'Loading:contract-fixture',
  'Offline:contract-fixture',
  'Populated:contract-fixture',
  'ScriptedRepair:contract-fixture',
  'SidebarDragToPane:contract-fixture',
  'Splits:contract-fixture',
  'SplitsDragAndDrop:contract-fixture',
  'TabSwitchGeometry:contract-fixture',
  'Unavailable:contract-fixture',
]
type Story = StoryObj<typeof meta>

// One story per state; the scheme control switches light and dark. The kit play gate runs each in one
// dark desktop case (APP_CASE_OVERRIDES in packages/fractal-ui/scripts/kit-play-commit-set.mjs).
export const Loading: Story = { args: { state: 'loading' } }
export const Empty: Story = { args: { state: 'empty' } }
export const Populated: Story = { args: { state: 'populated' } }
export const FailedTool: Story = { args: { state: 'failed-tool' } }
export const Offline: Story = { args: { state: 'offline' } }
export const Unavailable: Story = { args: { state: 'unavailable' } }
export const ScriptedRepair: Story = { args: { state: 'scripted-repair' } }

/** Every tab's box and the strip's height, rounded to a tenth of a pixel. */
const stripGeometry = (canvas: HTMLElement) => {
  const strip = canvas.querySelector<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"]')!
  const box = (element: Element) => { const rect = element.getBoundingClientRect(); return [rect.x, rect.y, rect.width, rect.height].map(value => Math.round(value * 10) / 10) }
  return { strip: box(strip)[3], tabs: [...strip.querySelectorAll('[role="tab"]')].map(box) }
}

/** Switching tabs changes colour and the accent line only: every tab keeps its box, and the strip its height. */
export const TabSwitchGeometry: Story = {
  args: { state: 'populated' },
  play: async ({ canvasElement, args }) => {
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 60000 })
    // Two more agents' tabs, opened as a link would.
    for (const agent of fixtureProjections.agents.filter(row => row.id !== appStoryAgent(args.state)).slice(0, 2)) {
      window.history.pushState(null, '', `/w/${agent.id}`)
      window.dispatchEvent(new PopStateEvent('popstate'))
    }
    const tabs = () => [...canvasElement.querySelectorAll<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"] [role="tab"]')]
    await waitFor(() => expect(tabs()).toHaveLength(3))
    await document.fonts.ready
    const before = stripGeometry(canvasElement)
    for (const index of [0, 1, 2, 0]) {
      tabs()[index]!.click()
      await waitFor(() => expect(tabs()[index]!.getAttribute('aria-selected')).toBe('true'))
      expect(stripGeometry(canvasElement)).toEqual(before)
    }
    canvasElement.dataset.proof = 'passed'
  },
}

const mac = () => /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)
const press = (init: KeyboardEventInit, target: EventTarget = document.body) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }))

/**
 * Splits, cmux style: a tab moves into a split from its menu, Alt+O zooms and back, Alt+Left moves to the
 * other split, a divider resizes from the keyboard, Mod+D makes an empty split and Alt+W closes it. The
 * moved tab's surface is the same node throughout: nothing remounts.
 */
export const Splits: Story = {
  args: { state: 'populated' },
  play: async ({ canvasElement, args }) => {
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 60000 })
    const other = fixtureProjections.agents.find(row => row.id !== appStoryAgent(args.state))!
    window.history.pushState(null, '', `/w/${other.id}`)
    window.dispatchEvent(new PopStateEvent('popstate'))
    const strips = () => [...canvasElement.querySelectorAll<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"]')]
    const tabs = () => strips().flatMap(strip => [...strip.querySelectorAll<HTMLElement>('[role="tab"]')])
    const shownPanes = () => [...canvasElement.querySelectorAll('[data-pane]')].filter(pane => pane.closest('[hidden]') === null)
    await waitFor(() => expect(tabs()).toHaveLength(2))
    const moved = tabs()[1]!
    const surface = () => canvasElement.querySelector(`[data-tab-surface="agent:${other.id}"]`)
    await waitFor(() => expect(surface()).not.toBeNull())
    const node = surface()
    moved.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    await waitFor(() => expect([...document.querySelectorAll('[role="menuitem"]')].some(item => item.textContent === 'Split right')).toBe(true))
    ;[...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(item => item.textContent === 'Split right')!.click()
    await waitFor(() => expect(strips()).toHaveLength(2))
    expect(surface()).toBe(node)
    press({ key: 'o', code: 'KeyO', altKey: true })
    await waitFor(() => expect(shownPanes()).toHaveLength(1))
    press({ key: 'o', code: 'KeyO', altKey: true })
    await waitFor(() => expect(shownPanes()).toHaveLength(2))
    expect(surface()).toBe(node)
    const focusedPane = () => canvasElement.querySelector('[data-pane][data-focused]')
    const right = focusedPane()
    press({ key: 'ArrowLeft', code: 'ArrowLeft', altKey: true })
    await waitFor(() => expect(focusedPane()).not.toBe(right))
    const divider = canvasElement.querySelector<HTMLElement>('[data-divider]')!
    const before = divider.getAttribute('aria-valuenow')
    divider.focus()
    press({ key: 'ArrowLeft', code: 'ArrowLeft' }, divider)
    await waitFor(() => expect(divider.getAttribute('aria-valuenow')).not.toBe(before))
    press({ key: 'd', code: 'KeyD', metaKey: mac(), ctrlKey: !mac() })
    await waitFor(() => expect(strips()).toHaveLength(3))
    press({ key: 'w', code: 'KeyW', altKey: true })
    await waitFor(() => expect(strips()).toHaveLength(2))
    expect(surface()).toBe(node)
    canvasElement.dataset.proof = 'passed'
  },
}

const drag = (from: Element, to: { readonly x: number; readonly y: number }) => {
  const start = from.getBoundingClientRect()
  const at = { x: start.x + start.width / 2, y: start.y + start.height / 2 }
  const event = (type: string, point: { readonly x: number; readonly y: number }) =>
    new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 7, pointerType: 'mouse', button: 0, buttons: type === 'pointerup' ? 0 : 1, clientX: point.x, clientY: point.y })
  from.dispatchEvent(event('pointerdown', at))
  for (let step = 1; step <= 8; step++) window.dispatchEvent(event('pointermove', { x: at.x + (to.x - at.x) * step / 8, y: at.y + (to.y - at.y) * step / 8 }))
  return () => window.dispatchEvent(event('pointerup', to))
}

/**
 * Pointer drag and drop, cmux style: a tab dragged to a pane's right edge previews the half it would take
 * under a live chip and splits there; dragged back onto the other strip it joins it, and the emptied
 * pane collapses. The tab's surface is the same node throughout.
 */
export const SplitsDragAndDrop: Story = {
  args: { state: 'populated' },
  play: async ({ canvasElement, args }) => {
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 60000 })
    const other = fixtureProjections.agents.find(row => row.id !== appStoryAgent(args.state))!
    window.history.pushState(null, '', `/w/${other.id}`)
    window.dispatchEvent(new PopStateEvent('popstate'))
    const strips = () => [...canvasElement.querySelectorAll<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"]')]
    const tabs = () => strips().flatMap(strip => [...strip.querySelectorAll<HTMLElement>('[role="tab"]')])
    await waitFor(() => expect(tabs()).toHaveLength(2))
    const surface = () => canvasElement.querySelector(`[data-tab-surface="agent:${other.id}"]`)
    await waitFor(() => expect(surface()).not.toBeNull())
    const node = surface()
    const slot = canvasElement.querySelector<HTMLElement>('[data-drop-content]')!.getBoundingClientRect()
    const drop = drag(tabs()[1]!, { x: slot.right - 12, y: slot.top + slot.height / 2 })
    await waitFor(() => expect(canvasElement.querySelector('[data-drop-overlay]')?.getAttribute('data-drop-overlay')).toBe('right'))
    // The chip names the entity as its tab does.
    expect(tabs()[1]!.getAttribute('aria-label')).toContain(document.querySelector('[data-testid="drag-chip"]')?.textContent ?? '∅')
    drop()
    await waitFor(() => expect(strips()).toHaveLength(2))
    expect(surface()).toBe(node)
    const home = strips()[0]!.querySelector('[role="tab"]')!.getBoundingClientRect()
    const back = drag(tabs()[1]!, { x: home.right - 4, y: home.top + home.height / 2 })
    back()
    await waitFor(() => expect(strips()).toHaveLength(1))
    expect(tabs()).toHaveLength(2)
    expect(surface()).toBe(node)
    canvasElement.dataset.proof = 'passed'
  },
}

/**
 * A sidebar row on the same pointer drag: an agent dragged from the roster to a pane's right edge
 * splits there and opens once, at the drop, under a live chip. Released over its own row, the drag
 * is not also a press that opens it.
 */
export const SidebarDragToPane: Story = {
  args: { state: 'populated' },
  play: async ({ canvasElement, args }) => {
    await waitFor(() => assertProductionAppState(canvasElement, args.state, true), { timeout: 60000 })
    const strips = () => [...canvasElement.querySelectorAll<HTMLElement>('[role="tablist"][aria-label="Workspace tabs"]')]
    const tabs = () => strips().flatMap(strip => [...strip.querySelectorAll<HTMLElement>('[role="tab"]')])
    await waitFor(() => expect(tabs()).toHaveLength(1))
    const other = fixtureProjections.agents.find(row => row.id !== appStoryAgent(args.state))!
    const row = () => canvasElement.querySelector<HTMLElement>(`[data-sidebar-tree-key="${other.id}"]`)
    await waitFor(() => expect(row()).not.toBeNull())
    // Released over itself: nothing opens.
    const own = row()!.getBoundingClientRect()
    drag(row()!, { x: own.left + own.width / 2 + 2, y: own.top + own.height / 2 + 12 })()
    await new Promise(resolve => setTimeout(resolve, 200))
    expect(tabs()).toHaveLength(1)
    const slot = canvasElement.querySelector<HTMLElement>('[data-drop-content]')!.getBoundingClientRect()
    const drop = drag(row()!, { x: slot.right - 12, y: slot.top + slot.height / 2 })
    await waitFor(() => expect(canvasElement.querySelector('[data-drop-overlay]')?.getAttribute('data-drop-overlay')).toBe('right'))
    await waitFor(() => expect(document.querySelector('[data-testid="drag-chip"]')).not.toBeNull())
    drop()
    await waitFor(() => expect(strips()).toHaveLength(2))
    expect(tabs()).toHaveLength(2)
    canvasElement.dataset.proof = 'passed'
  },
}
