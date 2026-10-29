import type { Agent, Attention, Mission, MissionActionOffer, MissionPresentation, MissionStep, MissionStepPresentation, PresentationActor } from '@fractal/client'

const inHand = (state: string): boolean => ['claimed', 'working', 'running', 'verifying'].includes(state)
const finished = (state: string): boolean => ['completed', 'failed', 'cancelled'].includes(state)
const rank: Record<MissionStepPresentation['word'], number> = { done: 0, cancelled: 0, needs_you: 1, failed: 1, working: 2, ready: 3, waiting: 4, pending: 5 }

interface Seat {
  readonly id: string
  readonly label: string
  readonly state: string
  readonly down: boolean
  readonly busy: Agent['current_work']
}

/** Same precedence and wire fields as fractal-ui-model missions::{word,presentation::present}. */
export const missionPresentation = (mission: Mission, agents: readonly Agent[], attention: readonly Attention[]): MissionPresentation => {
  const seats = new Map(agents.map((agent): [string, Seat] => [agent.id, {
    id: agent.id, label: agent.name, state: agent.state, down: ['failed', 'stopped'].includes(agent.state) || agent.fault != null, busy: agent.current_work,
  }]))
  const queues = new Map<string, Seat>()
  for (const agent of agents) for (const id of [agent.next_work_id, ...(agent.upcoming_work_ids ?? [])]) {
    const seat = seats.get(agent.id)
    if (id != null && seat !== undefined && !queues.has(id)) queues.set(id, seat)
  }
  const runs = mission.run_details ?? []
  const open = runs.filter((run) => !finished(run.status))
  const work = (open.length > 0 ? open : runs.slice(-1)).flatMap((run) => run.steps ?? [])
  const gates = attention.filter((item) => item.state === 'open' && item.attention_kind === 'human-gate' && item.mission_id === mission.id)
  const person = (id: string): PresentationActor => ({ kind: 'person', person: id, label: id.replace(/^person\//u, '') })
  const actor = (seat: Seat | undefined): PresentationActor => seat === undefined ? { kind: 'nobody' } : { kind: 'agent', agent: seat.id, label: seat.label }
  const seatOf = (step: MissionStep): Seat | undefined => {
    if (step.agentless) return undefined
    const held = step.claimant ?? step.assignee
    const seat = held == null ? undefined : seats.get(held) ?? { id: held, label: held.replace(/^agent\//u, ''), state: 'unknown', down: false, busy: [] }
    return step.claimant != null || step.state !== 'ready' ? seat : queues.get(step.id) ?? seat
  }
  const stepWord = (step: MissionStep, gated: boolean): MissionStepPresentation['word'] => {
    if (gated) return 'needs_you'
    if (step.state === 'completed') return 'done'
    if (inHand(step.state)) return 'working'
    if (step.state === 'ready') return 'ready'
    if (['waiting', 'pending', 'blocked'].includes(step.state)) return 'waiting'
    if (step.state === 'cancelled' || step.state === 'failed') return step.state
    return 'pending'
  }
  const rows = work.map((step) => {
    const gate = gates.find((item) => item.step_run_id === step.id)
    const word = stepWord(step, gate !== undefined)
    const seat = seatOf(step)
    const queued = step.state === 'ready' && step.claimant == null ? queues.get(step.id) : undefined
    const busy = queued?.busy?.[0]
    const note = queued === undefined ? ['waiting', 'blocked'].includes(step.state) ? step.blocked_reason ?? null : null
      : busy !== undefined && !queued.down ? `queued for ${queued.label}, which is busy with ${busy.mission_id.replace(/^mission\//u, '')} › ${busy.path}`
        : `queued for ${queued.label}, which is ${queued.state}`
    const value: MissionStepPresentation = {
      id: step.id, path: step.path, title: step.title ?? null, word,
      owner: gate !== undefined ? person(gate.person_id) : step.agentless ? { kind: 'st' } : actor(seat),
      note, attempts: step.attempt, blockers: step.blockers ?? [], keep_open: step.keep_open ?? false,
    }
    return { value, seat }
  }).sort((a, b) => rank[a.value.word] - rank[b.value.word])
  const ready = work.find((step) => step.state === 'ready' && step.claimant == null)
  const queue = ready === undefined ? undefined : queues.get(ready.id)
  const word = ((): MissionPresentation['word'] => {
    if (gates.length > 0) return 'decision'
    if (mission.runs.length === 0 && runs.length === 0 && work.length === 0) return 'not_started'
    if (mission.state === 'blocked' || work.some((step) => step.state === 'blocked')) return 'stalled'
    if (mission.state === 'completed') return 'done'
    if (mission.state === 'failed' || work.some((step) => step.state === 'failed')) return 'failed'
    if (mission.state === 'cancelled') return 'cancelled'
    if (work.length > 0 && work.every((step) => step.state === 'completed' || (inHand(step.state) && step.agentless && step.keep_open)) &&
      work.some((step) => step.state !== 'completed')) return 'watching'
    if (work.some((step) => inHand(step.state))) return 'working'
    if (ready !== undefined) return queue === undefined ? 'unclaimed' : queue.down ? 'unstaffed' : 'queued'
    if (work.some((step) => ['waiting', 'pending'].includes(step.state))) return 'held'
    return ['standing', 'running', 'ready', 'draft'].includes(mission.state) ? 'idle' : 'done'
  })()
  const stuckRow = ['stalled', 'failed', 'unstaffed', 'queued'].includes(word) ? rows.find(({ value }) => ['failed', 'ready', 'waiting'].includes(value.word)) : undefined
  const stuck = stuckRow === undefined ? null : {
    step: stuckRow.value.id, word: stuckRow.value.word, attempts: stuckRow.value.attempts, blockers: stuckRow.value.blockers,
    agent: stuckRow.seat === undefined ? null : { id: stuckRow.seat.id, label: stuckRow.seat.label, state: stuckRow.seat.state },
  }
  const card = ['stalled', 'failed', 'unstaffed'].includes(word) ? 'what-you-can-do' : word === 'queued' ? 'nothing-to-do' : null
  const actions: MissionActionOffer[] = []
  if (card === 'what-you-can-do' && stuck !== null) {
    if (stuck.word === 'failed') actions.push({ id: 'retry-step', action: 'work.retry', target: stuck.step, confirm: false })
    const seat = stuckRow?.seat
    if (seat !== undefined) {
      if (seat.down) actions.push({ id: 'restart-agent', action: seat.state === 'stopped' ? 'agent.start' : 'runtime.restart', target: seat.id, confirm: true })
      actions.push({ id: 'chat', action: 'message.send', target: seat.id, confirm: false })
    }
    const run = runs.find((run) => run.steps?.some((step) => step.id === stuck.step))
    if (run !== undefined) actions.push({ id: 'cancel-run', action: 'mission.cancel', target: run.id, confirm: true })
  }
  const busy = queue?.busy?.[0]
  return {
    word, system: mission.system ?? false,
    who: word === 'decision' ? person(gates[0]!.person_id)
      : word === 'working' || word === 'watching' ? rows.find(({ value }) => value.word === 'working')?.value.owner ?? { kind: 'nobody' }
        : ['queued', 'unstaffed', 'stalled', 'failed'].includes(word) ? actor(stuckRow?.seat) : { kind: 'nobody' },
    steps: rows.map(({ value }) => value), stuck,
    queued_behind: word !== 'queued' || queue === undefined ? null : {
      agent: queue.id, label: queue.label,
      busy_with: busy === undefined || queue.down ? null : { mission: busy.mission_id, path: busy.path, label: `${busy.mission_id.replace(/^mission\//u, '')} › ${busy.path}` },
      state: busy === undefined || queue.down ? queue.state : null,
    },
    card, actions, progress: { done: rows.filter(({ value }) => value.word === 'done').length, total: rows.length },
    last_progress: work.find((step) => inHand(step.state) && step.last_progress != null)?.last_progress ?? null,
    settled_at: finished(mission.state) ? mission.updated_at : null,
  }
}
