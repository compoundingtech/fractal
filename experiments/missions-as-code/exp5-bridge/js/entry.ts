// The bridge's only surface: decide(programName, historyJson) -> decisionJson.
// The host provides no functions. The script gets no clock, no randomness, no timers and no I/O.
import "./polyfill.ts"
import { decide, Session } from "../../sdk/engine.ts"
import { programs } from "./programs.ts"

let seed = 0x9e3779b9

// Live runs for incremental decisions: open once, then feed only new events.
const sessions = new Map<number, Session>()
let nextSession = 1
const withoutStats = ({ stats: _, ...rest }: any) => rest

;(globalThis as any).__open = (name: string, historyJson: string): string => {
  seed = 0x9e3779b9
  const program = (programs as any)[name]
  if (!program) throw new Error(`unknown program ${name}`)
  const id = nextSession++
  sessions.set(id, new Session(program, JSON.parse(historyJson)))
  return String(id)
}
;(globalThis as any).__apply = (id: string, eventsJson: string): string => {
  const session = sessions.get(Number(id))
  if (!session) throw new Error(`no session ${id}`)
  // Only what the caller acts on: the status and the commands, found from the ops these events
  // touched. The open list of a wide program is large and mostly unchanged, so __decision reads
  // it when needed.
  return JSON.stringify(session.step(JSON.parse(eventsJson)))
}
;(globalThis as any).__decision = (id: string): string => {
  const session = sessions.get(Number(id))
  if (!session) throw new Error(`no session ${id}`)
  return JSON.stringify(withoutStats(session.decision()))
}
;(globalThis as any).__close = (id: string): string => {
  sessions.delete(Number(id))
  return ""
}

;(globalThis as any).__decide = (name: string, historyJson: string): string => {
  seed = 0x9e3779b9 // every decision sees the same Math.random sequence
  const program = (programs as any)[name]
  if (!program) throw new Error(`unknown program ${name}`)
  return JSON.stringify(decide(program, JSON.parse(historyJson)))
}

// Close the doors a program could use to see the outside world. Effect's own clock and random
// are replaced inside decide(); these catch a program that reaches for the globals directly.
;(() => {
  const refuse = (name: string) => () => {
    throw new Error(`${name} is not available in a mission program; use Clock or Random from Effect`)
  }
  // Effect itself calls Math.random for object identity hashes (effect/Hash), so refusing it would
  // break Effect. A sequence reseeded before every decision keeps those hashes, and any
  // HashMap or HashSet order that depends on them, the same on every replay.
  ;(Math as any).random = () => {
    let t = (seed = (seed + 0x6d2b79f5) | 0)
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
  ;(Date as any).now = refuse("Date.now")
  const RealDate = Date
  const SafeDate = function (this: unknown, ...args: Array<unknown>) {
    if (args.length === 0) refuse("new Date()")()
    return new (RealDate as any)(...args)
  } as unknown as DateConstructor
  ;(SafeDate as any).prototype = RealDate.prototype
  SafeDate.now = Date.now
  SafeDate.parse = RealDate.parse
  SafeDate.UTC = RealDate.UTC
  ;(globalThis as any).Date = SafeDate
})()
