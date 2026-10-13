// Static plan extraction: read a mission program's source with the TypeScript compiler and
// recover its steps, gates, asks and waits, their order, parallel groups, branches, loops,
// retries, timeouts, escalations and failure handlers, without running anything.
//
// It follows helper functions into other modules through the type checker and substitutes
// literal arguments into their IDs. It does not evaluate anything else: an ID built from data
// stays a template or is marked dynamic, and a callback over input data is a fan-out of unknown
// width.

import ts from "typescript"
import * as path from "node:path"

export type IdForm = "literal" | "template" | "dynamic"

export interface Annotation {
  readonly type: "retry" | "timeout" | "escalate" | "catch" | "on-failure" | "result" | "ignore" | "scope" | "ensuring" | "timer-name"
  readonly detail: string
  readonly handler?: PlanNode
}

export type PlanNode =
  | {
      readonly type: "op"
      readonly kind: "step" | "gate" | "ask" | "wait" | "timer"
      id: string
      idForm: IdForm
      readonly title?: string
      readonly assignedTo?: string
      readonly uses: Array<string> // op IDs whose results this op's arguments read
      readonly at: string // file:line
    }
  | { readonly type: "seq"; readonly items: Array<PlanNode> }
  | { readonly type: "parallel"; readonly items: Array<PlanNode>; readonly over?: string; readonly concurrency?: string }
  | { readonly type: "branch"; readonly condition: string; readonly uses: Array<string>; readonly arms: Array<{ label: string; body: PlanNode }> }
  | { readonly type: "loop"; readonly header: string; readonly bound: number | null; readonly uses: Array<string>; readonly body: PlanNode }
  | { readonly type: "wrap"; readonly inner: PlanNode; readonly annotations: Array<Annotation> }
  // An Effect.gen body or an inlined helper: a return inside it hands a value to its caller.
  // Only the outermost frame's returns end the mission.
  | { readonly type: "frame"; readonly name: string; readonly body: PlanNode }
  | { readonly type: "exit"; readonly outcome: "return" | "fail" | "break"; readonly text: string }
  | { readonly type: "opaque"; readonly text: string; readonly reason: string; readonly at: string }

const OPS: Record<string, "step" | "gate" | "ask" | "wait"> = { step: "step", gate: "gate", ask: "ask", waitFor: "wait" }

interface Env {
  // Parameter name -> literal string value, when the caller passed a literal.
  readonly literals: Map<string, string>
  // Variable name -> op IDs whose results flowed into it.
  readonly bindings: Map<string, Array<string>>
  // Parameter name -> the plan of an effect (or a function returning one) the caller passed.
  readonly effects: Map<string, PlanNode>
  readonly depth: number
}

export interface Extraction {
  readonly mission: string | null
  readonly plan: PlanNode
  readonly diagnostics: Array<string>
}

