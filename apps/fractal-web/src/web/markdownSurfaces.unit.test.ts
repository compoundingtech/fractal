import { readdirSync, readFileSync } from 'node:fs'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as ts from 'typescript'
import { describe, expect, it } from 'vitest'

const web = dirname(fileURLToPath(import.meta.url))
const app = resolve(web, '..')
const kit = resolve(web, '../../../../packages/fractal-ui/src')
const root = resolve(web, '../../../..')

interface Surface {
  /** Agent- or user-written fields that must reach the screen through `InlineMarkdown` / `Markdown`, and accessible names through `toPlainText`. */
  readonly fields: readonly string[]
  /** Positive witness: the renderer the file adopts, so an unadopted file cannot pass by an empty field match. */
  readonly renderer: RegExp
}
const inline = /<InlineMarkdown\b/
const surfaces: Readonly<Record<string, Surface>> = {
  [resolve(kit, 'assistant-ui/questions/QuestionForm.tsx')]: { fields: ['title', 'request.agentLabel', 'header', 'question.header', 'question.prompt', 'option.label', 'option.description', 'preview.label', 'note'], renderer: inline },
  // `intent` is the agent prose; the verbatim path, pattern, query or command target stays raw by design.
  [resolve(kit, 'assistant-ui/EmbraceToolCall.tsx')]: { fields: ['question', 'option', 'intent'], renderer: inline },
  [resolve(kit, 'assistant-ui/composition/Transcript.tsx')]: { fields: ['summary', 'text', 'call.summary', 'call.title'], renderer: inline },
  [resolve(kit, 'assistant-ui/taste/WorkLogV1.tsx')]: { fields: ['call.summary', 'call.title'], renderer: inline },
  [resolve(kit, 'assistant-ui/composition/ErrorOverlay.tsx')]: { fields: ['detail'], renderer: inline },
  [resolve(kit, 'assistant-ui/sidebar/SidebarAgentRow.tsx')]: { fields: ['item.description', 'row.description'], renderer: inline },
  [resolve(app, 'conversation/todos/AgentTodos.tsx')]: { fields: ['task.content', 'task.blocker.value'], renderer: inline },
  [resolve(app, 'shell/Sidebar.tsx')]: { fields: ['item.title', 'view.mission.title', 'subjects.get(view.mission.id).title', 'subject.title'], renderer: inline },
  // Production work-log titles are agent-authored intent (conversationTranscript.ts toolSummary).
  [resolve(app, 'web/LiveAgentWorkspace.tsx')]: { fields: ['openedTool.title'], renderer: inline },
  [resolve(app, 'extensions/SourceBlocks.tsx')]: { fields: ['source'], renderer: /<KitMarkdown text=\{source\}/ },
}
/** Attributes that become tooltips or accessible names. */
const NAME_ATTRIBUTES: Record<string, true> = { title: true, 'aria-label': true, 'aria-description': true, textValue: true }
/**
 * Field reads that reach a JSX child or a name attribute: through `??`, `||`, `+`, the rendered side of `&&`, conditionals, templates, casts
 * and local `const` bindings (a derived `label`), but not through any call.
 */
function rawFieldReads(file: string, source: string, fields: readonly string[]): string[] {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const found: string[] = []
  const bindings: Record<string, ts.Expression[]> = {}
  const collect = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer !== undefined) (bindings[node.name.text] ??= []).push(node.initializer)
    ts.forEachChild(node, collect)
  }
  collect(tree)
  const followed: Record<string, true> = {}
  const leaves = (node: ts.Node, context: string) => {
    if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node)) return leaves(node.expression, context)
    if (ts.isConditionalExpression(node)) return [node.whenTrue, node.whenFalse].forEach(branch => leaves(branch, context))
    // `a && b` renders only `b`; comparisons render booleans, never the field.
    if (ts.isBinaryExpression(node)) {
      const operator = node.operatorToken.kind
      if (operator === ts.SyntaxKind.AmpersandAmpersandToken) return leaves(node.right, context)
      if (operator === ts.SyntaxKind.QuestionQuestionToken || operator === ts.SyntaxKind.BarBarToken || operator === ts.SyntaxKind.PlusToken) return [node.left, node.right].forEach(side => leaves(side, context))
      return
    }
    if (ts.isTemplateExpression(node)) return node.templateSpans.forEach(span => leaves(span.expression, context))
    if (ts.isIdentifier(node) || ts.isPropertyAccessExpression(node)) {
      const path = node.getText(tree).replace(/\?\./g, '.').replace(/!/g, '')
      if (fields.includes(path)) found.push(`${relative(root, file)}:${tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1} ${context} ${path}`)
      else if (ts.isIdentifier(node) && bindings[node.text] !== undefined && followed[`${context} ${node.text}`] !== true) {
        followed[`${context} ${node.text}`] = true
        bindings[node.text]!.forEach(initializer => leaves(initializer, `${context} via ${node.text}`))
      }
    }
  }
  const visit = (node: ts.Node) => {
    if (ts.isJsxExpression(node) && node.expression !== undefined && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) leaves(node.expression, 'child')
    if (ts.isJsxAttribute(node) && NAME_ATTRIBUTES[node.name.getText(tree)] === true && node.initializer !== undefined && ts.isJsxExpression(node.initializer) && node.initializer.expression !== undefined) leaves(node.initializer.expression, node.name.getText(tree))
    ts.forEachChild(node, visit)
  }
  visit(tree)
  return found
}

