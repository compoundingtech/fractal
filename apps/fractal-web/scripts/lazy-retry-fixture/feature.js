import { message } from './dependency.js'
import './feature.css'

sessionStorage.setItem('lazy-proof-root-executions', String(Number(sessionStorage.getItem('lazy-proof-root-executions') ?? 0) + 1))
export function mount(element) {
  element.className = 'lazy-feature'
  element.textContent = message
}
