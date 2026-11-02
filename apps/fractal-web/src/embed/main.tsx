import { mountEmbed } from './entry.tsx'

const rootElement = document.getElementById('root')
if (rootElement === null) throw new Error('Missing embed root')
const dispose = mountEmbed(rootElement)
window.addEventListener('pagehide', dispose, { once: true })
