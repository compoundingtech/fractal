import { availableParallelism } from 'node:os'
import { readFileSync } from 'node:fs'
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads'

// Fixed work, not a duration target: four full traversals of a 32 MiB pointer table.
// Allocation and table initialization happen before the ready/start barrier.
export const calibrationWorkload = Object.freeze({ memoryBytes: 32 * 1024 * 1024, steps: 32 * 1024 * 1024 })
const HASH_MULTIPLIER = 0x9e3779b1
const READY = 0
const START = 1

function positiveInteger(value, name) {
  if (!Number.isSafeInteger(value) || value < 1) throw new Error(`${name} must be a positive safe integer`)
}

function wordCount({ memoryBytes, steps }) {
  positiveInteger(memoryBytes, 'memoryBytes')
  positiveInteger(steps, 'steps')
  const words = memoryBytes / Uint32Array.BYTES_PER_ELEMENT
  if (!Number.isInteger(words) || words < 2 || words > 2 ** 26 || (words & (words - 1)) !== 0) {
    throw new Error('memoryBytes must hold a power-of-two number of words between 2 and 2^26')
  }
  if (steps % words !== 0) throw new Error('steps must be a whole number of pointer-table traversals')
  return words
}

export function createCalibrationMemory(workload) {
  const words = wordCount(workload)
  const memory = new Uint32Array(words)
  // This LCG is a full-cycle permutation for a power-of-two table (a = 1 mod 4,
  // c odd). Each load depends on the previous load, defeating prefetch/parallel loads.
  for (let index = 0; index < words; index++) {
    memory[index] = (Math.imul(index, 1664525) + 1013904223) & (words - 1)
  }
  return memory
}

export function calibrationKernel(memory, steps) {
  if (!(memory instanceof Uint32Array)) throw new Error('memory must be a Uint32Array')
  wordCount({ memoryBytes: memory.byteLength, steps })
  let pointer = 0
  let checksum = 0
  for (let step = 0; step < steps; step++) {
    pointer = memory[pointer]
    if (pointer >= memory.length) throw new Error('corrupt calibration pointer')
    checksum = (checksum + Math.imul(pointer ^ (pointer >>> 16), HASH_MULTIPLIER)) >>> 0
  }
  return checksum
}

export function expectedCalibrationChecksum(workload) {
  const words = wordCount(workload)
  // x ^ (x >>> 16) permutes this power-of-two range. Thus a complete
  // traversal's additive integer hash has this independently derived sum.
  const sum = ((words * (words - 1)) / 2) >>> 0
  return Math.imul(Math.imul(sum, HASH_MULTIPLIER), workload.steps / words) >>> 0
}

export function validateCalibrationResult(result, workload) {
  wordCount(workload)
  if (result?.memoryBytes !== workload.memoryBytes || result?.steps !== workload.steps) {
    throw new Error('calibration worker workload mismatch')
  }
  if (result.checksum !== expectedCalibrationChecksum(workload)) {
    throw new Error('calibration worker checksum mismatch')
  }
  if (!Number.isFinite(result.cpuMilliseconds) || result.cpuMilliseconds < 0 || typeof result.finishedNanoseconds !== 'bigint') {
    throw new Error('invalid calibration worker timing')
  }
  return result
}

export function releaseCalibrationBarrier(control, workerCount) {
  positiveInteger(workerCount, 'workerCount')
  if (Atomics.load(control, READY) !== workerCount) throw new Error('calibration workers are not all ready')
  if (Atomics.compareExchange(control, START, 0, 1) !== 0) throw new Error('calibration workers already started')
  Atomics.notify(control, START, workerCount)
}

function threadCpuMilliseconds() {
  if (typeof process.threadCpuUsage === 'function') {
    const { user, system } = process.threadCpuUsage()
    return (user + system) / 1000
  }
  // Older Node on Linux: schedstat's first field is this calling thread's
  // scheduled CPU runtime in nanoseconds, including user and kernel execution.
  // Unlike process.cpuUsage(), this does not count concurrent sibling workers.
  if (process.platform !== 'linux') throw new Error('calibration requires process.threadCpuUsage or Linux thread schedstat')
  const runtime = readFileSync('/proc/thread-self/schedstat', 'utf8').trim().split(/\s+/)[0]
  if (!/^\d+$/.test(runtime)) throw new Error('invalid Linux thread schedstat runtime')
  return Number(BigInt(runtime)) / 1e6
}

