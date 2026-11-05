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
    // Only eager value edges belong to the static graph: terminal demand loads its codec later.
    const source = ts.createSourceFile(path, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true)
    const imports = source.statements.flatMap((statement) => {
      if (ts.isImportDeclaration(statement)) {
        const clause = statement.importClause
        if (clause?.isTypeOnly) return []
        if (clause?.name === undefined && clause?.namedBindings !== undefined &&
          ts.isNamedImports(clause.namedBindings) && clause.namedBindings.elements.length > 0 &&
          clause.namedBindings.elements.every((element) => element.isTypeOnly)) return []
      } else if (ts.isExportDeclaration(statement)) {
        if (statement.isTypeOnly) return []
        if (statement.exportClause !== undefined && ts.isNamedExports(statement.exportClause) &&
          statement.exportClause.elements.length > 0 &&
          statement.exportClause.elements.every((element) => element.isTypeOnly)) return []
      } else return []
      return statement.moduleSpecifier !== undefined && ts.isStringLiteral(statement.moduleSpecifier)
        ? [statement.moduleSpecifier.text] : []
    })
    for (const fileName of imports) {
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
  // The transport client still includes read and mutation methods; generated schema modules
  // partition shared read codecs from optional validators without splitting that client class.
  // Only read validators and shared helpers may be constructed by the eager SDK graph.
  const clientRoot = resolve(repoRoot, 'clients/typescript/fractal-client')
  expect(visited.has(resolve(clientRoot, 'Schema.generated.ts'))).toBe(false)
  expect([...visited].filter((path) => path.startsWith(`${clientRoot}/`))
    .map((path) => relative(clientRoot, path)).sort()).toEqual([
      'Client.generated.ts', 'Models.generated.ts', 'ReadSchema.generated.ts',
      'SchemaSupport.generated.ts',
      'errors.ts', 'index.ts', 'subagent-drivers.ts',
    ])
})
