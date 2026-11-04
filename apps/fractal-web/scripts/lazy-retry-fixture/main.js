import { createLazyChunk, readLazyChunkAssets } from '@fractal/ui/lazy-chunk'
import { Schema } from 'effect'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { persistedAtom } from '../../src/state/persistence.ts'
import { createLocalComposerDraftStore } from '../../src/web/composerDrafts.ts'

const draft = document.querySelector('#draft')
const selection = document.querySelector('#selection')
const recover = document.querySelector('#recover')
const status = document.querySelector('#status')
const drafts = createLocalComposerDraftStore({ storage: localStorage, events: window })
const registry = AtomRegistry.make()
const selectedAgent = persistedAtom({ key: 'round2.agent', schema: Schema.String, defaultValue: 'Alpha' })
draft.value = drafts.read('lazy-proof')?.text ?? ''
selection.value = registry.get(selectedAgent)
const saved = JSON.parse(sessionStorage.getItem('lazy-proof-state') ?? 'null')
if (saved) {
  // The real composer store and workspace persistence restore text and selection.
  draft.setSelectionRange(saved.start, saved.end, saved.direction)
}
const persist = () => {
  drafts.write('lazy-proof', { version: 1, text: draft.value, updatedAt: Math.max(Date.now(), (drafts.read('lazy-proof')?.updatedAt ?? 0) + 1) })
  registry.set(selectedAgent, selection.value)
  sessionStorage.setItem('lazy-proof-state', JSON.stringify({
    start: draft.selectionStart, end: draft.selectionEnd, direction: draft.selectionDirection,
  }))
}
draft.addEventListener('input', persist)
draft.addEventListener('select', persist)
selection.addEventListener('change', persist)
window.lazyProof = { document: crypto.randomUUID(), attempts: 0 }
const chunk = createLazyChunk({
  load: () => {
    window.lazyProof.attempts++
    return import('./feature.js')
  },
  assets: () => readLazyChunkAssets('conversation'),
})
async function load(retry = false) {
  status.textContent = 'loading'
  recover.hidden = true
  try {
    const feature = await (retry ? chunk.retry() : chunk.load())
    feature.mount(document.querySelector('#feature'))
    status.textContent = 'ready'
  } catch {
    status.textContent = 'failed'
    recover.textContent = chunk.recovery === 'reload' ? 'Reload and retry' : 'Try again'
    recover.hidden = false
  }
}
document.querySelector('#load').onclick = () => {
  sessionStorage.setItem('lazy-proof-open', 'true')
  void load()
}
recover.onclick = async () => {
  persist()
  if (chunk.recovery === 'reload') await chunk.reload()
  else await load(true)
}
if (sessionStorage.getItem('lazy-proof-open') === 'true') void load()