export const extractFile = (file: string): Extraction => {
  const program = ts.createProgram([file], {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.NodeNext,
    moduleResolution: ts.ModuleResolutionKind.NodeNext,
    allowImportingTsExtensions: true,
    noEmit: true,
    strict: true,
    skipLibCheck: true
  })
  const checker = program.getTypeChecker()
  const source = program.getSourceFile(path.resolve(file))!
  const diagnostics: Array<string> = []
  const where = (node: ts.Node) => {
    const sf = node.getSourceFile()
    return `${path.basename(sf.fileName)}:${sf.getLineAndCharacterOfPosition(node.getStart()).line + 1}`
  }
  let branchCount = 0
  void branchCount

  // Resolve a callee to its declaration, following imports.
  const declarationOf = (expr: ts.Expression): ts.Declaration | undefined => {
    let symbol = checker.getSymbolAtLocation(ts.isPropertyAccessExpression(expr) ? expr.name : expr)
    if (!symbol) return undefined
    if (symbol.flags & ts.SymbolFlags.Alias) symbol = checker.getAliasedSymbol(symbol)
    return symbol.declarations?.[0]
  }
  const isSdk = (decl: ts.Declaration | undefined) => !!decl && decl.getSourceFile().fileName.endsWith(path.join("sdk", "mission.ts"))
  const calleeName = (expr: ts.Expression): string =>
    ts.isPropertyAccessExpression(expr) ? `${expr.expression.getText()}.${expr.name.text}` : expr.getText()

  // An op ID: literal, a template with the parts we know substituted, or dynamic.
  const idOf = (expr: ts.Expression | undefined, env: Env): { id: string; form: IdForm } => {
    if (!expr) return { id: "<missing>", form: "dynamic" }
    if (ts.isStringLiteral(expr) || ts.isNoSubstitutionTemplateLiteral(expr)) return { id: expr.text, form: "literal" }
    if ((ts.isIdentifier(expr) || ts.isPropertyAccessExpression(expr)) && env.literals.has(expr.getText()))
      return { id: env.literals.get(expr.getText())!, form: "literal" }
    if (ts.isTemplateExpression(expr)) {
      let text = expr.head.text
      let known = true
      for (const span of expr.templateSpans) {
        const e = span.expression
        if ((ts.isIdentifier(e) || ts.isPropertyAccessExpression(e)) && env.literals.has(e.getText())) text += env.literals.get(e.getText())
        else {
          known = false
          text += "${" + e.getText() + "}"
        }
        text += span.literal.text
      }
      return { id: text, form: known ? "literal" : "template" }
    }
    return { id: `<${expr.getText()}>`, form: "dynamic" }
  }

  const literalProp = (obj: ts.Expression | undefined, name: string, env: Env): string | undefined => {
    if (!obj || !ts.isObjectLiteralExpression(obj)) return undefined
    for (const p of obj.properties) {
      if (ts.isPropertyAssignment(p) && p.name.getText() === name) {
        const r = idOf(p.initializer, env)
        return r.form === "dynamic" ? p.initializer.getText() : r.id
      }
    }
    return undefined
  }

  // Which bound variables does an expression read?
  const usesIn = (node: ts.Node | undefined, env: Env): Array<string> => {
    const out = new Set<string>()
    const visit = (n: ts.Node) => {
      if (ts.isIdentifier(n) && env.bindings.has(n.text)) for (const op of env.bindings.get(n.text)!) out.add(op)
      n.forEachChild(visit)
    }
    if (node) visit(node)
    return [...out]
  }

  const opIds = (node: PlanNode): Array<string> => {
    switch (node.type) {
      case "op":
        return [node.id]
      case "seq":
      case "parallel":
        return node.items.flatMap(opIds)
      case "branch":
        return node.arms.flatMap((a) => opIds(a.body))
      case "loop":
        return opIds(node.body)
      case "wrap":
        return [...opIds(node.inner), ...node.annotations.flatMap((a) => (a.handler ? opIds(a.handler) : []))]
      case "frame":
        return opIds(node.body)
      default:
        return []
    }
  }

  const rank: Record<IdForm, number> = { literal: 0, template: 1, dynamic: 2 }
  const prefix = (node: PlanNode, scopeName: string, form: IdForm): void => {
    switch (node.type) {
      case "op":
        node.id = `${scopeName}/${node.id}`
        if (rank[form] > rank[node.idForm]) node.idForm = form
        return
      case "seq":
      case "parallel":
        return node.items.forEach((n) => prefix(n, scopeName, form))
      case "branch":
        return node.arms.forEach((a) => prefix(a.body, scopeName, form))
      case "loop":
      case "frame":
        return prefix(node.body, scopeName, form)
      case "wrap":
        prefix(node.inner, scopeName, form)
        node.annotations.forEach((a) => a.handler && prefix(a.handler, scopeName, form))
        return
    }
  }

  const clone = (node: PlanNode): PlanNode => JSON.parse(JSON.stringify(node))

  const seq = (items: Array<PlanNode>): PlanNode => {
    const flat = items.flatMap((i) => (i.type === "seq" ? i.items : [i]))
    return flat.length === 1 ? flat[0]! : { type: "seq", items: flat }
  }
  const empty = (n: PlanNode) => n.type === "seq" && n.items.length === 0

  // Walk a function-like body (arrow expression body, block, generator).
  const walkFunctionBody = (fn: ts.SignatureDeclaration & { body?: ts.ConciseBody | undefined }, env: Env): PlanNode => {
    if (!fn.body) return seq([])
    if (ts.isBlock(fn.body)) return walkStatements(fn.body.statements, env)
    return walkExpr(fn.body, env)
  }

  // Inline a resolved helper: bind its parameters to the literal arguments we can see.
  const inline = (decl: ts.Declaration, args: ReadonlyArray<ts.Expression>, env: Env, call: ts.CallExpression): PlanNode | undefined => {
    let fn: ts.FunctionLikeDeclaration | undefined
    if (ts.isFunctionDeclaration(decl)) fn = decl
    else if (ts.isVariableDeclaration(decl) && decl.initializer && (ts.isArrowFunction(decl.initializer) || ts.isFunctionExpression(decl.initializer)))
      fn = decl.initializer
    if (!fn || !fn.body) return undefined
    if (env.depth > 8) return { type: "opaque", text: call.getText(), reason: "helper nesting deeper than 8", at: where(call) }
    const literals = new Map<string, string>()
    const bindings = new Map(env.bindings)
    const effects = new Map(env.effects)
    fn.parameters.forEach((p, i) => {
      const arg = args[i]
      if (!arg || !ts.isIdentifier(p.name)) return
      const r = idOf(arg, env)
      if (r.form === "literal") literals.set(p.name.text, r.id)
      const u = usesIn(arg, env)
      if (u.length) bindings.set(p.name.text, u)
      const passed = walkExpr(arg, env)
      if (!empty(passed)) effects.set(p.name.text, passed)
      if (ts.isObjectLiteralExpression(arg)) flatten(p.name.text, arg, env, literals, effects)
    })
    const name = ts.isFunctionDeclaration(decl) ? decl.name?.text ?? "helper" : (decl as ts.VariableDeclaration).name.getText()
    return frame(name, walkFunctionBody(fn, { literals, bindings, effects, depth: env.depth + 1 }))
  }

  // An options object passed to a helper: remember its literal fields (job.first.id) and the
  // effects its callbacks build (options.round), so the helper's body can use them.
  const flatten = (prefix: string, obj: ts.ObjectLiteralExpression, env: Env, literals: Map<string, string>, effects: Map<string, PlanNode>) => {
    for (const p of obj.properties) {
      if (!ts.isPropertyAssignment(p) && !ts.isMethodDeclaration(p)) continue
      const key = `${prefix}.${p.name.getText()}`
      if (ts.isMethodDeclaration(p)) {
        const plan = walkFunctionBody(p, env)
        if (!empty(plan)) effects.set(key, plan)
        continue
      }
      const v = p.initializer
      if (ts.isStringLiteral(v) || ts.isNoSubstitutionTemplateLiteral(v) || ts.isNumericLiteral(v)) literals.set(key, v.text)
      else if (ts.isObjectLiteralExpression(v)) flatten(key, v, env, literals, effects)
      else {
        const r = idOf(v, env)
        if (r.form === "literal") literals.set(key, r.id)
        const plan = walkExpr(v, env)
        if (!empty(plan)) effects.set(key, plan)
      }
    }
  }

  const frame = (name: string, body: PlanNode): PlanNode => (empty(body) ? body : { type: "frame", name, body })


  const walkPipeStage = (inner: PlanNode, stage: ts.Expression, env: Env, innerOps: () => Array<string>): PlanNode => {
    if (!ts.isCallExpression(stage)) {
      const name = stage.getText()
      if (name === "Effect.result" || name === "Effect.either" || name === "Effect.exit")
        return { type: "wrap", inner, annotations: [{ type: "result", detail: "failure becomes a value" }] }
      if (name === "Effect.ignore") return { type: "wrap", inner, annotations: [{ type: "ignore", detail: "failure ignored" }] }
      if (name === "Effect.orDie" || name === "Effect.asVoid") return inner
      return { type: "wrap", inner, annotations: [{ type: "ignore", detail: `unrecognised stage ${name}` }] }
    }
    const name = calleeName(stage.expression)
    const [a0, a1] = stage.arguments
    const lambdaBody = (e: ts.Expression | undefined, bind: Array<string>): PlanNode => {
      if (!e) return seq([])
      if (ts.isArrowFunction(e) || ts.isFunctionExpression(e)) {
        const bindings = new Map(env.bindings)
        const p = e.parameters[0]
        if (p && ts.isIdentifier(p.name)) bindings.set(p.name.text, bind)
        return walkFunctionBody(e, { ...env, bindings })
      }
      return walkExpr(e, env)
    }
    switch (name) {
      case "Effect.flatMap":
      case "Effect.andThen":
      case "Effect.tap":
        return seq([inner, lambdaBody(a0, innerOps())])
      case "Effect.retry":
        return { type: "wrap", inner, annotations: [{ type: "retry", detail: a0 ? a0.getText().replace(/\s+/g, " ") : "" }] }
      case "Effect.timeout":
        return { type: "wrap", inner, annotations: [{ type: "timeout", detail: a0?.getText() ?? "" }] }
      case "Effect.timeoutOrElse": {
        const duration = literalProp(a0, "duration", env) ?? "?"
        const orElse = a0 && ts.isObjectLiteralExpression(a0)
          ? a0.properties.find((p): p is ts.PropertyAssignment => ts.isPropertyAssignment(p) && p.name.getText() === "orElse")?.initializer
          : undefined
        return { type: "wrap", inner, annotations: [{ type: "escalate", detail: `after ${duration}`, handler: lambdaBody(orElse, []) }] }
      }
      case "escalateAfter": {
        // escalateAfter(name, after, fallback): a named deadline timer, then the fallback.
        const a2 = stage.arguments[2]
        return { type: "wrap", inner, annotations: [{ type: "escalate", detail: `${idOf(a0, env).id} after ${idOf(a1, env).id}`, handler: lambdaBody(a2, []) }] }
      }
      case "timerName":
        return { type: "wrap", inner, annotations: [{ type: "timer-name", detail: idOf(a0, env).id }] }
      case "Effect.catchTag":
        return { type: "wrap", inner, annotations: [{ type: "catch", detail: idOf(a0, env).id, handler: lambdaBody(a1, []) }] }
      case "Effect.catch":
      case "Effect.catchCause":
      case "Effect.orElse":
        return { type: "wrap", inner, annotations: [{ type: "catch", detail: "any failure", handler: lambdaBody(a0, []) }] }
      case "Effect.tapError":
      case "Effect.onError":
        return { type: "wrap", inner, annotations: [{ type: "on-failure", detail: "then fail", handler: lambdaBody(a0, []) }] }
      case "Effect.ensuring":
        return { type: "wrap", inner, annotations: [{ type: "ensuring", detail: "always", handler: walkExpr(a0!, env) }] }
      case "scope": {
        const s = idOf(a0, env)
        prefix(inner, s.id, s.form)
        return { type: "wrap", inner, annotations: [{ type: "scope", detail: s.id }] }
      }
      case "Effect.map":
      case "Effect.as":
      case "Effect.asVoid":
        return inner
      default: {
        const decl = declarationOf(stage.expression)
        if (isSdk(decl) && decl && ts.isVariableDeclaration(decl) && decl.name.getText() === "scope") {
          const s = idOf(a0, env)
          prefix(inner, s.id, s.form)
          return { type: "wrap", inner, annotations: [{ type: "scope", detail: s.id }] }
        }
        diagnostics.push(`${where(stage)}: pipe stage ${name} not understood; kept the inner plan`)
        return { type: "wrap", inner, annotations: [{ type: "ignore", detail: `unrecognised stage ${name}` }] }
      }
    }
  }

  const walkExpr = (expr: ts.Expression, env: Env): PlanNode => {
    // Strip wrappers.
    if (ts.isParenthesizedExpression(expr) || ts.isAsExpression(expr) || ts.isNonNullExpression(expr) || ts.isSatisfiesExpression(expr))
      return walkExpr(expr.expression, env)
    if (ts.isYieldExpression(expr)) return expr.expression ? walkExpr(expr.expression, env) : seq([])
    if (ts.isAwaitExpression(expr)) return walkExpr(expr.expression, env)
    if (ts.isConditionalExpression(expr)) {
      const whenTrue = walkExpr(expr.whenTrue, env)
      const whenFalse = walkExpr(expr.whenFalse, env)
      if (empty(whenTrue) && empty(whenFalse)) return seq([])
      return {
        type: "branch",
        condition: expr.condition.getText(),
        uses: usesIn(expr.condition, env),
        arms: [
          { label: "yes", body: whenTrue },
          { label: "no", body: whenFalse }
        ]
      }
    }
    if (ts.isArrowFunction(expr) || ts.isFunctionExpression(expr)) return walkFunctionBody(expr, env)
    if (ts.isIdentifier(expr) && env.effects.has(expr.text)) return clone(env.effects.get(expr.text)!)
    if (!ts.isCallExpression(expr)) return seq([])
    if ((ts.isIdentifier(expr.expression) || ts.isPropertyAccessExpression(expr.expression)) && env.effects.has(expr.expression.getText()))
      return clone(env.effects.get(expr.expression.getText())!)

    const callee = expr.expression
    const name = calleeName(callee)
    const args = expr.arguments

    // x.pipe(stage, stage, ...)
    if (ts.isPropertyAccessExpression(callee) && callee.name.text === "pipe") {
      let node = walkExpr(callee.expression, env)
      for (const stage of args) {
        const current = node
        node = walkPipeStage(current, stage, env, () => opIds(current))
      }
      return node
    }

    // SDK ops.
    const short = ts.isIdentifier(callee) ? callee.text : ""
    if (short in OPS) {
      const decl = declarationOf(callee)
      if (!decl || isSdk(decl)) {
        const { id, form } = idOf(args[0], env)
        const title = literalProp(args[1], "title", env)
        const assignedTo = literalProp(args[1], "assignedTo", env) ?? literalProp(args[1], "for", env)
        return {
          type: "op",
          kind: OPS[short]!,
          id,
          idForm: form,
          ...(title ? { title } : {}),
          ...(assignedTo ? { assignedTo } : {}),
          uses: usesIn(args[1], env).concat(usesIn(args[0], env)),
          at: where(expr)
        }
      }
    }

    switch (name) {
      case "Effect.gen": {
        const fn = args.find((a) => ts.isFunctionExpression(a)) as ts.FunctionExpression | undefined
        return fn ? frame("Effect.gen", walkFunctionBody(fn, env)) : seq([])
      }
      case "Effect.all": {
        const first = args[0]
        const concurrency = literalProp(args[1], "concurrency", env)
        const sequential = !concurrency || concurrency === "1"
        if (first && ts.isArrayLiteralExpression(first)) {
          const items = first.elements.map((e) => walkExpr(e, env))
          return sequential ? seq(items) : { type: "parallel", items, ...(concurrency ? { concurrency } : {}) }
        }
        if (first && ts.isObjectLiteralExpression(first)) {
          const items = first.properties.filter(ts.isPropertyAssignment).map((p) => walkExpr(p.initializer, env))
          return sequential ? seq(items) : { type: "parallel", items, ...(concurrency ? { concurrency } : {}) }
        }
        // The common idiom: Effect.all(xs.map((x) => op(...))) is a fan-out over xs.
        if (first && ts.isCallExpression(first) && ts.isPropertyAccessExpression(first.expression) && first.expression.name.text === "map" && first.arguments[0]) {
          const body = walkExpr(first.arguments[0], env)
          return { type: "parallel", items: [body], over: first.expression.expression.getText(), ...(concurrency ? { concurrency } : {}) }
        }
        return { type: "opaque", text: expr.getText(), reason: "Effect.all over a computed collection", at: where(expr) }
      }
      case "Effect.forEach": {
        const concurrency = literalProp(args[2], "concurrency", env)
        const body = args[1] ? walkExpr(args[1], env) : seq([])
        return { type: "parallel", items: [body], over: args[0]?.getText() ?? "?", ...(concurrency ? { concurrency } : {}) }
      }
      case "Effect.result":
      case "Effect.exit":
      case "Effect.ignore":
      case "Effect.orDie":
      case "Effect.option":
        return args[0] ? walkPipeStage(walkExpr(args[0], env), callee, env, () => []) : seq([])
      case "Effect.fail":
      case "Effect.die":
        return { type: "exit", outcome: "fail", text: args[0]?.getText().replace(/\s+/g, " ") ?? "" }
      case "Effect.sleep":
        return { type: "op", kind: "timer", id: "timer", idForm: "literal", title: `sleep ${args[0]?.getText()}`, uses: [], at: where(expr) }
      case "Effect.succeed":
      case "Effect.void":
      case "Effect.sync":
      case "Effect.log":
        return seq([])
    }
    if (name.startsWith("Effect.")) {
      diagnostics.push(`${where(expr)}: ${name} not understood`)
      return { type: "opaque", text: name, reason: "Effect combinator the extractor does not model", at: where(expr) }
    }

    // A helper: follow it.
    const decl = declarationOf(callee)
    // The SDK's own primitive, used directly by code that adds op kinds.
    if (decl && decl.getSourceFile().fileName.endsWith(path.join("sdk", "engine.ts")) && short === "opEffect") {
      const kindArg = args[0]
      const kind = kindArg && ts.isStringLiteral(kindArg) && ["step", "gate", "ask", "wait", "timer"].includes(kindArg.text) ? (kindArg.text as any) : "step"
      const { id, form } = idOf(args[1], env)
      return { type: "op", kind, id, idForm: form, uses: usesIn(args[2], env), at: where(expr) }
    }
    // Never walk into the SDK's internals.
    if (decl && decl.getSourceFile().fileName.includes(`${path.sep}sdk${path.sep}`)) {
      return { type: "opaque", text: expr.getText(), reason: "an SDK internal", at: where(expr) }
    }
    if (decl && !decl.getSourceFile().isDeclarationFile && !isSdk(decl)) {
      const inlined = inline(decl, args, env, expr)
      if (inlined) return inlined
    }
    // A call we cannot follow may still yield inside its arguments: shipped.push(yield* land(...)).
    const yielded = args.filter(hasYield).map((a) => walkExpr(a, env)).filter((n) => !empty(n))
    if (yielded.length) return seq(yielded)
    // A call we cannot follow. If the checker says it returns an Effect, it may open ops we
    // cannot see: say so rather than drop it.
    if (returnsEffect(expr)) {
      diagnostics.push(`${where(expr)}: ${expr.getText()} returns an Effect the extractor cannot follow`)
      return { type: "opaque", text: expr.getText(), reason: "an Effect from a call the extractor cannot follow", at: where(expr) }
    }
    return seq([])
  }

  const returnsEffect = (expr: ts.Expression): boolean => {
    const text = checker.typeToString(checker.getTypeAtLocation(expr))
    return /^Effect</.test(text)
  }

  const bindPattern = (name: ts.BindingName, node: PlanNode, env: Env) => {
    if (ts.isIdentifier(name)) {
      env.bindings.set(name.text, opIds(node))
      return
    }
    if (ts.isArrayBindingPattern(name) && (node.type === "parallel" || node.type === "seq")) {
      name.elements.forEach((el, i) => {
        if (ts.isBindingElement(el) && ts.isIdentifier(el.name)) env.bindings.set(el.name.text, opIds(node.items[i] ?? node))
      })
      return
    }
    for (const id of collectNames(name)) env.bindings.set(id, opIds(node))
  }
  const collectNames = (name: ts.BindingName): Array<string> =>
    ts.isIdentifier(name) ? [name.text] : name.elements.flatMap((e) => (ts.isBindingElement(e) ? collectNames(e.name) : []))

  const hasYield = (node: ts.Node): boolean => {
    let found = false
    const visit = (n: ts.Node) => {
      if (found) return
      if (ts.isYieldExpression(n)) found = true
      else if (!ts.isFunctionLike(n)) n.forEachChild(visit)
    }
    visit(node)
    return found
  }

  const loopBound = (s: ts.ForStatement, env: Env): number | null => {
    const init = s.initializer && ts.isVariableDeclarationList(s.initializer) ? s.initializer.declarations[0] : undefined
    const start = init?.initializer && ts.isNumericLiteral(init.initializer) ? Number(init.initializer.text) : null
    const cond = s.condition
    if (start === null || !cond || !ts.isBinaryExpression(cond)) return null
    const right = cond.right
    const endText = ts.isNumericLiteral(right) ? right.text : env.literals.get(right.getText())
    if (endText === undefined || Number.isNaN(Number(endText))) return null
    const end = Number(endText)
    switch (cond.operatorToken.kind) {
      case ts.SyntaxKind.LessThanEqualsToken:
        return end - start + 1
      case ts.SyntaxKind.LessThanToken:
        return end - start
      default:
        return null
    }
  }

  const walkStatements = (statements: ReadonlyArray<ts.Statement>, env: Env): PlanNode => {
    const items: Array<PlanNode> = []
    for (const s of statements) items.push(walkStatement(s, env))
    return seq(items.filter((n) => !empty(n)))
  }

  const walkStatement = (s: ts.Statement, env: Env): PlanNode => {
    if (ts.isBlock(s)) return walkStatements(s.statements, env)
    if (ts.isVariableStatement(s)) {
      const out: Array<PlanNode> = []
      for (const d of s.declarationList.declarations) {
        if (!d.initializer) continue
        const node = walkExpr(d.initializer, env)
        if (hasYield(d.initializer)) bindPattern(d.name, node, env)
        else {
          // A plain computation: it carries the dependencies of what it reads.
          const u = usesIn(d.initializer, env)
          for (const n of collectNames(d.name)) if (u.length) env.bindings.set(n, u)
        }
        out.push(node)
      }
      return seq(out)
    }
    if (ts.isExpressionStatement(s)) {
      // round++ and other bookkeeping produce nothing.
      return walkExpr(s.expression, env)
    }
    if (ts.isReturnStatement(s)) {
      if (!s.expression) return { type: "exit", outcome: "return", text: "return" }
      const node = walkExpr(s.expression, env)
      // return Effect.gen(...) from a plain function hands back the program; it ends nothing.
      if (!hasYield(s.expression) && returnsEffect(s.expression)) return node
      if (node.type === "exit") return node
      const text = s.expression.getText().replace(/\s+/g, " ")
      return seq([node, { type: "exit", outcome: "return", text: text.length > 60 ? text.slice(0, 57) + "..." : text }])
    }
    if (ts.isBreakStatement(s)) return { type: "exit", outcome: "break", text: "break" }
    if (ts.isIfStatement(s)) {
      const yes = walkStatement(s.thenStatement, env)
      const no = s.elseStatement ? walkStatement(s.elseStatement, env) : seq([])
      return {
        type: "branch",
        condition: s.expression.getText(),
        uses: usesIn(s.expression, env),
        arms: [
          { label: "yes", body: yes },
          { label: "no", body: no }
        ]
      }
    }
    if (ts.isSwitchStatement(s)) {
      return {
        type: "branch",
        condition: `switch ${s.expression.getText()}`,
        uses: usesIn(s.expression, env),
        arms: s.caseBlock.clauses.map((c) => ({
          label: ts.isCaseClause(c) ? c.expression.getText() : "default",
          body: walkStatements(c.statements, env)
        }))
      }
    }
    if (ts.isForStatement(s)) {
      const header = `for (${s.initializer?.getText() ?? ""}; ${s.condition?.getText() ?? ""}; ${s.incrementor?.getText() ?? ""})`
      return { type: "loop", header, bound: loopBound(s, env), uses: usesIn(s.condition, env), body: walkStatement(s.statement, env) }
    }
    if (ts.isForOfStatement(s) || ts.isForInStatement(s)) {
      const header = `for (${s.initializer.getText()} of ${s.expression.getText()})`
      let over: ts.Expression = s.expression
      while (ts.isAsExpression(over) || ts.isParenthesizedExpression(over) || ts.isSatisfiesExpression(over)) over = over.expression
      const bound = ts.isArrayLiteralExpression(over) ? over.elements.length : null
      return { type: "loop", header, bound, uses: usesIn(s.expression, env), body: walkStatement(s.statement, env) }
    }
    if (ts.isWhileStatement(s) || ts.isDoStatement(s)) {
      return { type: "loop", header: `while (${s.expression.getText()})`, bound: null, uses: usesIn(s.expression, env), body: walkStatement(s.statement, env) }
    }
    if (ts.isTryStatement(s)) {
      return walkStatements(s.tryBlock.statements, env)
    }
    if (ts.isFunctionDeclaration(s)) return seq([])
    return seq([])
  }

  // Find the mission(...) call and walk its run function.
  let missionName: string | null = null
  let plan: PlanNode = seq([])
  const find = (n: ts.Node) => {
    if (ts.isCallExpression(n) && ts.isIdentifier(n.expression) && n.expression.text === "mission") {
      const [nameArg, run] = n.arguments
      if (nameArg && ts.isStringLiteral(nameArg)) missionName = nameArg.text
      if (run && (ts.isArrowFunction(run) || ts.isFunctionExpression(run))) {
        plan = walkFunctionBody(run, { literals: new Map(), bindings: new Map(), effects: new Map(), depth: 0 })
      }
      return
    }
    n.forEachChild(find)
  }
  find(source)
  if (missionName === null) diagnostics.push("no mission(...) call found")
  return { mission: missionName, plan, diagnostics }
}

