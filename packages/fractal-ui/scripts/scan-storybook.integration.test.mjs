import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, rm, symlink } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import { scanBooks } from './scan-storybook.mjs'

test('published path/host/private-policy mutations fail without hash or binary exclusions', async () => {
  const root = await mkdtemp(join(tmpdir(), 'storybook-privacy-'))
  try {
    const file = join(root, 'assets', 'iframe-c0ffee.js')
    await mkdir(join(root, 'assets'))
    await writeFile(file, 'public fixture example.test')
    assert.equal((await scanBooks([root])).findings.length, 0)
    for (const value of ['/ho' + 'me/operator/checkout/main.ts', '/Us' + 'ers/operator/project', '~/' + '.megarepo/project', '/workspace/checkouts/project', 'de' + 'v3', 'host.tail' + '123.ts.net', String.raw`\u002fho` + 'me/operator', 'Actual Operator', Buffer.from([0, ...Buffer.from('/ho' + 'me/operator'), 255])]) {
      await writeFile(file, value)
      assert.equal((await scanBooks([root], ['Actual Operator'])).findings.length, 1)
    }
    await writeFile(file, '/ho' + 'me/operator/checkout/main.ts')
    const planted = spawnSync(process.execPath, [new URL('./scan-storybook.mjs', import.meta.url).pathname, root], { encoding: 'utf8' })
    assert.equal(planted.status, 1, 'planted checkout path must reject publication')
    assert.ok(!planted.stderr.includes('operator'), 'never disclose matched bytes')
    await writeFile(file, 'public')
    const embedded = join(root, 'apps', 'fractal-web', 'storybook-static')
    await mkdir(embedded, { recursive: true })
    await writeFile(join(embedded, 'metadata.json'), 'host.tail' + '123.ts.net')
    assert.equal((await scanBooks([root])).findings.length, 1, 'embedded app must be scanned')
    await symlink(file, join(root, 'linked.js'))
    assert.ok((await scanBooks([root])).findings.some(finding => finding.kind === 'non-file'))
    await assert.rejects(scanBooks([root], []))
  } finally { await rm(root, { recursive: true, force: true }) }
})
