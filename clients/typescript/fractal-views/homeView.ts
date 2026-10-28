import type { Attention, TimelineEntry } from '@fractal/client';
import { ago } from './time.ts';

// Home, drawn the way the Fractal terminal UI draws it (crates/fractal-tui/src/ui/screens.rs home_list, adapt.rs
// attention): the requests and reviews Fractal holds for the person that are not resolved, grouped by
// how urgently someone is waiting. Fractal presents each item (`Attention.presentation`, see
// docs/clients/ui-contract.md, Attention row): its group, its card and the step it holds. A
// message stays in its conversation, and Fractal sends a fault to the agent that owns it, so Fractal puts
// neither on Now.

export const TIERS = ['stopped', 'today', 'later'] as const;
export type Tier = typeof TIERS[number];
export function tierTitle(tier: Tier): string {
  switch (tier) {
    case 'stopped': return 'somebody is stopped on you';
    case 'today': return 'today';
    case 'later': return 'when there is time';
  }
}

/** `unplaced`: an item Fractal did not place in a way this client knows (an older Fractal presents
 * nothing; a newer one may name a group or card this client does not know). It shows what Fractal
 * says, where Fractal's alert flag puts it, and the app offers only the answers Fractal lists. */
export type HomeKind = 'review' | 'launch' | 'revision' | 'request' | 'update' | 'prompt' | 'login' | 'unplaced';
/** Semantic colors; renderers resolve them through their own palette. */
export type HomeColor = 'person' | 'green';
export type HomeRow = {
  item: KeptAttention;
  tier: Tier;
  kind: HomeKind;
  glyph: string;
  color: HomeColor;
  title: string;
  /** Who or what is waiting, when Fractal says: `step build`. */
  waiting: string | null;
  age: string;
};

const HOME_KINDS: readonly string[] = ['review', 'launch', 'revision', 'request', 'update', 'prompt', 'login'];

/** An optional field as the plain model gives it (`T | null`) or as the web app's decoded model gives it (an Effect `Option`, which has a `_tag`). */
type OptionLike<T> = { readonly _tag: 'None' } | { readonly _tag: 'Some'; readonly value: T };
type Optional<T> = T | null | OptionLike<T>;
function plain<T>(value: Optional<T> | undefined): T | null {
  if (value === null || value === undefined) return null;
  if (typeof value === 'object' && '_tag' in value && (value._tag === 'None' || value._tag === 'Some')) return value._tag === 'Some' ? (value as { readonly value: T }).value : null;
  return value as T;
}
/** What Home reads of `Attention.presentation`, in either shape. */
type PresentationLike = { readonly kind: Optional<string>; readonly tier: Optional<string>; readonly waiting: Optional<{ readonly path: string }> };
type Kindish = { readonly attention_kind: string; readonly presentation?: PresentationLike | undefined; alert?: boolean; update?: unknown };
/**
 * Where Home shows an item, as Fractal presents it, or null for one Fractal puts on no Now (a null group
 * or card). A feedback gate is a review here. An item Fractal places in a way this client does not
 * know (an older Fractal presents nothing; a newer one may name a group or card this client does not
 * know) is unplaced, where Fractal's alert flag puts it, never dropped; Fractal puts neither a message nor
 * a fault on Now.
 */
export function homeKind(item: Kindish): { tier: Tier; kind: HomeKind } | null {
  const presented = item.presentation;
  const fallback: Tier = isAlert(item) ? 'stopped' : 'later';
  if (!presented) {
    const offNow = item.attention_kind === 'unread-message' || item.attention_kind === 'fault';
    return offNow ? null : { tier: fallback, kind: 'unplaced' };
  }
  const presentedTier = plain(presented.tier);
  const presentedKind = plain(presented.kind);
  if (presentedTier === null || presentedKind === null || presentedKind === 'fault') return null;
  const tier = TIERS.find(tier => tier === presentedTier) ?? fallback;
  const kind = presentedKind === 'feedback' ? 'review' : presentedKind;
  return { tier, kind: HOME_KINDS.includes(kind) ? kind as HomeKind : 'unplaced' };
}

