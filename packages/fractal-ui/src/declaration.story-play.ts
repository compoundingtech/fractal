import type { StoryObj } from '@storybook/react-vite'
import type { DeclarationPanelState } from './assistant-ui/declaration/DeclarationPanel'
import type { Scheme } from './assistant-ui/composition-theme'
interface PanelArgs { readonly scheme: Scheme; readonly title: string; readonly state: DeclarationPanelState }
type Play = NonNullable<StoryObj<PanelArgs>['play']>
export const panelIn = (canvas: HTMLElement): HTMLElement => {
  const panel = canvas.querySelector<HTMLElement>('[data-testid="declaration-panel"]')
  if (panel === null) throw new Error('The declaration panel did not render')
  return panel
}
export const tokens = (panel: HTMLElement, kind: string): readonly (string | null)[] => [...panel.querySelectorAll<HTMLElement>(`[data-token="${kind}"]`)].map(node => node.textContent)
/** Ready panels: one numbered row per source line, verbatim text, a copy action and the provenance line. */
export const readyPlay = (text: string): Play => async ({ canvasElement }) => {
  const panel = panelIn(canvasElement)
  const rows = panel.querySelectorAll('[data-line]')
  const expected = text.replace(/\n$/, '').split('\n')
  if (rows.length !== expected.length) throw new Error(`Expected ${expected.length} numbered lines, got ${rows.length}`)
  const numbers = [...panel.querySelectorAll('[data-line-number]')]
  if (numbers.some(node => node.getAttribute('aria-hidden') !== 'true')) throw new Error('Line numbers must stay out of the accessible text')
  if (numbers.at(-1)?.textContent !== String(expected.length)) throw new Error('The gutter must number every line')
  const shown = [...panel.querySelectorAll('[data-line] code')].map(node => node.textContent).join('\n')
  if (shown !== expected.join('\n')) throw new Error('The code view must render the source verbatim')
  if (panel.querySelector('button[aria-label="Copy all declaration source"]') === null) throw new Error('Ready panels offer copy-all')
  if (panel.querySelector('[role="group"][tabindex="0"]') === null) throw new Error('The code group must be keyboard scrollable')
}
