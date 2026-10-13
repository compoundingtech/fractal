import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "node:test"
import alert from "./an-alert-needs-an-answer.ts"
import { compile } from "./compile.ts"
import releaseDay from "./release-day.ts"

// Whitespace and comments aside, the beats block line by line.
const beats = (text: string) =>
  text.slice(text.indexOf("beats {")).split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("//"))
const original = (name: string) => readFileSync(new URL(name, import.meta.url), "utf8")

test("release-day as a program compiles to the demo mission's own beats", () => {
  assert.deepEqual(beats(compile(releaseDay)), beats(original("release-day.kdl")))
})

test("the alert demo's 'tonight' path is the demo mission's file", () => {
  assert.deepEqual(beats(compile(alert, ["tonight"])), beats(original("an-alert-needs-an-answer.kdl")))
})

test("the program also has the path the file cannot: Robin answers 'now'", () => {
  const now = beats(compile(alert, ["now"]))
  assert.ok(now.some((l) => l.includes("./run-migration --index due_date")))
  assert.ok(!now.some((l) => l.includes("tonight at 02:00\"") && l.startsWith("say")))
})
