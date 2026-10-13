import assert from "node:assert/strict"
import { test } from "node:test"
import { run } from "./matrix.ts"

const { cells } = await run()
const cell = (checkpoint: string, version: string) => cells.find((c) => c.checkpoint === checkpoint && c.version === version)!

test("code ahead of a run's position moves it to the new version", () => {
  assert.equal(cell("building", "v2-insert").report.verdict, "compatible")
  assert.ok(cell("building", "v2-insert").opsAfter.includes("security-scan"))
  assert.equal(cell("in review", "v2-rounds").report.verdict, "compatible")
})

test("code inserted behind a run's position cannot replay, so the run stays on its version", () => {
  const c = cell("fixing round 1", "v2-insert")
  assert.equal(c.report.verdict, "incompatible")
  assert.equal(c.ranUnder, "v1")
  assert.equal(c.outcome, "shipped, 3 rounds")
})

test("patched() makes the same insertion safe everywhere, and only new paths take it", () => {
  for (const checkpoint of ["building", "in review", "fixing round 1", "last CI round"])
    assert.equal(cell(checkpoint, "v2-patched").report.verdict, "compatible", checkpoint)
  assert.ok(cell("in review", "v2-patched").opsAfter.includes("security-scan"))
  assert.ok(!cell("fixing round 1", "v2-patched").opsAfter.includes("security-scan"))
})

test("removing an op the run already scheduled is incompatible at every point", () => {
  for (const checkpoint of ["building", "in review", "fixing round 1", "last CI round"])
    assert.equal(cell(checkpoint, "v2-remove").report.verdict, "incompatible", checkpoint)
})

test("a changed definition moves finished runs and holds an op that is still open", () => {
  assert.equal(cell("fixing round 1", "v2-text").report.verdict, "open-op-changed")
  assert.equal(cell("last CI round", "v2-text").report.verdict, "compatible-with-changes")
  assert.equal(cell("last CI round", "v2-text").outcome, "shipped, 3 rounds")
})
