import * as path from 'node:path'
import ts from 'typescript'

export type ExemptStoryMode = 'primitive' | 'contract-fixture'
export interface RuntimeImport {
  readonly specifier: string
  readonly kind: 'static' | 'dynamic' | 'require'
  readonly position: 'scenario-loader-runtime' | 'other'
}
export interface RuntimeModule {
  readonly imports: readonly RuntimeImport[]
  readonly marked: boolean
}
export interface RuntimeSourceGraph {
  readonly read: (file: string) => RuntimeModule
  readonly resolve: (specifier: string, importer: string) => string | undefined
  readonly isExternal: (file: string) => boolean
}
export interface FixtureGraphPolicy {
  readonly kitSource: string
  readonly uiSource: string
  readonly projectionHarness: string
  /** Known inline data owners supplement filename detection; inline primitive data remains review-only. */
  readonly contractDataModules?: readonly string[]
}

const normalized = (file: string): string => file.replaceAll('\\', '/')
const within = (file: string, directory: string): boolean => file.startsWith(`${directory}/`)
const fixtureNamed = (file: string): boolean => /fixture/i.test(path.basename(file))
const assetBasePath = (file: string): string => file.split('?')[0]!

/** Only domain roots carry markers. Local contract-data modules remain usable by contracts. */
export const requiresDomainMarker = (file: string, policy: FixtureGraphPolicy): boolean => {
  if (file === policy.projectionHarness || file === `${policy.kitSource}/index.ts`) return true
  const kitRoot = path.dirname(policy.kitSource)
  if (!within(file, kitRoot)) return false
  const relative = file.slice(kitRoot.length + 1)
  return relative.startsWith('src/worlds/') || relative.split('/').slice(0, -1).some(part => /^(?:recordings?|fixtures?)$/.test(part))
}

export interface FixtureGraphResult {
  readonly violations: readonly string[]
  readonly contractData: ReadonlyMap<string, readonly string[]>
  readonly visited: number
}

/** One traversal per CSF entry/mode; diagnostics retain the complete shortest import path. */
export const checkFixtureGraph = (
  entries: ReadonlyMap<string, ReadonlySet<ExemptStoryMode>>,
  graph: RuntimeSourceGraph,
  policy: FixtureGraphPolicy,
): FixtureGraphResult => {
  // The public Storybook entry defers its scenario loader until a slices declaration.
  // Exempt stories never invoke scenarioLoaders. Only its exact dynamic import
  // initializer is exempt; static/eager imports of the same resolved path still count.
  const allowedLoaderEdges: Readonly<Record<string, readonly string[]>> = {
    [`${policy.kitSource}/storybook/index.ts`]: [`${policy.kitSource}/storybook/runtime.ts`],
  }
  const violations: string[] = []
  const contractData = new Map<string, string[]>()
  for (const file of policy.contractDataModules ?? []) contractData.set(file, [])
  const allVisited = new Set<string>()
  for (const [entry, modes] of entries) {
    const primitive = modes.has('primitive')
    const pending: string[][] = [[entry]]
    const visited = new Set<string>()
    for (let index = 0; index < pending.length; index++) {
      const chain = pending[index]!
      const file = chain.at(-1)!
      if (visited.has(file)) continue
      visited.add(file)
      allVisited.add(file)
      const basePath = assetBasePath(file)
      const domainRoot = requiresDomainMarker(basePath, policy)
      let module: RuntimeModule
      try {
        module = domainRoot ? { imports: [], marked: false } : graph.read(file)
      } catch (cause) {
        violations.push(`${chain.join(' -> ')} (unreadable runtime module: ${String(cause)})`)
        continue
      }
      const forbidden = domainRoot || module.marked || (primitive && fixtureNamed(basePath))
      if (forbidden) {
        violations.push(`${primitive ? 'primitive' : 'contract-fixture'}: ${chain.join(' -> ')} (forbidden domain/fixture module)`)
        continue
      }
      if (within(file, policy.uiSource) && (fixtureNamed(file) || contractData.has(file))) {
        const importers = contractData.get(file) ?? []
        importers.push(chain.join(' -> '))
        contractData.set(file, importers)
      }
      for (const edge of module.imports) {
        const target = graph.resolve(edge.specifier, file)
        const allowedLoader = edge.kind === 'dynamic' && edge.position === 'scenario-loader-runtime'
          && target !== undefined && allowedLoaderEdges[file]?.includes(target)
        if (target === undefined) {
          violations.push(`${chain.join(' -> ')} -> ${edge.specifier} (unresolved runtime import)`)
        } else if (!allowedLoader && !graph.isExternal(target)) {
          pending.push([...chain, target])
        }
      }
    }
  }
  return { violations, contractData, visited: allVisited.size }
}

