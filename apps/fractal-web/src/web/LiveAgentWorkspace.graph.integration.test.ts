import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, extname, join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as ts from 'typescript'
import { describe, expect, it } from 'vitest'
import { storyGlobs } from '../../../../packages/fractal-ui/.storybook/stories.ts'

const web = dirname(fileURLToPath(import.meta.url))
const kit = resolve(web, '../../../../packages/fractal-ui')
const kitPackage: { readonly exports: Readonly<Record<string, string>> } = JSON.parse(readFileSync(resolve(kit, 'package.json'), 'utf8'))

/** Eager mode preserves the startup proof. Coverage mode additionally traverses
 * literal import() edges without evaluating modules or starting liveRuntime. */
const eagerGraph = (entry: string, includeLazy = false) => {
  const files = new Set<string>()
  const dependencies = new Set<string>()
  const visit = (file: string) => {
    if (files.has(file)) return
    files.add(file)
    const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true)
    const specifiers: string[] = []
    for (const statement of source.statements) {
      let specifier: string | undefined
      if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) {
        const clause = statement.importClause
        if (clause?.isTypeOnly) continue
        if (clause !== undefined && clause.name === undefined && clause.namedBindings !== undefined && ts.isNamedImports(clause.namedBindings) && clause.namedBindings.elements.every(element => element.isTypeOnly)) continue
        specifier = statement.moduleSpecifier.text
      } else if (ts.isExportDeclaration(statement) && statement.moduleSpecifier !== undefined && ts.isStringLiteral(statement.moduleSpecifier)) {
        if (statement.isTypeOnly) continue
        if (statement.exportClause !== undefined && ts.isNamedExports(statement.exportClause) && statement.exportClause.elements.every(element => element.isTypeOnly)) continue
        specifier = statement.moduleSpecifier.text
      }
      if (specifier !== undefined) specifiers.push(specifier)
    }
    if (includeLazy) {
      const findLazy = (node: ts.Node): void => {
        if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword &&
            node.arguments[0] !== undefined && ts.isStringLiteral(node.arguments[0]))
          specifiers.push(node.arguments[0].text)
        ts.forEachChild(node, findLazy)
      }
      findLazy(source)
    }
    for (const specifier of specifiers) {
      dependencies.add(specifier)
      // Lazily loaded modules import emitted assets (`?url` fonts, JSON word lists), not TS surfaces.
      // Keep the established eager traversal semantics unchanged.
      if (includeLazy && (specifier.includes('?') || /\.(?:json|woff2?|ttf|wasm)$/.test(specifier))) continue
      let local: string | undefined
      if (specifier.startsWith('.')) local = resolve(dirname(file), specifier)
      else if (specifier === '@fractal/ui' || specifier.startsWith('@fractal/ui/')) {
        const key = specifier === '@fractal/ui' ? '.' : `.${specifier.slice('@fractal/ui'.length)}`
        const target = kitPackage.exports[key]
        if (target === undefined) throw new Error(`Missing kit export ${specifier}`)
        local = resolve(kit, target)
      }
      if (local === undefined || ['.css', '.svg'].includes(extname(local))) continue
      const target = [local, `${local}.ts`, `${local}.tsx`, resolve(local, 'index.ts'), resolve(local, 'index.tsx')]
        .find(candidate => /\.(?:ts|tsx|mts)$/.test(candidate) && existsSync(candidate))
      if (target === undefined) throw new Error(`Unresolved runtime edge ${specifier} from ${file}`)
      visit(target)
    }
  }
  visit(entry)
  return { files: [...files], dependencies: [...dependencies] }
}

