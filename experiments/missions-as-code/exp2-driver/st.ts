// A thin, synchronous wrapper over the st CLI: the same public surface a person or agent uses.

import { execFileSync } from "node:child_process"

export interface StOptions {
  readonly bin: string
  readonly actor: string
  readonly env?: NodeJS.ProcessEnv
}

export class St {
  calls = 0
  callMs = 0
  readonly options: StOptions
  constructor(options: StOptions) {
    this.options = options
  }

  run(args: ReadonlyArray<string>, input?: string): string {
    const started = Date.now()
    this.calls++
    try {
      return execFileSync(this.options.bin, args as Array<string>, {
        encoding: "utf8",
        input,
        env: this.options.env ?? process.env,
        stdio: ["pipe", "pipe", "pipe"],
        maxBuffer: 64 << 20
      })
    } catch (error: any) {
      const detail = `${error.stderr ?? ""}${error.stdout ?? ""}`.trim()
      throw new StError(args, error.status ?? -1, detail)
    } finally {
      this.callMs += Date.now() - started
    }
  }

  json<T = any>(args: ReadonlyArray<string>): T {
    return JSON.parse(this.run([...args, "--json"]))
  }

  jsonLines<T = any>(args: ReadonlyArray<string>): Array<T> {
    return this.run([...args, "--json"])
      .split("\n")
      .filter((line) => line.trim().startsWith("{"))
      .map((line) => JSON.parse(line))
  }

  as(): Array<string> {
    return ["--as", this.options.actor]
  }
}

export class StError extends Error {
  readonly args: ReadonlyArray<string>
  readonly status: number
  readonly detail: string
  constructor(args: ReadonlyArray<string>, status: number, detail: string) {
    super(`st ${args.slice(0, 3).join(" ")} failed (${status}): ${detail.split("\n").slice(-3).join(" | ")}`)
    this.args = args
    this.status = status
    this.detail = detail
  }
}
