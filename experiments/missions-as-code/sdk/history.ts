// The durable record of one program run, and the decision a program makes from it.
//
// A run's history is an append-only list of events. The program never writes it. A driver
// enacts the program's commands (start a step, arm a timer, cancel a step) and appends what
// happened. `decide` replays the history through the program and returns what should be open
// now. Everything here is plain JSON so the same history can cross the Rust bridge.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json }

// What a program can wait on. Each kind maps onto something st already runs today.
export type OpKind =
  | "step" // agent work: a step assigned to a declared seat
  | "gate" // a check: exec, built-in or graph predicate, or a person's approval
  | "ask" // a person's answer to a structured question
  | "wait" // a graph condition such as a merged pull request or a passing CI check
  | "timer" // a durable sleep; Effect.sleep, Effect.timeout and Schedule delays all use it
  | "marker" // patched(id): records that a run took a code path added by a later version
  | "beat" // a scripted demo action for a player to perform (exp6-port/demo)

export type HistoryEvent =
  | { type: "started"; at: number; program: string; version: string; input: Json }
  // The driver enacted a command. `hash` fingerprints the op's spec so a replay can tell a
  // changed program from the one that asked.
  | { type: "scheduled"; at: number; op: string; kind: OpKind; hash: string }
  | { type: "completed"; at: number; op: string; value: Json }
  | { type: "failed"; at: number; op: string; error: OpError }
  | { type: "cancelled"; at: number; op: string }
  // A running program moved to another version; see experiment 4. `accepted` lists finished ops
  // the new version defines differently, whose recorded results stand.
  | { type: "upgraded"; at: number; version: string; accepted?: ReadonlyArray<string> }

export interface OpError {
  readonly reason: string
  readonly attempt?: number
  readonly evidence?: ReadonlyArray<string>
}

export interface OpCommand {
  readonly type: "schedule"
  readonly op: string
  readonly kind: OpKind
  readonly spec: Json
  readonly hash: string
}

export interface CancelCommand {
  readonly type: "cancel"
  readonly op: string
}

export type Command = OpCommand | CancelCommand

export type Status =
  | { state: "running" }
  | { state: "completed"; value: Json }
  | { state: "failed"; error: Json }

export interface Divergence {
  // The history index where replay stopped agreeing with the program.
  readonly at: number
  readonly op: string
  readonly reason: string
}

export interface Decision {
  readonly status: Status
  // Ops the program waits on right now, in the order it asked for them.
  readonly open: ReadonlyArray<{ op: string; kind: OpKind; spec: Json; scheduled: boolean }>
  // What the driver must do: schedule new ops and cancel ops nobody waits on any more.
  readonly commands: ReadonlyArray<Command>
  // Present when the history does not replay through this program.
  readonly divergence?: Divergence
  // With onChangedSpec: "report", ops whose definition differs from what the history scheduled.
  readonly changes?: ReadonlyArray<{ op: string; kind: OpKind; settled: boolean }>
  // Deterministic counters, useful for measuring per-step cost.
  readonly stats: { events: number; ops: number; drains: number }
}
