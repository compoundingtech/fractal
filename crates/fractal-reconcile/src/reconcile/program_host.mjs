// The Fractal program host (docs/st3/program-engine.md). The daemon's program engine starts one per
// daemon and talks to it over stdin/stdout, one JSON object per line. The host evaluates program
// bundles and nothing else: it holds no credential, and its only durable write is a journal entry
// the daemon acknowledges. It has no dependencies, so its behaviour is fixed by the daemon build;
// a program's own runtime (the `fractal` program API, Effect) is inside its pinned bundle.
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { createInterface } from 'node:readline'

/** The entry point a bundle's default export carries, version 1 of the host contract. */
const ENTRY = Symbol.for('fractal.program.v1')

const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`)
/** By step-run subject: the attempt this host runs. */
const runs = new Map()
/** By pinned module path: the loaded module. One path holds one immutable bundle. */
const modules = new Map()

const load = (path, sha256) => {
  if (!modules.has(path)) {
    const actual = createHash('sha256').update(readFileSync(path)).digest('hex')
    if (actual !== sha256) throw new Error(`bundle ${path} has sha256 ${actual}, the run pins ${sha256}`)
    modules.set(path, import(path))
  }
  return modules.get(path)
}

class Interrupted extends Error {}

const stop = (run, reason) => {
  run.controller.abort(new Interrupted(reason))
  for (const { reject } of run.acks.values()) reject(new Interrupted(reason))
  run.acks.clear()
  for (const child of run.children) kill(child)
}

const kill = (child) => {
  try {
    process.kill(-child.pid, 'SIGKILL')
  } catch {}
}

/** A journal entry is durable only once the daemon acknowledges it; the program waits for that. */
const record = (run, entry) =>
  new Promise((resolve, reject) => {
    if (run.controller.signal.aborted) return reject(run.controller.signal.reason)
    const seq = ++run.seq
    run.acks.set(seq, { resolve, reject })
    send({ t: 'journal', step: run.step, seq, ...entry })
  })

/** Step effects run as their own process groups, so a cancelled program leaves nothing behind. */
const spawnChild = (run, argv, options = {}) =>
  new Promise((resolve, reject) => {
    if (run.controller.signal.aborted) return reject(run.controller.signal.reason)
    const child = spawn(argv[0], argv.slice(1), {
      cwd: options.cwd ?? run.cwd,
      env: { ...process.env, ...run.env, ...options.env },
      stdio: ['ignore', 'pipe', 'pipe'],
      detached: true,
    })
    run.children.add(child)
    const out = []
    const err = []
    child.stdout.on('data', (chunk) => out.push(chunk))
    child.stderr.on('data', (chunk) => err.push(chunk))
    child.on('error', (error) => {
      run.children.delete(child)
      reject(error)
    })
    child.on('close', (code, signal) => {
      run.children.delete(child)
      resolve({ code: code ?? 128, signal, stdout: Buffer.concat(out).toString(), stderr: Buffer.concat(err).toString() })
    })
  })

const start = async (message) => {
  const previous = runs.get(message.step)
  if (previous !== undefined) stop(previous, 'a newer attempt replaced this one')
  const run = {
    step: message.step,
    attempt: message.attempt,
    cwd: message.cwd,
    env: message.env,
    seq: 0,
    acks: new Map(),
    children: new Set(),
    controller: new AbortController(),
  }
  runs.set(message.step, run)
  let exit
  try {
    const program = (await load(message.module, message.sha256)).default?.[ENTRY]
    if (typeof program !== 'function') throw new Error(`${message.module} does not export a Fractal program`)
    exit = await program({
      step: message.step,
      attempt: message.attempt,
      payload: message.payload,
      env: message.env,
      cwd: message.cwd,
      journal: new Map(message.journal.map((entry) => [entry.key, entry])),
      signal: run.controller.signal,
      started: (key) => send({ t: 'journal', step: run.step, key, status: 'started' }),
      record: (entry) => record(run, entry),
      spawn: (argv, options) => spawnChild(run, argv, options),
    })
  } catch (error) {
    exit = { _tag: 'Failure', error: { _tag: 'Defect', message: String(error?.stack ?? error).slice(0, 2000) } }
  }
  if (runs.get(message.step) !== run || run.controller.signal.aborted) return
  runs.delete(message.step)
  send({ t: 'done', step: message.step, exit })
}

const lines = createInterface({ input: process.stdin })
lines.on('line', (line) => {
  const message = JSON.parse(line)
  const run = runs.get(message.step)
  if (message.t === 'start') void start(message)
  else if (message.t === 'ack') {
    run?.acks.get(message.seq)?.resolve()
    run?.acks.delete(message.seq)
  } else if (message.t === 'cancel' && run !== undefined) {
    runs.delete(message.step)
    stop(run, message.reason)
  }
})
// The daemon is gone: its replacement resumes every program from the journal. Stop all effects now.
lines.on('close', () => {
  for (const run of runs.values()) stop(run, 'the daemon stopped')
  process.exit(0)
})