/** Match only `const runtime = await import(...)` directly in the exported loader array's async arrow. */
const isScenarioLoaderRuntimeImport = (node: ts.CallExpression): boolean => {
  const awaited = node.parent
  if (!ts.isAwaitExpression(awaited)) return false
  const variable = awaited.parent
  if (!ts.isVariableDeclaration(variable) || variable.initializer !== awaited
    || !ts.isIdentifier(variable.name) || variable.name.text !== 'runtime') return false
  const declarations = variable.parent
  if (!ts.isVariableDeclarationList(declarations) || declarations.declarations.length !== 1
    || (declarations.flags & ts.NodeFlags.Const) === 0) return false
  const statement = declarations.parent
  if (!ts.isVariableStatement(statement)) return false
  const block = statement.parent
  if (!ts.isBlock(block)) return false
  const arrow = block.parent
  if (!ts.isArrowFunction(arrow) || arrow.body !== block
    || !arrow.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.AsyncKeyword)) return false
  const array = arrow.parent
  if (!ts.isArrayLiteralExpression(array) || array.elements.length !== 1) return false
  const loader = array.parent
  if (!ts.isVariableDeclaration(loader) || loader.initializer !== array
    || !ts.isIdentifier(loader.name) || loader.name.text !== 'scenarioLoaders') return false
  const loaderDeclarations = loader.parent
  if (!ts.isVariableDeclarationList(loaderDeclarations)) return false
  const exported = loaderDeclarations.parent
  return ts.isVariableStatement(exported) && ts.isSourceFile(exported.parent)
    && (exported.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.ExportKeyword) ?? false)
}

/** Inspect emitted imports: ordinary imports used only as types disappear exactly as in TS. */
const emittedImports = (javascript: string): readonly RuntimeImport[] => {
  const source = ts.createSourceFile('runtime.js', javascript, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS)
  const imports: RuntimeImport[] = []
  const visit = (node: ts.Node): void => {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier !== undefined && ts.isStringLiteral(node.moduleSpecifier)) {
      imports.push({ specifier: node.moduleSpecifier.text, kind: 'static', position: 'other' })
    }
    if (ts.isCallExpression(node)
      && (node.expression.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(node.expression) && node.expression.text === 'require'))) {
      const argument = node.arguments[0]
      imports.push({
        specifier: argument !== undefined && ts.isStringLiteral(argument) ? argument.text : '<non-literal runtime import>',
        kind: node.expression.kind === ts.SyntaxKind.ImportKeyword ? 'dynamic' : 'require',
        position: isScenarioLoaderRuntimeImport(node) ? 'scenario-loader-runtime' : 'other',
      })
    }
    ts.forEachChild(node, visit)
  }
  visit(source)
  return imports
}

export interface TypeScriptGraph extends RuntimeSourceGraph {
  readonly hasMarker: (file: string) => boolean
  readonly hasTypedMarker: (file: string) => boolean
}