// Exported small-work seam: production always passes calibrationWorkload.
export async function runCalibrationSample({ workerCount, workload }) {
  positiveInteger(workerCount, 'workerCount')
  wordCount(workload)
  const control = new Int32Array(new SharedArrayBuffer(2 * Int32Array.BYTES_PER_ELEMENT))
  const workers = []
  const ready = []
  const results = []
  try {
    for (let index = 0; index < workerCount; index++) {
      const worker = new Worker(new URL(import.meta.url), {
        workerData: { kitCalibration: true, control: control.buffer, workload },
      })
      workers.push(worker)
      let resolveReady
      let rejectReady
      ready.push(new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject }))
      results.push(new Promise((resolve, reject) => {
        let received = false
        worker.on('message', (message) => {
          if (message.type === 'ready') resolveReady()
          else if (message.type === 'result') {
            received = true
            resolve(message.result)
          } else {
            const error = new Error('unexpected calibration worker message')
            rejectReady(error)
            reject(error)
          }
        })
        worker.on('error', (error) => { rejectReady(error); reject(error) })
        worker.on('exit', (code) => {
          if (code !== 0 || !received) {
            const error = new Error(`calibration worker exited without a result (code ${code})`)
            rejectReady(error)
            reject(error)
          }
        })
      }))
    }
    // Attach both rejection paths immediately: a worker can fail during setup,
    // before its ready message, or during the workload after the barrier.
    const samplesPromise = Promise.all(results)
    const readyPromise = Promise.all(ready)
    await Promise.race([readyPromise, samplesPromise])
    const startedNanoseconds = process.hrtime.bigint()
    releaseCalibrationBarrier(control, workerCount)
    const samples = (await samplesPromise).map((sample) => validateCalibrationResult(sample, workload))
    let finishedNanoseconds = startedNanoseconds
    for (const sample of samples) {
      if (sample.finishedNanoseconds < startedNanoseconds) throw new Error('calibration worker finished before start')
      if (sample.finishedNanoseconds > finishedNanoseconds) finishedNanoseconds = sample.finishedNanoseconds
    }
    return {
      wallMilliseconds: Number(finishedNanoseconds - startedNanoseconds) / 1e6,
      cpuMilliseconds: samples.reduce((sum, sample) => sum + sample.cpuMilliseconds, 0),
      checksums: samples.map((sample) => sample.checksum),
    }
  } finally {
    // Observe setup promises even if constructing a later Worker threw before
    // the aggregate handlers were installed. Termination settles pending ones.
    const settled = Promise.allSettled([...ready, ...results])
    await Promise.all(workers.map((worker) => worker.terminate()))
    await settled
  }
}

export async function runKitCalibration({ cpuCount = availableParallelism() } = {}) {
  positiveInteger(cpuCount, 'cpuCount')
  const single = await runCalibrationSample({ workerCount: 1, workload: calibrationWorkload })
  const all = await runCalibrationSample({ workerCount: cpuCount, workload: calibrationWorkload })
  return {
    version: 1,
    cpuCount,
    workload: { ...calibrationWorkload },
    single: { wallMilliseconds: single.wallMilliseconds, cpuMilliseconds: single.cpuMilliseconds, checksum: single.checksums[0] },
    all,
  }
}

if (!isMainThread && workerData?.kitCalibration === true) {
  const { workload } = workerData
  const memory = createCalibrationMemory(workload)
  // Ensure the CPU measurement source works before announcing readiness.
  threadCpuMilliseconds()
  const control = new Int32Array(workerData.control)
  Atomics.add(control, READY, 1)
  parentPort.postMessage({ type: 'ready' })
  while (Atomics.load(control, START) === 0) Atomics.wait(control, START, 0)
  const startedCpuMilliseconds = threadCpuMilliseconds()
  const checksum = calibrationKernel(memory, workload.steps)
  const finishedNanoseconds = process.hrtime.bigint()
  const cpuMilliseconds = threadCpuMilliseconds() - startedCpuMilliseconds
  parentPort.postMessage({ type: 'result', result: { ...workload, checksum, cpuMilliseconds, finishedNanoseconds } })
  parentPort.close()
}
