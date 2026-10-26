import '../web/app.css'
import './embed.css'
import { createRoot } from 'react-dom/client'
import { setTheme } from '../ui-compat/foundations.ts'
import { Embed, fragments } from './Embed.tsx'

const theme = new URLSearchParams(location.search).get('theme') === 'light' ? 'light' : 'dark'
setTheme(theme)
const rootElement = document.getElementById('root')
if (rootElement === null) throw new Error('Missing embed root')
const root = createRoot(rootElement)
const render = () => {
  const fragment = fragments.find(value => `#${value}` === location.hash) ?? 'web'
  root.render(<Embed fragment={fragment} scheme={theme} />)
}
window.addEventListener('hashchange', render)
render()