export function kindGlyph(kind: HomeKind): { glyph: string; color: HomeColor } {
  return kind === 'update' ? { glyph: '✦', color: 'green' } : kind === 'unplaced' ? { glyph: '?', color: 'person' } : { glyph: '◆', color: 'person' };
}

export const HOME_LEGEND: ReadonlyArray<{ glyph: string; color: HomeColor; word: string }> = [
  { glyph: '◆', color: 'person', word: 'decide' },
  { glyph: '✦', color: 'green', word: 'to read' },
];

/** Whether Home shows an item: it is not resolved and, when Fractal names a person, it is for this one. */
/** The person a session acts for: a paired device acts as `person/NAME/session/ID`. */
export function sessionPerson(actor: string | undefined): string | undefined {
  return actor?.replace(/\/session\/.*$/, '');
}

export function onHome(item: Pick<Attention, 'state' | 'person_id'>, actor: string | undefined): boolean {
  const person = sessionPerson(actor);
  return item.state !== 'resolved' && (!person || !item.person_id || item.person_id === person);
}

/** An item Fractal stopped listing (or resolved) that stays on Home until the person clears it. */
export type KeptAttention = Attention & { closedElsewhere?: boolean };

/**
 * What stays on Home when Fractal's list changes: nothing leaves by itself (Nathan, 2026-10-07).
 * An item shown before that Fractal no longer lists as open, and that the person did not act on from
 * here, is kept, marked closed, with no actions, until the person clears it.
 */
export const RECENTLY_CLOSED = 5;

export function keepClosed(previous: readonly KeptAttention[], next: readonly Attention[], acted: ReadonlySet<string>, actor?: string): KeptAttention[] {
  const kept: KeptAttention[] = [...next];
  const open = new Set(next.filter(item => onHome(item, actor)).map(item => item.id));
  // Items kept before go first, so the newly closed sit after them and the oldest drops first.
  for (const old of [...previous.filter(item => item.closedElsewhere), ...previous.filter(item => !item.closedElsewhere)]) {
    if (open.has(old.id) || acted.has(old.id) || !onHome(old, actor)) continue;
    const marked: KeptAttention = old.closedElsewhere ? old : { ...old, closedElsewhere: true, actions: [] };
    const at = kept.findIndex(item => item.id === old.id);
    if (at >= 0) kept[at] = marked; else kept.push(marked);
  }
  // Home lists only the latest few closed items under "Recently closed".
  let over = kept.filter(item => item.closedElsewhere).length - RECENTLY_CLOSED;
  return over > 0 ? kept.filter(item => !(item.closedElsewhere && over-- > 0)) : kept;
}

export function homeRows(items: KeptAttention[], actor: string | undefined, now = Date.now()): HomeRow[] {
  const rows = items.filter(item => onHome(item, actor)).flatMap((item): HomeRow[] => {
    const place = homeKind(item);
    if (!place) return [];
    const step = plain(item.presentation?.waiting)?.path;
    const waiting = item.closedElsewhere ? 'closed; stays until you clear it'
      : !item.presentation ? 'Fractal does not say what this is: this host\'s Fractal is older, or has not presented it yet'
      : place.kind === 'unplaced' ? 'Fractal names a card this app does not know'
      : step ? `step ${step}` : null;
    return [{ item, ...place, ...kindGlyph(place.kind), title: cleanTitle(item.title), waiting, age: ago(item.requested_at, now) }];
  });
  // A stable sort by tier keeps Fractal's order within each tier; what Fractal closed sits last, apart.
  const rank = (row: HomeRow) => (row.item.closedElsewhere ? TIERS.length : 0) + TIERS.indexOf(row.tier);
  return rows.map((row, index) => ({ row, index })).sort((a, b) => rank(a.row) - rank(b.row) || a.index - b.index).map(({ row }) => row);
}