export const createTypeScriptGraph = (rootNames: readonly string[], options: ts.CompilerOptions, injectedSources?: Readonly<Record<string, string>>): TypeScriptGraph => {
  const compilerOptions: ts.CompilerOptions = {
    ...options, noEmit: false, noEmitOnError: false, declaration: false, declarationMap: false,
    emitDeclarationOnly: false, sourceMap: false, inlineSourceMap: false,
    module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.Preserve,
  }
  const host = ts.createCompilerHost(compilerOptions)
  if (injectedSources !== undefined) {
    const originalReadFile = host.readFile
    const originalFileExists = host.fileExists
    host.readFile = file => injectedSources[file] ?? originalReadFile(file)
    host.fileExists = file => file in injectedSources || originalFileExists(file)
    host.directoryExists = directory => Object.keys(injectedSources).some(file => file.startsWith(`${directory}/`)) || ts.sys.directoryExists(directory)
    const originalGetSourceFile = host.getSourceFile
    host.getSourceFile = (file, languageVersion, onError, shouldCreateNewSourceFile) => {
      const text = injectedSources[file]
      return text === undefined
        ? originalGetSourceFile(file, languageVersion, onError, shouldCreateNewSourceFile)
        : ts.createSourceFile(file, text, languageVersion, true)
    }
  }
  // Vite's documented string/URL asset queries; worker and unknown queries
  // remain code edges or unresolved, never silently become asset leaves.
  const resolveAsset = (specifier: string, importer: string): string | undefined => {
    const query = specifier.slice(specifier.indexOf('?'))
    if (!specifier.startsWith('.') || !/^\?(?:raw|url|inline|no-inline)$/.test(query)) return undefined
    const base = normalized(path.resolve(path.dirname(importer), assetBasePath(specifier)))
    return host.fileExists(base) ? base : undefined
  }
  const resolutionCache = ts.createModuleResolutionCache(host.getCurrentDirectory(), file => file, compilerOptions)
  const repository = normalized(path.resolve(import.meta.dirname, '../../..'))
  host.resolveModuleNameLiterals = (literals, importer, redirectedReference, options, source) => literals.map(literal => {
    const result = ts.resolveModuleName(literal.text, importer, options, host, resolutionCache,
      redirectedReference, ts.getModeForUsageLocation(source, literal, options))
    const resolved = result.resolvedModule
    if (resolved === undefined) return result
    const file = normalized(host.realpath?.(resolved.resolvedFileName) ?? resolved.resolvedFileName)
    // Package aliases and pnpm symlinks can label repository source as external.
    // TS then skips program.emit for it, although the runtime graph must follow it.
    return within(file, repository) && !file.includes('/node_modules/')
      ? { ...result, resolvedModule: { ...resolved, resolvedFileName: file, isExternalLibraryImport: false } }
      : result
  })
  const program = ts.createProgram([...rootNames], compilerOptions, host)
  const checker = program.getTypeChecker()
  const cache = new Map<string, RuntimeModule>()
  const hasMarker = (file: string, owner: ts.Program = program): boolean => {
    const source = owner.getSourceFile(file)
    if (source === undefined) throw new Error(`Missing graph source: ${file}`)
    const ownerChecker = owner.getTypeChecker()
    const symbol = ownerChecker.getSymbolAtLocation(source)
    return symbol !== undefined && ownerChecker.getExportsOfModule(symbol).some(exported => exported.name === 'scenarioDomainFixtureMarker')
  }
  return {
    hasMarker,
    hasTypedMarker: file => {
      const source = program.getSourceFile(file)
      if (source === undefined) return false
      const symbol = checker.getSymbolAtLocation(source)
      const marker = symbol === undefined ? undefined : checker.getExportsOfModule(symbol).find(exported => exported.name === 'scenarioDomainFixtureMarker')
      if (marker === undefined) return false
      const type = checker.getTypeOfSymbolAtLocation(marker, source)
      return (type.flags & ts.TypeFlags.UniqueESSymbol) !== 0
        && type.symbol?.name === 'scenarioDomainFixture'
        && (type.symbol.declarations?.some(declaration => normalized(declaration.getSourceFile().fileName).endsWith('/domain-fixture.ts')) ?? false)
    },
    read: file => {
      if (file.includes('?')) {
        const base = assetBasePath(file)
        if (!/\.[cm]?[jt]sx?$/.test(base)) return { imports: [], marked: false }
        // Raw module text is not executable, but its exported marker still
        // makes it fixture data, even when TS never resolved the query import.
        const owner = program.getSourceFile(base) === undefined ? ts.createProgram([base], compilerOptions, host) : program
        return { imports: [], marked: hasMarker(base, owner) }
      }
      const previous = cache.get(file)
      if (previous !== undefined) return previous
      if (!/\.[cm]?[jt]sx?$/.test(file)) return { imports: [], marked: false }
      const source = program.getSourceFile(file)
      if (source === undefined) throw new Error(`Unresolved graph source: ${file}`)
      let javascript: string | undefined
      program.emit(source, (name, text) => { if (/\.[cm]?jsx?$/.test(name)) javascript = text })
      if (javascript === undefined) throw new Error(`No runtime emission for graph source: ${file}`)
      const module = { imports: emittedImports(javascript), marked: hasMarker(file) }
      cache.set(file, module)
      return module
    },
    resolve: (specifier, importer) => {
      if (specifier.startsWith('node:')) return `external:${specifier}`
      if (specifier.includes('?')) {
        const asset = resolveAsset(specifier, importer)
        return asset === undefined ? undefined : `${asset}${specifier.slice(specifier.indexOf('?'))}`
      }
      // Non-code assets are runtime leaves, not source graph escape hatches.
      if (/\.(?:css|svg|png|jpe?g|woff2?|json|jsonl)(?:\?.*)?$/.test(specifier) && specifier.startsWith('.')) {
        const asset = normalized(path.resolve(path.dirname(importer), specifier.split('?')[0]!))
        return host.fileExists(asset) ? asset : undefined
      }
      const resolved = ts.resolveModuleName(specifier, importer, compilerOptions, host, resolutionCache).resolvedModule
      if (resolved === undefined) return undefined
      return normalized(ts.sys.realpath?.(resolved.resolvedFileName) ?? resolved.resolvedFileName)
    },
    isExternal: file => file.startsWith('external:') || file.includes('/node_modules/') || file.endsWith('.d.ts'),
  }
}
