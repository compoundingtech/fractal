// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ClientError, type FractalClient } from '@fractal/client'
import type { BrowserPairingPort, BrowserPairingStatusReply } from '../data/source.ts'

vi.mock('@stylexjs/stylex', () => import('../testStylex.ts'))

import { gatewayAttachments } from '../data/attachmentPort.ts'
import { browserPairingPort, PairingHttpError } from '../data/browserPairing.ts'
import { BrowserPairing } from './BrowserPairing.tsx'
import { browserDeviceLabel, createPairingStore, pairingSendRefusal } from './browserPairingState.ts'
import { createComposerDraftAdapter } from './composerDraftAdapter.ts'
import { createLocalComposerDraftStore } from './composerDrafts.ts'
import { composerSendBinding } from './composerSend.ts'

const pollMs = 100
const fakePort = (initial: BrowserPairingStatusReply) => {
  const replies: BrowserPairingStatusReply[] = []
  const port = {
    status: vi.fn(async (): Promise<BrowserPairingStatusReply> => replies.length > 0 ? replies.shift() : initial),
    start: vi.fn(async (_label: string) => ({ requestId: 'pair-request/1', displayCode: 'K7Q-4MX' })),
    forget: vi.fn(async () => {}),
  } satisfies BrowserPairingPort
  return { port, next: (reply: BrowserPairingStatusReply) => { replies.push(reply) } }
}

let root: Root | undefined
let container: HTMLDivElement | undefined
// Only timers are faked: the poll is a timeout, and React's act still drains real microtasks.
beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
})
afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = undefined
  container = undefined
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

const mount = async (port: BrowserPairingPort, messageSend: 'granted' | 'ungranted' = 'ungranted') => {
  const credentialChanged = vi.fn()
  const store = createPairingStore({ port, deviceLabel: () => 'Firefox on Linux', credentialChanged, pollMs })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  await act(async () => root!.render(<BrowserPairing store={store} messageSend={messageSend} />))
  await flush()
  return { credentialChanged, store }
}
const notice = () => container!.querySelector('[data-testid="browser-pairing"]')
const text = () => notice()?.querySelector('[role="status"]')?.textContent ?? ''
const press = async (label: string) => {
  const button = [...container!.querySelectorAll('button')].find(each => each.textContent === label)
  if (button === undefined) throw new Error(`No ${label} button in: ${container!.textContent}`)
  await act(async () => button.click())
  await flush()
}
/** Let settled port replies reach the store without firing the next poll. */
const flush = () => act(async () => { await vi.advanceTimersByTimeAsync(0) })
const poll = async () => {
  await act(async () => { await vi.advanceTimersByTimeAsync(pollMs) })
  await flush()
}