describe('conversation eager graph boundary', () => {
  it('keeps ConversationPane, the kit Transcript/Markdown and refractor outside the production shell graph', () => {
    const graph = eagerGraph(resolve(web, 'main.tsx'))
    // Positive witnesses prevent an empty or incorrectly resolved graph from passing.
    expect(graph.files).toContain(resolve(web, 'LiveAgentWorkspace.tsx'))
    expect(graph.files).toContain(resolve(kit, 'src/assistant-ui/shell.ts'))
    expect(graph.files.some(file => /\/(?:ConversationPane|Transcript|Markdown)\.tsx$/.test(file))).toBe(false)
    expect(graph.dependencies.some(specifier => /^(?:refractor|react-markdown)(?:\/|$)/.test(specifier))).toBe(false)
    expect(graph.dependencies).not.toContain('@assistant-ui/react')
    expect(graph.files).toContain(resolve(web, 'ConversationPaneFallback.tsx'))
    expect(graph.files).not.toContain(resolve(web, 'ChangesInspector.tsx'))
    expect(graph.files).not.toContain(resolve(web, 'capturedChanges.ts'))
    expect(graph.files).not.toContain(resolve(kit, 'src/assistant-ui/composition/DiffPanel.tsx'))
  })

  it('still reaches the real heavy composition through the on-demand pane', () => {
    const graph = eagerGraph(resolve(web, 'ConversationPane.tsx'))
    expect(graph.files).toContain(resolve(kit, 'src/assistant-ui/composition/Transcript.tsx'))
    expect(graph.files).toContain(resolve(kit, 'src/assistant-ui/composition/Markdown.tsx'))
    expect(graph.files).not.toContain(resolve(kit, 'src/assistant-ui/index.ts'))
    expect(graph.files).not.toContain(resolve(kit, 'src/assistant-ui/workbench/Workbench.tsx'))
    expect(graph.files).not.toContain(resolve(kit, 'src/assistant-ui/composition/DiffPanel.tsx'))
    expect(graph.dependencies.some(specifier => /^refractor(?:\/|$)/.test(specifier))).toBe(true)
  })

  it('reaches the real diff viewer only through the on-demand Changes inspector', () => {
    const graph = eagerGraph(resolve(web, 'ChangesInspector.tsx'))
    expect(graph.files).toContain(resolve(web, 'capturedChanges.ts'))
    expect(graph.files).toContain(resolve(kit, 'src/assistant-ui/composition/DiffPanel.tsx'))
    expect(graph.files).toContain(resolve(kit, 'src/assistant-ui/composition/Markdown.tsx'))
    expect(graph.dependencies.some(specifier => /^refractor(?:\/|$)/.test(specifier))).toBe(true)
    expect(graph.files).not.toContain(resolve(web, 'ConversationPane.tsx'))
    expect(graph.files).not.toContain(resolve(kit, 'src/assistant-ui/index.ts'))
    expect(graph.files).not.toContain(resolve(kit, 'src/assistant-ui/workbench/Workbench.tsx'))
  })
})

/** A story titled `Fractal/App/<Module>` is a production module suite: its meta.component names that module. */
const moduleSuiteTitle = /^Fractal\/App\/([A-Z][A-Za-z0-9]*)$/

/** Resolve a module suite's component identity from imports/meta and require an actual JSX mount.
 * A local clone resolves to the story itself; React.lazy resolves its literal import target,
 * never to a misleading local alias. Other `Fractal/App/` titles are not module suites. */
const storySurface = (file: string) => {
  const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const imports = new Map<string, string>()
  const variables = new Map<string, ts.Expression>()
  const mounted = new Set<string>()
  let title: string | undefined
  let component: ts.Expression | undefined
  const collect = (node: ts.Node): void => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) && !node.importClause?.isTypeOnly) {
      const target = node.moduleSpecifier.text
      const clause = node.importClause
      if (clause?.name !== undefined) imports.set(clause.name.text, target)
      if (clause?.namedBindings !== undefined && ts.isNamedImports(clause.namedBindings))
        for (const binding of clause.namedBindings.elements)
          if (!binding.isTypeOnly) imports.set(binding.name.text, target)
    }
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer !== undefined)
      variables.set(node.name.text, node.initializer)
    // Only the meta object's own `component`, never a nested docs description of the same name.
    if (ts.isObjectLiteralExpression(node)) {
      const members = new Map(node.properties.flatMap(member =>
        ts.isPropertyAssignment(member) && ts.isIdentifier(member.name) ? [[member.name.text, member.initializer] as const] : []))
      const value = members.get('title')
      if (value !== undefined && ts.isStringLiteral(value) && moduleSuiteTitle.test(value.text)) {
        title = value.text
        component = members.get('component')
      }
    }
    if ((ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) && ts.isIdentifier(node.tagName))
      mounted.add(node.tagName.text)
    ts.forEachChild(node, collect)
  }
  collect(source)
  if (title === undefined) return undefined
  if (component === undefined || !ts.isIdentifier(component) || !mounted.has(component.text))
    throw new Error(`${title}: meta.component must identify a mounted component`)
  const identity = (name: string, seen = new Set<string>()): string => {
    if (seen.has(name)) throw new Error(`${title}: cyclic component alias ${name}`)
    seen.add(name)
    const imported = imports.get(name)
    if (imported !== undefined) {
      if (!imported.startsWith('.')) throw new Error(`${title}: App component must resolve to a production module`)
      return resolve(dirname(file), imported)
    }
    const initializer = variables.get(name)
    if (initializer !== undefined && ts.isIdentifier(initializer)) return identity(initializer.text, seen)
    if (initializer !== undefined && ts.isCallExpression(initializer) &&
        ts.isPropertyAccessExpression(initializer.expression) && initializer.expression.name.text === 'lazy') {
      const targets: string[] = []
      const findImport = (node: ts.Node): void => {
        if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword &&
            node.arguments[0] !== undefined && ts.isStringLiteral(node.arguments[0]))
          targets.push(resolve(dirname(file), node.arguments[0].text))
        ts.forEachChild(node, findImport)
      }
      findImport(initializer)
      if (targets.length !== 1) throw new Error(`${title}: lazy component needs exactly one literal module target`)
      return targets[0]!
    }
    return file
  }
  const module = identity(component.text)
  if (basename(module, extname(module)) !== moduleSuiteTitle.exec(title)?.[1])
    throw new Error(`${title}: meta.component resolves to another module: ${module}`)
  return { title, module }
}

