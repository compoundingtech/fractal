/**
 * KDL v2 nodes and their serialization. This module imports nothing, so Genie files that render
 * declarations (`slo/objectives.kdl.genie.ts`) bundle it without the package's dependencies.
 */

/** A KDL argument or property value. */
export type Value = string | number | boolean

/** A KDL node; `children: undefined` renders without a children block. */
export type Node = {
  readonly name: string
  readonly args: readonly Value[]
  readonly props: Readonly<Record<string, Value>>
  readonly children?: readonly Node[]
}

const immutableNodes = new WeakSet<Node>()

/** Builds a KDL node; omitted args and props are empty. */
export const node = ({
  name,
  args = [],
  props = {},
  children,
}: {
  readonly name: string
  readonly args?: readonly Value[]
  readonly props?: Readonly<Record<string, Value>>
  readonly children?: readonly Node[]
}): Node => {
  const value = Object.freeze({
    name,
    args: Object.freeze([...args]),
    props: Object.freeze({ ...props }),
    ...(children === undefined
      ? {}
      : {
          children: Object.freeze(
            children.map((child) => (immutableNodes.has(child) === true ? child : node(child))),
          ),
        }),
  })
  immutableNodes.add(value)
  return value
}

const renderValue = (value: Value): string => {
  if (typeof value === 'string') return JSON.stringify(value)
  if (typeof value === 'boolean') return `#${value}`
  if (Number.isFinite(value) === false) throw new RangeError('KDL numbers must be finite')
  return String(value)
}

const identifier = (text: string): string =>
  /^[A-Za-z_][\w-]*$/u.test(text) === true ? text : JSON.stringify(text)

const render = ({
  node: current,
  depth,
}: {
  readonly node: Node
  readonly depth: number
}): string => {
  const pad = '  '.repeat(depth)
  const args = current.args.map((arg) => ` ${renderValue(arg)}`).join('')
  const props = Object.entries(current.props)
    .toSorted(([a], [b]) => a.localeCompare(b, 'en'))
    .map(([key, value]) => ` ${identifier(key)}=${renderValue(value)}`)
    .join('')
  const header = `${pad}${identifier(current.name)}${args}${props}`
  if (current.children === undefined) return `${header}\n`
  const body = current.children.map((child) => render({ node: child, depth: depth + 1 })).join('')
  return `${header} {\n${body}${pad}}\n`
}

/** Serializes nodes as a KDL v2 document. */
export const emit = (nodes: readonly Node[]): string =>
  `version 2\n${nodes.map((current) => render({ node: current, depth: 0 })).join('')}`

/** One validation finding, in the shape Genie's `GenieValidationIssue` expects. */
export interface KdlValidationIssue {
  readonly severity: 'error'
  readonly packageName: string
  readonly dependency: string
  readonly rule: string
  readonly message: string
}

/**
 * A generated KDL declaration file. The shape is Genie's `GenieOutput` contract, declared
 * structurally so a Genie file can return it without this package depending on Genie.
 */
export interface KdlFileOutput {
  readonly data: readonly Node[]
  readonly stringify: () => string
  readonly validate: () => KdlValidationIssue[]
}

/** Genie output for a Fractal KDL declaration file. */
export const fractalKdl = (nodes: readonly Node[]): KdlFileOutput => ({
  data: nodes,
  stringify: () => emit(nodes),
  validate: () => {
    try {
      emit(nodes)
      return []
    } catch (cause) {
      return [
        {
          severity: 'error',
          packageName: 'fractal',
          dependency: '',
          rule: 'fractal-kdl',
          message: cause instanceof Error ? cause.message : String(cause),
        },
      ]
    }
  },
})