export type HomeSection = { tier: Tier; title: string; count: number; rows: HomeRow[]; closed?: boolean };
export function homeSections(rows: HomeRow[]): HomeSection[] {
  const sections: HomeSection[] = [];
  for (const row of rows) {
    const closed = !!row.item.closedElsewhere;
    const last = sections.at(-1);
    // Items Fractal closed while they were shown stay under a heading of their own, never mixed with
    // what needs the person (Nathan, 2026-10-07: "2 need you", 4 listed).
    if (closed) {
      if (last?.closed) { last.rows.push(row); last.count++; }
      else sections.push({ tier: row.tier, title: 'Recently closed: clear each', count: 1, rows: [row], closed: true });
    } else if (last?.tier === row.tier && !last.closed) { last.rows.push(row); last.count++; }
    else sections.push({ tier: row.tier, title: tierTitle(row.tier), count: 1, rows: [row] });
  }
  return sections;
}

function cleanTitle(title: string): string {
  return title.replace(/^\[PING\]\s*\?\s*/, '').replace(/\s*\[id:message\/[^\]]+\]\s*$/, '').trim() || title;
}

// Alerts: whatever blocks or waits on the person (an ask, a human gate, a launch or revision
// approval, a native harness prompt, a provider sign-in). An update asks nothing, and a message
// stays in its conversation, so neither is an alert. The person reads them in two places: the
// count (nothing at zero, `N alerts` otherwise) and the conversation of the agent each belongs to.

type Alertish = Pick<Attention, 'attention_kind'> & { alert?: boolean; update?: unknown };

/** Whether the item is an alert, as Fractal's CLI reads it. A daemon that predates alerts does not say, and then everything but an update or an unread message is one. */
export function isAlert(item: Alertish): boolean {
  return item.alert ?? (!item.update && item.attention_kind !== 'unread-message');
}

/** As much of an attention row as choosing alerts reads, so a client's decoded rows work too. */
export type AlertRow = Pick<Attention, 'state' | 'person_id' | 'attention_kind' | 'source_id'> & {
  alert?: boolean; update?: unknown; closedElsewhere?: boolean;
  conversation_id?: string; conversation_ids?: readonly string[]; requester_id?: string | null;
};

/**
 * The alerts the person can see: open, theirs, an alert, of a kind the app shows, and not an item
 * Fractal closed that only waits to be cleared. The same rows Home lists, so the count and the list agree.
 */
export function openAlerts<T extends AlertRow>(items: readonly T[], actor: string | undefined): T[] {
  return items.filter(item => !item.closedElsewhere && onHome(item, actor) && isAlert(item) && homeKind(item) !== null);
}

/** `3 alerts`, `1 alert`, and nothing at all for none. */
export function alertsHeading(count: number): string {
  return count <= 0 ? '' : count === 1 ? '1 alert' : `${count} alerts`;
}

/** The agent conversations an alert shows in, the one it belongs to first. */
export function alertConversations(item: Pick<Attention, 'source_id'> & { conversation_id?: string; conversation_ids?: readonly string[]; requester_id?: string | null }): string[] {
  const named = [...(item.conversation_id ? [item.conversation_id] : []), ...(item.conversation_ids ?? [])];
  // A daemon that predates alerts names no conversation: the agent that asked, else the seat itself.
  if (!named.length) named.push(...[item.requester_id, item.source_id].filter((id): id is string => !!id && id.startsWith('agent/')).slice(0, 1));
  return [...new Set(named)];
}

/** The alerts that show in one agent's conversation. */
export function alertsIn<T extends AlertRow>(items: readonly T[], agentId: string, actor: string | undefined): T[] {
  return openAlerts(items, actor).filter(item => alertConversations(item).includes(agentId));
}

/**
 * A native prompt's answers, as its alert offers them; null when the alert offers none. `allow`
 * or `deny` a permission, `approve` or `deny` a plan, `answer` (the choices of a question, see
 * `questionAnswer`) or `deny` a question, and a screen prompt's own answers (`use`, `use-all`,
 * `skip`, `trust`, `exit`, `quit`).
 */