/** Bidirectional module coverage: every module suite mounts a module of the lazy-inclusive
 * production graph, and every required production surface has a configured module suite.
 * The production graph proves on-demand panes are reachable without evaluating them. */
const assertAppStoryCoverage = ({ entry, stories, required }: {
  readonly entry: string
  readonly stories: readonly string[]
  readonly required: readonly string[]
}) => {
  const production = new Set(eagerGraph(entry, true).files)
  for (const file of required)
    if (!production.has(file)) throw new Error(`Required App surface is not production-mounted: ${file}`)
  const storied = new Set<string>()
  for (const file of stories) {
    const surface = storySurface(file)
    if (surface === undefined) continue
    if (!production.has(surface.module))
      throw new Error(`Storied App surface is not production-mounted: ${surface.title} -> ${surface.module}`)
    storied.add(surface.module)
  }
  for (const file of required)
    if (!storied.has(file)) throw new Error(`Production App surface has no canonical story: ${file}`)
}

describe('lazy-inclusive canonical App story identity', () => {
  it('covers the required production surfaces in both directions without changing the eager proof', () => {
    // Kit Storybook loads app stories only by explicit file entries; those are the deployed suites.
    const app = resolve(web, '../..')
    const stories = storyGlobs.map(glob => resolve(kit, '.storybook', glob)).filter(file => file.startsWith(`${app}${sep}`))
    expect(stories.filter(file => /[*?[\]{}]/.test(file)), 'App story entries must name files').toEqual([])
    const required = [resolve(web, 'LiveAgentWorkspace.tsx'), resolve(web, 'ConversationPane.tsx')]
    assertAppStoryCoverage({ entry: resolve(web, 'main.tsx'), stories, required })
    expect(stories).toContain(resolve(web, 'LiveAgentWorkspace.stories.tsx'))
    expect(stories).toContain(resolve(web, 'ConversationPane.stories.tsx'))
  })

  it.each(['local', 'lazy', 'other'] as const)('rejects a planted %s ConversationPane identity', mode => {
    const dir = mkdtempSync(join(tmpdir(), 'fractal-app-identity-'))
    try {
      const pane = join(dir, 'ConversationPane.tsx')
      const entry = join(dir, 'App.tsx')
      const story = join(dir, 'ConversationPane.stories.tsx')
      writeFileSync(pane, 'export const ConversationPane = () => <div data-testid="transcript-scroll" />')
      writeFileSync(join(dir, 'Other.tsx'), 'export const Other = () => <div />')
      writeFileSync(entry, "import { ConversationPane } from './ConversationPane.tsx'; import { Other } from './Other.tsx'; export const App = () => <><ConversationPane /><Other /></>")
      const canonical = "import { ConversationPane } from './ConversationPane.tsx'; const meta = { title: 'Fractal/App/ConversationPane', component: ConversationPane, parameters: { docs: { description: { component: 'Pane' } } }, render: () => <ConversationPane /> }; export default meta"
      writeFileSync(story, canonical)
      const args = { entry, stories: [story], required: [pane] }
      expect(() => assertAppStoryCoverage(args)).not.toThrow()
      if (mode === 'local') {
        writeFileSync(story, canonical.replace("import { ConversationPane } from './ConversationPane.tsx';", 'const ConversationPane = () => <div data-testid="transcript-scroll" />;'))
        expect(() => assertAppStoryCoverage(args)).toThrow('meta.component resolves to another module')
      } else if (mode === 'lazy') {
        writeFileSync(join(dir, 'ConversationPaneCopy.tsx'), 'export const ConversationPane = () => <div data-testid="transcript-scroll" />')
        writeFileSync(story, canonical.replace("import { ConversationPane } from './ConversationPane.tsx';", "import * as React from 'react'; const ConversationPane = React.lazy(() => import('./ConversationPaneCopy.tsx').then(module => ({ default: module.ConversationPane })));"))
        expect(() => assertAppStoryCoverage(args)).toThrow('meta.component resolves to another module')
        // A copy under the canonical module name is still outside the production graph.
        writeFileSync(story, canonical.replace("import { ConversationPane } from './ConversationPane.tsx';", "import * as React from 'react'; const ConversationPane = React.lazy(() => import('./copy/ConversationPane.tsx'));"))
        expect(() => assertAppStoryCoverage(args)).toThrow(`Storied App surface is not production-mounted: Fractal/App/ConversationPane -> ${join(dir, 'copy', 'ConversationPane.tsx')}`)
      } else {
        writeFileSync(story, canonical.replace("import { ConversationPane } from './ConversationPane.tsx';", "import { Other as ConversationPane } from './Other.tsx';"))
        expect(() => assertAppStoryCoverage(args)).toThrow('meta.component resolves to another module')
      }
      writeFileSync(story, canonical)
      expect(() => assertAppStoryCoverage({ ...args, stories: [] })).toThrow('Production App surface has no canonical story')
    } finally { rmSync(dir, { recursive: true, force: true }) }
  })
})