// ---- graph and mermaid ----------------------------------------------------------------------------

export interface GraphNode {
  readonly id: string
  readonly label: string
  readonly shape: "op" | "decision" | "loop" | "exit" | "opaque" | "start"
  readonly kind?: string
}
export interface GraphEdge {
  readonly from: string
  readonly to: string
  readonly label?: string
  readonly kind: "order" | "data" | "handler" | "loop-back"
}
export interface PlanGraph {
  readonly nodes: Array<GraphNode>
  readonly edges: Array<GraphEdge>
  readonly groups: Array<{ readonly label: string; readonly members: Array<string> }>
}

const opCount = (node: PlanNode): number => staticOps(node).length

export const toGraph = (plan: PlanNode): PlanGraph => {
  const nodes: Array<GraphNode> = [{ id: "start", label: "start", shape: "start" }]
  const edges: Array<GraphEdge> = []
  const groups: PlanGraph["groups"] = []
  const opNode = new Map<string, string>()
  const pendingData: Array<{ uses: Array<string>; to: string }> = []
  let n = 0
  const fresh = (p: string) => `${p}${++n}`
  const breakStack: Array<Array<string>> = []
  const frameStack: Array<Array<string>> = []
  const exitLabel = new Map<string, string>()
  const connect = (from: string, to: string, label?: string, kind: GraphEdge["kind"] = "order") => {
    const l = label ?? exitLabel.get(from)
    edges.push({ from, to, kind, ...(l ? { label: l } : {}) })
  }

  // Returns entry node IDs and exit node IDs (where control continues).
  const build = (node: PlanNode, annotationsLabel = ""): { entry: Array<string>; exits: Array<string> } => {
    switch (node.type) {
      case "op": {
        const id = fresh("op")
        const who = node.assignedTo ? `\n${node.assignedTo}` : ""
        const name = node.kind === "timer" ? node.title ?? "timer" : `${node.kind} ${node.id}`
        nodes.push({ id, label: `${name}${annotationsLabel}${who}`, shape: "op", kind: node.kind })
        if (!opNode.has(node.id)) opNode.set(node.id, id)
        if (node.uses.length) pendingData.push({ uses: node.uses, to: id })
        return { entry: [id], exits: [id] }
      }
      case "seq": {
        let entry: Array<string> = []
        let exits: Array<string> = []
        let first = true
        for (const item of node.items) {
          const g = build(item)
          if (first) {
            entry = g.entry
            first = false
          } else for (const from of exits) for (const to of g.entry) connect(from, to)
          exits = g.exits
          if (item.type === "exit") break
        }
        return { entry, exits }
      }
      case "parallel": {
        const parts = node.items.map((i) => build(i))
        const members = parts.flatMap((p) => p.entry)
        groups.push({
          label: node.over ? `for each ${node.over}${node.concurrency ? `, ${node.concurrency} at a time` : ""}` : `parallel${node.concurrency ? ` (${node.concurrency})` : ""}`,
          members
        })
        return { entry: members, exits: parts.flatMap((p) => p.exits) }
      }
      case "branch": {
        const id = fresh("branch")
        nodes.push({ id, label: node.condition, shape: "decision" })
        if (node.uses.length) pendingData.push({ uses: node.uses, to: id })
        const exits: Array<string> = []
        for (const arm of node.arms) {
          const g = build(arm.body)
          if (g.entry.length === 0) {
            exitLabel.set(id, arm.label)
            exits.push(id)
            continue
          }
          for (const to of g.entry) edges.push({ from: id, to, label: arm.label, kind: "order" })
          exits.push(...g.exits)
        }
        return { entry: [id], exits }
      }
      case "loop": {
        const id = fresh("loop")
        nodes.push({ id, label: `${node.header}${node.bound !== null ? ` ×${node.bound}` : " (unbounded)"}`, shape: "loop" })
        if (node.uses.length) pendingData.push({ uses: node.uses, to: id })
        breakStack.push([])
        const g = build(node.body)
        const breaks = breakStack.pop()!
        for (const to of g.entry) edges.push({ from: id, to, label: "round", kind: "order" })
        for (const from of g.exits) connect(from, id, "next", "loop-back")
        exitLabel.set(id, node.bound !== null ? "done" : "exit")
        return { entry: [id], exits: [id, ...breaks] }
      }
      case "wrap": {
        const tags = node.annotations
          .filter((a) => a.type === "retry" || a.type === "timeout" || a.type === "result" || a.type === "ignore")
          .map((a) => (a.type === "retry" ? `retry ${a.detail}` : a.type === "timeout" ? `timeout ${a.detail}` : a.type === "result" ? "failure as value" : "ignore"))
        const label = tags.length ? `\n[${tags.join("; ")}]` : ""
        const g = node.inner.type === "op" ? build(node.inner, label) : build(node.inner)
        if (node.inner.type !== "op" && tags.length) groups.push({ label: tags.join("; "), members: g.entry })
        const exits = [...g.exits]
        for (const a of node.annotations) {
          if (!a.handler) continue
          const h = build(a.handler)
          const edgeLabel = a.type === "escalate" ? a.detail : a.type === "catch" ? `on ${a.detail}` : a.type === "ensuring" ? "finally" : "on failure"
          let sources = a.type === "ensuring" ? g.exits : g.entry
          if (a.type === "ensuring" && sources.length === 0) {
            const point = fresh("finally")
            nodes.push({ id: point, label: "when the block above ends, however it ends", shape: "decision" })
            sources = [point]
          }
          if ((a.type === "on-failure" || a.type === "catch") && node.inner.type !== "op" && opCount(node.inner) > 0) {
            // The handler runs when any op inside fails; draw that as one point, not an edge per op.
            const point = fresh("failure")
            nodes.push({ id: point, label: `any of ${opCount(node.inner)} ops above fails`, shape: "decision" })
            sources = [point]
          }
          for (const from of sources) for (const to of h.entry) edges.push({ from, to, label: edgeLabel, kind: "handler" })
          if (a.type === "on-failure") {
            const fail = fresh("exit")
            nodes.push({ id: fail, label: "fail", shape: "exit" })
            for (const from of h.exits) edges.push({ from, to: fail, kind: "order" })
          } else exits.push(...h.exits)
        }
        return { entry: g.entry, exits }
      }
      case "frame": {
        frameStack.push([])
        const g = build(node.body)
        const returns = frameStack.pop()!
        return { entry: g.entry, exits: [...g.exits, ...returns] }
      }
      case "exit": {
        const id = fresh("exit")
        nodes.push({ id, label: `${node.outcome}${node.outcome === "break" ? "" : ` ${node.text}`}`, shape: "exit" })
        if (node.outcome === "break") breakStack.at(-1)?.push(id)
        // A return inside a nested frame continues in the caller.
        if (node.outcome === "return" && frameStack.length >= 2) frameStack.at(-1)!.push(id)
        return { entry: [id], exits: [] }
      }
      case "opaque": {
        const id = fresh("opaque")
        nodes.push({ id, label: `? ${node.text}\n(${node.reason})`, shape: "opaque" })
        return { entry: [id], exits: [id] }
      }
    }
  }

  const g = build(plan)
  for (const to of g.entry) connect("start", to)
  for (const d of pendingData)
    for (const op of d.uses) {
      const from = opNode.get(op)
      if (from && from !== d.to) edges.push({ from, to: d.to, kind: "data", label: "reads" })
    }
  return { nodes, edges, groups }
}

