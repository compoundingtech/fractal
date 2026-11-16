import { watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

// Playwright's pipe can be ready before Chromium publishes its separate TCP endpoint.
// Watch before reading so publication during the initial read cannot be missed.
export const waitForDevtoolsEndpoint = (profile, browser, timeoutMs = 10_000) => new Promise((resolve, reject) => {
  let watcher, deadline
  let finished = false
  let reading = false
  let changed = false
  const finish = (error, endpoint) => {
    if (finished) return
    finished = true
    watcher?.close()
    clearTimeout(deadline)
    browser.off('disconnected', disconnected)
    if (error) reject(error)
    else resolve(endpoint)
  }
  const disconnected = () => finish(new Error('Chromium disconnected before publishing its DevTools endpoint'))
  const read = async () => {
    changed = true
    if (reading || finished) return
    reading = true
    try {
      do {
        changed = false
        let contents
        try {
          contents = await readFile(join(profile, 'DevToolsActivePort'), 'utf8')
        } catch (error) {
          if (error.code !== 'ENOENT') throw error
        }
        if (finished) return
        const match = /^(\d+)\n(\/devtools\/browser\/[A-Za-z0-9-]+)\s*$/.exec(contents ?? '')
        if (match && Number(match[1]) > 0 && Number(match[1]) <= 65535) {
          finish(undefined, `ws://127.0.0.1:${match[1]}${match[2]}`)
        }
        // A change during readFile requires another read even if fs.watch coalesces events.
      } while (changed && !finished)
    } catch (error) {
      finish(error)
    } finally {
      reading = false
    }
  }
  try {
    watcher = watch(profile, () => { void read() })
    watcher.on('error', error => finish(error))
    browser.on('disconnected', disconnected)
    deadline = setTimeout(() => finish(new Error(`Chromium did not publish a complete DevTools endpoint within ${timeoutMs} ms`)), timeoutMs)
    if (!browser.isConnected()) disconnected()
    else void read()
  } catch (error) {
    finish(error)
  }
})
