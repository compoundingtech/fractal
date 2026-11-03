import '../web/app.css'
import './embed.css'
import './entry.css'
import { createRoot } from 'react-dom/client'
import { setTheme } from '../ui-compat/foundations.ts'
import { Embed, fragments } from './Embed.tsx'

/** The production entry and tests share exactly the same CSS, theme and lifecycle setup. */
export const mountEmbed = (rootElement: HTMLElement): (() => void) => {
  const theme = new URLSearchParams(window.location.search).get('theme') === 'light' ? 'light' : 'dark'
  setTheme(theme)
  const root = createRoot(rootElement)
  const render = () => {
    const fragment = fragments.find(value => `#${value}` === window.location.hash) ?? 'web'
    root.render(<Embed key={`${fragment}:${theme}`} fragment={fragment} scheme={theme} />)
  }
  window.addEventListener('hashchange', render)
  render()
  return () => { window.removeEventListener('hashchange', render); root.unmount() }
}
