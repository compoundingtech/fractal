// The person's availability as Now shows it, above the alerts as `fractal now` prints it: the
// effective status and where it comes from, the interruptions they accept, their deadline clock and
// their directive note, each with its end. Agents read all of it; only the person changes it, here
// by picking a value and then how long it holds.
import { ClientError, plainError, type AvailabilitySet, type Capabilities, type DirectiveNote, type PersonAvailability } from '../../clients/typescript/fractal-client';

export type AvailabilityRow = { key: 'status' | 'interruptions' | 'deadline_clock' | 'note'; title: string; context: string };

/** A member that serves availability lists its write scopes among its capabilities, granted or not. */
export function servesAvailability(caps: Capabilities | null): boolean {
  return caps?.capabilities.some(capability => capability.id === 'write.availability') ?? false;
}

/** The session person's own entries: a person's read answers only them, but the lists are lists. */
export function ownAvailability(people: readonly PersonAvailability[], notes: readonly DirectiveNote[], actor: string): { person: PersonAvailability | null; note: DirectiveNote | null } {
  return { person: people.find(entry => entry.person === actor) ?? null, note: notes.find(entry => entry.person === actor) ?? null };
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** A moment in local time as a person says it near now: `18:00`, `tomorrow 09:00`, `yesterday 16:00`, `Nov 20 09:00`. */
export function whenText(iso: string, now: Date): string {
  const at = new Date(iso);
  const clock = `${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')}`;
  const days = Math.round((new Date(at.getFullYear(), at.getMonth(), at.getDate()).getTime() - new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) / 86_400_000);
  if (days === 0) return clock;
  if (days === 1) return `tomorrow ${clock}`;
  if (days === -1) return `yesterday ${clock}`;
  return `${MONTHS[at.getMonth()]} ${at.getDate()} ${clock}`;
}

/** How long ago: `just now`, `12m ago`, `5h ago`. */
function agoText(iso: string, now: Date): string {
  const minutes = Math.max(0, Math.floor((now.getTime() - Date.parse(iso)) / 60_000));
  return minutes < 1 ? 'just now' : minutes < 60 ? `${minutes}m ago` : `${Math.floor(minutes / 60)}h ago`;
}

const until = (iso: string | null, now: Date) => iso ? `until ${whenText(iso, now)}` : '';
const joined = (...parts: string[]) => parts.filter(Boolean).join(' · ');

const INTERRUPTIONS = {
  all: ['Interruptions: all', 'Agents may ask you anything'],
  urgent: ['Interruptions: urgent only', 'Only asks due within an hour reach you'],
  none: ['Interruptions: none', 'No asks reach you'],
} as const;
const CLOCK = {
  run: ['Deadline clock: running', 'Ask deadlines run as set'],
  'hold-routine': ['Deadline clock: routine asks held', 'Urgent asks keep running; held time is added to the rest'],
  'hold-all': ['Deadline clock: every ask held', 'Held time is added to every deadline'],
} as const;

/** The four rows: status, interruptions, deadline clock and note. */
export function availabilityRows(availability: PersonAvailability, note: DirectiveNote | null, now: Date): AvailabilityRow[] {
  const presence = availability.presence && availability.presence !== 'unknown' && availability.presence_observed_at
    ? `${availability.presence}, reported ${agoText(availability.presence_observed_at, now)}`
    : 'no recent report';
  const [interruptions, interruptionsDetail] = INTERRUPTIONS[availability.interruptions];
  const [clock, clockDetail] = CLOCK[availability.deadline_clock];
  const held = availability.deadline_clock !== 'run';
  return [
    {
      key: 'status',
      title: availability.status === 'available' ? 'Available' : 'Away',
      context: availability.status_source === 'override' ? joined('Set by you', until(availability.status_until, now)) : `From your devices: ${presence}`,
    },
    { key: 'interruptions', title: interruptions, context: joined(interruptionsDetail, until(availability.interruptions_until, now)) },
    {
      key: 'deadline_clock',
      title: clock,
      context: joined(clockDetail, held && availability.deadline_clock_since ? `since ${whenText(availability.deadline_clock_since, now)}` : '', until(availability.deadline_clock_until, now)),
    },
    note
      ? { key: 'note', title: note.text, context: joined('Note for agents', note.expires_at ? until(note.expires_at, now) : 'until you clear it') }
      : { key: 'note', title: 'No note', context: 'A note tells agents what matters to you now' },
  ];
}

export type Axis = 'status' | 'interruptions' | 'deadline_clock';
/** Each axis's choices, as its action sheet lists them. */
export const AXES = {
  status: { title: 'Status', choices: [['Available', 'available'], ['Away', 'away'], ['Follow my devices', null]] },
  interruptions: { title: 'Interruptions', choices: [['All', 'all'], ['Urgent only', 'urgent'], ['None', 'none']] },
  deadline_clock: { title: 'Deadline clock', choices: [['Run', 'run'], ['Hold routine asks', 'hold-routine'], ['Hold every ask', 'hold-all']] },
} as const;
/** The value that clears an axis, so it takes no end: status follows the devices, every ask may interrupt, the clock runs. */
export const CLEARED = { status: null, interruptions: 'all', deadline_clock: 'run' } as const;

/** One axis's change. An absent `until` holds until the person changes it. */
export function axisChange(axis: Axis, value: string | null, until: Date | null): AvailabilitySet {
  return { [axis]: { value, ...(until ? { until: until.toISOString() } : {}) } } as AvailabilitySet;
}

export const UNTIL_PRESETS = ['30 minutes', '1 hour', '2 hours', 'Until tomorrow 9:00'] as const;

/** When a preset ends: 30 minutes, one or two hours from now, or 09:00 local time tomorrow. */
export function presetUntil(preset: typeof UNTIL_PRESETS[number], now: Date): Date {
  switch (preset) {
    case '30 minutes': return new Date(now.getTime() + 30 * 60_000);
    case '1 hour': return new Date(now.getTime() + 60 * 60_000);
    case '2 hours': return new Date(now.getTime() + 120 * 60_000);
    case 'Until tomorrow 9:00': return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9);
  }
}

export const CUSTOM_HINT = 'A time such as 17:30, or a span such as 45m, 3h or 1h30m.';

/** A typed end: a local clock time, the next one to come, or a span from now. Anything else is the hint. */
export function customUntil(text: string, now: Date): Date | string {
  const typed = text.trim().toLowerCase();
  const clock = /^(\d{1,2})[:.](\d{2})$/.exec(typed);
  if (clock) {
    const hours = Number(clock[1]), minutes = Number(clock[2]);
    if (hours > 23 || minutes > 59) return CUSTOM_HINT;
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate(), hours, minutes);
    return today > now ? today : new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, hours, minutes);
  }
  const span = /^(?:(\d+)\s*h)?\s*(?:(\d+)\s*m)?$/.exec(typed);
  const minutes = span ? Number(span[1] ?? 0) * 60 + Number(span[2] ?? 0) : 0;
  return minutes > 0 ? new Date(now.getTime() + minutes * 60_000) : CUSTOM_HINT;
}

export const REPAIR_FOR_AVAILABILITY = 'This phone’s pairing cannot change availability or the note. Re-pair it in Fleet › this connection › re-pair this device to change them here.';

/** A refused write as the person can act on it: a pairing without the write scope is re-paired. */
export function writeRefusal(error: unknown): string {
  if (!(error instanceof ClientError) || error.status !== 403) return plainError(error);
  return /not granted `write\./.test(error.response.message) ? REPAIR_FOR_AVAILABILITY : 'Only a person changes their own availability and note, and this connection is not signed in as one.';
}
