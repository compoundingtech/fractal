// A program version is one self-contained bundle: the program, the SDK and the decide() engine
// that replays it, with Effect inside. Its version is the SHA-256 of the bundle bytes, the same
// hash st gives the bundle when it is stored as a document, so `doc/NAME@HASH` names it exactly.
// A bundle brings its own engine, so a later SDK cannot change how an old version replays.

import { createHash } from "node:crypto"
import { mkdirSync, writeFileSync } from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { pathToFileURL } from "node:url"
import * as esbuild from "esbuild"
import type { DecideOptions, Program } from "../sdk/engine.ts"
import type { Decision, HistoryEvent } from "../sdk/history.ts"

export interface Version {
  readonly hash: string
  readonly file: string
  readonly program: Program
  readonly decide: (program: Program, history: ReadonlyArray<HistoryEvent>, options?: DecideOptions) => Decision
}

const engine = resolve(import.meta.dirname, "../sdk/engine.ts")

export const bundle = async (programFile: string, outDir = join(import.meta.dirname, "out")): Promise<Version> => {
  const result = await esbuild.build({
    stdin: {
      contents: `export { default as program } from ${JSON.stringify(resolve(programFile))}\nexport { decide } from ${JSON.stringify(engine)}\n`,
      resolveDir: dirname(resolve(programFile)),
      loader: "ts"
    },
    bundle: true,
    format: "esm",
    platform: "neutral",
    target: "es2022",
    write: false,
    nodePaths: [resolve(import.meta.dirname, "../node_modules")]
  })
  const code = result.outputFiles[0]!.text
  const hash = createHash("sha256").update(code).digest("hex")
  mkdirSync(outDir, { recursive: true })
  const file = join(outDir, `${basename(programFile, ".ts")}@${hash.slice(0, 16)}.mjs`)
  writeFileSync(file, code)
  return { hash, file, ...(await load(file)) }
}

export const load = async (file: string): Promise<Pick<Version, "program" | "decide">> => {
  const mod = await import(pathToFileURL(file).href)
  return { program: mod.program, decide: mod.decide }
}
