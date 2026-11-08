// A small layered (Sugiyama-style) layout for mission DAGs, no dependency: longest-path layering, then a few barycenter
// sweeps to cut crossings. Mission plans are tens of steps, which this handles in well under a millisecond.
import type { PlanStep } from './model.ts'

export interface Placed {
  readonly id: string
  readonly layer: number
  readonly order: number
}

export const layered = (steps: readonly PlanStep[]): { readonly nodes: ReadonlyMap<string, Placed>; readonly layers: number; readonly widest: number } => {
  const ids = new Set(steps.map(step => step.id))
  const deps = new Map(steps.map(step => [step.id, step.after.filter(dep => ids.has(dep))]))
  const layerOf = new Map<string, number>()
  const layer = (id: string, seen = new Set<string>()): number => {
    const known = layerOf.get(id)
    if (known !== undefined) return known
    if (seen.has(id)) return 0
    seen.add(id)
    const parents = deps.get(id) ?? []
    const value = parents.length === 0 ? 0 : 1 + Math.max(...parents.map(parent => layer(parent, seen)))
    layerOf.set(id, value)
    return value
  }
  steps.forEach(step => layer(step.id))
  const count = Math.max(0, ...layerOf.values()) + 1
  const rows: string[][] = Array.from({ length: count }, () => [])
  steps.forEach(step => rows[layerOf.get(step.id)!]!.push(step.id))
  const children = new Map<string, string[]>()
  for (const [id, parents] of deps) for (const parent of parents) children.set(parent, [...children.get(parent) ?? [], id])
  const position = () => new Map(rows.flatMap(row => row.map((id, index) => [id, index] as const)))
  const mean = (values: readonly number[]) => values.length === 0 ? Number.NaN : values.reduce((a, b) => a + b, 0) / values.length
  for (let sweep = 0; sweep < 4; sweep++) {
    const down = sweep % 2 === 0
    const order = position()
    const range = down ? rows.keys() : [...rows.keys()].reverse()
    for (const index of range) {
      const row = rows[index]!
      const score = new Map(row.map((id, i) => {
        const value = mean((down ? deps.get(id) ?? [] : children.get(id) ?? []).map(other => order.get(other)!))
        return [id, Number.isNaN(value) ? i : value] as const
      }))
      row.sort((a, b) => score.get(a)! - score.get(b)!)
    }
  }
  const nodes = new Map<string, Placed>()
  rows.forEach((row, layerIndex) => row.forEach((id, order) => nodes.set(id, { id, layer: layerIndex, order })))
  return { nodes, layers: count, widest: Math.max(...rows.map(row => row.length)) }
}
