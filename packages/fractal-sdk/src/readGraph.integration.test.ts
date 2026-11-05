import { readFileSync } from 'node:fs'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'
import { expect, it } from 'vitest'

it('isolates SDK optional services while bounding the shared client dependency', () => {
  const packageRoot = fileURLToPath(new URL('../', import.meta.url))
  const manifest = JSON.parse(readFileSync(resolve(packageRoot, 'package.json'), 'utf8'))
  const pending = [resolve(packageRoot, manifest.exports['./read'])]
  const repoRoot = resolve(packageRoot, '../..')
  const workspaceManifest = JSON.parse(readFileSync(resolve(repoRoot, 'package.json'), 'utf8'))
  const packages = workspaceManifest.workspaces.map((path: string) => ({
    root: resolve(repoRoot, path),
    manifest: JSON.parse(readFileSync(resolve(repoRoot, path, 'package.json'), 'utf8')),
  }))
  const visited = new Set<string>()
  while (pending.length > 0) {
    const path = pending.pop()!
    if (visited.has(path)) continue
    visited.add(path)
    expect(relative(packageRoot, path)).not.toMatch(/^src\/effect\/(?:pty|mutations?)(?:[./-]|$)/i)
    // Type imports, re-exports and literal dynamic imports count too.
    const imports = ts.preProcessFile(readFileSync(path, 'utf8'), true, true).importedFiles
    for (const { fileName } of imports) {
      if (fileName.startsWith('.')) pending.push(resolve(dirname(path), fileName))
      else {
        const dependency = packages.find(
          (pkg: { manifest: { name: string } }) =>
            fileName === pkg.manifest.name || fileName.startsWith(`${pkg.manifest.name}/`),
        )
        if (dependency === undefined) continue // External packages are outside the workspace boundary.
        const subpath = `.${fileName.slice(dependency.manifest.name.length)}`
        const target = dependency.manifest.exports[subpath]
        expect(typeof target, `workspace export ${fileName}`).toBe('string')
        pending.push(resolve(dependency.root, target))
      }
    }
  }
  expect(visited.size).toBeGreaterThan(1)
  // The generated client is deliberately NOT read-only: its one FractalClient class includes
  // collection reads and mutation methods. Splitting that shared generator is a separate change.
  // These are the only client source modules admitted by this SDK boundary (including type edges).
  const clientRoot = resolve(repoRoot, 'clients/typescript/fractal-client')
  expect([...visited].filter((path) => path.startsWith(`${clientRoot}/`))
    .map((path) => relative(clientRoot, path)).sort()).toEqual([
      'Client.generated.ts', 'Models.generated.ts', 'Schema.generated.ts',
      'errors.ts', 'index.ts', 'subagent-drivers.ts',
    ])
})