export const toMermaid = (graph: PlanGraph): string => {
  const esc = (s: string) => s.replace(/"/g, "'").replace(/\n/g, "<br/>")
  const lines = ["flowchart TD"]
  const grouped = new Set<string>()
  graph.groups.forEach((group, i) => {
    lines.push(`  subgraph g${i}["${esc(group.label)}"]`)
    for (const m of group.members) {
      if (grouped.has(m)) continue
      grouped.add(m)
      lines.push(`    ${m}`)
    }
    lines.push("  end")
  })
  for (const node of graph.nodes) {
    const l = esc(node.label)
    const shape =
      node.shape === "decision" ? `{"${l}"}` : node.shape === "loop" ? `[["${l}"]]` : node.shape === "exit" ? `(["${l}"])` : node.shape === "start" ? `(("${l}"))` : node.shape === "opaque" ? `[/"${l}"/]` : `["${l}"]`
    lines.push(`  ${node.id}${shape}`)
  }
  for (const e of graph.edges) {
    const arrow = e.kind === "data" ? "-.->" : e.kind === "handler" ? "==>" : e.kind === "loop-back" ? "-.->" : "-->"
    lines.push(`  ${e.from} ${arrow}${e.label ? `|"${esc(e.label)}"|` : ""} ${e.to}`)
  }
  return lines.join("\n")
}

// Every op the static plan names, with its ID form.
export const staticOps = (plan: PlanNode): Array<{ id: string; kind: string; idForm: IdForm }> => {
  const out: Array<{ id: string; kind: string; idForm: IdForm }> = []
  const visit = (n: PlanNode) => {
    switch (n.type) {
      case "op":
        out.push({ id: n.id, kind: n.kind, idForm: n.idForm })
        return
      case "seq":
      case "parallel":
        return n.items.forEach(visit)
      case "branch":
        return n.arms.forEach((a) => visit(a.body))
      case "loop":
      case "frame":
        return visit(n.body)
      case "wrap":
        visit(n.inner)
        n.annotations.forEach((a) => a.handler && visit(a.handler))
        return
    }
  }
  visit(plan)
  return out
}
