import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  calibrationWorkload,
  createCalibrationMemory,
  calibrationKernel,
  expectedCalibrationChecksum,
  validateCalibrationResult,
  releaseCalibrationBarrier,
  runCalibrationSample,
  runKitCalibration,
} from './kit-play-calibration.mjs'

const tinyWorkload = { memoryBytes: 16, steps: 4 }

function validResult(overrides = {}) {
  return {
    ...tinyWorkload,
    checksum: 3041712678,
    cpuMilliseconds: 0.5,
    finishedNanoseconds: 10n,
    ...overrides,
  }
}

test('the fixed production workload does not adapt to CPU count or elapsed time', () => {
  assert.deepEqual(calibrationWorkload, { memoryBytes: 33554432, steps: 33554432 })
  assert.equal(Object.isFrozen(calibrationWorkload), true)
})

test('the tiny pointer chase and integer hash have a fixed known result', () => {
  const memory = createCalibrationMemory(tinyWorkload)
  assert.deepEqual([...memory], [3, 0, 1, 2])
  assert.equal(calibrationKernel(memory, tinyWorkload.steps), 3041712678)
  assert.equal(expectedCalibrationChecksum(tinyWorkload), 3041712678)
  assert.equal(calibrationKernel(memory, 8), 1788458060)
  assert.equal(expectedCalibrationChecksum({ ...tinyWorkload, steps: 8 }), 1788458060)
})

test('independent expected sums match complete traversals at different small sizes', () => {
  for (const memoryBytes of [8, 16, 32, 64, 256, 1024]) {
    for (const traversals of [1, 2, 3]) {
      const workload = { memoryBytes, steps: (memoryBytes / 4) * traversals }
      assert.equal(calibrationKernel(createCalibrationMemory(workload), workload.steps), expectedCalibrationChecksum(workload))
    }
  }
})

test('invalid workloads are rejected before allocation or worker startup', async () => {
  for (const workload of [
    { memoryBytes: 0, steps: 4 },
    { memoryBytes: 12, steps: 3 },
    { memoryBytes: 16, steps: 3 },
    { memoryBytes: 16, steps: 0 },
    { memoryBytes: 16, steps: 4.5 },
  ]) {
    assert.throws(() => createCalibrationMemory(workload), /memoryBytes|steps/)
    await assert.rejects(runCalibrationSample({ workerCount: 1, workload }), /memoryBytes|steps/)
  }
  for (const cpuCount of [0, -1, 1.5, NaN, Infinity, '2']) {
    await assert.rejects(runKitCalibration({ cpuCount }), /cpuCount/)
    await assert.rejects(runCalibrationSample({ workerCount: cpuCount, workload: tinyWorkload }), /workerCount/)
  }
})

test('corrupt pointers and incorrect worker results cannot become calibration samples', () => {
  const memory = createCalibrationMemory(tinyWorkload)
  memory[0] = memory.length
  assert.throws(() => calibrationKernel(memory, tinyWorkload.steps), /corrupt calibration pointer/)
  memory[0] = 0
  assert.notEqual(calibrationKernel(memory, tinyWorkload.steps), expectedCalibrationChecksum(tinyWorkload))
  assert.equal(validateCalibrationResult(validResult(), tinyWorkload).checksum, 3041712678)
  for (const overrides of [{ memoryBytes: 32 }, { steps: 8 }, { checksum: 0 }, { checksum: '3041712678' }]) {
    assert.throws(() => validateCalibrationResult(validResult(overrides), tinyWorkload), /mismatch/)
  }
  for (const overrides of [{ cpuMilliseconds: NaN }, { cpuMilliseconds: -1 }, { cpuMilliseconds: Infinity }, { finishedNanoseconds: 10 }]) {
    assert.throws(() => validateCalibrationResult(validResult(overrides), tinyWorkload), /timing/)
  }
})

test('the start barrier refuses early release and can only be released once', () => {
  const control = new Int32Array(new SharedArrayBuffer(8))
  assert.throws(() => releaseCalibrationBarrier(control, 2), /not all ready/)
  assert.equal(Atomics.load(control, 1), 0)
  Atomics.add(control, 0, 1)
  assert.throws(() => releaseCalibrationBarrier(control, 2), /not all ready/)
  assert.equal(Atomics.load(control, 1), 0)
  Atomics.add(control, 0, 1)
  releaseCalibrationBarrier(control, 2)
  assert.equal(Atomics.load(control, 1), 1)
  assert.throws(() => releaseCalibrationBarrier(control, 2), /already started/)
})

test('real small workers join with identical checksums and separate wall/CPU units', async () => {
  const sample = await runCalibrationSample({ workerCount: 2, workload: tinyWorkload })
  assert.deepEqual(sample.checksums, [3041712678, 3041712678])
  assert.ok(Number.isFinite(sample.wallMilliseconds) && sample.wallMilliseconds > 0)
  assert.ok(Number.isFinite(sample.cpuMilliseconds) && sample.cpuMilliseconds >= 0)
  assert.deepEqual(Object.keys(sample).sort(), ['checksums', 'cpuMilliseconds', 'wallMilliseconds'])
})
