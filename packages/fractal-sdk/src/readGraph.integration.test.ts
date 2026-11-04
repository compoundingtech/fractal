import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { expect, it } from 'vitest'

it('keeps the read subpath import graph free of PTY and mutations', () => {
  const packageRoot = fileURLToPath(new URL('../', import.meta.url))
  const manifest = JSON.parse(readFileSync(resolve(packageRoot, 'package.json'), 'utf8'))
  const pending = [resolve(packageRoot, manifest.exports['./read'])]
  const visited = new Set<string>()
  while (pending.length > 0) {
    const path = pending.pop()!
    if (visited.has(path)) continue
    visited.add(path)
    expect(path).not.toMatch(/\/(?:pty|mutations?)(?:[./-]|$)/i)
    // Type imports, re-exports and literal dynamic imports count too.
    const imports = ts.preProcessFile(readFileSync(path, 'utf8'), true, true).importedFiles
    for (const { fileName } of imports) {
      if (fileName.startsWith('.')) pending.push(resolve(dirname(path), fileName))
    }
  }
  expect(visited.size).toBeGreaterThan(1)
})
