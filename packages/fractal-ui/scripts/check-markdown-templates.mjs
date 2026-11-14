// Fails when plain text is spliced into Markdown unescaped. Inside `markdownBlock(…)` or `markdownInline(…)`, every
// template `${…}` and every `+` operand must say how it became Markdown: escaped plain text (`inlineFromPlain`,
// `markdownFromPlain`), a code fence (`markdownCodeFence`), or the source of a value that already is Markdown
// (`markdownSource`). Anything else lets a name such as `*draft*` or a path with `_` change the rendered text.
// A reviewed exception carries `// markdown-template-reviewed: <reason>` on the line above the call.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const MARKDOWN = new Set(['markdownBlock', 'markdownInline'])
const SAFE = new Set(['inlineFromPlain', 'markdownFromPlain', 'markdownCodeFence', 'markdownSource', 'optionalSource'])
const calleeName = node => ts.isIdentifier(node) ? node.text : ts.isPropertyAccessExpression(node) ? node.name.text : undefined

/** Every unsafe splice in one file's source, as `file:line: text`. */
export function findViolations(source, fileName = 'input.ts') {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true, fileName.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const lines = source.split('\n')
  const violations = []
  const safe = expression => {
    const inner = ts.isParenthesizedExpression(expression) ? expression.expression : expression
    if (ts.isStringLiteralLike(inner)) return true
    return ts.isCallExpression(inner) && SAFE.has(calleeName(inner.expression))
  }
  const operands = expression => ts.isBinaryExpression(expression) && expression.operatorToken.kind === ts.SyntaxKind.PlusToken
    ? [...operands(expression.left), ...operands(expression.right)] : [expression]
  const visit = node => {
    if (ts.isCallExpression(node) && MARKDOWN.has(calleeName(node.expression)) && node.arguments[0] !== undefined) {
      const argument = node.arguments[0]
      const spliced = ts.isTemplateExpression(argument) ? argument.templateSpans.map(span => span.expression)
        : ts.isBinaryExpression(argument) ? operands(argument) : []
      const line = file.getLineAndCharacterOfPosition(node.getStart(file)).line
      const reviewed = /\/\/ markdown-template-reviewed: \S/.test(lines[line - 1] ?? '')
      for (const expression of spliced) if (!safe(expression) && !reviewed) violations.push(`${fileName}:${line + 1}: ${expression.getText(file)}`)
    }
    ts.forEachChild(node, visit)
  }
  visit(file)
  return violations
}

const sources = root => readdirSync(root).flatMap(name => {
  const path = join(root, name)
  if (name === 'node_modules' || name.startsWith('.')) return []
  return statSync(path).isDirectory() ? sources(path) : /\.(?:ts|tsx|mts)$/.test(name) && !name.endsWith('.d.ts') ? [path] : []
})

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
  const roots = ['apps/fractal-web/src', 'apps/ios', 'packages/fractal-ui/src', 'clients/typescript'].map(root => join(repo, root))
  const violations = roots.flatMap(sources).flatMap(path => findViolations(readFileSync(path, 'utf8'), relative(repo, path)))
  if (violations.length > 0) {
    console.error(`Plain text spliced into Markdown without escaping:\n${violations.join('\n')}`)
    process.exit(1)
  }
  console.log(JSON.stringify({ pass: true }))
}