describe('BrowserPairing', () => {
  it('renders nothing on a host without browser pairing, so the composer stays as it was', async () => {
    const { port } = fakePort(undefined)
    const { store } = await mount(port)
    expect(container!.innerHTML).toBe('')
    await poll()
    await act(() => store.sendRefused())
    expect(container!.innerHTML).toBe('')
    expect(port.status).toHaveBeenCalledTimes(1)
  })

  it('asks an unpaired browser that cannot send, and stays out of the way when sending is already granted', async () => {
    const ungranted = fakePort({ status: 'unpaired' })
    await mount(ungranted.port)
    expect(text()).toContain('This browser can read but not send.')
    expect(container!.querySelector('button')?.textContent).toBe('Enable sending on this browser')
    act(() => root!.unmount())
    container!.remove()

    const granted = fakePort({ status: 'unpaired' })
    await mount(granted.port, 'granted')
    expect(notice()).toBeNull()
  })

  it.each(['paired', 'unpaired'] as const)('reloads when the first status changes the credential to %s', async status => {
    const { port } = fakePort({ status, credentialChanged: true })
    const { store, credentialChanged } = await mount(port)
    expect(store.snapshot()).toEqual({ _tag: 'Reloading' })
    expect(credentialChanged).toHaveBeenCalledOnce()
  })

  it('does not reload for an already valid browser credential', async () => {
    const { port } = fakePort({ status: 'paired' })
    const { store, credentialChanged } = await mount(port)
    expect(store.snapshot()).toEqual({ _tag: 'Paired' })
    expect(credentialChanged).not.toHaveBeenCalled()
  })

  it('shows the code to approve, polls while pending and reloads once approved', async () => {
    const { port, next } = fakePort({ status: 'unpaired' })
    const { credentialChanged } = await mount(port)
    await press('Enable sending on this browser')
    expect(port.start).toHaveBeenCalledWith('Firefox on Linux')
    expect(text()).toContain('Approve in Fractal')
    expect(container!.querySelector('[data-testid="pairing-code"]')?.textContent).toBe('K7Q-4MX')
    expect(text()).toContain('the browser you are using')
    expect(text()).toContain('Browser labels and login names are not proof')

    next({ status: 'pending', displayCode: 'K7Q-4MX' })
    await poll()
    expect(port.status).toHaveBeenCalledTimes(2)
    expect(credentialChanged).not.toHaveBeenCalled()

    next({ status: 'paired' })
    await poll()
    expect(credentialChanged).toHaveBeenCalledTimes(1)
    expect(notice()?.getAttribute('data-pairing-state')).toBe('Reloading')
    await poll()
    expect(port.status).toHaveBeenCalledTimes(3)
  })

  it('asks a paired browser to pair again once Fractal refuses its send, keeping the draft', async () => {
    const seat = 'agent/example/scratch'
    localStorage.clear()
    const drafts = createLocalComposerDraftStore({ storage: localStorage, events: window })
    const draft = createComposerDraftAdapter({ store: drafts, conversationId: seat, getHandle: () => undefined, now: () => 10 })
    draft.onDraftChange({ text: 'hello', revision: 1, savedAt: 10, cause: 'user' })
    const { port, next } = fakePort({ status: 'paired' })
    // Grants read at load still say the browser can send; its credential no longer can.
    const { store } = await mount(port, 'granted')
    expect(text()).toBe('This browser is paired.')
    const forbidden = new ClientError({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/x', code: 'forbidden', message: 'refused', retryable: false, details: {} }, 403)
    const client = { capabilities: vi.fn(async () => { throw forbidden }) } as unknown as FractalClient
    const setRefusal = vi.fn()
    const binding = composerSendBinding({
      source: { attachments: gatewayAttachments(client, async () => ({ _tag: 'Success', value: 'snapshot/example' })) },
      agentRef: seat,
      grants: { actions: 'granted', messageSend: 'granted', attachmentUpload: 'granted', terminalInput: 'ungranted', terminalControl: 'ungranted' },
      readable: true, items: [], onRefused: pairingSendRefusal(store, setRefusal), onSending: draft.onSending,
    })
    // The server drops the cookie it no longer accepts and answers unpaired.
    next({ status: 'unpaired' })
    draft.onDraftChange({ text: '', revision: 2, savedAt: 11, cause: 'submit-reset' })
    await act(async () => binding.runtime.onNew({
      role: 'user', content: [{ type: 'text', text: 'hello' }], createdAt: new Date(0),
      metadata: { custom: {} }, parentId: null, sourceId: null, runConfig: undefined,
    }))
    await flush()
    expect(setRefusal).toHaveBeenCalledWith({ reason: 'ungranted' })
    expect(port.status).toHaveBeenCalledTimes(2)
    expect(text()).toContain('Fractal did not accept sending from this browser.')
    expect(drafts.read(seat)?.text).toBe('hello')
    await press('Enable sending on this browser')
    expect(container!.querySelector('[data-testid="pairing-code"]')?.textContent).toBe('K7Q-4MX')
    draft.close()
  })

  it('names a refusal and lets the person request a new code', async () => {
    const { port, next } = fakePort({ status: 'unpaired' })
    const { credentialChanged } = await mount(port)
    await press('Enable sending on this browser')
    next({ status: 'denied' })
    await poll()
    expect(text()).toContain('Fractal declined this browser.')
    expect(credentialChanged).not.toHaveBeenCalled()
    await press('Try again')
    expect(port.start).toHaveBeenCalledTimes(2)
    expect(container!.querySelector('[data-testid="pairing-code"]')?.textContent).toBe('K7Q-4MX')
  })

  it('names an expired code, including one restored from an earlier visit', async () => {
    const { port, next } = fakePort({ status: 'pending', displayCode: 'ABC-123' })
    await mount(port)
    expect(container!.querySelector('[data-testid="pairing-code"]')?.textContent).toBe('ABC-123')
    next({ status: 'expired' })
    await poll()
    expect(text()).toContain('The pairing code expired before it was approved.')
    expect(container!.querySelector('button')?.textContent).toBe('Try again')
  })

  it('keeps polling through a missed status read', async () => {
    const { port, next } = fakePort({ status: 'pending', displayCode: 'ABC-123' })
    await mount(port)
    port.status.mockRejectedValueOnce(new TypeError('Failed to fetch'))
    await poll()
    expect(notice()?.getAttribute('data-pairing-state')).toBe('Pending')
    next({ status: 'denied' })
    await poll()
    expect(text()).toContain('Fractal declined this browser.')
  })

  it('reports a failed request with a retry', async () => {
    const { port } = fakePort({ status: 'unpaired' })
    port.start.mockRejectedValueOnce(new PairingHttpError(503))
    await mount(port)
    await press('Enable sending on this browser')
    expect(text()).toBe('Could not request a pairing code (HTTP 503). Try again.')
    await press('Enable sending on this browser')
    expect(container!.querySelector('[data-testid="pairing-code"]')?.textContent).toBe('K7Q-4MX')
  })

  it('forgets a paired browser and reloads without its credential', async () => {
    const { port } = fakePort({ status: 'paired' })
    port.forget.mockRejectedValueOnce(new TypeError('Failed to fetch'))
    const { credentialChanged } = await mount(port, 'granted')
    await press('Forget this browser')
    expect(text()).toBe('Could not reach Fractal to forget this browser. Check the connection and try again.')
    expect(credentialChanged).not.toHaveBeenCalled()
    await press('Forget this browser')
    expect(port.forget).toHaveBeenCalledTimes(2)
    expect(credentialChanged).toHaveBeenCalledTimes(1)
  })

  it('stops polling when the last composer unmounts', async () => {
    const { port } = fakePort({ status: 'pending', displayCode: 'ABC-123' })
    await mount(port)
    act(() => root!.unmount())
    root = undefined
    await poll()
    expect(port.status).toHaveBeenCalledTimes(1)
  })
})

describe('browserPairingPort', () => {
  const recorded = (respond: (url: URL) => Response) => {
    const calls: { readonly url: URL; readonly init: RequestInit }[] = []
    const port = browserPairingPort({ baseUrl: 'https://fractal.example', fetch: async (url, init) => { calls.push({ url, init }); return respond(url) } })
    return { port, calls }
  }

  it('reads status from the serving host, and reports a host without pairing as undefined', async () => {
    const paired = recorded(() => Response.json({ status: 'pending', displayCode: 'K7Q-4MX' }))
    expect(await paired.port.status()).toEqual({ status: 'pending', displayCode: 'K7Q-4MX' })
    expect(paired.calls[0]!.url.href).toBe('https://fractal.example/_fractal/pair/status')
    expect(paired.calls[0]!.init).toMatchObject({ method: 'GET', credentials: 'same-origin', cache: 'no-store' })
    expect(paired.calls[1]!.url.pathname).toBe('/_fractal/pair/redeem')
    expect(paired.calls[1]!.init).toMatchObject({ method: 'POST', credentials: 'same-origin', body: '{}' })
    expect(await recorded(() => new Response('', { status: 404 })).port.status()).toBeUndefined()
    await expect(recorded(() => new Response('', { status: 500 })).port.status()).rejects.toBeInstanceOf(PairingHttpError)
  })

  it.each([
    [{ status: 'pending', displayCode: 'K7Q-4MX' }, { status: 'paired', credentialChanged: true }],
    [{ status: 'unpaired', credentialRejected: true }, { status: 'unpaired', credentialChanged: true }],
  ])('adopts or clears credentials by POST after reading %j', async (status, redeemed) => {
    const { port, calls } = recorded(url => Response.json(url.pathname.endsWith('/status') ? status : redeemed))
    expect(await port.status()).toEqual(redeemed)
    expect(calls.map(call => [call.url.pathname, call.init.method])).toEqual([
      ['/_fractal/pair/status', 'GET'], ['/_fractal/pair/redeem', 'POST'],
    ])
  })

  it.each(['paired', 'unpaired'])('does not POST for an ordinary %s status', async status => {
    const { port, calls } = recorded(() => Response.json({ status }))
    expect(await port.status()).toEqual({ status })
    expect(calls).toHaveLength(1)
  })

  it('starts with the device label and forgets with a POST', async () => {
    const { port, calls } = recorded(url => url.pathname.endsWith('/start')
      ? Response.json({ requestId: 'pair-request/1', displayCode: 'K7Q-4MX', expiresAt: '2030-01-01T00:00:00Z' })
      : Response.json({ status: 'unpaired' }))
    expect(await port.start('Safari on iOS')).toEqual({ requestId: 'pair-request/1', displayCode: 'K7Q-4MX' })
    expect(calls[0]!.url.pathname).toBe('/_fractal/pair/start')
    expect(calls[0]!.init).toMatchObject({ method: 'POST', body: JSON.stringify({ deviceLabel: 'Safari on iOS' }) })
    await port.forget()
    expect(calls[1]!.url.pathname).toBe('/_fractal/pair/forget')
    expect(calls[1]!.init.method).toBe('POST')
  })
})

describe('browserDeviceLabel', () => {
  it.each([
    ['Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0', 'Firefox on Linux'],
    ['Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15', 'Safari on macOS'],
    ['Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1', 'Safari on iOS'],
    ['Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36', 'Chrome on Android'],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0', 'Edge on Windows'],
    ['curl/8.0', 'Web browser'],
  ])('labels %s', (userAgent, label) => {
    expect(browserDeviceLabel(userAgent)).toBe(label)
  })
})
