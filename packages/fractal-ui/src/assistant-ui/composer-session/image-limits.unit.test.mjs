import assert from 'node:assert/strict'
import { after, test } from 'node:test'
import { createServer } from 'vite'

const server = await createServer({ configFile: false, root: new URL('../../../', import.meta.url).pathname, optimizeDeps: { noDiscovery: true, include: [] }, server: { middlewareMode: true } })
after(() => server.close())
const { attachmentAddErrorNotice, composerImageLimits, composerImageRejection, composerImageRejectionMessage, formatImageBytes } = await server.ssrLoadModule('/src/assistant-ui/composer-session/image-limits.ts')
const MB = 1024 * 1024
const png = (name, size = 1) => ({ name, type: 'image/png', size })
const notice = (file, staged) => {
  const rejection = composerImageRejection(file, staged)
  return rejection === undefined ? undefined : composerImageRejectionMessage(rejection)
}

test('limits are four PNG, JPEG, GIF or WebP images of at most 10 MB', () => {
  assert.deepEqual(composerImageLimits, { maxCount: 4, maxBytes: 10 * MB, types: ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] })
})
test('an image inside every limit has no notice', () => {
  for (const type of composerImageLimits.types) assert.equal(notice({ name: 'ok', type, size: 10 * MB }, 3), undefined)
})
test('a fifth image names the file and the four-image limit', () => {
  assert.deepEqual(composerImageRejection(png('photo-5.png'), 4), { _tag: 'Count', name: 'photo-5.png' })
  assert.equal(notice(png('photo-5.png'), 4), 'photo-5.png wasn\'t added. A message can include up to 4 images.')
})
test('one byte over the limit names the file and the 10 MB limit', () => {
  assert.deepEqual(composerImageRejection(png('photo.png', 10 * MB + 1), 0), { _tag: 'Size', name: 'photo.png', bytes: 10 * MB + 1 })
  assert.equal(notice(png('photo.png', 10 * MB + 1), 0), 'photo.png is over the 10 MB image limit.')
})
test('an unsupported type names the file and the accepted formats', () => {
  assert.equal(notice({ name: 'diagram.svg', type: 'image/svg+xml', size: 1 }, 0), 'diagram.svg isn\'t a supported image. Use PNG, JPEG, GIF or WebP.')
})
test('the first broken limit wins: type, then size, then count', () => {
  assert.equal(composerImageRejection({ name: 'x.pdf', type: 'application/pdf', size: 20 * MB }, 4)._tag, 'Type')
  assert.equal(composerImageRejection(png('x.png', 20 * MB), 4)._tag, 'Size')
})
test('custom limits flow into the copy', () => {
  const limits = { maxCount: 2, maxBytes: 2.5 * MB, types: ['image/png', 'image/svg+xml'] }
  assert.equal(composerImageRejectionMessage(composerImageRejection(png('a.png'), 2, limits), limits), 'a.png wasn\'t added. A message can include up to 2 images.')
  assert.equal(composerImageRejectionMessage(composerImageRejection(png('a.png', 3 * MB), 0, limits), limits), 'a.png is over the 2.5 MB image limit.')
  assert.equal(composerImageRejectionMessage(composerImageRejection({ name: 'a.gif', type: 'image/gif', size: 1 }, 0, limits), limits), 'a.gif isn\'t a supported image. Use PNG or SVG.')
  assert.equal(formatImageBytes(10 * MB), '10 MB')
})
test('runtime refusals map to announced copy', () => {
  const accept = composerImageLimits.types.join(',')
  assert.equal(attachmentAddErrorNotice({ reason: 'adapter-error', message: 'photo.png is over the 10 MB image limit.' }, accept), 'photo.png is over the 10 MB image limit.')
  assert.equal(attachmentAddErrorNotice({ reason: 'not-accepted', message: 'File type application/pdf is not accepted.', contentType: 'application/pdf' }, accept), 'application/pdf files can\'t be attached. Use PNG, JPEG, GIF or WebP.')
  assert.equal(attachmentAddErrorNotice({ reason: 'not-accepted', message: 'File type unknown is not accepted.', contentType: '' }, accept), 'This file type can\'t be attached. Use PNG, JPEG, GIF or WebP.')
  assert.equal(attachmentAddErrorNotice({ reason: 'not-accepted', message: '', contentType: 'image/svg+xml' }, 'image/*'), 'SVG files can\'t be attached. Use any image.')
  assert.equal(attachmentAddErrorNotice({ reason: 'no-adapter', message: 'Attachments are not supported' }, '*'), 'Images can\'t be attached here.')
})
