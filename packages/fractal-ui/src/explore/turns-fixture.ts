import type { TextItem } from '../assistant-ui/embrace-data/model'
import type { WorkLogTurn } from '../assistant-ui/taste/work-log'

export type TurnKind = 'Person' | 'Agent' | 'Relay' | 'System' | 'Seat'
export interface TurnBlock { readonly id: string; readonly kind: TurnKind; readonly sender: string; readonly text: string; readonly at: string; readonly verified: boolean; readonly subject?: string }
export const turnNames: Readonly<Record<string, string>> = {
  'relay-hold': 'Merge hold announcement',
  'reply-hold': 'Merge hold reply',
  'relay-release': 'Merge release announcement',
  'reply-release': 'Merge release reply',
  'relay-routing': 'Model routing announcement',
  'reply-routing': 'Model routing reply',
  'relay-resume': 'Resume request',
  'reply-resume': 'Resume reply',
  human: 'Alex’s coverage request',
  mail: 'Retained review update',
  notice: 'Session restored notice',
}
// Exact screenshot prose; only scanner-flagged tokens receive equal-length substitutions.
// The fourth reply is transcribed from the approved clearer screenshot (9:49 reply).
export const turnBlocks: readonly TurnBlock[] = [
  { id: 'relay-hold', kind: 'Relay', sender: 'agent/worktree/manager', verified: true, at: '2026-10-07T05:48:00Z', text: 'Relayed from the coordinator (agent/worktree/manager, message fc1fc47f): worktree merge HOLD on hot files. From about 03:50Z for 45 min, until the laptop7 agent lands #4918 or the manager sends a release: do NOT merge worktree PRs that touch devenv.nix, flake.nix/flake.lock, apps.nix, the product-revs files/shards, or deployment spec.md. This includes ci/Hypermerge. Other merges are fine. The manager sends the release.' },
  { id: 'reply-hold', kind: 'Seat', sender: 'worktree/mutationlane', verified: true, at: '2026-10-07T05:50:00Z', text: "The merge hold doesn't affect me right now: I have no open PR. Two of the q4 options, disable-coverage and drop-pending-only, would change `devenv.nix`. If Michelle picks either, I won't merge that PR until the manager sends the release.\n\nq4 is still waiting for Michelle to choose disable-coverage, drop-pending-only, or defer-to–4102." },
  { id: 'relay-release', kind: 'Relay', sender: 'agent/worktree/manager', verified: true, at: '2026-10-07T06:30:00Z', text: 'Relayed from the coordinator (agent/worktree/manager, message 350797ae): #4918 landed (28500522). The worktree hot-file merge hold (devenv.nix, flake.nix/flake.lock, apps.nix, product-revs files/shards, deployment spec.md) is RELEASED. Merge normally.' },
  { id: 'reply-release', kind: 'Seat', sender: 'worktree/mutationlane', verified: true, at: '2026-10-07T06:32:00Z', text: "The hold on merging hot files is released. I have nothing waiting to merge, so this changes nothing for me.\n\nq4 is still waiting for Michelle. He needs to choose one of three options: disable-coverage, drop-pending-only, or defer-to–4102. Once he answers, I can merge the resulting PR as usual, even if it touches `devenv.nix`." },
  { id: 'relay-routing', kind: 'Relay', sender: 'agent/worktree/manager', verified: true, at: '2026-10-07T09:17:00Z', text: 'Coordinator relays Michelle q45 (quota-routing record chxpnb): until 2026-10-07 16:05Z, pass model "zai/glm-5.3" on subagent spawns for LOW-STAKES work only: research/collection, scouting, triage, summaries, drafts, log/CI digging, mechanical edits, test-gap lists, docs drift checks. Do NOT pin it for security review, final PR review, merges or deploys. OMP falls back automatically if GLM hits its 5h cap. Nothing to do after 16:05Z. No reply needed.' },
  { id: 'reply-routing', kind: 'Seat', sender: 'worktree/mutationlane', verified: true, at: '2026-10-07T09:19:00Z', text: "I have no sub-agents to start right now, so the GLM-5.3 routing changes nothing for me today. If I start low-stakes research or scouting before 16:05Z, I'll pass `zai/glm-5.3` on that spawn. Reviews and merges won't use it.\n\nThe `task` tool now has a per-spawn `model` field, which is what issue #2688 asked for. I'll check whether #2688 can be closed.\n\nq4 is still waiting for Michelle to choose disable-coverage, drop-pending-only, or defer-to–4102." },
  { id: 'relay-resume', kind: 'Relay', sender: 'agent/worktree/manager', verified: true, at: '2026-10-07T09:47:00Z', text: 'Coordinator, per Michelle: you are idle. The GLM note was informational only, and host also had an unplanned reset at 07:27Z that restarted your session. Resume your e2e goal NOW from your todo list and durable state (now.md / axe work). Do not wait for instructions and do not reply to this message. Push your current item to its next verifiable step: PR, gate, review request, deploy handoff. If you are truly blocked, record the blocker (axe decision or a message to the owner) and work on your next unblocked item. Report only real completions or hard blockers.' },
  { id: 'reply-resume', kind: 'Seat', sender: 'worktree/mutationlane', verified: true, at: '2026-10-07T09:49:00Z', text: "Before your correction arrived, the resume nudge led me to do two things:\n\n- **Re-filed decision:** I re-filed q4 as Q1 (record `137j9t`) in this seat's decision store. I didn't withdraw it because it is the same question as before, not a new one. Michelle can answer it or ignore it when he resumes this seat.\n- **Message:** I sent the manager a summary of that question (message `4517c7f8ccbd84e6`).\n\nNothing else changed. Everything needed to resume is in `tmp/mutation-lane-review/`." },
  { id: 'human', kind: 'Person', sender: 'Alex', verified: true, at: '2026-10-07T09:51:00Z', text: 'Keep pending-only coverage. Please include the keyboard path in the review.\n\n— Alex' },
  { id: 'mail', kind: 'Agent', sender: 'Workspace.Review', verified: false, subject: 'Refresh the retained review', at: '2026-10-07T09:52:00Z', text: 'The review bridge now keeps typed choices. Run `review refresh` once in the owning seat. It opens a typed card for each pending ask and cancels the old mirror only after the card exists. If your shell reports an older command, use the current seat binary. Report failures to the review owner.' },
  { id: 'notice', kind: 'System', sender: 'Session', verified: true, at: '2026-10-07T09:53:00Z', text: 'Session restored. Durable notes and retained messages are available.' },
]
export const replyItems: readonly (TextItem & { role: 'assistant' })[] = turnBlocks.filter(block => block.kind === 'Seat').map(block => ({ _tag: 'Text', role: 'assistant', id: block.id, text: block.text, attachments: [], streaming: false, at: block.at }))
export const workTurn: WorkLogTurn = { durationMs: 9000, running: false, failed: false, interrupted: false, commands: 1, calls: [{ id: 'review-read', kind: 'read', title: 'Read retained review', argsSummary: 'notes/mutation-review/', status: 'success', startedAt: '2026-10-07T09:50:00Z', endedAt: '2026-10-07T09:50:09Z', detail: 'The retained review contains the pending coverage choice and the keyboard path.' }] }
