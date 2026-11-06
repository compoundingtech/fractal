import assert from 'node:assert/strict'
import { after, beforeEach, test } from 'node:test'
import { JSDOM } from 'jsdom'
import { configure } from 'storybook/test'

// History readiness runs in the page with the browser's own frames; this page's frames are driven by the test.
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/' })
const saved = new Map()
for (const name of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'MutationObserver', 'getComputedStyle']) {
  saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value: name === 'getComputedStyle' ? dom.window.getComputedStyle.bind(dom.window) : dom.window[name] })
}
let callbacks = new Map(), nextFrame = 0
dom.window.requestAnimationFrame = callback => { callbacks.set(++nextFrame, callback); return nextFrame }
dom.window.cancelAnimationFrame = id => { callbacks.delete(id) }
const renderFrame = () => { const due = callbacks; callbacks = new Map(); for (const callback of due.values()) callback(dom.window.performance.now()) }
const { committedTranscriptHistory, tooltipFor } = await import('./story-play.ts')
after(() => { dom.window.close(); for (const [name, descriptor] of saved) { if (descriptor === undefined) Reflect.deleteProperty(globalThis, name); else Object.defineProperty(globalThis, name, descriptor) } })

const deadline = 200
configure({ asyncUtilTimeout: deadline })
const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
let canvas, controls, viewport
beforeEach(() => {
  callbacks = new Map()
  document.body.innerHTML = '<div id="canvas"><div data-testid="transcript-history-controls" data-state="materializing" aria-busy="true"></div><div data-testid="transcript-scroll"></div></div>'
  canvas = document.getElementById('canvas')
  controls = canvas.querySelector('[data-testid="transcript-history-controls"]')
  viewport = canvas.querySelector('[data-testid="transcript-scroll"]')
  viewport.innerHTML = '<div data-testid="transcript-turn"></div>'.repeat(10)
})
const turns = count => { viewport.insertAdjacentHTML('afterbegin', '<div data-testid="transcript-turn"></div>'.repeat(count)) }
const finish = () => { controls.setAttribute('aria-busy', 'false'); controls.dataset.state = 'complete' }

test('a browser that renders no frames past the deadline gives history the frames it needs before it fails', async () => {
  const readiness = committedTranscriptHistory(canvas, 14, new AbortController().signal)
  readiness.catch(() => {})
  // The compositor pauses: page timers run past the deadline and no frame renders.
  await wait(deadline * 2)
  // The first frame after the pause runs the viewport's batch; its commit completes the history.
  renderFrame()
  turns(4)
  finish()
  const shown = await readiness
  assert.equal(shown, viewport)
  assert.equal(viewport.querySelectorAll('[data-testid="transcript-turn"]').length, 14)
})

test('a pause that outlasts the deadline after a frame still lets the next frame commit history', async () => {
  const readiness = committedTranscriptHistory(canvas, 14, new AbortController().signal)
  readiness.catch(() => {})
  renderFrame()
  await wait(deadline * 2)
  renderFrame()
  turns(4)
  finish()
  assert.equal(await readiness, viewport)
})

test('history that adds no turn while frames render still fails at the deadline', async () => {
  const started = performance.now()
  const readiness = committedTranscriptHistory(canvas, 14, new AbortController().signal)
  const frames = setInterval(renderFrame, 16)
  try {
    await assert.rejects(readiness, /History readiness stalled for 200ms(?: and \d+ frames)?: expected data-state="complete" and 14 turns; last data-state="materializing", mounted turns=10/)
  } finally { clearInterval(frames) }
  assert.ok(performance.now() - started >= deadline)
})

test('each added turn restarts the deadline, and a stall after it is still reported', async () => {
  const readiness = committedTranscriptHistory(canvas, 41, new AbortController().signal)
  const frames = setInterval(renderFrame, 16)
  try {
    for (let batch = 0; batch < 3; batch++) { await wait(deadline * 0.75); turns(4) }
    await assert.rejects(readiness, /stalled for 200ms(?: and \d+ frames)?: .*mounted turns=22/)
  } finally { clearInterval(frames) }
})

// The tab strip opens a keyboard-focus tooltip from a frame callback, so it cannot appear while the compositor is paused.
const tab = () => {
  document.body.insertAdjacentHTML('beforeend', '<div role="tab" id="tab" aria-label="release-checklist.md"></div>')
  return document.getElementById('tab')
}
const disclose = anchor => {
  document.body.insertAdjacentHTML('beforeend', '<div role="tooltip" id="tip">release-checklist.md</div>')
  anchor.setAttribute('aria-describedby', 'tip')
}

test('a tooltip opened by the first frame after a pause past the deadline is found', async () => {
  const anchor = tab()
  const tooltip = tooltipFor(anchor, 'release-checklist.md')
  tooltip.catch(() => {})
  // The compositor pauses: page timers run past the deadline and no frame renders.
  await wait(deadline * 2)
  // The first frame after the pause runs the tab's focus callback, which opens the tooltip.
  renderFrame()
  disclose(anchor)
  assert.equal(await tooltip, document.getElementById('tip'))
})

test('a tooltip that mounts hidden after a pause stays pending until RAC places it', async () => {
  const anchor = tab()
  let settled = false
  const tooltip = tooltipFor(anchor, 'release-checklist.md')
  tooltip.then(() => { settled = true }, () => { settled = true })
  await wait(deadline * 2)
  // RAC mounts the overlay hidden and shows it once it has measured a placement.
  renderFrame()
  disclose(anchor)
  const tip = document.getElementById('tip')
  tip.style.visibility = 'hidden'
  for (let frame = 0; frame < 3; frame++) { await wait(0); renderFrame() }
  await wait(0)
  assert.equal(settled, false)
  tip.style.visibility = ''
  assert.equal(await tooltip, tip)
})

test('a tooltip that stays hidden while frames render fails past the deadline', async () => {
  const anchor = tab()
  disclose(anchor)
  document.getElementById('tip').style.visibility = 'hidden'
  const frames = setInterval(renderFrame, 16)
  try {
    await assert.rejects(tooltipFor(anchor, 'release-checklist.md'), /toHaveAccessibleName/)
  } finally { clearInterval(frames) }
})

test('a tooltip that never opens while frames render still fails at the deadline', async () => {
  const started = performance.now()
  const tooltip = tooltipFor(tab(), 'release-checklist.md')
  const frames = setInterval(renderFrame, 16)
  try {
    await assert.rejects(tooltip, /^Error: Missing tooltip for release-checklist.md/)
  } finally { clearInterval(frames) }
  assert.ok(performance.now() - started >= deadline)
})

test('a tooltip already open is returned without waiting for a frame', async () => {
  const anchor = tab()
  disclose(anchor)
  assert.equal(await tooltipFor(anchor, 'release-checklist.md'), document.getElementById('tip'))
})