export type PromptAnswers = {
  target_id: string; episode: string; answers: string[];
  /**
   * The exact call the prompt asks about (a tool use or Codex item id), or null when its harness
   * named none; undefined from a daemon that predates it.
   */
  call_id?: string | null;
};
export function promptAnswers(item: { readonly actions: readonly string[]; readonly source_id: string; readonly episode?: string; readonly action_parameters?: unknown }): PromptAnswers | null {
  if (!item.actions.includes('prompt.respond')) return null;
  const parameters = (item.action_parameters as Record<string, unknown> | undefined)?.['prompt.respond'] as { target_id?: unknown; episode?: unknown; answers?: unknown; call_id?: unknown } | undefined;
  if (!parameters) return null;
  const answers = (Array.isArray(parameters.answers) ? parameters.answers : []).filter((answer): answer is string => typeof answer === 'string' && /^[a-z][a-z-]*$/.test(answer));
  if (!answers.length) return null;
  const target_id = typeof parameters.target_id === 'string' ? parameters.target_id : item.source_id;
  const episode = typeof parameters.episode === 'string' ? parameters.episode : item.episode ?? '';
  if (!episode) return null;
  const prompt: PromptAnswers = { target_id, episode, answers };
  if ('call_id' in parameters) prompt.call_id = typeof parameters.call_id === 'string' ? parameters.call_id : null;
  return prompt;
}

/**
 * Which prompt an alert's answer controls belong to: the alert, the episode it asks about and its
 * revision. A client keys the person's choices on it, so a new prompt behind the same alert, or a
 * new revision of it, starts with nothing chosen: positions chosen for one question never answer
 * another.
 */
export function promptAlertKey(item: { readonly id: string; readonly revision: string; readonly actions: readonly string[]; readonly source_id: string; readonly episode?: string; readonly action_parameters?: unknown }): string {
  return [item.id, promptAnswers(item)?.episode ?? item.episode ?? '', item.revision].join('\n');
}

/** The call a harness recorded before it asked, in full: its tool and each input, lines intact. */
export type PendingCall = { id: string; tool: string; lines: string[]; input?: unknown };

function inputLines(args: unknown): string[] {
  let value = args;
  if (typeof args === 'string') { try { value = JSON.parse(args); } catch { return args.split('\n'); } }
  if (value === null || value === undefined) return [];
  if (typeof value !== 'object' || Array.isArray(value)) return JSON.stringify(value, null, 2).split('\n');
  const lines: string[] = [];
  for (const [key, field] of Object.entries(value as Record<string, unknown>)) {
    const text = typeof field === 'string' ? field : JSON.stringify(field, null, 2);
    const [first = '', ...rest] = text.split('\n');
    lines.push(`${key}: ${first}`, ...rest.map(line => `  ${line}`));
  }
  return lines;
}

function parsedInput(args: unknown): unknown {
  if (typeof args !== 'string') return args;
  try { return JSON.parse(args); } catch { return args; }
}

/**
 * Every tool call with no result yet, oldest first. A permission prompt asks about one of them,
 * but the prompt carries no call ID, so with more than one unanswered a client cannot say which.
 */
export function pendingCalls(entries: ReadonlyArray<Pick<TimelineEntry, 'type' | 'body' | 'id'>>): PendingCall[] {
  const answered = new Set<string>();
  for (const entry of entries) {
    if (entry.type === 'tool_result') { const call = (entry.body as { call_id?: unknown }).call_id; if (typeof call === 'string') answered.add(call); }
  }
  const open: PendingCall[] = [];
  for (const entry of entries) {
    if (entry.type !== 'tool_call') continue;
    const body = entry.body as { call_id?: unknown; name?: unknown; arguments?: unknown };
    const call = typeof body.call_id === 'string' ? body.call_id : entry.id;
    if (answered.has(call) || open.some(other => other.id === call)) continue;
    open.push({ id: call, tool: typeof body.name === 'string' && body.name ? body.name : 'tool', lines: inputLines(body.arguments), input: parsedInput(body.arguments) });
  }
  return open;
}

