import { readdirSync, readFileSync } from 'node:fs'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as ts from 'typescript'
import { describe, expect, it } from 'vitest'

const web = dirname(fileURLToPath(import.meta.url))
const app = resolve(web, '..')
const kit = resolve(web, '../../../../packages/fractal-ui/src')
const root = resolve(web, '../../../..')

/** Agent- or user-written fields that must reach the screen through `InlineMarkdown` / `Markdown`, and accessible names through `toPlainText`. */
const surfaces: Readonly<Record<string, readonly string[]>> = {
  [resolve(kit, 'assistant-ui/questions/QuestionForm.tsx')]: ['title', 'request.agentLabel', 'header', 'question.header', 'question.prompt', 'option.label', 'option.description', 'preview.label', 'note'],
  [resolve(kit, 'assistant-ui/EmbraceToolCall.tsx')]: ['question', 'option', 'target'],
  [resolve(kit, 'assistant-ui/composition/Transcript.tsx')]: ['summary', 'text', 'call.summary'],
  [resolve(kit, 'assistant-ui/taste/WorkLogV1.tsx')]: ['call.summary'],
  [resolve(kit, 'assistant-ui/composition/ErrorOverlay.tsx')]: ['detail'],
  [resolve(kit, 'assistant-ui/sidebar/SidebarAgentRow.tsx')]: ['item.description', 'row.description'],
  [resolve(app, 'conversation/todos/AgentTodos.tsx')]: ['task.content', 'task.blocker.value'],
  [resolve(app, 'shell/Sidebar.tsx')]: ['item.title', 'view.mission.title', 'subjects.get(view.mission.id).title', 'subject.title'],
}
/** Attributes that become tooltips or accessible names. */
const NAME_ATTRIBUTES: Record<string, true> = { title: true, 'aria-label': true, 'aria-description': true, textValue: true }
/** Field reads that reach a JSX child or a name attribute directly: through `??`, `||`, `+`, the rendered side of `&&`, conditionals, templates and casts, but not through any call. */
function rawFieldReads(file: string, source: string, fields: readonly string[]): string[] {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const found: string[] = []
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
    const violations = Object.entries(surfaces).flatMap(([file, fields]) => {
      const source = readFileSync(file, 'utf8')
      // Positive witness: an unadopted file cannot pass by an empty field match.
      expect(source, relative(root, file)).toMatch(/\bInlineMarkdown\b/)
      return rawFieldReads(file, source, fields)
    })
    expect(violations).toEqual([])
  })

  it('flags a raw field render and accepts the adopted forms', () => {
    const fields = ['question.prompt', 'item.title']
    expect(rawFieldReads(resolve(root, 'raw.tsx'), 'const a = <span title={`${item.title} · x`}>{question.prompt ?? "-"}</span>', fields)).toEqual(['raw.tsx:1 title item.title', 'raw.tsx:1 child question.prompt'])
    expect(rawFieldReads(resolve(root, 'ok.tsx'), 'const a = <span title={toPlainText(item.title)}><InlineMarkdown text={question.prompt} /></span>', fields)).toEqual([])
  })

  it('keeps react-markdown, remark and rehype imports inside composition/Markdown.tsx', () => {
    const markdown = resolve(kit, 'assistant-ui/composition/Markdown.tsx')
    const importers = [...sourceFiles(kit), ...sourceFiles(app)].filter(file => ts.preProcessFile(readFileSync(file, 'utf8'), true, true).importedFiles
      .some(({ fileName }) => /^(?:react-markdown|remark-|rehype-|micromark|marked$|markdown-it)/.test(fileName)))
    expect(importers).toEqual([markdown])
  })
})
