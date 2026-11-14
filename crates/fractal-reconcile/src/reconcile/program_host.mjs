// The Fractal program host (docs/st3/program-engine.md §2, protocol v1). The daemon's program
// engine starts one per member and talks to it over stdin/stdout, one JSON object per line. The
// host evaluates program bundles and nothing else: it holds no credential, and its only durable
// write is a journal entry the daemon acknowledges. It has no dependencies, so its behaviour is
// fixed by the daemon build; a program's own runtime (the `fractal` program API, Effect) is inside
// its pinned bundle.
//
// The main thread owns the pipe, every effect's process group and liveness. Each program attempt
// runs in its own worker, so one program's busy loop cannot starve the others: a worker that does
// not answer a ping within FRACTAL_PROGRAM_WORKER_DEADLINE_MS is terminated (DQ8). Effects run as
// their own process groups; the main thread records each in FRACTAL_PROGRAM_GROUPS with the boot
// id and the leader's start time, kills the group when its leader exits, when its attempt ends or
// is cancelled, and when stdin closes, and removes the record once it has (DQ2).
import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads'

/** The entry point a bundle's default export carries, version 1 of the host contract. */
const ENTRY = Symbol.for('fractal.program.v1')

const defect = (error) => ({ _tag: 'Failure', error: { _tag: 'Defect', message: String(error?.stack ?? error).slice(0, 2000) } })

/** The leader's start time as the daemon reads it before it kills a recorded group. */
const startTime = (pid) => {
  try {
    if (process.platform === 'linux') {
      const stat = readFileSync(`/proc/${pid}/stat`, 'utf8')
      return stat.slice(stat.lastIndexOf(')') + 2).split(' ')[19]
    }
    const out = spawnSync('ps', ['-o', 'lstart=', '-p', String(pid)], { encoding: 'utf8' }).stdout.trim()
    return out === '' ? undefined : out
  } catch {
    return undefined
  }
}

