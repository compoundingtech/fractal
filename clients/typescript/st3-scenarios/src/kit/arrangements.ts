import type { Agent, Arrangement, ArrangementFolder, ArrangementPlacementRegister } from '@smalltalk/st3-client'

import type { FactoryContext } from './context.ts'
import { fork, int } from './rng.ts'
import { ANCHOR_MS } from './time.ts'

/** Stable identities use the committed anchor, not the consumer's shifted clock. */
const uuid = (ctx: FactoryContext, label: string): string => {
  const rng = fork(ctx.rng, `arrangements/${label}`)
  const timestamp = (ANCHOR_MS - 60_000).toString(16).padStart(12, '0')
  const hex = (length: number): string => Array.from({ length }, () => int(rng, 0, 15).toString(16)).join('')
  return `${timestamp.slice(0, 8)}-${timestamp.slice(8)}-7${hex(3)}-${int(rng, 8, 11).toString(16)}${hex(3)}-${hex(12)}`
}

const DIGITS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'
/** Three-character base-62 integer keys, ordered lexically without fractional suffixes. */
const key = (index: number): string => `b${DIGITS[Math.floor(index / 62)]!}${DIGITS[index % 62]!}`

/** Sidebar data has no recorder source: it is generated alongside the synthetic roster. */
export const sidebarArrangement = (ctx: FactoryContext, agents: readonly Agent[], huge: boolean): Arrangement => {
  const revision = 'arrangement-1'
  const folders: Record<string, ArrangementFolder> = {}
  const placements: Record<string, ArrangementPlacementRegister> = {}
  const folder = (label: string, name: string, parent: string | null, position: number): string => {
    const id = uuid(ctx, `folder/${label}`)
    folders[id] = { name: { revision, value: name }, position: { revision, value: { parent, key: key(position) } }, tombstone: null }
    return id
  }
  const project = folder('project', ctx.cast.project, null, 0)
  if (huge) {
    const roles = ['builder', 'reviewer', 'migrator', 'docs', 'tester', 'release']
    const destinations = Array.from({ length: 8 }, (_, group) => {
      const parent = folder(`group/${group}`, `Migration group ${group + 1}`, project, group)
      return roles.map((role, index) => folder(`group/${group}/${role}`, role, parent, index))
    })
    agents.forEach((agent, index) => {
      placements[agent.id] = { revision, value: { folder: destinations[Math.floor(index / 6) % 8]![index % 6]!, key: key(Math.floor(index / 48)) } }
    })
  } else {
    const destinations = [folder('implementation', 'Implementation', project, 0), folder('review', 'Review', project, 1)]
    agents.slice(0, 3).forEach((agent, index) => {
      placements[agent.id] = { revision, value: { folder: destinations[index === 1 ? 1 : 0]!, key: key(index) } }
    })
  }
  return {
    id: `arrangement/${ctx.cast.people[0]!.id}/${uuid(ctx, 'sidebar')}`,
    kind: 'arrangement', owner: ctx.cast.people[0]!.id, deleted: false,
    revision, updated_at: ctx.t.at(-60_000),
    body: { version: 1, name: { revision, value: 'Sidebar' }, folders, placements },
  }
}
