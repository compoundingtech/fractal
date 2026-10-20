import { readFileSync, realpathSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as ts from 'typescript'
import { describe, expect, it } from 'vitest'
import { scenarioDomainFixture } from '@smalltalk/st3-scenarios/domain-fixture'

/** Only runtime static edges count: loaders' dynamic imports and erased types do not. */
const staticImports = (source: string): readonly string[] => {
  const file = ts.createSourceFile('entry.ts', source, ts.ScriptTarget.Latest, true)
  return file.statements.flatMap((statement) => {
    if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) {
      const clause = statement.importClause
      if (clause?.isTypeOnly) return []
      const bindings = clause?.namedBindings
      if (clause?.name === undefined && bindings !== undefined && ts.isNamedImports(bindings)
        && bindings.elements.length > 0 && bindings.elements.every((element) => element.isTypeOnly)) return []
      return [statement.moduleSpecifier.text]
    }
    if (ts.isExportDeclaration(statement) && !statement.isTypeOnly
      && statement.moduleSpecifier !== undefined && ts.isStringLiteral(statement.moduleSpecifier)) {
      const clause = statement.exportClause
      if (clause !== undefined && ts.isNamedExports(clause) && clause.elements.length > 0
        && clause.elements.every((element) => element.isTypeOnly)) return []
      return [statement.moduleSpecifier.text]
    }
    return []
  })
}

const packageRoot = realpathSync(fileURLToPath(new URL('..', import.meta.url)))
const runtimeGraph = (entry: string): readonly string[] => {
  const visited = new Set<string>()
  const visit = (path: string): void => {
    if (visited.has(path)) return
    visited.add(path)
    for (const specifier of staticImports(readFileSync(path, 'utf8'))) {
      const resolution = ts.resolveModuleName(specifier, path, {
        module: ts.ModuleKind.ESNext,
        moduleResolution: ts.ModuleResolutionKind.Bundler,
        allowImportingTsExtensions: true,
        resolveJsonModule: true,
      }, ts.sys).resolvedModule
      if (resolution === undefined) throw new Error(`Unresolved static import ${specifier} from ${path}`)
      const target = realpathSync(resolution.resolvedFileName)
      // External packages cannot contain this package's worlds or recordings.
      if (!relative(packageRoot, target).startsWith('..')) visit(target)
    }
  }
  visit(resolve(packageRoot, entry))
  return [...visited].map((path) => relative(packageRoot, path))
}

describe('light preview entry boundaries', () => {
  it('finds runtime imports and re-exports without following types or dynamic loader edges', () => {
    expect(staticImports(`
      import type { World } from './worlds/types.ts'
      import { type Other } from './worlds/other.ts'
      export type * from './worlds/types.ts'
      export { type Third } from './worlds/third.ts'
      import './side-effect.ts'
      import { foldSlice, type Slice } from './fold.ts'
      export * from './barrel.ts'
      export { value, type Value } from './value.ts'
      const load = () => import('./worlds/index.ts')
    `)).toEqual(['./side-effect.ts', './fold.ts', './barrel.ts', './value.ts'])
  })

  it.each(['src/domain-fixture.ts', 'src/react/index.ts', 'src/storybook/index.ts'])('%s has no static catalog or recording dependency', (entry) => {
    const graph = runtimeGraph(entry)
    expect(graph).toContain(entry)
    expect(graph.filter((path) => /^(?:src\/worlds|recordings)\//.test(path))).toEqual([])
    expect(graph).not.toContain('src/index.ts')
  })

  it('exports a dependency-free fixture identity from its public light entry', () => {
    expect(typeof scenarioDomainFixture).toBe('symbol')
    expect(runtimeGraph('src/domain-fixture.ts')).toEqual(['src/domain-fixture.ts'])
  })
})