const main = () => {
  const groupsDir = process.env.FRACTAL_PROGRAM_GROUPS
  const bootId = process.env.FRACTAL_PROGRAM_BOOT_ID
  const deadline = Number(process.env.FRACTAL_PROGRAM_WORKER_DEADLINE_MS)
  const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`)
  /** By step-run subject: the attempt this host runs. */
  const runs = new Map()
  /** Attempts awaiting actual worker exit, including attempts replaced for the same step. */
  const ending = new Map()
  const attemptKey = (step, attempt) => JSON.stringify([step, attempt])

  const record = (run, pid) => {
    const path = join(groupsDir, `${pid}.json`)
    const body = JSON.stringify({ pid, boot_id: bootId, start_time: startTime(pid), step: run.step, attempt: run.attempt })
    writeFileSync(`${path}.tmp`, body)
    renameSync(`${path}.tmp`, path)
  }
  const killGroup = (pid) => {
    try {
      process.kill(-pid, 'SIGKILL')
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
    rmSync(join(groupsDir, `${pid}.json`), { force: true })
  }
  /** End an attempt: its worker and every effect group it started. */
  const end = (run) => {
    if (run.termination !== undefined) return run.termination
    run.ended = true
    clearInterval(run.ping)
    const key = attemptKey(run.step, run.attempt)
    ending.set(key, run)
    send({ t: 'ending', step: run.step, attempt: run.attempt })
    // Native I/O can delay termination. Keep this exact attempt addressable without
    // blocking the host or letting a replacement overwrite its exit proof.
    run.termination = run.worker.terminate().then(() => {
      if (runs.get(run.step) === run) runs.delete(run.step)
      ending.delete(key)
      send({ t: 'cancelled', step: run.step, attempt: run.attempt })
    })
    for (const pid of run.groups) killGroup(pid)
    run.groups.clear()
    return run.termination
  }

  const spawnEffect = (run, { id, argv: requested, options = {} }) => {
    const reply = (message) => run.ended || run.worker.postMessage({ t: 'spawned', id, ...message })
    const argv =
      options.priority === 'background'
        ? process.platform === 'darwin'
          ? ['taskpolicy', '-b', ...requested]
          : ['nice', '-n', '19', ...requested]
        : requested
    let child
    try {
      child = spawn(argv[0], argv.slice(1), {
        cwd: options.cwd ?? run.cwd,
        env: { ...process.env, ...options.env, ...run.env },
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: true,
      })
    } catch (error) {
      return reply({ error: String(error) })
    }
    const pid = child.pid
    if (pid !== undefined) {
      record(run, pid)
      run.groups.add(pid)
    }
    const out = []
    const err = []
    child.stdout.on('data', (chunk) => out.push(chunk))
    child.stderr.on('data', (chunk) => err.push(chunk))
    child.on('error', (error) => reply({ error: String(error) }))
    // A group ends with its leader, so nothing it started outlives the step's effect.
    child.on('exit', () => {
      if (pid === undefined) return
      run.groups.delete(pid)
      killGroup(pid)
    })
    child.on('close', (code, signal) =>
      reply({ result: { code: code ?? 128, signal, stdout: Buffer.concat(out).toString(), stderr: Buffer.concat(err).toString() } }),
    )
  }

  const start = (message) => {
    const previous = runs.get(message.step)
    if (previous !== undefined) end(previous)
    // Context is per attempt, never a mutation of the shared host's environment.
    const worker = new Worker(fileURLToPath(import.meta.url), {
      workerData: message,
      env: { ...process.env, ...message.env },
    })
    const run = {
      step: message.step,
      attempt: message.attempt,
      cwd: message.cwd,
      env: message.env,
      worker,
      groups: new Set(),
      ended: false,
      answered: Date.now(),
      ping: undefined,
    }
    runs.set(message.step, run)
    const ids = { step: run.step, attempt: run.attempt }
    worker.on('message', (message) => {
      if (run.ended) return
      switch (message.t) {
        case 'pong':
          run.answered = Date.now()
          return
        case 'spawn':
          return spawnEffect(run, message)
        case 'done':
          end(run)
          return send({ t: 'done', ...ids, exit: message.exit })
        case 'journal':
        case 'run':
        case 'ask':
          // The daemon decides; the worker waits for its answer.
          return send({ ...message, ...ids })
        default:
          return // Host-control messages must never be forwarded from a worker.
      }
    })
    worker.on('error', (error) => {
      if (run.ended) return
      end(run)
      send({ t: 'done', ...ids, exit: defect(error) })
    })
    worker.on('exit', () => {
      if (run.ended) return
      end(run)
      send({ t: 'done', ...ids, exit: defect('the program worker exited without a result') })
    })
    run.ping = setInterval(() => {
      if (run.ended) return
      if (Date.now() - run.answered > deadline) {
        // A busy loop fails its own step once instead of looping across restarts.
        end(run)
        return send({ t: 'unresponsive', ...ids, ms: deadline })
      }
      run.worker.postMessage({ t: 'ping' })
    }, Math.max(10, Math.floor(deadline / 4)))
  }

  const lines = createInterface({ input: process.stdin })
  lines.on('line', (line) => {
    const message = JSON.parse(line)
    if (message.t === 'ping') return send({ t: 'pong', seq: message.seq })
    if (message.t === 'start') return start(message)
    const run = runs.get(message.step)
    if (message.t === 'cancel') {
      const attempt = ending.get(attemptKey(message.step, message.attempt))
        ?? (run?.attempt === message.attempt ? run : undefined)
      if (attempt !== undefined) return end(attempt)
      return send({ t: 'cancelled', step: message.step, attempt: message.attempt })
    }
    if (run === undefined || run.attempt !== message.attempt || run.ended) return // another or ended attempt
    run.worker.postMessage(message) // ack, nack, resolved
  })
  // The daemon is gone: its replacement resumes every program from the journal. End all effects now.
  lines.on('close', () => {
    for (const run of new Set([...runs.values(), ...ending.values()])) end(run)
    process.exit(0)
  })
}

const worker = async () => {
  const start = workerData
  /** By sequence number: journal entries waiting for ack, requests waiting for `resolved`. */
  const waiting = new Map()
  const spawns = new Map()
  let seq = 0
  let spawned = 0
  const controller = new AbortController()
  parentPort.on('message', (message) => {
    if (message.t === 'ping') return parentPort.postMessage({ t: 'pong' })
    if (message.t === 'spawned') {
      const pending = spawns.get(message.id)
      spawns.delete(message.id)
      return message.error === undefined ? pending?.resolve(message.result) : pending?.reject(new Error(message.error))
    }
    const pending = waiting.get(message.seq)
    waiting.delete(message.seq)
    if (message.t === 'ack') pending?.resolve()
    else if (message.t === 'nack') pending?.reject(new Error(message.error))
    else if (message.t === 'resolved') pending?.resolve(message.exit)
  })
  /** Ask the daemon and wait: a journal entry is durable only once the daemon acknowledges it. */
  const request = (message) =>
    new Promise((resolve, reject) => {
      const n = ++seq
      waiting.set(n, { resolve, reject })
      parentPort.postMessage({ ...message, seq: n })
    })
  let exit
  try {
    const actual = createHash('sha256').update(readFileSync(start.module)).digest('hex')
    if (actual !== start.sha256) throw new Error(`bundle ${start.module} has sha256 ${actual}, the run pins ${start.sha256}`)
    const program = (await import(start.module)).default?.[ENTRY]
    if (typeof program !== 'function') throw new Error(`${start.module} does not export a Fractal program`)
    exit = await program({
      step: start.step,
      attempt: start.attempt,
      run: start.run,
      payload: start.payload,
      env: start.env,
      cwd: start.cwd,
      journal: new Map(
        start.journal.map((entry) => [
          entry.key,
          entry._tag === 'Success' ? { _tag: 'Success', value: entry.value } : { _tag: 'Failure', error: entry.error },
        ]),
      ),
      signal: controller.signal,
      started: (key) => parentPort.postMessage({ t: 'journal', key, status: 'started' }),
      record: (entry) => request({ t: 'journal', ...entry }),
      child: (child) => request({ t: 'run', key: child.key, mission: child.mission, input: child.input }),
      ask: (ask) => request({ t: 'ask', ...ask }),
      spawn: (argv, options) =>
        new Promise((resolve, reject) => {
          const id = ++spawned
          spawns.set(id, { resolve, reject })
          parentPort.postMessage({ t: 'spawn', id, argv, options })
        }),
    })
  } catch (error) {
    exit = defect(error)
  }
  parentPort.postMessage({ t: 'done', exit })
}

if (isMainThread) main()
else void worker()