/** One question of a harness's pending `AskUserQuestion` call. */
export type PendingQuestion = { question: string; header: string; options: Array<{ label: string; description: string }>; multiSelect: boolean };

/** The questions of the one pending call, when it is an `AskUserQuestion` whose questions can be read. */
export function pendingQuestions(calls: readonly PendingCall[]): PendingQuestion[] | null {
  const [call] = calls;
  if (calls.length !== 1 || call?.tool !== 'AskUserQuestion') return null;
  const asked = (call.input as { questions?: unknown } | undefined)?.questions;
  if (!Array.isArray(asked) || !asked.length) return null;
  const questions: PendingQuestion[] = [];
  for (const each of asked) {
    const question = each as { question?: unknown; header?: unknown; options?: unknown; multiSelect?: unknown };
    if (typeof question.question !== 'string' || !Array.isArray(question.options) || !question.options.length) return null;
    const options = question.options.map(option => option as { label?: unknown; description?: unknown });
    if (!options.every(option => typeof option.label === 'string' && option.label)) return null;
    questions.push({
      question: question.question,
      header: typeof question.header === 'string' ? question.header : '',
      options: options.map(option => ({ label: option.label as string, description: typeof option.description === 'string' ? option.description : '' })),
      multiSelect: question.multiSelect === true,
    });
  }
  return questions;
}

/**
 * The answer a question alert sends once every question has a choice: positions only, each
 * question's position to the positions of the options chosen, such as {"0":[1],"1":[0,2]}. The
 * questions and labels stay in the conversation; the seat's hook turns positions into labels.
 * Null while a question has none.
 */
export function questionAnswer(questions: readonly PendingQuestion[], chosen: ReadonlyArray<ReadonlyArray<number>>): string | null {
  const answers: Record<string, number[]> = {};
  for (const [index, question] of questions.entries()) {
    const options = [...new Set(chosen[index] ?? [])].filter(option => option >= 0 && option < question.options.length).sort((a, b) => a - b);
    if (!options.length) return null;
    answers[String(index)] = options;
  }
  return JSON.stringify(answers);
}

/** How an answer a prompt offers reads on its button. */
export function answerLabel(answer: string): string {
  switch (answer) {
    case 'allow': return 'Allow';
    case 'deny': return 'Deny';
    case 'approve': return 'Approve plan';
    case 'answer': return 'Send';
    case 'use': return 'Use it';
    case 'use-all': return 'Use all';
    case 'skip': return 'Skip';
    case 'trust': return 'Trust';
    case 'exit': return 'Exit';
    case 'quit': return 'Quit';
    case 'approve-once': return 'Allow once';
    case 'approve-kind': return 'Allow this kind';
    case 'dismiss': return 'Dismiss';
    default: return answer;
  }
}

/**
 * The answers a client may offer. `allow` and `approve` only beside the exact call the prompt asks
 * about, shown in full (from an older daemon: the one call unanswered); `answer` only when that
 * call's questions can be shown and chosen from; every other answer (deny, and a screen prompt's
 * or denial's own answers) always.
 */
export function offeredAnswers(prompt: PromptAnswers, calls: readonly PendingCall[]): string[] {
  const shown = promptCalls(prompt, calls);
  const certain = shown.length === 1 && (shown[0]?.lines.length ?? 0) > 0;
  return prompt.answers.filter(answer => {
    if (answer === 'allow' || answer === 'approve') return certain;
    if (answer === 'answer') return pendingQuestions(shown) !== null;
    return true;
  });
}

/**
 * The calls to show beside a prompt's answers: the exact call it asks about, when its daemon names
 * it (none when it named none); from a daemon that predates that, every unanswered call, of which
 * only a lone one is certain.
 */
export function promptCalls(prompt: PromptAnswers, calls: readonly PendingCall[]): PendingCall[] {
  if (prompt.call_id === undefined) return [...calls];
  return calls.filter(call => call.id === prompt.call_id);
}
