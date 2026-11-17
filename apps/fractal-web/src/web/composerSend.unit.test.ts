// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import type { AttachmentPort, Grants } from '../data/source.ts'
import { composerSendBinding, imageUploadAdapter, sendFailureDetail } from './composerSend.ts'
import { ActionResult, decodeUnknownSync } from '@fractal/client/schema'
import * as React from 'react'
import { createRoot } from 'react-dom/client'
import { MessageNotSentError, type AssistantRuntime } from '@assistant-ui/react'
import { useConversationRuntime } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceRuntime.tsx'
import { createComposerDraftSession, createLocalComposerDraftStore } from './composerDrafts.ts'
import { createComposerDraftAdapter } from './composerDraftAdapter.ts'

const message = {
  role: 'user', content: [{ type: 'text', text: 'hello' }], createdAt: new Date(0),
  metadata: { custom: {} }, parentId: null, sourceId: null, runConfig: undefined,
} as const
const grants = { actions: 'granted', messageSend: 'granted', attachmentUpload: 'granted', terminalInput: 'ungranted', terminalControl: 'ungranted' } as const
const seat = 'agent/example/scratch'

describe('composer send contract', () => {
  it.each(['completed', 'accepted', 'rejected', 'failed'] as const)('clears a saved draft only on a completed send, not %s submission', async status => {
    localStorage.clear()
    const store = createLocalComposerDraftStore({ storage: localStorage, events: window })
    const adapter = createComposerDraftAdapter({ store, conversationId: seat, getHandle: () => undefined, now: () => 10 })
    adapter.onDraftChange({ text: 'hello', revision: 1, savedAt: 10, cause: 'user' })
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue(status === 'failed'
      ? { _tag: 'Refused', reason: 'failed', detail: 'Send did not complete' }
      : { _tag: 'Success', value: decodeUnknownSync(ActionResult)({
          kind: 'action-result', action_id: 'action/example', operation_id: 'operation/example',
          snapshot_id: 'snapshot/example', affected_ids: ['message/example'], status,
        }) })
    const input = {
      source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } },
      agentRef: seat, grants, readable: true, items: [], onRefused: vi.fn(),
      onSending: adapter.onSending,
    }
    const binding = composerSendBinding(input)
    adapter.onDraftChange({ text: '', revision: 2, savedAt: 11, cause: 'submit-reset' })
    await binding.runtime.onNew(message)
    expect(store.read(seat)?.text).toBe(status === 'completed' ? '' : 'hello')
    adapter.close()
    if (status === 'failed') {
      // A failed submission must not erase the browser-local draft.
      const reloaded = createComposerDraftSession({
        store: createLocalComposerDraftStore({ storage: localStorage, events: window }), conversationId: seat,
      })
      expect(reloaded.getSnapshot().text).toBe('hello')
      reloaded.close()
    }
  })

  it('never acknowledges a newly authored same-text draft when retrying an older outbox item', async () => {
    localStorage.clear()
    const store = createLocalComposerDraftStore({ storage: localStorage, events: window })
    const adapter = createComposerDraftAdapter({ store, conversationId: seat, getHandle: () => undefined, now: () => 10 })
    adapter.onDraftChange({ text: 'hello', revision: 1, savedAt: 10, cause: 'user' })
    const send = vi.fn<AttachmentPort['send']>()
      .mockResolvedValueOnce({ _tag: 'Refused', reason: 'failed', detail: 'Send did not complete' })
      .mockResolvedValueOnce({ _tag: 'Success', value: decodeUnknownSync(ActionResult)({
        kind: 'action-result', action_id: 'action/example', operation_id: 'operation/example',
        snapshot_id: 'snapshot/example', affected_ids: ['message/example'], status: 'completed',
      }) })
    const binding = composerSendBinding({
      source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } },
      agentRef: seat, grants, readable: true, items: [], onRefused: vi.fn(), onSending: adapter.onSending,
    })
    await binding.runtime.onNew(message)
    adapter.onDraftChange({ text: 'hello', revision: 2, savedAt: 11, cause: 'user' })
    const authored = store.read(seat)
    await binding.retry({
      _tag: 'Text', id: 'pending/original-outbox-key', role: 'user', text: 'hello', attachments: [],
      streaming: false, at: new Date(0).toISOString(), sendState: { _tag: 'Failed', reason: 'failed', detail: 'Send did not complete' },
    })
    expect(send.mock.calls[1]?.[0]).toMatchObject({ _tag: 'Resend', idempotencyKey: 'original-outbox-key' })
    expect(store.read(seat)).toEqual(authored)
    adapter.close()
  })

  it('sends Send then Resend with the source key and leaves the fence to the data source', async () => {
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue({ _tag: 'Refused', reason: 'failed', detail: 'Actual refusal' })
    const onRefused = vi.fn()
    const binding = composerSendBinding({ source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } }, agentRef: seat, grants, readable: true, items: [], onRefused })
    await binding.runtime.onNew(message)
    await binding.retry({ _tag: 'Text', id: 'pending/exact-source-key', role: 'user', text: 'hello', attachments: [], streaming: false, at: new Date(0).toISOString(), sendState: { _tag: 'Failed', reason: 'failed', detail: 'Actual refusal' } })
    expect(send).toHaveBeenCalledTimes(2)
    expect(send.mock.calls[0]![0]).toMatchObject({ api_version: 'st3.client.v0', _tag: 'Send', parameters: { to: seat, content: 'hello' } })
    expect(send.mock.calls[1]![0]).toMatchObject({ api_version: 'st3.client.v0', _tag: 'Resend', idempotencyKey: 'exact-source-key' })
    expect(send.mock.calls.every(([request]) => !('fence' in request))).toBe(true)
    expect(binding.runtime.onCancel).toBeUndefined()
    expect(onRefused).not.toHaveBeenCalled()
  })

  it('keeps an unconfirmed send on its outbox row without dispatching its retry callback', async () => {
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue({ _tag: 'Refused', reason: 'unconfirmed', detail: 'st could not confirm the send in time' })
    const onRefused = vi.fn()
    const binding = composerSendBinding({ source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } }, agentRef: seat, grants, readable: true, items: [], onRefused })
    await binding.runtime.onNew(message)
    // Unconfirmed belongs to the row; it never disables the next draft.
    expect(onRefused).not.toHaveBeenCalled()
    await binding.retry({ _tag: 'Text', id: 'pending/unconfirmed-key', role: 'user', text: 'hello', attachments: [], streaming: false, at: new Date(0).toISOString(), sendState: { _tag: 'Unconfirmed', detail: 'st could not confirm the send in time' } })
    expect(send).toHaveBeenCalledTimes(1)
    expect(sendFailureDetail('unconfirmed')).toBe('Fractal may have taken this message and has not confirmed it yet. Do not send it again.')
  })

  it('disables absent ports and actual refusals instead of inventing a capability or fence', async () => {
    const onRefused = vi.fn()
    const binding = composerSendBinding({ source: {}, agentRef: seat, grants, readable: true, items: [], onRefused })
    expect(binding.disabledReason).toBe('This view cannot send messages.')
    expect(binding.runtime.isDisabled).toBe(true)
    await binding.runtime.onNew(message)
    expect(onRefused).not.toHaveBeenCalled()
    const refused = composerSendBinding({ source: {}, agentRef: seat, grants, readable: true, items: [], onRefused, refusal: { reason: 'ungranted' } })
    expect(refused.disabledReason).toBe('Message send is not granted for this device.')
    expect(refused.runtime.isDisabled).toBe(true)
  })

  it('keeps the composer but waits to send while the conversation cannot be read, under the send grant model', async () => {
    const send = vi.fn<AttachmentPort['send']>()
    const attachments = { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send }
    const unreadable = composerSendBinding({ source: { attachments }, agentRef: seat, grants, readable: false, items: [], onRefused: vi.fn() })
    expect(unreadable.disabledReason).toBe('Messages can be sent once this conversation loads.')
    expect(unreadable.runtime.isDisabled).toBe(true)
    await unreadable.runtime.onNew(message)
    expect(send).not.toHaveBeenCalled()
    // The send grant is the more fundamental refusal and keeps its own reason.
    const ungranted = composerSendBinding({ source: { attachments }, agentRef: seat, grants: { ...grants, messageSend: 'ungranted' }, readable: false, items: [], onRefused: vi.fn() })
    expect(ungranted.disabledReason).toBe('Message send is not granted for this device.')
  })

  it('waits for the capabilities answer instead of calling unread grants a refusal', async () => {
    const send = vi.fn<AttachmentPort['send']>()
    const attachments = { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send }
    const unread = composerSendBinding({ source: { attachments }, agentRef: seat, grants: { ...grants, messageSend: 'ungranted', unread: true }, readable: true, items: [], onRefused: vi.fn() })
    expect(unread.disabledReason).toBe('Checking whether this browser can send messages…')
    expect(unread.runtime.isDisabled).toBe(true)
    await unread.runtime.onNew(message)
    expect(send).not.toHaveBeenCalled()
  })

  it('stages local images without uploads and passes their original bytes alongside existing blob references', async () => {
    const blob = `blob/${'a'.repeat(64)}`
    const upload = vi.fn<AttachmentPort['upload']>()
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue({ _tag: 'Refused', reason: 'failed', detail: 'Actual refusal' })
    const adapter = imageUploadAdapter()
    await expect(adapter.add({ file: new File(['<svg/>'], 'vector.svg', { type: 'image/svg+xml' }) })).rejects.toThrow('Only PNG, JPEG, GIF and WebP images can be attached.')
    const file = new File([new Uint8Array([1, 2, 3])], 'screen.png', { type: 'image/png' })
    const pending = await adapter.add({ file })
    expect(upload).not.toHaveBeenCalled()
    const complete = await adapter.send(pending)
    expect(complete.file).toBe(file)
    expect(complete.content).toEqual([])
    expect(upload).not.toHaveBeenCalled()
    const binding = composerSendBinding({ source: { attachments: { capabilities: vi.fn(), upload, chunk: vi.fn(), send } }, agentRef: seat, grants, readable: true, items: [], onRefused: vi.fn(), attachmentAdapter: adapter })
    expect(binding.runtime.adapters?.attachments).toBe(adapter)
    expect(binding.imageAttachments).toEqual({ max: 4 })
    await binding.runtime.onNew({ ...message, attachments: [complete, { ...complete, id: 'existing', content: [{ type: 'file', data: blob, mimeType: 'image/png', sourceType: 'id' }] }] })
    expect(send.mock.calls[0]![0]).toMatchObject({
      _tag: 'Send', images: [{ bytes: file, mediaType: 'image/png', name: 'screen.png' }],
      parameters: { content: 'hello', attachments: [{ blob, media_type: 'image/png', name: 'screen.png' }] },
    })
    expect(send.mock.calls[0]![0].images?.[0]?.bytes).toBe(file)
    expect(upload).not.toHaveBeenCalled()
  })

  it('rejects storage refusal with the runtime not-sent contract and safe actionable copy', async () => {
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue({ _tag: 'Refused', reason: 'storage', detail: 'raw-diagnostic-sentinel' })
    const onRefused = vi.fn()
    const options = { source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } }, agentRef: seat, grants, readable: true, items: [], onRefused }
    const binding = composerSendBinding(options)
    await expect(binding.runtime.onNew(message)).rejects.toBeInstanceOf(MessageNotSentError)
    expect(onRefused).toHaveBeenCalledWith({ reason: 'storage' })
    const refused = composerSendBinding({ ...options, refusal: { reason: 'storage' } })
    expect(refused.disabledReason).toBe(sendFailureDetail('storage'))
    expect(refused.disabledReason).toContain('Nothing was sent.')
    expect(refused.disabledReason).toContain('Free browser storage and try again.')
    expect(refused.disabledReason).not.toContain('raw-diagnostic-sentinel')
  })

  it('restores an identity-unavailable submission with visible retry-later copy', async () => {
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue({ _tag: 'Refused', reason: 'identity-unavailable', detail: 'raw-diagnostic-sentinel' })
    const onRefused = vi.fn()
    const options = { source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } }, agentRef: seat, grants, readable: true, items: [], onRefused }
    await expect(composerSendBinding(options).runtime.onNew(message)).rejects.toBeInstanceOf(MessageNotSentError)
    expect(onRefused).toHaveBeenCalledWith({ reason: 'identity-unavailable' })
    expect(sendFailureDetail('identity-unavailable')).toContain('Identity unavailable, retry later.')
    expect(sendFailureDetail('identity-unavailable')).toContain('draft and images have been restored')
  })

  it('restores the submitted draft and original image through the actual assistant-ui runtime on storage refusal', async () => {
    const send = vi.fn<AttachmentPort['send']>().mockResolvedValue({ _tag: 'Refused', reason: 'storage', detail: 'Storage unavailable' })
    const binding = composerSendBinding({
      source: { attachments: { capabilities: vi.fn(), upload: vi.fn(), chunk: vi.fn(), send } },
      agentRef: seat, grants, readable: true, items: [], onRefused: vi.fn(), attachmentAdapter: imageUploadAdapter(),
    })
    let runtime: AssistantRuntime | undefined
    const Probe = () => { runtime = useConversationRuntime(binding.runtime); return null }
    const host = document.createElement('div')
    const root = createRoot(host)
    const file = new File(['image'], 'retained.png', { type: 'image/png' })
    try {
      await React.act(async () => { root.render(React.createElement(Probe)) })
      const composer = runtime!.thread.composer
      await React.act(async () => {
        composer.setText('keep this draft')
        await composer.addAttachment(file)
        composer.send()
      })
      await vi.waitFor(() => {
        expect(send).toHaveBeenCalledTimes(1)
        expect(composer.getState().text).toBe('keep this draft')
        expect(composer.getState().attachments).toHaveLength(1)
      })
      expect(composer.getState().attachments[0]?.file).toBe(file)
      expect(send.mock.calls[0]?.[0].images?.[0]?.bytes).toBe(file)
    } finally {
      await React.act(async () => root.unmount())
    }
  })

  it('does not start an upload when composer send is already canceled', async () => {
    const upload = vi.fn<AttachmentPort['upload']>()
    const adapter = imageUploadAdapter()
    const pending = await adapter.add({ file: new File(['image'], 'screen.png', { type: 'image/png' }) })
    const controller = new AbortController()
    controller.abort()
    await expect(adapter.send(pending, { signal: controller.signal })).rejects.toBe(controller.signal.reason)
    expect(upload).not.toHaveBeenCalled()
  })

  it('revalidates image format at finalization without network I/O', async () => {
    const adapter = imageUploadAdapter()
    const pending = await adapter.add({ file: new File(['image'], 'screen.png', { type: 'image/png' }) })
    await expect(adapter.send({ ...pending, contentType: 'image/svg+xml' })).rejects.toThrow('Only PNG, JPEG, GIF and WebP images can be attached.')
  })

  it('refuses images over 10 MiB before staging, including drafts staged before validation', async () => {
    const upload = vi.fn<AttachmentPort['upload']>()
    const adapter = imageUploadAdapter()
    const oversized = new File([new Uint8Array(12 * 1024 * 1024)], 'large.png', { type: 'image/png' })
    await expect(adapter.add({ file: oversized })).rejects.toThrow('Images must be 10 MiB or smaller. Choose a smaller image and try again.')
    const pending = await adapter.add({ file: new File(['image'], 'small.png', { type: 'image/png' }) })
    await expect(adapter.send({ ...pending, file: oversized })).rejects.toThrow('Images must be 10 MiB or smaller.')
    expect(upload).not.toHaveBeenCalled()
    // The server's exact limit remains accepted; only larger images are refused.
    await expect(adapter.add({ file: new File([new Uint8Array(10 * 1024 * 1024)], 'limit.png', { type: 'image/png' }) })).resolves.toMatchObject({ name: 'limit.png' })
  })

  it('retains attachment presentation but offers uploads only while both grants are present', async () => {
    const attachments = { capabilities: vi.fn(), upload: vi.fn<AttachmentPort['upload']>(), chunk: vi.fn(), send: vi.fn<AttachmentPort['send']>() }
    const adapter = imageUploadAdapter()
    const bind = (overrides: Partial<Grants>, readable = true) => composerSendBinding({ source: { attachments }, agentRef: seat, grants: { ...grants, ...overrides }, readable, items: [], onRefused: vi.fn(), attachmentAdapter: adapter })
    const pending = await adapter.add({ file: new File(['image'], 'staged.png', { type: 'image/png' }) })
    // The upload grant names a permission loss; other losses are temporary. The composer appends the way out.
    for (const [binding, cause] of [[bind({ attachmentUpload: 'ungranted' }), 'You can no longer send images here.'], [bind({ messageSend: 'ungranted' }), "Images can't be sent right now."], [bind({}, false), "Images can't be sent right now."]] as const) {
      const removalOnly = binding.runtime.adapters!.attachments!
      await expect(removalOnly.remove(pending)).resolves.toBeUndefined()
      await expect(removalOnly.send(pending)).rejects.toThrow(cause)
      expect(attachments.upload).not.toHaveBeenCalled()
      expect(binding.imageAttachments).toEqual({ max: 4, disabledReason: cause })
    }
    expect(bind({}).runtime.adapters?.attachments).toBe(adapter)
    // Losing upload permission must not prevent a recovered text-only draft from sending.
    attachments.send.mockResolvedValue({ _tag: 'Refused', reason: 'failed', detail: 'Unavailable' })
    const noUpload = bind({ attachmentUpload: 'ungranted' })
    await noUpload.runtime.onNew(message)
    expect(attachments.send).toHaveBeenCalledWith(expect.objectContaining({ parameters: expect.objectContaining({ attachments: [] }) }))
  })

  it.each(['future-reason', 'constructor', '__proto__', 'raw-diagnostic-sentinel'])('uses generic copy for unclassified reason %s', reason => {
    expect(sendFailureDetail(reason)).toBe('The message could not be sent. Check your connection and retry.')
    const binding = composerSendBinding({ source: {}, agentRef: seat, grants, readable: true, items: [], onRefused: vi.fn(), refusal: { reason } })
    expect(binding.disabledReason).toBe('The message could not be sent. Check your connection and retry.')
  })
})