const sourceFiles = (directory: string): string[] => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
  const path = resolve(directory, entry.name)
  if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : sourceFiles(path)
  return /\.(?:ts|tsx|mts|mjs)$/.test(entry.name) ? [path] : []
})

describe('one canonical Markdown renderer', () => {
  it('renders every listed free-text field through InlineMarkdown, Markdown or toPlainText', () => {
    const violations = Object.entries(surfaces).flatMap(([file, { fields, renderer }]) => {
      const source = readFileSync(file, 'utf8')
      expect(source, relative(root, file)).toMatch(renderer)
      return rawFieldReads(file, source, fields)
    })
    expect(violations).toEqual([])
  })

  it('flags a raw field render and accepts the adopted forms', () => {
    const fields = ['question.prompt', 'item.title']
    expect(rawFieldReads(resolve(root, 'raw.tsx'), 'const a = <span title={`${item.title} · x`}>{question.prompt ?? "-"}</span>', fields)).toEqual(['raw.tsx:1 title item.title', 'raw.tsx:1 child question.prompt'])
    expect(rawFieldReads(resolve(root, 'ok.tsx'), 'const a = <span title={toPlainText(item.title)}><InlineMarkdown text={question.prompt} /></span>', fields)).toEqual([])
  })

  it('rejects the pre-adoption forms of the work log, tool detail and source fallback', () => {
    const workLog = surfaces[resolve(kit, 'assistant-ui/taste/WorkLogV1.tsx')]!
    const revertedRow = [
      'const label = call.argsSummary === undefined ? call.title : `${call.title} ${call.argsSummary}`',
      'const row = <span title={label}>{call.kind === "run" ? <code>{label}</code> : <>{call.title}</>}</span>',
      'const tip = <Tooltip><div>{label}</div></Tooltip>',
    ].join('\n')
    expect(rawFieldReads(resolve(root, 'WorkLogV1.tsx'), revertedRow, workLog.fields)).toEqual([
      'WorkLogV1.tsx:1 title via label call.title', 'WorkLogV1.tsx:1 title via label call.title',
      'WorkLogV1.tsx:1 child via label call.title', 'WorkLogV1.tsx:1 child via label call.title',
      'WorkLogV1.tsx:2 child call.title',
    ])
    const header = surfaces[resolve(app, 'web/LiveAgentWorkspace.tsx')]!
    expect(rawFieldReads(resolve(root, 'LiveAgentWorkspace.tsx'), "const h = <strong>{openedTool.title}{openedTool.argsSummary === undefined ? '' : ` ${openedTool.argsSummary}`}</strong>", header.fields))
      .toEqual(['LiveAgentWorkspace.tsx:1 child openedTool.title'])
    const fallback = surfaces[resolve(app, 'extensions/SourceBlocks.tsx')]!
    const revertedFallback = 'function SourceMarkdown({ source }: { source: string }) { return <pre>{source}</pre> }'
    expect(rawFieldReads(resolve(root, 'SourceBlocks.tsx'), revertedFallback, fallback.fields)).toEqual(['SourceBlocks.tsx:1 child source'])
    expect(revertedFallback).not.toMatch(fallback.renderer)
    expect('const row = <span><InlineMarkdown text={call.title} /></span>').toMatch(workLog.renderer)
  })

  it('keeps react-markdown, remark and rehype imports inside composition/Markdown.tsx', () => {
    const markdown = resolve(kit, 'assistant-ui/composition/Markdown.tsx')
    const importers = [...sourceFiles(kit), ...sourceFiles(app)].filter(file => ts.preProcessFile(readFileSync(file, 'utf8'), true, true).importedFiles
      .some(({ fileName }) => /^(?:react-markdown|remark-|rehype-|micromark|marked$|markdown-it)/.test(fileName)))
    expect(importers).toEqual([markdown])
  })
})
